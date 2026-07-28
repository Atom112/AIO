/// 本地推理引擎管理相关的 Tauri 命令：启动、停止、检查状态、扫描安装、拉取模型列表。
use crate::core::models::EngineInstallInfo;
use crate::core::state::LocalEngineState;
use crate::plugins::engine::EngineManager;
use crate::utils::file_parser::validate_model_path;
use serde::Serialize;
use tauri::{AppHandle, State};
use tokio::time::{sleep, Duration};

/// 启动本地大模型服务器
/// @param model_path 模型文件的绝对路径（H8 沙箱校验）
/// @param port 指定服务器运行的端口
/// @param gpu_layers 卸载到 GPU 的模型层数
/// @param engine_type 可选的引擎类型标识，不传时默认使用 llama_cpp（兼容旧配置）
#[tauri::command]
#[allow(
    clippy::too_many_arguments,
    reason = "Tauri IPC compatibility requires the existing flat command parameters"
)]
pub async fn start_local_server(
    app: AppHandle,
    state: State<'_, LocalEngineState>,
    engine_mgr: State<'_, EngineManager>,
    model_path: String,
    port: u16,
    gpu_layers: i32,
    engine_type: Option<String>,
    trust_remote_code: Option<bool>,
) -> Result<String, String> {
    let engine_id = engine_type.unwrap_or_else(|| "llama_cpp".to_string());
    let trust_remote = trust_remote_code.unwrap_or(false);

    let plugin = engine_mgr
        .get(&engine_id)
        .ok_or_else(|| format!("不支持的本地引擎: {}", engine_id))?;

    // H8 沙箱：拒绝 home/AppData 外的模型路径（Ollama 等外部引擎免检）
    let safe_path = if engine_id == "ollama" {
        std::path::PathBuf::from(&model_path)
    } else {
        validate_model_path(&model_path)?
    };
    // 启动前清理：停止所有其他类型的引擎（同时间只允许一个本地引擎运行）
    {
        let engines = state.lock();
        let other_ids: Vec<String> = engines
            .keys()
            .filter(|k| *k != &engine_id)
            .cloned()
            .collect();
        drop(engines);
        for id in other_ids {
            tracing::info!("[engine] 停止旧引擎 {} 以启动 {}", id, engine_id);
            stop_engine_internal(&state, &id);
        }
    }
    // 如果同类型引擎已在运行，也先停止（清理旧进程）
    if is_engine_running_internal(&state, &engine_id) {
        stop_engine_internal(&state, &engine_id);
        sleep(Duration::from_millis(500)).await;
    }
    // 调用插件启动
    let url = plugin
        .start(
            app,
            &state,
            safe_path.to_string_lossy().as_ref(),
            port,
            gpu_layers,
            trust_remote,
        )
        .await?;

    Ok(url)
}

/// 停止本地服务器
/// @param engine_type 可选的引擎类型。不传时停止所有运行中的引擎。
#[tauri::command]
pub async fn stop_local_server(
    state: State<'_, LocalEngineState>,
    engine_type: Option<String>,
) -> Result<(), String> {
    match engine_type {
        Some(id) => {
            stop_engine_internal(&state, &id);
        }
        None => {
            let mut engines = state.lock();
            for (_, mut inner) in engines.drain() {
                if let Some(mut child) = inner.child_process.take() {
                    tracing::debug!("[engine] 正在停止引擎...");
                    let _ = child.kill();
                }
            }
        }
    }
    Ok(())
}

/// 检查本地服务器是否正在运行
/// @param engine_type 可选的引擎类型。不传时返回是否有任一引擎在运行。
#[tauri::command]
pub fn is_local_server_running(
    state: State<'_, LocalEngineState>,
    engine_type: Option<String>,
) -> bool {
    match engine_type {
        Some(id) => is_engine_running_internal(&state, &id),
        None => {
            let mut engines = state.lock();
            // 先清理已退出的进程，再判断是否有存活进程
            engines.retain(|_, inner| {
                if let Some(child) = inner.child_process.as_mut() {
                    match child.try_wait() {
                        Ok(None) => true, // still running
                        _ => false,
                    }
                } else {
                    false
                }
            });
            !engines.is_empty()
        }
    }
}

/// 扫描系统上已安装的推理引擎
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EngineScanResult {
    pub id: String,
    pub name: String,
    pub supported: bool,
    pub installed: bool,
    pub version: Option<String>,
    pub default_port: u16,
    pub default_api_url: String,
}

#[tauri::command]
pub fn scan_installed_engines(
    app: AppHandle,
    engine_mgr: State<'_, EngineManager>,
) -> Result<Vec<EngineScanResult>, String> {
    let plugins = engine_mgr.all_plugins();
    let mut results = Vec::with_capacity(plugins.len());

    for plugin in plugins {
        let info: EngineInstallInfo = plugin.detect_installation(&app);
        let port = plugin.default_port();

        // Ollama uses root URL (no /v1); llama.cpp / vLLM use /v1
        let default_api_url = if plugin.identifier() == "ollama" {
            format!("http://localhost:{}", port)
        } else {
            format!("http://localhost:{}/v1", port)
        };

        results.push(EngineScanResult {
            id: plugin.identifier().to_string(),
            name: plugin.name().to_string(),
            supported: plugin.is_platform_supported(),
            installed: info.installed,
            version: info.version,
            default_port: port,
            default_api_url,
        });
    }

    Ok(results)
}

/// 从运行中的引擎拉取模型列表
/// @param api_url 引擎的 API URL（如 http://localhost:11434）
/// @param engine_type 引擎类型标识
#[tauri::command]
pub async fn list_engine_models(
    api_url: String,
    engine_type: String,
) -> Result<Vec<String>, String> {
    let base = api_url.trim_end_matches('/').to_string();

    let models_url = if engine_type == "ollama" {
        format!("{}/api/tags", base)
    } else {
        format!("{}/models", base)
    };

    let client = reqwest::Client::builder()
        .user_agent("AIO-Desktop/0.4")
        .connect_timeout(std::time::Duration::from_secs(3))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let resp = client
        .get(&models_url)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {}", e))?;

    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("响应非 JSON: {}", e))?;

    if engine_type == "ollama" {
        // Ollama: { "models": [ { "name": "llama3.2:3b", ... }, ... ] }
        let models = v["models"]
            .as_array()
            .ok_or("Unexpected Ollama response format")?;
        Ok(models
            .iter()
            .filter_map(|m| m["name"].as_str().map(|s| s.to_string()))
            .collect())
    } else {
        // OpenAI-compat: { "data": [ { "id": "model-name", ... }, ... ] }
        let data = v["data"]
            .as_array()
            .ok_or("Unexpected OpenAI-compat response format")?;
        Ok(data
            .iter()
            .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
            .collect())
    }
}

// ─── Internal helpers ───

fn is_engine_running_internal(state: &LocalEngineState, engine_type: &str) -> bool {
    let mut engines = state.lock();
    if let Some(inner) = engines.get_mut(engine_type) {
        if let Some(child) = inner.child_process.as_mut() {
            match child.try_wait() {
                Ok(None) => return true, // still running
                _ => {
                    inner.child_process = None;
                }
            }
        }
    }
    false
}

fn stop_engine_internal(state: &LocalEngineState, engine_type: &str) {
    let mut engines = state.lock();
    if let Some(mut inner) = engines.remove(engine_type) {
        if let Some(mut child) = inner.child_process.take() {
            tracing::debug!("[engine] 正在停止引擎 {}...", engine_type);
            let _ = child.kill();
        }
    }
}

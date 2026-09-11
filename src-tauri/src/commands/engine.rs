//! 本地推理引擎管理相关的 Tauri 命令：启动、停止、检查状态、扫描安装、拉取模型列表。
//!
//! 引擎差异（可执行文件、端口、启动参数）由 `plugins/engine` 的
//! `LocalEnginePlugin` 提供；本模块仅做编排与 IPC。
//!
//! 注意：代码中不得硬编码引擎名——引擎标识一律取自 `ActivatedModel.engine_type`
//! 或插件的 `identifier()`（AGENTS.md 约定）。

use crate::core::models::EngineInstallInfo;
use crate::core::state::LocalEngineState;
use crate::plugins::engine::EngineManager;
use crate::utils::file_parser::validate_model_path;
use serde::Serialize;
use tauri::{AppHandle, State};
use tokio::time::{sleep, Duration};

/// 旧前端调用未带 engine_type 时使用的缺省引擎标识（默认本地引擎 = llama.cpp 插件）。
/// 常量集中一次，避免字面量散落在命令体内。
const DEFAULT_ENGINE_ID: &str = "llama_cpp";

/// 启动本地大模型服务器。
///
/// # 参数
/// - `model_path` — 模型文件绝对路径（经 `validate_model_path` 沙箱校验）
/// - `port` — 监听端口
/// - `gpu_layers` — 卸载到 GPU 的层数
/// - `engine_type` — 引擎标识；缺省为 `llama_cpp`（兼容旧配置）
/// - `trust_remote_code` — 仅 vLLM 使用：是否允许执行模型仓库中的自定义代码
///   （前端会先二次确认）
///
/// # 返回
/// 启动成功后的引擎状态描述；同一时刻只允许一个本地引擎运行。
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
    let engine_id = engine_type.unwrap_or_else(|| DEFAULT_ENGINE_ID.to_string());
    let trust_remote = trust_remote_code.unwrap_or(false);

    let plugin = engine_mgr
        .get(&engine_id)
        .ok_or_else(|| format!("不支持的本地引擎: {}", engine_id))?;

    // 沙箱：拒绝 home/AppData 外的模型路径（外部引擎免检——其模型由引擎自身管理）
    let safe_path = if plugin.is_external() {
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

/// 扫描所有本地推理引擎的安装状态（应用启动时调用一次，结果缓存在前端）。
///
/// # 参数
/// - `engine_mgr` — 引擎插件管理器（注册顺序决定返回顺序）
///
/// # 行为
/// 逐个调用插件的 `detect_installation` 与 `default_port`，聚合为
/// `EngineScanResult` 列表；单个插件探测失败不影响其他引擎，仅记录为空信息。
#[tauri::command(async)]
pub fn scan_installed_engines(
    app: AppHandle,
    engine_mgr: State<'_, EngineManager>,
) -> Result<Vec<EngineScanResult>, String> {
    let plugins = engine_mgr.all_plugins();
    let mut results = Vec::with_capacity(plugins.len());

    for plugin in plugins {
        let info: EngineInstallInfo = plugin.detect_installation(&app);
        let port = plugin.default_port();

        // URL 形态由插件声明（Ollama 根 URL，OpenAI 兼容引擎挂 /v1）
        let default_api_url = plugin.default_api_url(port);

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

/// 从运行中的引擎拉取模型列表。
///
/// # 参数
/// - `api_url` — 引擎的 API URL（如 http://localhost:11434）
/// - `engine_type` — 引擎类型标识（用于选择插件：URL 形态与响应解析由插件声明，
///   不再在此硬编码引擎名）
///
/// # 返回
/// 模型 id 列表；HTTP 或解析失败返回错误串。
#[tauri::command]
pub async fn list_engine_models(
    engine_mgr: State<'_, EngineManager>,
    api_url: String,
    engine_type: String,
) -> Result<Vec<String>, String> {
    let plugin = engine_mgr
        .get(&engine_type)
        .ok_or_else(|| format!("不支持的引擎类型: {}", engine_type))?;

    // 端点形态由插件声明（Ollama=/api/tags，OpenAI 兼容=/models）
    let models_url = plugin.models_url(&api_url);

    let client = reqwest::Client::builder()
        .user_agent(format!("AIO-Desktop/{}", env!("CARGO_PKG_VERSION")))
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

    // 响应解析由插件声明（Ollama 的 {models:[{name}]} vs OpenAI 的 {data:[{id}]}）
    Ok(plugin.parse_model_ids(&v))
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

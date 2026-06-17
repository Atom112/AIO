/// 本地推理引擎管理相关的 Tauri 命令：启动、停止、检查状态以及引擎安装管理。

use crate::core::state::LocalEngineState;
use crate::plugins::engine::installer::{EngineInstaller, EngineStatus, EngineUpdateInfo};
use crate::plugins::engine::EngineManager;
use tauri::{AppHandle, Emitter, State};
use tokio::time::{sleep, Duration};

/// 启动本地大模型服务器
/// @param model_path 模型文件的绝对路径
/// @param port 指定服务器运行的端口
/// @param gpu_layers 卸载到 GPU 的模型层数
/// @param engine_type 可选的引擎类型标识，不传时默认使用 llama_cpp（兼容旧配置）
#[tauri::command]
pub async fn start_local_server(
    app: AppHandle,
    state: State<'_, LocalEngineState>,
    engine_mgr: State<'_, EngineManager>,
    model_path: String,
    port: u16,
    gpu_layers: i32,
    engine_type: Option<String>,
) -> Result<String, String> {
    let engine_id = engine_type.unwrap_or_else(|| "llama_cpp".to_string());

    let plugin = engine_mgr
        .get(&engine_id)
        .ok_or_else(|| format!("不支持的本地引擎: {}", engine_id))?;

    // 启动前清理：如果已经有一个正在运行的服务器，先关闭它
    stop_local_server(state.clone()).await?;
    sleep(Duration::from_millis(500)).await;

    // 调用插件启动
    let url = plugin.start(app, &state, &model_path, port, gpu_layers).await?;

    Ok(url)
}

/// 停止本地服务器
#[tauri::command]
pub async fn stop_local_server(state: State<'_, LocalEngineState>) -> Result<(), String> {
    let mut proc_lock = state.child_process.lock().unwrap();
    if let Some(mut child) = proc_lock.take() {
        println!("[DEBUG] 正在停止本地服务器...");
        let _ = child.kill();
    }

    let mut type_lock = state.engine_type.lock().unwrap();
    *type_lock = String::new();

    Ok(())
}

/// 检查本地服务器是否正在运行
#[tauri::command]
pub fn is_local_server_running(state: State<'_, LocalEngineState>) -> bool {
    let mut proc_lock = state.child_process.lock().unwrap();
    if let Some(child) = proc_lock.as_mut() {
        match child.try_wait() {
            Ok(None) => return true,
            _ => return false,
        }
    }
    false
}

/// 获取所有引擎的安装状态
#[tauri::command]
pub async fn get_engines_status(app: AppHandle) -> Result<Vec<EngineStatus>, String> {
    let mut statuses = Vec::new();

    // llama.cpp 状态
    let installed = EngineInstaller::is_installed(&app);
    let version = EngineInstaller::get_installed_version(&app);
    let latest = EngineInstaller::fetch_latest_release()
        .await
        .map(|r| r.tag_name)
        .ok();
    statuses.push(EngineStatus {
        id: "llama_cpp".into(),
        name: "llama.cpp".into(),
        installed,
        version,
        latest_version: latest,
        platform_supported: true,
        error: None,
    });

    // vLLM 状态（Windows 上不可用）
    #[cfg(target_os = "windows")]
    statuses.push(EngineStatus {
        id: "vllm".into(),
        name: "vLLM".into(),
        installed: false,
        version: None,
        latest_version: None,
        platform_supported: false,
        error: Some("vLLM 不支持 Windows 平台".into()),
    });

    #[cfg(not(target_os = "windows"))]
    {
        use crate::plugins::engine::vllm::VllmPlugin;
        use crate::plugins::engine::LocalEnginePlugin;
        let vllm = VllmPlugin;
        statuses.push(EngineStatus {
            id: "vllm".into(),
            name: "vLLM".into(),
            installed: vllm.is_installed(&app),
            version: None,
            latest_version: None,
            platform_supported: vllm.is_platform_supported(),
            error: None,
        });
    }

    Ok(statuses)
}

/// 安装/更新 llama.cpp 引擎（后台任务，通过 Tauri Event 发射进度）
#[tauri::command]
pub async fn install_engine(app: AppHandle) -> Result<String, String> {
    let app_clone = app.clone();
    let progress = move |p: f64| {
        let _ = app_clone.emit("engine-install-progress", p);
    };
    EngineInstaller::install(&app, progress).await
}

/// 检查 llama.cpp 是否有更新
#[tauri::command]
pub async fn check_llama_update(app: AppHandle) -> Result<EngineUpdateInfo, String> {
    EngineInstaller::check_update(&app).await
}

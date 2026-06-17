/// 本地推理引擎管理相关的 Tauri 命令：启动、停止和检查本地大模型服务器的状态。

use crate::core::state::LocalEngineState;
use crate::plugins::engine::EngineManager;
use tauri::{AppHandle, State};
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

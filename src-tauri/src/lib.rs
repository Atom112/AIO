//! # 核心库模块
//!
//! 本模块是整个 Tauri 应用程序后端的入口点。主要负责：
//! 1. 初始化并管理全局共享状态（如流式会话和本地进程）。
//! 2. 注册暴露给前端调用的 Rust 命令（Commands）。
//! 3. 监听程序窗口事件以执行清理任务（如关闭本地服务器进程）。

mod commands;
mod models;
mod utils;

use crate::utils::process_file_content;
use dashmap::DashMap;
use std::sync::{Arc, Mutex};
use tauri::Manager;
use tokio::task::JoinHandle;

// --- 状态定义 ---

/// 流式任务管理器
///
/// 用于跟踪所有活跃的 LLM 生成任务。
/// 使用 `DashMap` (分段加锁的哈希表) 以支持跨线程并发安全地访问任务句柄。
/// 键通常是对话的 ID，值是对应的异步任务句柄 `JoinHandle`。
pub struct StreamManager(pub Arc<DashMap<String, JoinHandle<()>>>);

/// 本地 Llama 服务状态
///
/// 存储本地运行的模型服务进程信息。
/// 使用 `Mutex` 确保在启动和停止服务时对子进程句柄的独占访问。
pub struct LocalLlamaState {
    /// 存储子进程句柄。如果服务未运行，则为 `None`。
    pub child_process: Mutex<Option<std::process::Child>>,
}

/// 应用程序启动入口
///
/// 该函数由 `main.rs` 调用，配置并运行 Tauri 运行时环境。
///
/// # 功能包括：
/// * **状态管理**: 注入 `StreamManager` 和 `LocalLlamaState` 供后端各处使用。
/// * **命令注册**: 将 `commands` 模块中定义的所有异步函数注册到前端。
/// * **事件监听**: 监听窗口销毁事件，确保在应用关闭时强行杀死残留的本地模型服务进程。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 注入全局状态，前端命令可以通过 State<'_, T> 获取
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(StreamManager(Arc::new(DashMap::new())))
        .manage(LocalLlamaState {
            child_process: Mutex::new(None),
        })
        // 注册前端唤起接口 (Invoke Handlers)
        .invoke_handler(tauri::generate_handler![
            // 配置相关命令
            commands::config::load_assistants,
            commands::config::save_assistant,
            commands::config::delete_assistant,
            commands::config::save_app_config,
            commands::config::load_app_config,
            commands::config::save_activated_models,
            commands::config::load_activated_models,
            commands::config::save_fetched_models,
            commands::config::load_fetched_models,
            // LLM 交互相关命令
            commands::llm::call_llm_stream,
            commands::llm::stop_llm_stream,
            commands::llm::fetch_models,
            // 本地服务器控制相关命令
            commands::server::start_local_server,
            commands::server::stop_local_server,
            commands::server::is_local_server_running,
            // 文件解析工具命令
            process_file_content,
            commands::config::upload_avatar,
            commands::llm::summarize_history,
            commands::auth::login_to_backend,
            commands::auth::register_to_backend
        ])
        // 注册窗口事件回调
        .on_window_event(|window, event| {
            // 当窗口被销毁（应用准备退出）时触发
            if let tauri::WindowEvent::Destroyed = event {
                let state = window.state::<LocalLlamaState>();

                // 从全局状态中取出子进程句柄
                let child_opt = {
                    let mut guard = state.child_process.lock().unwrap();
                    guard.take() // 将 Option 设置为 None 并取出原有值
                };

                // 如果本地服务器正在运行，强制退出该子进程，防止资源泄漏
                if let Some(mut child) = child_opt {
                    let _ = child.kill();
                }
            }
        })
        // 生成上下文并正式运行
        .run(tauri::generate_context!())
        .expect("运行 tauri 应用程序时发生错误");
}

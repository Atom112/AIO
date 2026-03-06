mod commands;
mod models;
mod utils;
mod db;

use crate::utils::process_file_content;
use dashmap::DashMap;
use std::sync::{Arc, Mutex};
use tauri::Manager;
use tokio::task::JoinHandle;

pub struct StreamManager(pub Arc<DashMap<String, JoinHandle<()>>>);
pub struct DbState(pub std::sync::Mutex<rusqlite::Connection>);
pub struct LocalLlamaState {
    pub child_process: Mutex<Option<std::process::Child>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let conn = db::init_db(app.handle())?;
            app.manage(DbState(std::sync::Mutex::new(conn)));
            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .manage(StreamManager(Arc::new(DashMap::new())))
        .manage(LocalLlamaState {
            child_process: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            commands::config::load_assistants,
            commands::config::save_assistant,
            commands::config::delete_assistant,
            commands::config::save_app_config,
            commands::config::load_app_config,
            commands::config::save_activated_models,
            commands::config::load_activated_models,
            commands::config::save_fetched_models,
            commands::config::load_fetched_models,
            commands::config::upload_avatar,
            commands::config::clear_local_avatar_cache,
            commands::llm::call_llm_stream,
            commands::llm::stop_llm_stream,
            commands::llm::fetch_models,
            commands::llm::summarize_history,
            commands::llm::append_message,
            commands::server::start_local_server,
            commands::server::stop_local_server,
            commands::server::is_local_server_running,
            commands::auth::login_to_backend,
            commands::auth::register_to_backend,
            commands::auth::validate_token,
            commands::auth::sync_avatar_to_backend,
            process_file_content,
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

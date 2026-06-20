//! # 核心库模块
//!
//! 本模块是整个 Tauri 应用程序后端的入口点。主要负责：
//! 1. 初始化并管理全局共享状态（如流式会话和本地引擎进程）。
//! 2. 注册暴露给前端调用的 Rust 命令（Commands）。
//! 3. 监听程序窗口事件以执行清理任务（如关闭本地引擎进程）。

mod cloud_backend;
mod commands;
mod core;
mod plugins;
mod utils;

use std::sync::Arc;
use crate::utils::process_file_content;
use crate::core::state::{DbState, LocalEngineState, StreamManager};
use crate::plugins::engine::EngineManager;
use tauri::Manager;
use tracing_subscriber::EnvFilter;

/// 初始化 tracing（生产默认 warn，调试可通过 RUST_LOG=info 开启）
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn,info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

/// 应用程序启动入口
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();
    tauri::Builder::default()
        .setup(|app| {
            let conn = core::db::init_db(app.handle())?;
            app.manage(DbState(std::sync::Mutex::new(conn)));
            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(StreamManager(Arc::new(dashmap::DashMap::new())))
        .manage(LocalEngineState::new())
        .manage(EngineManager::new())
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
            commands::llm::call_llm_stream,
            commands::llm::stop_llm_stream,
            commands::llm::fetch_models,
            commands::engine::start_local_server,
            commands::engine::stop_local_server,
            commands::engine::is_local_server_running,
            commands::engine::get_engines_status,
            commands::engine::install_engine,
            commands::engine::check_llama_update,
            process_file_content,
            commands::config::upload_avatar,
            commands::llm::summarize_history,
            commands::llm::append_message,
            commands::llm::generate_topic_title,
            // 云端后端鉴权（集中在 cloud_backend 模块）
            cloud_backend::auth::login_to_backend,
            cloud_backend::auth::register_to_backend,
            cloud_backend::auth::validate_token,
            cloud_backend::auth::sync_avatar_to_backend,
            cloud_backend::auth::logout_clear,
            cloud_backend::auth::read_auth_token,
            commands::config::clear_local_avatar_cache,
            commands::config::read_avatar_source,
            commands::update::check_app_update,
            commands::update::install_app_update,
            commands::update::restart_app,
            commands::update::get_updater_endpoint,
            commands::catalog::load_models_catalog,
            commands::catalog::load_models_catalog_full,
            commands::catalog::update_models_catalog,
            commands::catalog::get_catalog_url,
            commands::provider_config::load_provider_configs,
            commands::provider_config::save_provider_configs,
            commands::provider_config::test_provider_connection,
            commands::provider_config::fetch_provider_models,
            commands::provider_config::read_provider_api_key,
            commands::provider_config::delete_provider_api_key,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                let state = window.state::<LocalEngineState>();
                let child_opt = {
                    let mut inner = state.lock();
                    inner.child_process.take()
                };
                if let Some(mut child) = child_opt {
                    let _ = child.kill();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("运行 tauri 应用程序时发生错误");
}

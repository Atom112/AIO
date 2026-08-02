//! # 核心库模块
//!
//! 本模块是整个 Tauri 应用程序后端的入口点。主要负责：
//! 1. 初始化并管理全局共享状态（如流式会话和本地引擎进程）。
//! 2. 注册暴露给前端调用的 Rust 命令（Commands）。
//! 3. 监听程序窗口事件以执行清理任务（如关闭本地引擎进程）。

mod commands;
mod core;
pub mod mcp_fs_server;
mod plugins;
mod utils;

use crate::core::state::{DbState, LocalEngineState, StreamManager, SubagentHandles};
use crate::plugins::engine::EngineManager;
use crate::plugins::lsp::LspManager;
use crate::plugins::mcp::{McpRequestManager, McpServerManager, McpServerState, PendingApprovals};
use std::sync::Arc;
use tauri::Manager;
use tracing_subscriber::EnvFilter;

/// 初始化 tracing（生产默认 warn，调试可通过 RUST_LOG=info 开启）
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn,info"));
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
            let window = app.get_webview_window("main").unwrap();
            let _ = window.set_background_color(Some(tauri::webview::Color(10, 14, 26, 255)));
            let conn = core::db::init_db(app.handle())?;
            app.manage(DbState(parking_lot::Mutex::new(conn)));
            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(StreamManager(Arc::new(dashmap::DashMap::new())))
        .manage(LocalEngineState::new())
        .manage(EngineManager::new())
        .manage(McpServerManager::builtin())
        .manage(LspManager::new())
        .manage(McpServerState::default())
        .manage(McpRequestManager::new())
        .manage(PendingApprovals::new())
        .manage(SubagentHandles::new())
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
            commands::attachment::store_chat_attachment,
            commands::attachment::discard_chat_attachment,
            commands::export::save_binary_file,
            commands::export::copy_stored_image,
            commands::llm::call_llm_stream,
            commands::image_gen::generate_image,
            commands::llm::stop_llm_stream,
            commands::llm::run_agent_turn,
            commands::llm::fetch_models,
            commands::engine::start_local_server,
            commands::engine::stop_local_server,
            commands::engine::is_local_server_running,
            commands::engine::scan_installed_engines,
            commands::engine::list_engine_models,
            commands::attachment::process_file_content,
            commands::config::upload_avatar,
            commands::llm::summarize_history,
            commands::llm::append_message,
            commands::llm::btw::ask_btw_question,
            commands::llm::delete_topic_message,
            commands::llm::generate_topic_title,
            commands::llm::get_usage_summary,
            commands::llm::get_usage_summary_by_model,
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
            commands::provider_config::probe_engine_health,
            commands::provider_config::fetch_provider_models,
            commands::provider_config::read_provider_api_key,
            commands::provider_config::delete_provider_api_key,
            // 项目管理
            commands::project::create_project,
            commands::project::list_projects,
            commands::project::update_project,
            commands::project::delete_project,
            commands::project::open_project_directory,
            commands::project::get_project_by_path,
            commands::project::validate_project_path,
            commands::git::get_git_branch,
            commands::git::list_git_branches,
            commands::git::switch_git_branch,
            commands::git::revert_file_change,
            commands::git::revert_file_changes_batch,
            // Skill 管理
            commands::skill::list_skills,
            commands::skill::save_skill,
            commands::skill::delete_skill,
            commands::skill::list_skill_market_categories,
            commands::skill::list_skill_market,
            commands::skill::download_market_skill,
            commands::skill::discover_npx_skills,
            commands::skill::import_npx_skill,
            commands::skill::refresh_npx_skill,
            // MCP 服务器管理
            commands::mcp::list_mcp_servers,
            commands::mcp::add_mcp_server,
            commands::mcp::save_mcp_server_secret,
            commands::mcp::remove_mcp_server,
            commands::mcp::start_mcp_server,
            commands::mcp::stop_mcp_server,
            commands::mcp::list_mcp_server_status,
            commands::mcp::list_mcp_tools,
            commands::mcp::list_mcp_tools_for_assistant,
            commands::mcp::call_mcp_tool,
            commands::mcp::test_mcp_server_connection,
            commands::mcp::check_tool_permission,
            commands::mcp::respond_tool_approval,
            commands::mcp::list_mcp_transports,
            commands::mcp::list_mcp_resources,
            commands::mcp::read_mcp_resource,
            commands::mcp::list_mcp_prompts,
            commands::mcp::get_mcp_prompt,
            commands::mcp_catalog::list_mcp_catalog,
            commands::mcp_catalog::check_mcp_catalog_runtime,
            commands::mcp_catalog::install_mcp_catalog_server,
            // LSP 语言服务器管理
            commands::lsp::start_lsp_server,
            commands::lsp::stop_lsp_server,
            commands::lsp::get_diagnostics,
            commands::lsp::auto_detect_ls,
            commands::lsp::list_supported_languages,
            commands::lsp::stop_all_lsp_servers,
            // Per-Profile 模型覆盖
            commands::config::load_profile_model_overrides,
            commands::config::save_profile_model_overrides,
            commands::config::list_custom_subagent_profiles,
            commands::config::save_custom_subagent_profile,
            // 自定义子智能体配置文件
            commands::config::delete_custom_subagent_profile,
            // 系统自启
            commands::config::set_auto_start,
            commands::config::is_auto_start_enabled,
            // 会话分支
            commands::config::branch_topic,
            // Token 计数
            commands::llm::count_tokens_cmd,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                // 清理本地引擎子进程（所有引擎）
                let state = window.state::<LocalEngineState>();
                {
                    let mut engines = state.lock();
                    for (_, mut inner) in engines.drain() {
                        if let Some(mut child) = inner.child_process.take() {
                            let _ = child.kill();
                        }
                    }
                }
                // 清理活跃 LLM 流任务：cancel 所有 token（任务自身负责 emit done + 移除）
                let stream_mgr = window.state::<StreamManager>();
                for entry in stream_mgr.0.iter() {
                    entry.value().1.cancel();
                }
                stream_mgr.0.clear();
                // 清理 MCP 状态（在途调用 abort + 连接池清空）
                let req_mgr = window.state::<McpRequestManager>();
                req_mgr.abort_all();
                let mcp_state = window.state::<McpServerState>();
                mcp_state.lock().clear();
            }
        })
        .run(tauri::generate_context!())
        .expect("运行 tauri 应用程序时发生错误");
}

//! # Core library
//!
//! Entry point of the whole Tauri app backend. Main responsibilities:
//! 1. Initialize and manage global shared state (e.g. streaming sessions and local engine processes).
//! 2. Register the Rust commands exposed to the frontend.
//! 3. Listen for window events to run cleanup tasks (e.g. shutting down local engine processes).

mod commands;
mod core;
pub mod mcp_fs_server;
mod plugins;
mod services;
mod utils;

use crate::core::state::{
    DbState, LocalEngineState, StreamManager, SubagentCompletions, SubagentHandles,
};
use crate::plugins::engine::EngineManager;
use crate::plugins::lsp::LspManager;
use crate::plugins::mcp::{McpRequestManager, McpServerManager, McpServerState, PendingApprovals};
use crate::services::memory::MemoryStoreManager;
use std::sync::Arc;
use tauri::Manager;
use tracing_subscriber::EnvFilter;

/// Initialize tracing (warn by default in production; enable debug output via RUST_LOG=info).
///
/// Note: do not write "warn,info" - in EnvFilter the later directive overrides the global
/// directive at the same granularity, so the effective level becomes INFO and hot-path per-message logs land in production logs.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

/// Application startup entry point
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();
    tauri::Builder::default()
        .setup(|app| {
            // The main window is created in code: transparent only on Windows/macOS (rounded panel corners show the desktop);
            // on Linux/WebKitGTK transparency requires whole-window ARGB compositing with a heavy cost, so keep an opaque square-cornered window.
            #[cfg(not(target_os = "linux"))]
            let transparent = true;
            #[cfg(target_os = "linux")]
            let transparent = false;
            let window = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("AIO")
            .inner_size(1600.0, 900.0)
            .min_inner_size(960.0, 540.0)
            .decorations(false)
            .transparent(transparent)
            .shadow(false)
            .build()?;
            // Webview base color: transparent on non-Linux (desktop shows through outside the rounded corners); opaque on Linux to avoid compositing overhead
            #[cfg(not(target_os = "linux"))]
            let _ = window.set_background_color(Some(tauri::webview::Color(0, 0, 0, 0)));
            #[cfg(target_os = "linux")]
            let _ = window.set_background_color(Some(tauri::webview::Color(10, 14, 26, 255)));
            let conn = core::db::init_db(app.handle())?;
            app.manage(DbState(parking_lot::Mutex::new(conn)));
            // Local model download/load needs the app data dir (injected process-wide for static load paths without an AppHandle)
            if let Ok(dir) = app.path().app_data_dir() {
                crate::plugins::rerank::local_onnx::init_app_data_dir(dir);
            }
            // Daily background consolidation (sleep-time consolidation): first run 2 minutes after startup, then every 24h;
            // per project, fully async in the background, never blocking the main interaction
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(120)).await;
                    loop {
                        crate::services::memory::consolidate::run_daily_consolidation(&handle)
                            .await;
                        tokio::time::sleep(std::time::Duration::from_secs(24 * 3600)).await;
                    }
                });
            }
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
        .manage(SubagentCompletions::new())
        .manage(core::file_locks::FileLockRegistry::new())
        .manage(core::state::AgentTaskBoard::new())
        .manage(core::state::PendingAgentQuestions::new())
        .manage(MemoryStoreManager::new())
        .invoke_handler(tauri::generate_handler![
            commands::config::load_assistants,
            commands::config::save_assistant,
            commands::config::delete_assistant,
            commands::config::set_locale,
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
            commands::llm::stop_subagent,
            commands::llm::checkpoints::get_active_checkpoint,
            commands::llm::fetch_models,
            commands::engine::start_local_server,
            commands::engine::stop_local_server,
            commands::engine::is_local_server_running,
            commands::engine::scan_installed_engines,
            commands::engine::list_engine_models,
            commands::attachment::process_file_content,
            commands::llm::summarize_history,
            commands::llm::archive_history,
            commands::llm::vision_analyze_images,
            commands::llm::append_message,
            commands::llm::btw::ask_btw_question,
            commands::llm::tasks::respond_agent_question,
            commands::llm::delete_topic_message,
            commands::llm::generate_topic_title,
            commands::llm::get_usage_summary,
            commands::llm::get_usage_summary_by_model,
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
            // Project management
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
            // Skill management
            commands::skill::list_skills,
            commands::skill::save_skill,
            commands::skill::delete_skill,
            commands::skill::list_skill_market,
            commands::skill::download_market_skill,
            commands::skill::discover_npx_skills,
            commands::skill::import_npx_skill,
            commands::skill::refresh_npx_skill,
            // MCP server management
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
            // LSP language server management
            commands::lsp::start_lsp_server,
            commands::lsp::stop_lsp_server,
            commands::lsp::get_diagnostics,
            commands::lsp::auto_detect_ls,
            commands::lsp::list_supported_languages,
            commands::lsp::stop_all_lsp_servers,
            // Per-profile model overrides
            commands::config::load_profile_model_overrides,
            commands::config::save_profile_model_overrides,
            commands::config::list_custom_subagent_profiles,
            commands::config::save_custom_subagent_profile,
            // Custom subagent profiles
            commands::config::delete_custom_subagent_profile,
            // System auto-start
            commands::config::set_auto_start,
            commands::config::is_auto_start_enabled,
            // Topic branching
            commands::config::branch_topic,
            // Token counting
            commands::llm::count_tokens_cmd,
            // Project memory (RAG)
            commands::memory::memory_get_status,
            commands::memory::memory_get_versions,
            commands::memory::memory_merge_facts,
            commands::memory::memory_archive,
            commands::memory::memory_set_pinned,
            commands::memory::memory_prune,
            commands::memory::memory_export_file,
            commands::memory::memory_import_file,
            commands::memory::memory_code_index,
            commands::memory::memory_code_search,
            commands::memory::memory_code_status,
            commands::memory::memory_get,
            commands::memory::memory_set_enabled,
            commands::memory::memory_reset_enabled,
            commands::memory::memory_search,
            commands::memory::memory_list,
            commands::memory::memory_add,
            commands::memory::memory_update,
            commands::memory::memory_delete,
            commands::memory::memory_clear,
            commands::memory::memory_reindex,
            commands::memory::memory_update_embedding,
            commands::memory::memory_stats,
            commands::memory::memory_update_rerank,
            commands::memory::memory_consolidate,
            commands::memory::memory_graph_extract,
            commands::embedding::embedding_test,
            commands::rerank::rerank_test,
            commands::model_download::embedding_model_list,
            commands::model_download::embedding_model_status,
            commands::model_download::embedding_model_download,
            commands::model_download::embedding_model_pause_download,
            commands::model_download::embedding_model_cancel_download,
            commands::model_download::embedding_model_delete,
            commands::model_download::embedding_model_set_enabled,
            commands::model_download::embedding_models_get_dir,
            commands::model_download::embedding_models_set_dir,
            commands::local_model_market::local_model_market_list,
            commands::local_model_market::local_model_market_files,
            commands::local_model_market::local_model_hardware_profile,
            commands::local_model_market::local_model_hf_token_get,
            commands::local_model_market::local_model_hf_token_set,
            commands::local_model_download::local_model_download,
            commands::local_model_download::local_model_pause_download,
            commands::local_model_download::local_model_cancel_download,
            commands::local_model_download::local_model_delete,
            commands::local_model_download::local_model_installed_list,
            commands::local_model_download::local_models_get_dir,
            commands::local_model_download::local_models_set_dir,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                // Clean up local engine child processes (all engines)
                let state = window.state::<LocalEngineState>();
                {
                    let mut engines = state.lock();
                    for (_, mut inner) in engines.drain() {
                        if let Some(mut child) = inner.child_process.take() {
                            let _ = child.kill();
                        }
                    }
                }
                // Clean up active LLM stream tasks: cancel all tokens (each task emits its own done event and removes itself)
                let stream_mgr = window.state::<StreamManager>();
                for entry in stream_mgr.0.iter() {
                    entry.value().1.cancel();
                }
                stream_mgr.0.clear();
                // Clean up MCP state (abort in-flight calls + clear the connection pool)
                let req_mgr = window.state::<McpRequestManager>();
                req_mgr.abort_all();
                let mcp_state = window.state::<McpServerState>();
                mcp_state.lock().clear();
            }
        })
        .run(tauri::generate_context!())
        .expect("运行 tauri 应用程序时发生错误");
}

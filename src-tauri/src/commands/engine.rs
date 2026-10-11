//! Tauri commands for local inference engine management: start, stop, status check,
//! install scan, and model listing.
//!
//! Engine differences (executable, port, launch arguments) are provided by the
//! `LocalEnginePlugin` trait in `plugins/engine`; this module only handles
//! orchestration and IPC.
//!
//! Note: never hardcode engine names in code. Engine identifiers always come from
//! `ActivatedModel.engine_type` or the plugin's `identifier()` (AGENTS.md convention).

use crate::core::models::EngineInstallInfo;
use crate::core::state::LocalEngineState;
use crate::plugins::engine::EngineManager;
use crate::utils::file_parser::validate_model_path;
use serde::Serialize;
use tauri::{AppHandle, State};
use tokio::time::{sleep, Duration};

use crate::ierr;

/// Default engine identifier used when an older frontend call omits engine_type
/// (default local engine = the llama.cpp plugin).
/// Declared once here so the literal is not scattered across command bodies.
const DEFAULT_ENGINE_ID: &str = "llama_cpp";

/// ENG-8: serializes engine starts process-wide. The "stop other engines, then start
/// this one" sequence is not atomic — two concurrent `start_local_server` calls both
/// observed "no other engine running" and both spawned, leaving two engines up in
/// violation of the one-active-engine invariant.
fn engine_start_mutex() -> &'static tokio::sync::Mutex<()> {
    static M: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    M.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Starts the local LLM server.
///
/// # Arguments
/// - `model_path`: absolute path to the model file (sandbox-checked via `validate_model_path`)
/// - `port`: port to listen on
/// - `gpu_layers`: number of layers to offload to the GPU
/// - `engine_type`: engine identifier; defaults to `llama_cpp` (old-config compatibility)
/// - `trust_remote_code`: vLLM only; whether running custom code from the model repo
///   is allowed (the frontend asks for confirmation first)
///
/// # Returns
/// The engine state description after a successful start; only one local engine may run at a time.
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

    let plugin = engine_mgr.get(&engine_id).ok_or_else(|| {
        ierr!(
            "不支持的本地引擎: {}",
            "Unsupported local engine: {}",
            engine_id
        )
    })?;

    // Sandbox: reject model paths outside home/AppData (external engines are exempt; their models are managed by the engine itself)
    let safe_path = if plugin.is_external() {
        std::path::PathBuf::from(&model_path)
    } else {
        validate_model_path(&model_path)?
    };
    // ENG-8: hold the start mutex across the whole stop-others-then-start sequence;
    // a concurrent starter waits here and then observes (and replaces) our engine.
    let _start_guard = engine_start_mutex().lock().await;
    // Pre-start cleanup: stop engines of all other types (only one local engine may run at a time)
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
    // If an engine of the same type is already running, stop it first as well (clears the old process)
    if is_engine_running_internal(&state, &engine_id) {
        stop_engine_internal(&state, &engine_id);
        sleep(Duration::from_millis(500)).await;
    }
    // Start via the plugin
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

/// Stops the local server
/// @param engine_type optional engine type. When omitted, stops all running engines.
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
                    // ENG-06: kill 后 wait，避免 Unix 每次启停泄漏僵尸进程
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        }
    }
    Ok(())
}

/// Checks whether the local server is running
/// @param engine_type optional engine type. When omitted, returns whether any engine is running.
#[tauri::command]
pub fn is_local_server_running(
    state: State<'_, LocalEngineState>,
    engine_type: Option<String>,
) -> bool {
    match engine_type {
        Some(id) => is_engine_running_internal(&state, &id),
        None => {
            let mut engines = state.lock();
            // Clean up exited processes first, then check whether any live process remains
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

/// Scan result for an inference engine installed on the system.
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

/// Scans the install status of all local inference engines (called once at app startup;
/// the frontend caches the result).
///
/// # Arguments
/// - `engine_mgr`: engine plugin manager (registration order decides result order)
///
/// # Behavior
/// Calls each plugin's `detect_installation` and `default_port` in turn and aggregates
/// them into `EngineScanResult`s; one plugin's detection failure only records empty
/// info and never affects other engines.
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

        // URL shape is declared by the plugin (Ollama root URL; OpenAI-compatible engines append /v1)
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

/// Fetches the model list from a running engine.
///
/// # Arguments
/// - `api_url`: the engine's API URL (e.g. http://localhost:11434)
/// - `engine_type`: engine type identifier (used to pick the plugin: URL shape and
///   response parsing are declared by the plugin; no engine name is hardcoded here)
///
/// # Returns
/// A list of model ids; HTTP or parsing failures return an error string.
#[tauri::command]
pub async fn list_engine_models(
    engine_mgr: State<'_, EngineManager>,
    api_url: String,
    engine_type: String,
) -> Result<Vec<String>, String> {
    let plugin = engine_mgr.get(&engine_type).ok_or_else(|| {
        ierr!(
            "不支持的引擎类型: {}",
            "Unsupported engine type: {}",
            engine_type
        )
    })?;

    // Endpoint shape is declared by the plugin (Ollama=/api/tags, OpenAI-compatible=/models)
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
        .map_err(|e| ierr!("请求失败: {}", "Request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| ierr!("读取响应失败: {}", "Failed to read response: {}", e))?;

    let v: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| ierr!("响应非 JSON: {}", "Response is not JSON: {}", e))?;

    // Response parsing is declared by the plugin (Ollama's {models:[{name}]} vs OpenAI's {data:[{id}]})
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
            // ENG-06: kill 后 wait，避免 Unix 每次启停泄漏僵尸进程
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

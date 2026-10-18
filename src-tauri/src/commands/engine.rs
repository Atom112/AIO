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
    // M-6: the guard lives in its own block — an await inside the loop below must
    // not keep the (non-Send) parking_lot guard captured in the async state machine.
    let other_ids: Vec<String> = {
        let engines = state.lock();
        engines
            .keys()
            .filter(|k| *k != &engine_id)
            .cloned()
            .collect()
    };
    for id in other_ids {
        tracing::info!("[engine] 停止旧引擎 {} 以启动 {}", id, engine_id);
        stop_engine_internal(&state, &id).await;
    }
    // If an engine of the same type is already running, stop it first as well (clears the old process)
    if is_engine_running_internal(&state, &engine_id) {
        stop_engine_internal(&state, &engine_id).await;
        sleep(Duration::from_millis(500)).await;
    }
    // ENG-03: port preflight — AFTER our own engines are stopped (a same-port
    // restart must not trip over itself), but BEFORE the new spawn. A port
    // already claimed by an unrelated process used to surface later as a
    // confusing health-check failure — or worse, the health probe hit the
    // foreign listener and reported a bogus "engine ready".
    {
        let addr = format!("127.0.0.1:{port}");
        // A successful connect means something is already listening on the port.
        let occupied = matches!(
            tokio::time::timeout(
                Duration::from_millis(300),
                tokio::net::TcpStream::connect(&addr)
            )
            .await,
            Ok(Ok(_))
        );
        if occupied {
            return Err(ierr!(
                "端口 {} 已被其他进程占用，请更换端口或释放该端口",
                "Port {} is already in use by another process; pick a different port or free it",
                port
            ));
        }
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
            stop_engine_internal(&state, &id).await;
        }
        None => {
            // M-6: drain under the lock, kill+wait each child on the blocking pool
            // (outside the lock, off the async worker).
            let children = {
                let mut engines = state.lock();
                engines
                    .drain()
                    .filter_map(|(_, mut inner)| inner.child_process.take())
                    .collect::<Vec<_>>()
            };
            for mut child in children {
                tracing::debug!("[engine] 正在停止引擎...");
                // ENG-06: kill 后 wait，避免 Unix 每次启停泄漏僵尸进程
                let _ = tokio::task::spawn_blocking(move || {
                    let _ = child.kill();
                    let _ = child.wait();
                })
                .await;
            }
        }
    }
    Ok(())
}

/// Checks whether the local server is running
/// @param engine_type optional engine type. When omitted, returns whether any engine is running.
/// (审查修复 M-9) async 属性：内部 try_wait 逐个收割子进程状态，不占主线程。
#[tauri::command(async)]
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
pub async fn scan_installed_engines(
    app: AppHandle,
    engine_mgr: State<'_, EngineManager>,
) -> Result<Vec<EngineScanResult>, String> {
    // (审查修复 H-2) detect_installation 是阻塞子进程探测链（llama-server
    // --version / sc query ollama / 逐候选 python -c "import vllm"——import vllm
    // 会加载 torch，单次数秒，且 vllm 探测还会把同一 import 跑两遍）。此前整个
    // 同步扫描直接跑在 async runtime 的 worker 线程上（#[tauri::command(async)]
    // 的语义），启动扫描可占用一个 worker 数秒至数十秒，挤压并发 agent 回合与
    // 流式任务。整链移入 blocking pool（项目已有同款范式：skill.rs 的 npx 探测）。
    let plugins = engine_mgr.all_plugins();
    let results = tauri::async_runtime::spawn_blocking(move || {
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
        results
    })
    .await
    .map_err(|e| e.to_string())?;
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

/// M-6: stop one engine. The map removal happens under the state lock; the
/// kill+wait (a big engine's teardown can block for hundreds of ms) runs on the
/// blocking pool OUTSIDE the lock and the async worker, so a stop neither
/// serializes engine-state users nor stalls the runtime.
async fn stop_engine_internal(state: &LocalEngineState, engine_type: &str) {
    let child = {
        let mut engines = state.lock();
        engines
            .remove(engine_type)
            .and_then(|mut inner| inner.child_process.take())
    };
    if let Some(mut child) = child {
        tracing::debug!("[engine] 正在停止引擎 {}...", engine_type);
        // ENG-06: kill 后 wait，避免 Unix 每次启停泄漏僵尸进程
        let _ = tokio::task::spawn_blocking(move || {
            let _ = child.kill();
            let _ = child.wait();
        })
        .await;
    }
}

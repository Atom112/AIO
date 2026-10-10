/// Local inference engine plugin system
/// Provides the unified LocalEnginePlugin trait and the EngineManager registry
pub mod installer;
pub mod llama_cpp;
pub mod ollama;
pub mod vllm;

use crate::core::events as ev;
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use tauri::AppHandle;

/// Local inference engine plugin trait
/// All local inference backends (llama.cpp, vLLM, etc.) must implement this trait
pub trait LocalEnginePlugin: Send + Sync {
    /// Human-readable name, e.g. "llama.cpp" (shown in engine scan results)
    fn name(&self) -> &'static str;
    /// Machine identifier, e.g. "llama_cpp"
    fn identifier(&self) -> &'static str;

    /// Whether this engine is supported on the current platform
    fn is_platform_supported(&self) -> bool {
        true
    }

    /// Get the engine install path under app data.
    /// Note: the installer currently hardcodes the llama-cpp path (plugins/engine/installer.rs),
    /// so this method is not called yet; kept as the plugin extension contract (auto-install hook point).
    #[allow(dead_code)]
    fn install_path(&self, app: &AppHandle) -> PathBuf;

    /// Whether the engine is installed
    fn is_installed(&self, app: &AppHandle) -> bool;

    /// Start the engine and return the OpenAI-compatible API Base URL it exposes
    fn start<'a>(
        &'a self,
        app: AppHandle,
        state: &'a crate::core::state::LocalEngineState,
        model_path: &'a str,
        port: u16,
        gpu_layers: i32,
        trust_remote_code: bool,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>>;

    /// Event name used to emit progress events
    fn progress_event_name(&self) -> &'static str {
        ev::ENGINE_PROGRESS
    }

    /// Build the command line (used internally by start)
    fn build_command(
        &self,
        exe_path: &std::path::Path,
        model_path: &str,
        port: u16,
        gpu_layers: i32,
    ) -> std::process::Command;

    /// Deep detection: check whether the engine is actually installed on the system (PATH / package managers / default port reachable).
    /// The default implementation calls is_installed(); external engines should override to check PATH + HTTP reachability.
    fn detect_installation(&self, app: &AppHandle) -> crate::core::models::EngineInstallInfo {
        crate::core::models::EngineInstallInfo {
            installed: self.is_installed(app),
            version: None,
        }
    }

    /// Default port for this engine (frontend prefills the URL with it and uses it at startup).
    fn default_port(&self) -> u16 {
        8080
    }

    /// Default API base URL (for frontend prefill). Defaults to the OpenAI-compatible shape `http://localhost:{port}/v1`;
    /// engines with a non-OpenAI shape (e.g. Ollama's root URL) must override.
    fn default_api_url(&self, port: u16) -> String {
        format!("http://localhost:{port}/v1")
    }

    /// Model list endpoint URL. Defaults to `{base}/models` (OpenAI compatible); other shapes must override.
    fn models_url(&self, api_url: &str) -> String {
        format!("{}/models", api_url.trim_end_matches('/'))
    }

    /// Parse model ids from the model list response. Defaults to the OpenAI `{{"data":[{{"id":...}}]}}` shape.
    fn parse_model_ids(&self, body: &serde_json::Value) -> Vec<String> {
        body["data"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether this is an "external engine" (its child process is not managed by us, e.g. a system-installed Ollama).
    /// External engines skip model-path sandbox validation (their models are managed by the engine itself).
    fn is_external(&self) -> bool {
        false
    }
}

/// Engine manager: holds all registered plugins
pub struct EngineManager {
    plugins: HashMap<String, Box<dyn LocalEnginePlugin>>,
}

impl EngineManager {
    pub fn new() -> Self {
        let mut mgr = Self {
            plugins: HashMap::new(),
        };
        mgr.register(Box::new(llama_cpp::LlamaCppPlugin));
        mgr.register(Box::new(vllm::VllmPlugin));
        mgr.register(Box::new(ollama::OllamaPlugin));
        mgr
    }

    pub fn register(&mut self, plugin: Box<dyn LocalEnginePlugin>) {
        self.plugins.insert(plugin.identifier().to_string(), plugin);
    }

    pub fn get(&self, id: &str) -> Option<&dyn LocalEnginePlugin> {
        self.plugins.get(id).map(|b| b.as_ref())
    }

    /// Return references to all registered plugins (sorted alphabetically by identifier).
    pub fn all_plugins(&self) -> Vec<&dyn LocalEnginePlugin> {
        let mut plugins: Vec<&dyn LocalEnginePlugin> =
            self.plugins.values().map(|b| b.as_ref()).collect();
        plugins.sort_by_key(|p| p.identifier());
        plugins
    }
}

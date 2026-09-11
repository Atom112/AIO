/// 本地推理引擎插件系统
/// 提供统一的 LocalEnginePlugin trait 和 EngineManager 注册中心
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

/// 本地推理引擎插件 trait
/// 所有本地推理后端（llama.cpp, vLLM 等）必须实现此 trait
pub trait LocalEnginePlugin: Send + Sync {
    /// 人类可读名称，如 "llama.cpp"（用于引擎扫描结果展示）
    fn name(&self) -> &'static str;
    /// 机器标识符，如 "llama_cpp"
    fn identifier(&self) -> &'static str;

    /// 当前平台是否支持此引擎
    fn is_platform_supported(&self) -> bool {
        true
    }

    /// 获取引擎在 app data 下的安装路径。
    /// 注：当前 installer 对 llama-cpp 硬编码了路径（plugins/engine/installer.rs），
    /// 此方法尚未被调用——保留为插件扩展契约（自动安装流程的挂接点）。
    #[allow(dead_code)]
    fn install_path(&self, app: &AppHandle) -> PathBuf;

    /// 引擎是否已安装
    fn is_installed(&self, app: &AppHandle) -> bool;

    /// 启动引擎，返回其暴露的 OpenAI-compatible API Base URL
    fn start<'a>(
        &'a self,
        app: AppHandle,
        state: &'a crate::core::state::LocalEngineState,
        model_path: &'a str,
        port: u16,
        gpu_layers: i32,
        trust_remote_code: bool,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>>;

    /// 发送进度事件的事件名
    fn progress_event_name(&self) -> &'static str {
        ev::ENGINE_PROGRESS
    }

    /// 构建命令行参数（供 start 内部使用）
    fn build_command(
        &self,
        exe_path: &std::path::Path,
        model_path: &str,
        port: u16,
        gpu_layers: i32,
    ) -> std::process::Command;

    /// 深度检测：检查引擎是否实际安装在系统上（PATH / 包管理器 / 默认端口可访问）。
    /// 默认实现调用 is_installed()；外部引擎应覆盖以检查 PATH + HTTP 可达。
    fn detect_installation(&self, app: &AppHandle) -> crate::core::models::EngineInstallInfo {
        crate::core::models::EngineInstallInfo {
            installed: self.is_installed(app),
            version: None,
        }
    }

    /// 此引擎的默认端口（供前端预填 URL 和启动时使用）。
    fn default_port(&self) -> u16 {
        8080
    }

    /// 默认 API base URL（前端预填用）。默认 OpenAI 兼容形态 `http://localhost:{port}/v1`；
    /// 非 OpenAI 形态的引擎（如 Ollama 的根 URL）必须覆盖。
    fn default_api_url(&self, port: u16) -> String {
        format!("http://localhost:{port}/v1")
    }

    /// 模型列表端点 URL。默认 `{base}/models`（OpenAI 兼容）；其他形态必须覆盖。
    fn models_url(&self, api_url: &str) -> String {
        format!("{}/models", api_url.trim_end_matches('/'))
    }

    /// 从模型列表响应解析模型名。默认按 OpenAI `{{"data":[{{"id":...}}]}}` 解析。
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

    /// 是否为"外部引擎"（不由本进程管理子进程，如系统安装的 Ollama）。
    /// 外部引擎的模型路径不做沙箱校验（其模型由引擎自身管理）。
    fn is_external(&self) -> bool {
        false
    }
}

/// 引擎管理器：维护所有已注册插件
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

    /// 返回所有已注册插件的引用（按标识符字母序排列）。
    pub fn all_plugins(&self) -> Vec<&dyn LocalEnginePlugin> {
        let mut plugins: Vec<&dyn LocalEnginePlugin> =
            self.plugins.values().map(|b| b.as_ref()).collect();
        plugins.sort_by_key(|p| p.identifier());
        plugins
    }
}

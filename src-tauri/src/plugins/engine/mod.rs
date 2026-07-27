/// 本地推理引擎插件系统
/// 提供统一的 LocalEnginePlugin trait 和 EngineManager 注册中心
pub mod installer;
pub mod llama_cpp;
pub mod ollama;
pub mod vllm;

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use tauri::AppHandle;

/// 本地推理引擎插件 trait
/// 所有本地推理后端（llama.cpp, vLLM 等）必须实现此 trait
#[allow(dead_code)]
pub trait LocalEnginePlugin: Send + Sync {
    /// 人类可读名称，如 "llama.cpp"
    fn name(&self) -> &'static str;
    /// 机器标识符，如 "llama_cpp"
    fn identifier(&self) -> &'static str;
    /// 支持的模型文件扩展名
    fn supported_extensions(&self) -> &[&'static str];

    /// 当前平台是否支持此引擎
    fn is_platform_supported(&self) -> bool {
        true
    }

    /// 获取引擎在 app data 下的安装路径
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
        "engine-progress"
    }

    /// 构建命令行参数（供 start 内部使用）
    fn build_command(
        &self,
        exe_path: &std::path::Path,
        model_path: &str,
        port: u16,
        gpu_layers: i32,
    ) -> std::process::Command;
    /// 解析 stderr 日志并返回进度值 (0.0~1.0)
    fn parse_progress_from_log(&self, line: &str) -> Option<f64>;

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

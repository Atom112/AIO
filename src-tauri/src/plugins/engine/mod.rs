/// 本地推理引擎插件系统
/// 提供统一的 LocalEnginePlugin trait 和 EngineManager 注册中心

pub mod llama_cpp;
pub mod vllm;

use std::collections::HashMap;
use std::pin::Pin;
use std::future::Future;
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

    /// 启动引擎，返回其暴露的 OpenAI-compatible API Base URL
    fn start<'a>(
        &'a self,
        app: AppHandle,
        state: &'a crate::core::state::LocalEngineState,
        model_path: &'a str,
        port: u16,
        gpu_layers: i32,
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
}

/// 引擎管理器：维护所有已注册插件
#[allow(dead_code)]
pub struct EngineManager {
    plugins: HashMap<String, Box<dyn LocalEnginePlugin>>,
}

impl EngineManager {
    pub fn new() -> Self {
        let mut mgr = Self {
            plugins: HashMap::new(),
        };
        mgr.register(Box::new(llama_cpp::LlamaCppPlugin));
        // mgr.register(Box::new(vllm::VllmPlugin)); // 后续注册
        mgr
    }

    pub fn register(&mut self, plugin: Box<dyn LocalEnginePlugin>) {
        self.plugins.insert(plugin.identifier().to_string(), plugin);
    }

    pub fn get(&self, id: &str) -> Option<&dyn LocalEnginePlugin> {
        self.plugins.get(id).map(|b| b.as_ref())
    }
}

/// vLLM 本地推理引擎插件（预留框架）

use crate::core::state::LocalEngineState;
use crate::plugins::engine::LocalEnginePlugin;
use std::path::Path;
use tauri::AppHandle;

#[allow(dead_code)]
pub struct VllmPlugin;

impl LocalEnginePlugin for VllmPlugin {
    fn name(&self) -> &'static str {
        "vLLM"
    }

    fn identifier(&self) -> &'static str {
        "vllm"
    }

    fn supported_extensions(&self) -> &[&'static str] {
        &["gguf", "safetensors"]
    }

    fn build_command(
        &self,
        _exe_path: &Path,
        model_path: &str,
        port: u16,
        _gpu_layers: i32,
    ) -> std::process::Command {
        // vLLM 通常通过 Python 模块启动
        let mut cmd = std::process::Command::new("python");
        cmd.arg("-m")
            .arg("vllm.entrypoints.openai.api_server")
            .arg("--model")
            .arg(model_path)
            .arg("--port")
            .arg(port.to_string())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        cmd
    }

    fn parse_progress_from_log(&self, line: &str) -> Option<f64> {
        if line.contains("Application startup complete") {
            Some(1.0)
        } else if line.contains("Loading model weights") {
            Some(0.5)
        } else {
            None
        }
    }

    fn start<'a>(
        &'a self,
        _app: AppHandle,
        _state: &'a LocalEngineState,
        _model_path: &'a str,
        _port: u16,
        _gpu_layers: i32,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + 'a>> {
        Box::pin(async move {
            Err("vLLM 支持即将推出。请确保已在 resources/engines/vllm/ 放置 vLLM 运行时，或系统已安装 vllm 包。".to_string())
        })
    }
}

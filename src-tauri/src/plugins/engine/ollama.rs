/// Ollama 本地推理引擎插件（轻量级，不做进程管理）。
///
/// Ollama 由用户自行管理生命周期（systemd / launchd / docker / 手动启动），
/// 本插件仅提供引擎标识与运行态检测，不调用外部进程。
use crate::core::state::LocalEngineState;
use crate::plugins::engine::LocalEnginePlugin;
use std::future::Future;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use tauri::AppHandle;
use tracing::debug;

/// 检查 PATH 上是否存在 `ollama` 二进制
fn detect_ollama_on_path() -> bool {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("where")
            .arg("ollama")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("which")
            .arg("ollama")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

/// 检查 Ollama HTTP 端点是否可达
fn ollama_http_reachable() -> bool {
    std::thread::spawn(|| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let client = reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(2))
                .timeout(std::time::Duration::from_secs(3))
                .build()
                .ok();
            match client {
                Some(c) => match c.get("http://localhost:11434/api/version").send().await {
                    Ok(r) => r.status().is_success(),
                    Err(_) => false,
                },
                None => false,
            }
        })
    })
    .join()
    .unwrap_or(false)
}

/// 通过 `ollama --version` 获取版本号
fn ollama_version_from_cli() -> Option<String> {
    let mut cmd = std::process::Command::new("ollama");
    cmd.arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);
    match cmd.output() {
        Ok(output) => {
            let raw = String::from_utf8_lossy(if output.stdout.is_empty() {
                &output.stderr
            } else {
                &output.stdout
            });
            let first_line = raw.lines().next().map(|s| s.trim().to_string());
            // Ollama output: "ollama version is 0.5.7" → extract version number
            first_line.and_then(|s| s.split_whitespace().last().map(|v| v.to_string()))
        }
        Err(_) => None,
    }
}
pub struct OllamaPlugin;

impl LocalEnginePlugin for OllamaPlugin {
    fn name(&self) -> &'static str {
        "Ollama"
    }

    fn identifier(&self) -> &'static str {
        "ollama"
    }

    fn supported_extensions(&self) -> &[&'static str] {
        &[] // Ollama 通过 API 管理模型，无本地模型文件
    }

    fn install_path(&self, _app: &AppHandle) -> PathBuf {
        // Ollama 无独立安装路径（用户自行安装）
        PathBuf::new()
    }

    fn is_installed(&self, _app: &AppHandle) -> bool {
        // 检查 PATH 上是否有 ollama 二进制，或 HTTP 端点可达
        detect_ollama_on_path() || ollama_http_reachable()
    }

    fn detect_installation(&self, _app: &AppHandle) -> crate::core::models::EngineInstallInfo {
        let on_path = detect_ollama_on_path();
        let http_ok = ollama_http_reachable();
        let installed = on_path || http_ok;
        let version = if on_path {
            ollama_version_from_cli()
        } else {
            None
        };
        crate::core::models::EngineInstallInfo { installed, version }
    }

    fn default_port(&self) -> u16 {
        11434
    }

    fn start<'a>(
        &'a self,
        _app: AppHandle,
        _state: &'a LocalEngineState,
        model_path: &'a str,
        _port: u16,
        _gpu_layers: i32,
        _trust_remote_code: bool,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>> {
        Box::pin(async move {
            debug!(
                "Ollama 引擎由用户自行管理，跳过启动流程 (model_path={})",
                model_path
            );
            // 直接返回 api_url（Ollama 的 model_path 实际是 base URL）
            Ok(model_path.to_string())
        })
    }
    fn build_command(
        &self,
        _exe_path: &Path,
        _model_path: &str,
        _port: u16,
        _gpu_layers: i32,
    ) -> std::process::Command {
        // Ollama 不需要构建启动命令
        std::process::Command::new("echo")
    }

    fn parse_progress_from_log(&self, _line: &str) -> Option<f64> {
        None // Ollama 无启动进度日志
    }
}

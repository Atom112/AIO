/// vLLM 本地推理引擎插件实现
///
/// ⚠️ 注意：vLLM 官方仅支持 Linux 和 macOS，Windows 上不提供此引擎选项。
/// 启动策略：
/// 1. 检查系统是否已安装 vllm (python -c "import vllm")
/// 2. 若未安装但 resources/engines/vllm/ 下有 .whl 文件，自动 pip install
/// 3. 通过 python -m vllm.entrypoints.openai.api_server 启动 OpenAI 兼容服务

use crate::core::state::LocalEngineState;
use crate::plugins::engine::LocalEnginePlugin;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager};
use tokio::task;
use tokio::time::{sleep, Duration};
use tracing::debug;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

pub struct VllmPlugin;

fn create_progress_cmd(program: &str, args: &[&str]) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);

    cmd
}

fn check_vllm_installed() -> bool {
    let mut cmd = create_progress_cmd("python", &["-c", "import vllm; print(vllm.__version__)"]);
    cmd.spawn()
        .and_then(|mut c| c.wait())
        .map(|s| s.success())
        .unwrap_or(false)
}

fn find_python() -> Result<String, String> {
    for python in ["python", "python3"] {
        let mut cmd = create_progress_cmd(python, &["--version"]);
        if cmd.spawn().and_then(|mut c| c.wait()).map(|s| s.success()).unwrap_or(false) {
            return Ok(python.to_string());
        }
    }
    Err("未找到 Python 运行时。请安装 Python 3.8+ 并确保已加入 PATH。".to_string())
}

fn find_bundled_wheels(resource_dir: &Path) -> Vec<std::path::PathBuf> {
    let mut wheels = Vec::new();
    if let Ok(entries) = std::fs::read_dir(resource_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(ext) = path.extension() {
                if ext == "whl" {
                    wheels.push(path);
                }
            }
        }
    }
    wheels.sort();
    wheels
}

fn install_from_wheels(python: &str, wheels: &[std::path::PathBuf]) -> Result<(), String> {
    let mut args = vec!["-m", "pip", "install", "--quiet"];
    for w in wheels {
        args.push(w.to_str().ok_or_else(|| format!("无效的 wheel 路径: {:?}", w))?);
    }

    debug!("[vLLM] 正在从 bundled .whl 安装 vllm...");
    let mut cmd = create_progress_cmd(python, &args);
    let status = cmd.spawn()
        .map_err(|e| format!("pip install 启动失败: {}", e))?
        .wait()
        .map_err(|e| format!("pip install 执行失败: {}", e))?;

    if !status.success() {
        return Err("pip install vllm 失败。请检查 Python 环境和 CUDA 工具链是否正确安装。".to_string());
    }
    debug!("[vLLM] vllm 安装成功");
    Ok(())
}

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

    fn is_platform_supported(&self) -> bool {
        cfg!(not(target_os = "windows"))
    }

    fn install_path(&self, app: &AppHandle) -> PathBuf {
        let mut path = app
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| PathBuf::from("."));
        path.push("engines");
        path.push("vllm");
        path
    }

    fn is_installed(&self, _app: &AppHandle) -> bool {
        // 通过 Python 包检查 vLLM 是否已安装
        check_vllm_installed()
    }

    fn progress_event_name(&self) -> &'static str {
        "engine-progress"
    }

    fn build_command(
        &self,
        _exe_path: &Path,
        _model_path: &str,
        _port: u16,
        _gpu_layers: i32,
    ) -> std::process::Command {
        std::process::Command::new("python")
    }

    fn parse_progress_from_log(&self, line: &str) -> Option<f64> {
        if line.contains("Loading model weights") {
            Some(0.3)
        } else if line.contains("Model loaded") || line.contains("model loaded") {
            Some(0.6)
        } else if line.contains("Uvicorn running on") {
            Some(0.8)
        } else if line.contains("Application startup complete") {
            Some(1.0)
        } else {
            None
        }
    }

    fn start<'a>(
        &'a self,
        app: AppHandle,
        state: &'a LocalEngineState,
        model_path: &'a str,
        port: u16,
        gpu_layers: i32,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + 'a>> {
        Box::pin(async move {
            debug!(
                "启动参数 - 引擎: vLLM, 模型: {}, 端口: {}, GPU层数: {}",
                model_path, port, gpu_layers
            );

            if !Path::new(model_path).exists() {
                return Err(format!("模型文件/目录不存在: {}", model_path));
            }

            let _ = app.emit(self.progress_event_name(), 0.02);

            let python = find_python()?;

            let _ = app.emit(self.progress_event_name(), 0.05);

            if !check_vllm_installed() {
                let resource_dir = app
                    .path()
                    .resolve("resources/engines/vllm", BaseDirectory::Resource)
                    .map_err(|e| format!("无法解析资源路径: {}", e))?;

                let wheels = find_bundled_wheels(&resource_dir);

                if wheels.is_empty() {
                    return Err(
                        "系统未安装 vllm 包，且 resources/engines/vllm/ 下未找到 .whl 安装包。\n\
                         请执行以下步骤之一：\n\
                         1. 手动安装: pip install vllm\n\
                         2. 将 vllm 的 .whl 文件放入 resources/engines/vllm/ 目录后重试\n\n\
                         注意：vLLM 需要 CUDA 工具链支持，请确保已安装 NVIDIA GPU 驱动和 CUDA。"
                            .to_string(),
                    );
                }

                let _ = app.emit(self.progress_event_name(), 0.1);
                install_from_wheels(&python, &wheels)?;
            } else {
                debug!("[vLLM] 检测到系统已安装 vllm 包");
            }

            let _ = app.emit(self.progress_event_name(), 0.15);

            let mut cmd = create_progress_cmd(
                &python,
                &[
                    "-m",
                    "vllm.entrypoints.openai.api_server",
                    "--model",
                    model_path,
                    "--port",
                    &port.to_string(),
                    "--host",
                    "127.0.0.1",
                    "--dtype",
                    "auto",
                    "--max-model-len",
                    "4096",
                    "--trust-remote-code",
                ],
            );

            let mut child = cmd.spawn().map_err(|e| {
                format!(
                    "启动 vLLM 失败: {}。\n请确保 Python 和 vllm 已正确安装 (pip install vllm)。",
                    e
                )
            })?;

            let _ = app.emit(self.progress_event_name(), 0.2);

            let stderr = match child.stderr.take() {
                Some(s) => s,
                None => return Err("无法获取 vLLM 子进程 stderr".to_string()),
            };
            let app_clone = app.clone();
            let event_name = self.progress_event_name().to_string();
            task::spawn_blocking(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines() {
                    if let Ok(line) = line {
                        debug!("[vllm-server] {}", line);

                        let progress = if line.contains("Loading model weights") {
                            Some(0.3)
                        } else if line.contains("Model loaded") || line.contains("model loaded") {
                            Some(0.6)
                        } else if line.contains("Uvicorn running on") {
                            Some(0.8)
                        } else if line.contains("Application startup complete") {
                            Some(1.0)
                        } else {
                            None
                        };

                        if let Some(p) = progress {
                            let _ = app_clone.emit(&event_name, p);
                        }
                    }
                }
            });

            sleep(Duration::from_secs(5)).await;
            match child.try_wait() {
                Ok(None) => {}
                Ok(Some(status)) => {
                    return Err(format!("vLLM 进程启动后立即退出，退出码: {}。\n常见原因：CUDA 不可用、模型路径错误、显存不足。", status));
                }
                Err(e) => return Err(format!("无法检查进程状态: {}", e)),
            }

            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .connect_timeout(Duration::from_secs(3))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());
            let health_url = format!("http://127.0.0.1:{}/health", port);

            match client
                .get(&health_url)
                .send()
                .await
            {
                Ok(_) => {
                    let _ = app.emit(self.progress_event_name(), 1.0);
                }
                Err(_) => {
                    let _ = child.kill();
                    return Err("vLLM 服务未响应健康检查，可能启动失败。\n请检查：1) CUDA 工具链是否正确安装 2) 显存是否充足 3) 模型路径是否有效".to_string());
                }
            }

            let mut inner = state.lock();
            inner.engine_type = self.identifier().to_string();
            inner.child_process = Some(child);

            Ok(format!("http://127.0.0.1:{}/v1", port))
        })
    }
}

/// llama.cpp 本地推理引擎插件实现
///
/// 启动策略：
/// 1. 优先使用 app data 下通过自动安装的引擎（EngineInstaller）
/// 2. 回退到 resources/engines/llama-cpp/ 下的 bundled 版本（旧版打包兼容）

use crate::core::state::LocalEngineState;
use crate::plugins::engine::installer::EngineInstaller;
use crate::plugins::engine::LocalEnginePlugin;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager};
use tokio::task;
use tokio::time::{sleep, Duration};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

pub struct LlamaCppPlugin;

impl LocalEnginePlugin for LlamaCppPlugin {
    fn name(&self) -> &'static str {
        "llama.cpp"
    }

    fn identifier(&self) -> &'static str {
        "llama_cpp"
    }

    fn supported_extensions(&self) -> &[&'static str] {
        &["gguf"]
    }

    fn is_platform_supported(&self) -> bool {
        true // llama.cpp 全平台支持
    }

    fn install_path(&self, app: &AppHandle) -> PathBuf {
        EngineInstaller::get_engine_dir(app)
    }

    fn is_installed(&self, app: &AppHandle) -> bool {
        EngineInstaller::is_installed(app)
    }

    fn progress_event_name(&self) -> &'static str {
        "llama-progress"
    }

    fn build_command(
        &self,
        exe_path: &Path,
        model_path: &str,
        port: u16,
        gpu_layers: i32,
    ) -> std::process::Command {
        let resource_dir = exe_path.parent().unwrap_or_else(|| Path::new("."));
        let mut cmd = std::process::Command::new(exe_path);
        cmd.current_dir(resource_dir)
            .args([
                "-m",
                model_path,
                "--port",
                &port.to_string(),
                "-ngl",
                &gpu_layers.to_string(),
                "-c",
                "4096",
                "--host",
                "127.0.0.1",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        #[cfg(target_os = "windows")]
        cmd.creation_flags(0x08000000);
        cmd
    }

    fn parse_progress_from_log(&self, line: &str) -> Option<f64> {
        if line.contains("build info") || line.contains("system info") {
            Some(0.1)
        } else if line.contains("loading model") {
            Some(0.2)
        } else if line.contains("model loaded") || line.contains("done") {
            Some(0.5)
        } else if line.contains("HTTP server listening") || line.contains("listening on") {
            Some(0.8)
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
            println!(
                "[DEBUG] 启动参数 - 引擎: llama.cpp, 模型: {}, 端口: {}, GPU层数: {}",
                model_path, port, gpu_layers
            );

            if gpu_layers <= 0 {
                return Err("GPU 层数必须大于 0，建议设置为 99 或 999".to_string());
            }

            // 优先使用自动安装的引擎，再回退到 bundled 路径
            let exe_path = if EngineInstaller::get_exe_path(&app).exists() {
                EngineInstaller::get_exe_path(&app)
            } else {
                let resource_dir = app
                    .path()
                    .resolve("resources/engines/llama-cpp", BaseDirectory::Resource)
                    .map_err(|e| format!("无法解析资源路径: {}", e))?;
                let fallback = resource_dir.join("llama-server.exe");
                if !fallback.exists() {
                    return Err(
                        "找不到 llama.cpp 引擎。请先在设置页面中安装引擎。".to_string()
                    );
                }
                fallback
            };

            if !Path::new(model_path).exists() {
                return Err(format!("模型文件不存在: {}", model_path));
            }

            let mut cmd = self.build_command(&exe_path, model_path, port, gpu_layers);
            let mut child = cmd.spawn().map_err(|e| format!("启动失败: {}", e))?;

            let _ = app.emit(self.progress_event_name(), 0.05);

            let stderr = child.stderr.take().expect("无法获取 stderr");
            let app_clone = app.clone();
            let event_name = self.progress_event_name().to_string();
            task::spawn_blocking(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines() {
                    if let Ok(line) = line {
                        println!("[llama-server] {}", line);
                        if line.contains("offloaded") {
                            println!("GPU 卸载状态: {}", line);
                        }
                        if line.contains("CUDA") {
                            println!("CUDA 信息: {}", line);
                        }
                        if line.contains("error") || line.contains("Error") || line.contains("failed") {
                            println!("LLAMA 错误: {}", line);
                        }
                        let progress = if line.contains("build info") || line.contains("system info") {
                            Some(0.1)
                        } else if line.contains("loading model") {
                            Some(0.2)
                        } else if line.contains("model loaded") || line.contains("done") {
                            Some(0.5)
                        } else if line.contains("HTTP server listening") || line.contains("listening on") {
                            Some(0.8)
                        } else {
                            None
                        };
                        if let Some(p) = progress {
                            let _ = app_clone.emit(&event_name, p);
                        }
                    }
                }
            });

            sleep(Duration::from_millis(2000)).await;
            match child.try_wait() {
                Ok(None) => println!("进程正常运行中"),
                Ok(Some(status)) => {
                    return Err(format!("进程启动后立即退出，退出码: {}", status));
                }
                Err(e) => return Err(format!("无法检查进程状态: {}", e)),
            }

            let client = reqwest::Client::new();
            let health_url = format!("http://127.0.0.1:{}/health", port);
            match client
                .get(&health_url)
                .timeout(Duration::from_secs(5))
                .send()
                .await
            {
                Ok(_) => {
                    println!("健康检查通过");
                    let _ = app.emit(self.progress_event_name(), 1.0);
                }
                Err(_) => {
                    let _ = child.kill();
                    return Err("服务未响应健康检查，可能启动失败".to_string());
                }
            }

            *state.engine_type.lock().unwrap() = self.identifier().to_string();
            *state.child_process.lock().unwrap() = Some(child);

            Ok(format!("http://127.0.0.1:{}/v1", port))
        })
    }
}

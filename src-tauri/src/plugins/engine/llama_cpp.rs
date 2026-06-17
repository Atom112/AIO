/// llama.cpp 本地推理引擎插件实现

use crate::core::state::LocalEngineState;
use crate::plugins::engine::LocalEnginePlugin;
use std::io::{BufRead, BufReader};
use std::path::Path;
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

    fn progress_event_name(&self) -> &'static str {
        // 保留旧事件名以兼容前端
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
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

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

        // 参数验证
        if gpu_layers <= 0 {
            return Err("GPU 层数必须大于 0，建议设置为 99 或 999".to_string());
        }

        // 路径解析：获取引擎二进制文件路径
        // 预期路径: resources/engines/llama-cpp/llama-server.exe
        let resource_dir = app
            .path()
            .resolve("resources/engines/llama-cpp", BaseDirectory::Resource)
            .map_err(|e| format!("无法解析资源路径: {}", e))?;

        let exe_path = resource_dir.join("llama-server.exe");

        if !exe_path.exists() {
            return Err(format!("找不到执行文件: {:?}", exe_path));
        }

        if !Path::new(model_path).exists() {
            return Err(format!("模型文件不存在: {}", model_path));
        }

        // 构建命令并启动
        let mut cmd = self.build_command(&exe_path, model_path, port, gpu_layers);
        let mut child = cmd.spawn().map_err(|e| format!("启动失败: {}", e))?;

        // 发射初始进度
        let _ = app.emit(self.progress_event_name(), 0.05);

        // 日志实时监控
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

        // 等待并检查进程是否崩溃
        sleep(Duration::from_millis(2000)).await;
        match child.try_wait() {
            Ok(None) => println!("进程正常运行中"),
            Ok(Some(status)) => {
                return Err(format!("进程启动后立即退出，退出码: {}", status));
            }
            Err(e) => return Err(format!("无法检查进程状态: {}", e)),
        }

        // 健康检查
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

        // 存储进程句柄和引擎类型到全局状态
        {
            let mut type_lock = state.engine_type.lock().unwrap();
            *type_lock = self.identifier().to_string();
        }
        {
            let mut proc_lock = state.child_process.lock().unwrap();
            *proc_lock = Some(child);
        }

        Ok(format!("http://127.0.0.1:{}/v1", port))
        })
    }
}

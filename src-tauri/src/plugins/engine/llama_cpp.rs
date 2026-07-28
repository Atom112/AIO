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
use tracing::debug;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

pub struct LlamaCppPlugin;

impl LlamaCppPlugin {
    /// 在多个已知位置搜索 `llama-server` 可执行文件，返回第一个找到的路径。
    /// 搜索顺序：PATH → 各平台包管理器安装目录。
    fn find_llama_server() -> Option<std::path::PathBuf> {
        // ── 1. PATH 查找 ──
        #[cfg(target_os = "windows")]
        {
            let output = std::process::Command::new("where")
                .arg("llama-server")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .creation_flags(0x08000000)
                .output()
                .ok()?;
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if let Some(line) = stdout.lines().next() {
                    let p = std::path::PathBuf::from(line.trim());
                    if p.exists() {
                        return Some(p);
                    }
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let output = std::process::Command::new("which")
                .arg("llama-server")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .output()
                .ok()?;
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if let Some(line) = stdout.lines().next() {
                    let p = std::path::PathBuf::from(line.trim());
                    if p.exists() {
                        return Some(p);
                    }
                }
            }
        }

        let exe_name = if cfg!(target_os = "windows") {
            "llama-server.exe"
        } else {
            "llama-server"
        };

        // ── 2. WinGet (Windows) ──
        #[cfg(target_os = "windows")]
        {
            if let Ok(dir) = std::env::var("LOCALAPPDATA") {
                let winget = std::path::PathBuf::from(dir).join("Microsoft\\WinGet\\Packages");
                if let Some(found) = Self::search_dir_for(&winget, exe_name) {
                    return Some(found);
                }
            }
        }

        // ── 3. Conda (全平台) ──
        // Conda 把 llama-server 装在 $CONDA_PREFIX/bin (Unix) 或 %CONDA_PREFIX%\Library\bin (Win)
        for conda_root in Self::conda_roots() {
            #[cfg(target_os = "windows")]
            let candidate = conda_root.join("Library").join("bin").join(exe_name);
            #[cfg(not(target_os = "windows"))]
            let candidate = conda_root.join("bin").join(exe_name);
            if candidate.exists() {
                return Some(candidate);
            }
        }

        // ── 4. Homebrew (macOS / Linux) ──
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        for brew_prefix in Self::homebrew_prefixes() {
            let candidate = brew_prefix.join("bin").join(exe_name);
            if candidate.exists() {
                return Some(candidate);
            }
        }

        // ── 5. MacPorts (macOS) ──
        #[cfg(target_os = "macos")]
        {
            let candidate = std::path::PathBuf::from("/opt/local/bin").join(exe_name);
            if candidate.exists() {
                return Some(candidate);
            }
        }

        // ── 6. Nix (全平台) ──
        for nix_profile in Self::nix_profile_dirs() {
            let candidate = nix_profile.join("bin").join(exe_name);
            if candidate.exists() {
                return Some(candidate);
            }
        }

        None
    }

    /// 在目录（非递归）下查找指定文件名，返回第一个匹配的完整路径。
    fn search_dir_for(dir: &std::path::Path, exe_name: &str) -> Option<std::path::PathBuf> {
        let entries = std::fs::read_dir(dir).ok()?;
        for entry in entries.filter_map(|e| e.ok()) {
            let candidate = entry.path().join(exe_name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
        None
    }

    /// Conda 可能的根目录列表。
    fn conda_roots() -> Vec<std::path::PathBuf> {
        let mut roots = Vec::new();
        // $CONDA_PREFIX 环境变量（当前激活的环境）
        if let Ok(prefix) = std::env::var("CONDA_PREFIX") {
            roots.push(std::path::PathBuf::from(&prefix));
            // Windows 上 CONDA_PREFIX 可能指向 .../Library，需要取其父目录
            #[cfg(target_os = "windows")]
            if prefix.ends_with("Library") {
                if let Some(parent) = std::path::Path::new(&prefix).parent() {
                    roots.push(parent.to_path_buf());
                }
            }
        }
        // 常见 conda 安装路径
        if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
            for name in &[
                "miniconda3",
                "anaconda3",
                "miniconda",
                "anaconda",
                "miniforge3",
                "mambaforge",
                "micromamba",
            ] {
                roots.push(std::path::PathBuf::from(&home).join(name));
            }
        }
        // Linux/macOS: 系统级 conda
        #[cfg(not(target_os = "windows"))]
        for sys_prefix in &[
            "/opt/conda",
            "/usr/local/anaconda3",
            "/usr/local/miniconda3",
        ] {
            roots.push(std::path::PathBuf::from(sys_prefix));
        }
        roots
    }

    /// Homebrew 可能的 prefix 列表（macOS / Linux）。
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn homebrew_prefixes() -> Vec<std::path::PathBuf> {
        let mut prefixes = Vec::new();
        #[cfg(target_os = "macos")]
        {
            // Apple Silicon 默认
            prefixes.push(std::path::PathBuf::from("/opt/homebrew"));
            // Intel Mac 默认
            prefixes.push(std::path::PathBuf::from("/usr/local"));
        }
        #[cfg(target_os = "linux")]
        {
            if let Ok(home) = std::env::var("HOME") {
                prefixes.push(std::path::PathBuf::from(&home).join(".linuxbrew"));
            }
            prefixes.push(std::path::PathBuf::from("/home/linuxbrew/.linuxbrew"));
        }
        prefixes
    }

    /// Nix profile 可能的 bin 父目录列表。
    fn nix_profile_dirs() -> Vec<std::path::PathBuf> {
        let mut dirs = Vec::new();
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(std::path::PathBuf::from(&home).join(".nix-profile"));
        }
        dirs.push(std::path::PathBuf::from("/nix/var/nix/profiles/default"));
        dirs
    }

    /// 检查系统中是否存在 llama-server（任意位置）
    fn detect_on_path() -> bool {
        Self::find_llama_server().is_some()
    }

    /// 通过 `llama-server --version` 获取版本号
    fn get_version(app: &AppHandle) -> Option<String> {
        // 优先使用 AIO bundled 引擎
        let exe_path = EngineInstaller::get_exe_path(app);
        let exe = if exe_path.exists() {
            exe_path
        } else {
            Self::find_llama_server()?
        };

        let mut cmd = std::process::Command::new(&exe);
        cmd.arg("--version")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        #[cfg(target_os = "windows")]
        cmd.creation_flags(0x08000000);
        match cmd.output() {
            Ok(output) => {
                let raw = String::from_utf8_lossy(if output.stderr.is_empty() {
                    &output.stdout
                } else {
                    &output.stderr
                });
                raw.lines().next().map(|s| s.trim().to_string())
            }
            Err(_) => None,
        }
    }
}

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
        EngineInstaller::is_installed(app) || Self::detect_on_path()
    }

    fn detect_installation(&self, app: &AppHandle) -> crate::core::models::EngineInstallInfo {
        let installed = self.is_installed(app);
        let version = if installed {
            Self::get_version(app)
        } else {
            None
        };
        crate::core::models::EngineInstallInfo { installed, version }
    }

    fn default_port(&self) -> u16 {
        8080
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
        _trust_remote_code: bool,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + 'a>>
    {
        Box::pin(async move {
            debug!(
                "启动参数 - 引擎: llama.cpp, 模型: {}, 端口: {}, GPU层数: {}",
                model_path, port, gpu_layers
            );

            if gpu_layers <= 0 {
                return Err("GPU 层数必须大于 0，建议设置为 99 或 999".to_string());
            }

            // 优先级：AIO bundled > resources fallback > 系统安装 (WinGet/conda/brew/nix/PATH)
            let exe_path = if EngineInstaller::get_exe_path(&app).exists() {
                EngineInstaller::get_exe_path(&app)
            } else {
                let resource_dir = app
                    .path()
                    .resolve("resources/engines/llama-cpp", BaseDirectory::Resource)
                    .map_err(|e| format!("无法解析资源路径: {}", e))?;
                #[cfg(target_os = "windows")]
                let fallback = resource_dir.join("llama-server.exe");
                #[cfg(not(target_os = "windows"))]
                let fallback = resource_dir.join("llama-server");
                if fallback.exists() {
                    fallback
                } else if let Some(found) = Self::find_llama_server() {
                    found
                } else {
                    return Err("找不到 llama.cpp 引擎。请先在设置页面中安装引擎。".to_string());
                }
            };

            if !Path::new(model_path).exists() {
                return Err(format!("模型文件不存在: {}", model_path));
            }

            let mut cmd = self.build_command(&exe_path, model_path, port, gpu_layers);
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => return Err(format!("启动失败: {}", e)),
            };

            let _ = app.emit(self.progress_event_name(), 0.05);

            let stderr = match child.stderr.take() {
                Some(s) => s,
                None => return Err("无法获取子进程 stderr".to_string()),
            };
            let app_clone = app.clone();
            let event_name = self.progress_event_name().to_string();
            task::spawn_blocking(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    // 只在调试时记录子进程日志，避免泄露
                    debug!("[llama-server] {}", line);
                    let progress = if line.contains("build info") || line.contains("system info") {
                        Some(0.1)
                    } else if line.contains("loading model") {
                        Some(0.2)
                    } else if line.contains("model loaded") || line.contains("done") {
                        Some(0.5)
                    } else if line.contains("HTTP server listening")
                        || line.contains("listening on")
                    {
                        Some(0.8)
                    } else {
                        None
                    };
                    if let Some(p) = progress {
                        let _ = app_clone.emit(&event_name, p);
                    }
                }
            });

            sleep(Duration::from_millis(2000)).await;
            match child.try_wait() {
                Ok(None) => {}
                Ok(Some(status)) => {
                    return Err(format!("进程启动后立即退出，退出码: {}", status));
                }
                Err(e) => return Err(format!("无法检查进程状态: {}", e)),
            }

            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .connect_timeout(Duration::from_secs(2))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());
            let health_url = format!("http://127.0.0.1:{}/health", port);
            match client.get(&health_url).send().await {
                Ok(_) => {
                    let _ = app.emit(self.progress_event_name(), 1.0);
                }
                Err(_) => {
                    let _ = child.kill();
                    return Err("服务未响应健康检查，可能启动失败".to_string());
                }
            }

            state.lock().insert(
                self.identifier().to_string(),
                crate::core::state::LocalEngineInner {
                    child_process: Some(child),
                },
            );

            Ok(format!("http://127.0.0.1:{}/v1", port))
        })
    }
}

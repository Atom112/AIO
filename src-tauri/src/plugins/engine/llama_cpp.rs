/// llama.cpp local inference engine plugin implementation
///
/// Startup strategy:
/// 1. Prefer the engine auto-installed under app data (EngineInstaller)
/// 2. Fall back to a system-installed llama-server (PATH / WinGet / conda / brew / nix)
use crate::core::state::LocalEngineState;
use crate::ierr;
use crate::plugins::engine::installer::EngineInstaller;
use crate::plugins::engine::LocalEnginePlugin;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};
use tokio::task;
use tokio::time::{sleep, Duration};
use tracing::debug;

use crate::core::events as ev;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

pub struct LlamaCppPlugin;

impl LlamaCppPlugin {
    /// Search known locations for the `llama-server` executable, returning the first path found.
    /// Search order: PATH → per-platform package manager install directories.
    fn find_llama_server() -> Option<std::path::PathBuf> {
        // ── 1. PATH lookup ──
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

        // ── 3. Conda (all platforms) ──
        // Conda installs llama-server into $CONDA_PREFIX/bin (Unix) or %CONDA_PREFIX%\Library\bin (Win)
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

        // ── 6. Nix (all platforms) ──
        for nix_profile in Self::nix_profile_dirs() {
            let candidate = nix_profile.join("bin").join(exe_name);
            if candidate.exists() {
                return Some(candidate);
            }
        }

        None
    }

    /// Search a directory (non-recursively) for the given file name, returning the first matching full path.
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

    /// List of possible conda root directories.
    fn conda_roots() -> Vec<std::path::PathBuf> {
        let mut roots = Vec::new();
        // $CONDA_PREFIX environment variable (currently activated env)
        if let Ok(prefix) = std::env::var("CONDA_PREFIX") {
            roots.push(std::path::PathBuf::from(&prefix));
            // On Windows, CONDA_PREFIX may point to .../Library; use its parent directory
            #[cfg(target_os = "windows")]
            if prefix.ends_with("Library") {
                if let Some(parent) = std::path::Path::new(&prefix).parent() {
                    roots.push(parent.to_path_buf());
                }
            }
        }
        // Common conda install paths
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
        // Linux/macOS: system-level conda
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

    /// List of possible Homebrew prefixes (macOS / Linux).
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn homebrew_prefixes() -> Vec<std::path::PathBuf> {
        let mut prefixes = Vec::new();
        #[cfg(target_os = "macos")]
        {
            // Apple Silicon default
            prefixes.push(std::path::PathBuf::from("/opt/homebrew"));
            // Intel Mac default
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

    /// List of possible parent directories of a nix profile's bin.
    fn nix_profile_dirs() -> Vec<std::path::PathBuf> {
        let mut dirs = Vec::new();
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(std::path::PathBuf::from(&home).join(".nix-profile"));
        }
        dirs.push(std::path::PathBuf::from("/nix/var/nix/profiles/default"));
        dirs
    }

    /// Check whether llama-server exists anywhere on the system
    fn detect_on_path() -> bool {
        Self::find_llama_server().is_some()
    }

    /// Get the version string via `llama-server --version`
    fn get_version(app: &AppHandle) -> Option<String> {
        // Prefer the AIO bundled engine
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

    fn is_platform_supported(&self) -> bool {
        true // llama.cpp supports all platforms
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
        ev::LLAMA_PROGRESS
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
            // ENG-07: stdout piped 从无人读——64KB 管道填满后引擎静默挂死（长会话
            // 必现）；stderr 有人读保持 piped，stdout 改 null
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());

        #[cfg(target_os = "windows")]
        cmd.creation_flags(0x08000000);
        cmd
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
                return Err(ierr!(
                    "GPU 层数必须大于 0，建议设置为 99 或 999",
                    "GPU layers must be greater than 0, recommended values are 99 or 999"
                ));
            }

            // Priority: app data install > system install (PATH/WinGet/conda/brew/nix)
            let exe_path = if EngineInstaller::get_exe_path(&app).exists() {
                EngineInstaller::get_exe_path(&app)
            } else if let Some(found) = Self::find_llama_server() {
                found
            } else {
                return Err(ierr!(
                    "找不到 llama.cpp 引擎。请安装 llama-server 并确保其在 PATH 中，或参照设置页面的安装指南。",
                    "llama.cpp engine not found. Install llama-server and make sure it is on PATH, or follow the installation guide in Settings."
                ));
            };

            if !Path::new(model_path).exists() {
                return Err(ierr!(
                    "模型文件不存在: {}",
                    "Model file does not exist: {}",
                    model_path
                ));
            }

            let mut cmd = self.build_command(&exe_path, model_path, port, gpu_layers);
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => return Err(ierr!("启动失败: {}", "Failed to start: {}", e)),
            };

            let _ = app.emit(self.progress_event_name(), 0.05);

            let stderr = match child.stderr.take() {
                Some(s) => s,
                None => {
                    return Err(ierr!(
                        "无法获取子进程 stderr",
                        "Failed to capture child process stderr"
                    ))
                }
            };
            let app_clone = app.clone();
            let event_name = self.progress_event_name().to_string();
            task::spawn_blocking(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    // Log child process output only at debug level to avoid leaking it
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
                    return Err(ierr!(
                        "进程启动后立即退出，退出码: {}",
                        "Process exited immediately after startup, exit code: {}",
                        status
                    ));
                }
                Err(e) => {
                    return Err(ierr!(
                        "无法检查进程状态: {}",
                        "Failed to check process status: {}",
                        e
                    ))
                }
            }

            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .connect_timeout(Duration::from_secs(2))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());
            let health_url = format!("http://127.0.0.1:{}/health", port);
            // ENG-03: 轮询代替"固定等待 2s + 单发"——14B+ 模型加载普遍超过 7s，
            // 单发窗口内必然失败被误杀；健康判定要求 2xx（端口被占时旧服务
            // 的任意响应不再被当作健康）。对齐 ollama 插件的 30s 轮询范本。
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            loop {
                if std::time::Instant::now() > deadline {
                    // ENG-06: kill 后 wait，避免 Unix 僵尸进程
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(ierr!(
                        "服务启动超时（30 秒内健康检查未通过）",
                        "Service startup timed out (health check did not pass within 30s)"
                    ));
                }
                // 子进程先于健康检查退出：直接报错
                match child.try_wait() {
                    Ok(None) => {}
                    Ok(Some(status)) => {
                        return Err(ierr!(
                            "进程启动后立即退出，退出码: {}",
                            "Process exited immediately after startup, exit code: {}",
                            status
                        ));
                    }
                    Err(e) => {
                        return Err(ierr!(
                            "无法检查进程状态: {}",
                            "Failed to check process status: {}",
                            e
                        ))
                    }
                }
                if let Ok(resp) = client.get(&health_url).send().await {
                    if resp.status().is_success() {
                        let _ = app.emit(self.progress_event_name(), 1.0);
                        break;
                    }
                }
                sleep(Duration::from_millis(500)).await;
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

/// vLLM local inference engine plugin implementation
///
/// Note: vLLM officially supports Linux and macOS only; this engine option is
/// unavailable on Windows.
/// Startup strategy:
/// 1. Scan candidate Python interpreters and pick one with vllm installed
/// 2. Start the OpenAI-compatible server via python -m vllm.entrypoints.openai.api_server
use crate::core::state::LocalEngineState;
use crate::ierr;
use crate::plugins::engine::LocalEnginePlugin;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager};
use tokio::task;
use tokio::time::{sleep, Duration};
use tracing::debug;

use crate::core::events as ev;
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

/// Check whether the vllm package is installed in the given Python interpreter
fn check_vllm_installed(python: &str) -> bool {
    let mut cmd = create_progress_cmd(python, &["-c", "import vllm; print(vllm.__version__)"]);
    cmd.spawn()
        .and_then(|mut c| c.wait())
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Get the vLLM version string (using the resolved Python interpreter)
fn get_vllm_version(python: &str) -> Option<String> {
    let mut cmd = create_progress_cmd(python, &["-c", "import vllm; print(vllm.__version__)"]);
    match cmd.output() {
        Ok(output) => {
            let raw = String::from_utf8_lossy(&output.stdout);
            Some(raw.trim().to_string())
        }
        Err(_) => None,
    }
}

/// Run `python --version` to verify the interpreter works
fn python_runs(python: &str) -> bool {
    create_progress_cmd(python, &["--version"])
        .spawn()
        .and_then(|mut c| c.wait())
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Collect all candidate Python interpreter paths.
///
/// Search order: PATH names → VIRTUAL_ENV → system absolute paths → CONDA_PREFIX → HOME dirs (conda/venv/pyenv/pip --user).
/// Processes launched by the installer or desktop launcher may hold a stale PATH snapshot
/// (missing user-installed Python), so relying on `python` / `python3` from PATH alone
/// is not enough.
fn python_candidates() -> Vec<String> {
    let mut candidates: Vec<String> = Vec::new();

    // 1. PATH names
    for name in ["python3", "python"] {
        candidates.push(name.to_string());
    }

    // 2. VIRTUAL_ENV (user virtual env)
    if let Ok(venv) = std::env::var("VIRTUAL_ENV") {
        candidates.push(
            Path::new(&venv)
                .join("bin")
                .join("python")
                .display()
                .to_string(),
        );
    }

    // 3. system absolute paths
    #[cfg(target_os = "macos")]
    {
        candidates.push("/opt/homebrew/bin/python3".to_string());
        candidates.push("/usr/local/bin/python3".to_string());
        candidates.push("/usr/bin/python3".to_string());
    }
    #[cfg(target_os = "linux")]
    {
        candidates.push("/usr/local/bin/python3".to_string());
        candidates.push("/usr/bin/python3".to_string());
    }

    // 4. conda via CONDA_PREFIX
    if let Ok(prefix) = std::env::var("CONDA_PREFIX") {
        candidates.push(
            Path::new(&prefix)
                .join("bin")
                .join("python3")
                .display()
                .to_string(),
        );
    }

    // 5. HOME based dirs and extra fallbacks
    if let Ok(home) = std::env::var("HOME") {
        for name in ["miniconda3", "anaconda3", "miniforge3", "mambaforge"] {
            candidates.push(
                Path::new(&home)
                    .join(name)
                    .join("bin")
                    .join("python")
                    .display()
                    .to_string(),
            );
        }
        // ~/.venv
        candidates.push(
            Path::new(&home)
                .join(".venv")
                .join("bin")
                .join("python")
                .display()
                .to_string(),
        );
        // pyenv shims
        candidates.push(
            Path::new(&home)
                .join(".pyenv")
                .join("shims")
                .join("python3")
                .display()
                .to_string(),
        );
        candidates.push(
            Path::new(&home)
                .join(".pyenv")
                .join("shims")
                .join("python")
                .display()
                .to_string(),
        );
        // ~/.local/bin (pip install --user) — critical: the desktop launcher PATH lacks user install paths
        // The Ollama plugin falls back to this same path; see plugins/engine/ollama.rs
        candidates.push(
            Path::new(&home)
                .join(".local")
                .join("bin")
                .join("python3")
                .display()
                .to_string(),
        );
    }

    candidates
}

/// Filter the candidate list with the `verify` closure, returning the first matching Python path.
fn find_python_impl(verify: impl Fn(&str) -> bool) -> Result<String, String> {
    for python in python_candidates() {
        if verify(&python) {
            return Ok(python);
        }
    }
    Err(ierr!(
        "未找到 Python 运行时。请安装 Python 3.8+ 并确保已加入 PATH。",
        "Python runtime not found. Install Python 3.8+ and make sure it is on PATH."
    ))
}

/// Scan all candidate Pythons and return the first interpreter that **both** runs
/// **and** has vllm installed.
///
/// Trying each candidate (e.g. `~/.local/bin/python3`, conda env, pyenv shims) fixes
/// the "detected in dev, undetected in the packaged app" problem caused by the
/// incomplete PATH in desktop-launcher environments.
fn find_python_with_vllm() -> Result<String, String> {
    find_python_impl(|p| python_runs(p) && check_vllm_installed(p))
}

/// Check whether a vLLM service is already running on the default port 8000
/// (HTTP fallback, consistent with the ollama plugin: a running service counts
/// as the engine being available even when Python is not detectable from the
/// process environment)
fn vllm_http_reachable() -> bool {
    let url = "http://127.0.0.1:8000/health";
    std::thread::spawn(move || {
        reqwest::blocking::Client::new()
            .get(url)
            .timeout(Duration::from_secs(2))
            .send()
            .is_ok()
    })
    .join()
    .unwrap_or(false)
}

impl LocalEnginePlugin for VllmPlugin {
    fn name(&self) -> &'static str {
        "vLLM"
    }

    fn identifier(&self) -> &'static str {
        "vllm"
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
        // Scan all candidate Python interpreters; any one with vllm installed means ready.
        // Per-candidate probing works around the incomplete PATH of desktop launchers.
        // Fallback: a vLLM service already running on port 8000 (started manually by the user).
        find_python_with_vllm().is_ok() || vllm_http_reachable()
    }

    fn detect_installation(&self, _app: &AppHandle) -> crate::core::models::EngineInstallInfo {
        // Prefer find_python_with_vllm: scan all candidates for an interpreter that both runs and has vllm installed.
        // If none is found (no interpreter with vllm, or the HTTP fallback hit), fall back to service detection.
        let python = find_python_with_vllm().ok();
        let has_vllm = python.is_some();
        let installed = has_vllm || vllm_http_reachable();
        let version = if let Some(ref p) = python {
            get_vllm_version(p)
        } else {
            None
        };
        crate::core::models::EngineInstallInfo { installed, version }
    }

    fn default_port(&self) -> u16 {
        8000
    }

    fn progress_event_name(&self) -> &'static str {
        ev::ENGINE_PROGRESS
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

    fn start<'a>(
        &'a self,
        app: AppHandle,
        state: &'a LocalEngineState,
        model_path: &'a str,
        port: u16,
        gpu_layers: i32,
        trust_remote_code: bool,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + 'a>>
    {
        Box::pin(async move {
            debug!(
                "启动参数 - 引擎: vLLM, 模型: {}, 端口: {}, GPU层数: {}",
                model_path, port, gpu_layers
            );

            if !Path::new(model_path).exists() {
                return Err(ierr!(
                    "模型文件/目录不存在: {}",
                    "Model file/directory does not exist: {}",
                    model_path
                ));
            }
            let _ = app.emit(self.progress_event_name(), 0.02);

            // Step 1: try find_python_with_vllm() first — scan all candidates for one with vllm installed.
            // Verify each candidate (system Python may lack vllm while ~/.local/bin/python3, conda env, pyenv shims etc. have it).
            let python = match tokio::task::spawn_blocking(find_python_with_vllm)
                .await
                .map_err(|_| ierr!("python 检测线程 panic", "Python detection thread panicked"))?
            {
                Ok(p) => {
                    debug!("[vLLM] 找到已安装 vllm 的 Python: {}", p);
                    p
                }
                Err(_) => {
                    return Err(ierr!(
                        "未找到已安装 vllm 的 Python 解释器。\n\
                         请执行: pip install --user vllm\n\
                         或: pip install vllm\n\n\
                         注意：vLLM 仅支持 Linux/macOS，需要 CUDA 工具链支持。",
                        "No Python interpreter with vllm installed.\n\
                         Run: pip install --user vllm\n\
                         Or: pip install vllm\n\n\
                         Note: vLLM supports Linux/macOS only and requires the CUDA toolchain."
                    ));
                }
            };

            let _ = app.emit(self.progress_event_name(), 0.05);

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
                ],
            );
            if trust_remote_code {
                cmd.arg("--trust-remote-code");
            }

            let mut child = cmd.spawn().map_err(|e| {
                ierr!(
                    "启动 vLLM 失败: {}。\n请确保 Python 和 vllm 已正确安装 (pip install --user vllm)。",
                    "Failed to start vLLM: {}.\nMake sure Python and vllm are properly installed (pip install --user vllm).",
                    e
                )
            })?;

            let _ = app.emit(self.progress_event_name(), 0.2);

            let stderr = match child.stderr.take() {
                Some(s) => s,
                None => {
                    return Err(ierr!(
                        "无法获取 vLLM 子进程 stderr",
                        "Failed to capture vLLM child process stderr"
                    ))
                }
            };
            let app_clone = app.clone();
            let event_name = self.progress_event_name().to_string();
            task::spawn_blocking(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
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
            });

            sleep(Duration::from_secs(5)).await;
            match child.try_wait() {
                Ok(None) => {}
                Ok(Some(status)) => {
                    return Err(ierr!(
                        "vLLM 进程启动后立即退出，退出码: {}。\n常见原因：CUDA 不可用、模型路径错误、显存不足。",
                        "vLLM process exited immediately after startup, exit code: {}.\nCommon causes: CUDA unavailable, wrong model path, or insufficient VRAM.",
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
                .timeout(Duration::from_secs(15))
                .connect_timeout(Duration::from_secs(3))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());
            let health_url = format!("http://127.0.0.1:{}/health", port);

            match client.get(&health_url).send().await {
                Ok(_) => {
                    let _ = app.emit(self.progress_event_name(), 1.0);
                }
                Err(_) => {
                    let _ = child.kill();
                    return Err(ierr!(
                        "vLLM 服务未响应健康检查，可能启动失败。\n请检查：1) CUDA 工具链是否正确安装 2) 显存是否充足 3) 模型路径是否有效",
                        "vLLM service did not respond to the health check, startup may have failed.\nPlease check: 1) the CUDA toolchain is correctly installed 2) VRAM is sufficient 3) the model path is valid"
                    ));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn restore_env(key: &str, old: Option<std::ffi::OsString>) {
        match old {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    // The tests below read and write process-wide environment variables (HOME / CONDA_PREFIX / VIRTUAL_ENV);
    // Rust tests run in parallel and would race, causing flaky failures, so they are serialized with a global lock.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn find_python_prefers_path_name_when_working() {
        let result = find_python_impl(|c| c == "python3");
        assert_eq!(result, Ok("python3".to_string()));
    }

    #[test]
    fn find_python_falls_back_to_home_venv() {
        let tmp = std::env::temp_dir().join(format!("aio-vllm-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let _guard = ENV_LOCK.lock().unwrap();
        let old_home = std::env::var_os("HOME");
        std::env::set_var("HOME", &tmp);
        // Remove other vars to isolate
        let old_conda = std::env::var_os("CONDA_PREFIX");
        let old_venv = std::env::var_os("VIRTUAL_ENV");
        std::env::remove_var("CONDA_PREFIX");
        std::env::remove_var("VIRTUAL_ENV");

        let expected = tmp
            .join(".venv")
            .join("bin")
            .join("python")
            .display()
            .to_string();
        let result = find_python_impl(|c| c == expected);
        assert_eq!(result, Ok(expected));

        // restore env
        restore_env("HOME", old_home);
        match old_conda {
            Some(v) => std::env::set_var("CONDA_PREFIX", v),
            None => std::env::remove_var("CONDA_PREFIX"),
        }
        match old_venv {
            Some(v) => std::env::set_var("VIRTUAL_ENV", v),
            None => std::env::remove_var("VIRTUAL_ENV"),
        }
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn python_candidates_includes_local_bin() {
        let _guard = ENV_LOCK.lock().unwrap();
        // Verify ~/.local/bin/python3 is in the candidate list (only effective when HOME is set)
        if let Ok(home) = std::env::var("HOME") {
            let candidates = python_candidates();
            let expected = Path::new(&home)
                .join(".local")
                .join("bin")
                .join("python3")
                .display()
                .to_string();
            assert!(
                candidates.contains(&expected),
                "候选列表应包含 ~/.local/bin/python3 (pip install --user)，实际列表: {:?}",
                candidates
            );
        }
        // When HOME is unset this candidate is not added, so skip the assertion
    }

    #[test]
    fn python_candidates_starts_with_path_names() {
        let _guard = ENV_LOCK.lock().unwrap();
        // Verify the first two entries of the candidate list are PATH names
        let old_conda = std::env::var_os("CONDA_PREFIX");
        let old_venv = std::env::var_os("VIRTUAL_ENV");
        std::env::remove_var("CONDA_PREFIX");
        std::env::remove_var("VIRTUAL_ENV");

        let candidates = python_candidates();
        assert_eq!(candidates[0], "python3", "第一个候选应为 python3");
        assert_eq!(candidates[1], "python", "第二个候选应为 python");

        restore_env("CONDA_PREFIX", old_conda);
        restore_env("VIRTUAL_ENV", old_venv);
    }

    #[test]
    fn find_python_with_vllm_returns_err_when_not_installed() {
        let _guard = ENV_LOCK.lock().unwrap();
        // When no Python has vllm installed, this should return Err (not panic).
        // Use a nonexistent HOME path to isolate from system environment interference.
        let old_home = std::env::var_os("HOME");
        let old_conda = std::env::var_os("CONDA_PREFIX");
        let old_venv = std::env::var_os("VIRTUAL_ENV");
        std::env::set_var("HOME", "/nonexistent-aaaa");
        std::env::remove_var("CONDA_PREFIX");
        std::env::remove_var("VIRTUAL_ENV");

        let result = find_python_with_vllm();
        assert!(result.is_err(), "无 vllm 时应返回 Err: {:?}", result);

        restore_env("HOME", old_home);
        restore_env("CONDA_PREFIX", old_conda);
        restore_env("VIRTUAL_ENV", old_venv);
    }
}

/// Ollama local inference engine plugin.
///
/// Supports GGUF model files: once the user picks a .gguf file, the plugin ensures the
/// Ollama service is running, imports the model into Ollama, and returns the API Base URL.
///
/// Process lifecycle: if Ollama was already started by the user (systemd / launchd / docker),
/// the plugin reuses that service; otherwise it spawns an `ollama serve` child process.
use crate::core::state::LocalEngineState;
use crate::ierr;
use crate::plugins::engine::LocalEnginePlugin;
use std::future::Future;
use std::io::Write;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;
use tauri::AppHandle;
use tracing::{debug, warn};

/// Locate the `ollama` executable via PATH, returning the first hit.
fn ollama_on_path() -> Option<PathBuf> {
    // SEC-06: canonical PATH-only resolver (replaces the `where`/`which` subprocess
    // probe, whose bare-name spawn also consulted the cwd on Windows).
    crate::utils::exec_resolve::resolve_executable("ollama")
}

/// Search common install directories for the `ollama` executable.
///
/// Fallback for environments where PATH is stale: processes launched by the installer/
/// updater, autostart, or elevation may hold an old environment snapshot lacking the
/// user PATH entry the Ollama installer wrote (dev terminals have the fresh env, hence
/// the "visible in dev, invisible in the installed app" discrepancy).
fn ollama_in_known_dirs() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let candidates: Vec<PathBuf> = {
        let mut v = Vec::new();
        // Ollama Windows installer defaults to %LOCALAPPDATA%\Programs\Ollama (per-user)
        if let Ok(dir) = std::env::var("LOCALAPPDATA") {
            v.push(
                PathBuf::from(dir)
                    .join("Programs")
                    .join("Ollama")
                    .join("ollama.exe"),
            );
        }
        // Machine-level install (older versions or custom install location)
        if let Ok(dir) = std::env::var("PROGRAMFILES") {
            v.push(PathBuf::from(dir).join("Ollama").join("ollama.exe"));
        }
        if let Ok(dir) = std::env::var("PROGRAMFILES(X86)") {
            v.push(PathBuf::from(dir).join("Ollama").join("ollama.exe"));
        }
        v
    };
    #[cfg(target_os = "macos")]
    let candidates: Vec<PathBuf> = vec![
        // Ollama macOS bundle always lives inside /Applications/Ollama.app
        PathBuf::from("/Applications/Ollama.app/Contents/Resources/ollama"),
    ];
    #[cfg(target_os = "linux")]
    let candidates: Vec<PathBuf> = {
        let mut v = vec![
            PathBuf::from("/usr/local/bin/ollama"),
            PathBuf::from("/usr/bin/ollama"),
            PathBuf::from("/opt/ollama/bin/ollama"),
        ];
        if let Ok(home) = std::env::var("HOME") {
            v.push(
                PathBuf::from(home)
                    .join(".local")
                    .join("bin")
                    .join("ollama"),
            );
        }
        v
    };
    candidates.into_iter().find(|p| p.exists())
}

/// Resolve the `ollama` executable path: PATH first, then common install directories.
fn resolve_ollama_exe() -> Option<PathBuf> {
    ollama_on_path().or_else(ollama_in_known_dirs)
}

/// Windows: the Ollama installer registers a user service named `ollama`; its existence counts as installed (even when stopped).
#[cfg(target_os = "windows")]
fn ollama_service_exists() -> bool {
    // SEC-06: PATH-only resolution — sc.exe lives in System32.
    let Some(sc) = crate::utils::exec_resolve::resolve_executable("sc") else {
        return false;
    };
    std::process::Command::new(sc)
        .args(["query", "ollama"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(0x08000000)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
fn ollama_service_exists() -> bool {
    false
}

/// macOS: the Ollama installer writes a login launch agent; its existence counts as installed (even when not running).
#[cfg(target_os = "macos")]
fn ollama_launch_agent_exists() -> bool {
    std::env::var("HOME")
        .map(|h| {
            PathBuf::from(h)
                .join("Library")
                .join("LaunchAgents")
                .join("com.ollama.ollama.plist")
                .exists()
        })
        .unwrap_or(false)
}

#[cfg(not(target_os = "macos"))]
fn ollama_launch_agent_exists() -> bool {
    false
}

/// Check whether the Ollama HTTP endpoint at host:port is reachable (blocking, for trait methods)
/// ENG-12: requires a 2xx response — the historic `.send().is_ok()` counted ANY HTTP
/// answer (404/502 from a stray proxy on 11434) as "Ollama running", so the engine
/// reported installed/alive when it wasn't (same class as the ENG-03 llama/vLLM fix).
fn ollama_http_reachable_at(host: &str, port: u16) -> bool {
    let url = format!("http://{}:{}/api/version", host, port);
    std::thread::spawn(move || {
        reqwest::blocking::Client::new()
            .get(&url)
            .timeout(Duration::from_secs(2))
            .send()
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    })
    .join()
    .unwrap_or(false)
}

/// Async variant: check whether the Ollama HTTP endpoint is reachable (for polling inside start())
/// ENG-12: same 2xx requirement as the blocking variant.
async fn ollama_http_reachable_async(host: &str, port: u16) -> bool {
    let url = format!("http://{}:{}/api/version", host, port);
    reqwest::Client::new()
        .get(&url)
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Check whether the default port 11434 is reachable
fn ollama_http_reachable() -> bool {
    ollama_http_reachable_at("127.0.0.1", 11434)
}

/// Get the version string via `ollama --version`
fn ollama_version_from_cli() -> Option<String> {
    let exe = resolve_ollama_exe()?;
    let mut cmd = std::process::Command::new(exe);
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
            raw.lines()
                .next()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        }
        Err(_) => None,
    }
}

/// Derive the model name from the GGUF file path (filename without extension)
fn model_name_from_path(model_path: &str) -> String {
    Path::new(model_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("aio-model")
        .to_string()
}

/// Import a GGUF file into Ollama via `ollama create` (async, to avoid blocking the tokio runtime).
/// Returns the model name; skips the import when the model already exists.
async fn import_model(model_path: &str, api_url: &str) -> Result<String, String> {
    let model_name = model_name_from_path(model_path);

    // Check whether the model already exists in ollama (must check the HTTP status code, not just is_ok)
    let check_url = format!("{}/api/show", api_url);
    let body = serde_json::json!({ "name": model_name });
    let client = reqwest::Client::new();
    let resp = client
        .post(&check_url)
        .timeout(Duration::from_secs(3))
        .json(&body)
        .send()
        .await;
    if let Ok(r) = resp {
        if r.status().is_success() {
            debug!("[ollama] 模型 {} 已存在，跳过导入", model_name);
            return Ok(model_name);
        }
    }

    debug!("[ollama] 导入 GGUF 模型: {} -> {}", model_path, model_name);

    // Write a temporary Modelfile
    let tmpdir = std::env::temp_dir();
    let modelfile_path = tmpdir.join(format!("aio-ollama-{}.Modelfile", model_name));
    let mut f = std::fs::File::create(&modelfile_path).map_err(|e| {
        ierr!(
            "无法创建 Modelfile: {}",
            "Failed to create Modelfile: {}",
            e
        )
    })?;
    writeln!(f, "FROM {}", model_path)
        .map_err(|e| ierr!("无法写入 Modelfile: {}", "Failed to write Modelfile: {}", e))?;
    drop(f);

    // Run ollama create (async, to avoid blocking the runtime); use the resolved real path to survive PATH-less environments
    let exe = resolve_ollama_exe().unwrap_or_else(|| PathBuf::from("ollama"));
    let output = tokio::process::Command::new(exe)
        .arg("create")
        .arg(&model_name)
        .arg("-f")
        .arg(&modelfile_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| {
            ierr!(
                "无法执行 ollama create: {}",
                "Failed to run ollama create: {}",
                e
            )
        })?;

    // Clean up the Modelfile
    let _ = std::fs::remove_file(&modelfile_path);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ierr!(
            "ollama create 失败: {}",
            "ollama create failed: {}",
            stderr.trim()
        ));
    }

    debug!("[ollama] 模型 {} 导入成功", model_name);
    Ok(model_name)
}

pub struct OllamaPlugin;

impl LocalEnginePlugin for OllamaPlugin {
    fn name(&self) -> &'static str {
        "Ollama"
    }

    fn identifier(&self) -> &'static str {
        "ollama"
    }

    /// Ollama uses the root URL (no /v1 suffix); model listing goes through /api/tags
    fn default_api_url(&self, port: u16) -> String {
        format!("http://localhost:{port}")
    }

    fn models_url(&self, api_url: &str) -> String {
        format!("{}/api/tags", api_url.trim_end_matches('/'))
    }

    /// Ollama response shape: { "models": [ { "name": ... } ] }
    fn parse_model_ids(&self, body: &serde_json::Value) -> Vec<String> {
        body["models"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m["name"].as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Ollama is an externally installed system service; model paths are managed by Ollama itself and skip sandbox validation
    fn is_external(&self) -> bool {
        true
    }

    fn install_path(&self, _app: &AppHandle) -> PathBuf {
        PathBuf::new()
    }

    fn is_installed(&self, _app: &AppHandle) -> bool {
        resolve_ollama_exe().is_some()
            || ollama_service_exists()
            || ollama_launch_agent_exists()
            || ollama_http_reachable()
    }

    fn detect_installation(&self, _app: &AppHandle) -> crate::core::models::EngineInstallInfo {
        let exe = resolve_ollama_exe();
        let http_ok = ollama_http_reachable();
        let installed =
            exe.is_some() || ollama_service_exists() || ollama_launch_agent_exists() || http_ok;
        let version = if exe.is_some() {
            ollama_version_from_cli()
        } else {
            None
        };
        crate::core::models::EngineInstallInfo { installed, version }
    }

    fn default_port(&self) -> u16 {
        11434
    }

    fn build_command(
        &self,
        exe_path: &Path,
        _model_path: &str,
        port: u16,
        _gpu_layers: i32,
    ) -> std::process::Command {
        let mut cmd = std::process::Command::new(exe_path);
        // ENG-07: serve 进程的双管道此前均 piped 且无人读——ollama serve 日志量大，
        // 64KB 管道填满后引擎静默挂死（长会话必现）。日志本就无人消费，改 null。
        cmd.arg("serve")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        if port != 11434 {
            cmd.env("OLLAMA_HOST", format!("127.0.0.1:{}", port));
        }
        #[cfg(target_os = "windows")]
        cmd.creation_flags(0x08000000);
        cmd
    }

    fn start<'a>(
        &'a self,
        _app: AppHandle,
        state: &'a LocalEngineState,
        model_path: &'a str,
        port: u16,
        gpu_layers: i32,
        _trust_remote_code: bool,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>> {
        Box::pin(async move {
            let api_url = format!("http://127.0.0.1:{}", port);
            debug!(
                "[ollama] 启动 - 模型: {}, 端口: {}, GPU层数: {}",
                model_path, port, gpu_layers
            );

            if !Path::new(model_path).exists() {
                return Err(ierr!(
                    "模型文件不存在: {}",
                    "Model file does not exist: {}",
                    model_path
                ));
            }

            // 1. Ensure the Ollama service is running
            let already_running = ollama_http_reachable_async("127.0.0.1", port).await;
            if !already_running {
                debug!("[ollama] 服务未在端口 {} 运行，尝试拉起 ollama serve", port);
                // Use the resolved real path (falls back to common install dirs when PATH is missing)
                let exe_path = resolve_ollama_exe().unwrap_or_else(|| PathBuf::from("ollama"));
                let mut cmd = self.build_command(&exe_path, model_path, port, gpu_layers);
                let child = cmd.spawn().map_err(|e| {
                    ierr!(
                        "启动 ollama serve 失败: {}",
                        "Failed to start ollama serve: {}",
                        e
                    )
                })?;

                // Register the child process in state so the stop command can manage it
                {
                    let mut engines = state.lock();
                    let inner = engines.entry(self.identifier().to_string()).or_default();
                    inner.child_process = Some(child);
                }

                // Wait for ollama to become ready (poll /api/version)
                let deadline = std::time::Instant::now() + Duration::from_secs(30);
                loop {
                    if std::time::Instant::now() > deadline {
                        return Err(ierr!(
                            "Ollama 服务启动超时",
                            "Ollama service startup timed out"
                        ));
                    }
                    if ollama_http_reachable_async("127.0.0.1", port).await {
                        debug!("[ollama] 服务就绪");
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            } else {
                debug!("[ollama] 服务已在运行，复用现有实例");
            }

            // 2. Import the GGUF model (if not imported yet)
            let model_name = match import_model(model_path, &api_url).await {
                Ok(name) => name,
                Err(e) => {
                    warn!("[ollama] 模型导入失败: {}", e);
                    return Err(e);
                }
            };

            debug!("[ollama] 模型 {} 已就绪，API: {}", model_name, api_url);
            Ok(api_url)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test: the installed app may hold a stale PATH snapshot (without the
    /// Ollama directory); detection must fall back to common install directories.
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[test]
    fn detects_ollama_via_known_dirs_when_path_is_stale() {
        let tmp = std::env::temp_dir().join(format!("aio-ollama-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();

        #[cfg(target_os = "windows")]
        {
            let exe = tmp.join("Programs").join("Ollama").join("ollama.exe");
            std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
            std::fs::write(&exe, "").unwrap();
            let old = std::env::var_os("LOCALAPPDATA");
            std::env::set_var("LOCALAPPDATA", &tmp);
            let found = ollama_in_known_dirs();
            assert_eq!(found, Some(exe));
            restore_env("LOCALAPPDATA", old);
        }

        #[cfg(target_os = "linux")]
        {
            let exe = tmp.join(".local").join("bin").join("ollama");
            std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
            std::fs::write(&exe, "").unwrap();
            let old = std::env::var_os("HOME");
            std::env::set_var("HOME", &tmp);
            let found = ollama_in_known_dirs();
            assert_eq!(found, Some(exe));
            restore_env("HOME", old);
        }

        std::fs::remove_dir_all(&tmp).ok();
    }

    fn restore_env(key: &str, old: Option<std::ffi::OsString>) {
        match old {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }
}

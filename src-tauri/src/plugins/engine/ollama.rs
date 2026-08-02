/// Ollama 本地推理引擎插件。
///
/// 支持 GGUF 模型文件：用户选择 .gguf 文件后，插件负责确保 Ollama 服务运行、
/// 将模型导入 Ollama，并返回 API Base URL。
///
/// 进程生命周期：若 Ollama 已由用户自行启动（systemd / launchd / docker），
/// 插件直接使用现有服务；否则自动拉起 ollama serve 子进程。
use crate::core::state::LocalEngineState;
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

/// 通过 PATH 查找 `ollama` 可执行文件（Windows `where` / Unix `which`），返回第一个命中路径。
fn ollama_on_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let output = std::process::Command::new("where")
            .arg("ollama")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .creation_flags(0x08000000)
            .output()
            .ok()?;
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let p = PathBuf::from(stdout.lines().next()?.trim());
            if p.exists() {
                return Some(p);
            }
        }
        None
    }
    #[cfg(not(target_os = "windows"))]
    {
        let output = std::process::Command::new("which")
            .arg("ollama")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        if output.status.success() {
            let p = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim().to_string());
            if p.exists() {
                return Some(p);
            }
        }
        None
    }
}

/// 在常见安装目录中查找 `ollama` 可执行文件。
///
/// 兜底 PATH 未生效的环境：安装包/更新器拉起、开机自启、提权启动等场景下，
/// 进程环境变量可能是旧快照，不含 Ollama 安装器写入的用户 PATH 条目（dev 调试时
/// 终端环境是最新的，因此会出现"dev 能识别、安装包识别不到"的差异）。
fn ollama_in_known_dirs() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let candidates: Vec<PathBuf> = {
        let mut v = Vec::new();
        // Ollama Windows 安装器默认安装到 %LOCALAPPDATA%\Programs\Ollama（用户级）
        if let Ok(dir) = std::env::var("LOCALAPPDATA") {
            v.push(
                PathBuf::from(dir)
                    .join("Programs")
                    .join("Ollama")
                    .join("ollama.exe"),
            );
        }
        // 管理员级安装（旧版本或自定义安装位置）
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
        // Ollama macOS 安装包固定在 /Applications/Ollama.app 内
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

/// 解析 `ollama` 可执行文件路径：PATH 优先，其次常见安装目录。
fn resolve_ollama_exe() -> Option<PathBuf> {
    ollama_on_path().or_else(ollama_in_known_dirs)
}

/// Windows: Ollama 安装器会注册名为 `ollama` 的用户服务；服务存在即视为已安装（即使当前停止）。
#[cfg(target_os = "windows")]
fn ollama_service_exists() -> bool {
    std::process::Command::new("sc")
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

/// macOS: Ollama 安装包会写入登录启动代理；存在即视为已安装（即使当前未运行）。
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

/// 检查指定 host:port 的 Ollama HTTP 端点是否可达（同步，供 trait 方法使用）
fn ollama_http_reachable_at(host: &str, port: u16) -> bool {
    let url = format!("http://{}:{}/api/version", host, port);
    std::thread::spawn(move || {
        reqwest::blocking::Client::new()
            .get(&url)
            .timeout(Duration::from_secs(2))
            .send()
            .is_ok()
    })
    .join()
    .unwrap_or(false)
}

/// 异步版本：检查 Ollama HTTP 端点是否可达（供 start() 内部轮询使用）
async fn ollama_http_reachable_async(host: &str, port: u16) -> bool {
    let url = format!("http://{}:{}/api/version", host, port);
    reqwest::Client::new()
        .get(&url)
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .is_ok()
}

/// 检查默认端口 11434 是否可达
fn ollama_http_reachable() -> bool {
    ollama_http_reachable_at("127.0.0.1", 11434)
}

/// 通过 `ollama --version` 获取版本号
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

/// 从 GGUF 文件路径派生模型名（取文件名去扩展名）
fn model_name_from_path(model_path: &str) -> String {
    Path::new(model_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("aio-model")
        .to_string()
}

/// 通过 ollama create 将 GGUF 文件导入 Ollama（异步，避免阻塞 tokio runtime）。
/// 返回模型名；若模型已存在则跳过导入。
async fn import_model(model_path: &str, api_url: &str) -> Result<String, String> {
    let model_name = model_name_from_path(model_path);

    // 检查模型是否已存在于 ollama（必须检查 HTTP 状态码，不能仅靠 is_ok）
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

    // 写入临时 Modelfile
    let tmpdir = std::env::temp_dir();
    let modelfile_path = tmpdir.join(format!("aio-ollama-{}.Modelfile", model_name));
    let mut f =
        std::fs::File::create(&modelfile_path).map_err(|e| format!("无法创建 Modelfile: {}", e))?;
    writeln!(f, "FROM {}", model_path).map_err(|e| format!("无法写入 Modelfile: {}", e))?;
    drop(f);

    // 执行 ollama create（异步，避免阻塞 runtime）—— 使用解析出的真实路径，规避 PATH 缺失环境
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
        .map_err(|e| format!("无法执行 ollama create: {}", e))?;

    // 清理 Modelfile
    let _ = std::fs::remove_file(&modelfile_path);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ollama create 失败: {}", stderr.trim()));
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

    fn supported_extensions(&self) -> &[&'static str] {
        &["gguf"]
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
        cmd.arg("serve")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
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
                return Err(format!("模型文件不存在: {}", model_path));
            }

            // 1. 确保 Ollama 服务运行
            let already_running = ollama_http_reachable_async("127.0.0.1", port).await;
            if !already_running {
                debug!("[ollama] 服务未在端口 {} 运行，尝试拉起 ollama serve", port);
                // 用解析出的真实路径（PATH 缺失环境下兜底常见安装目录）
                let exe_path = resolve_ollama_exe().unwrap_or_else(|| PathBuf::from("ollama"));
                let mut cmd = self.build_command(&exe_path, model_path, port, gpu_layers);
                let child = cmd
                    .spawn()
                    .map_err(|e| format!("启动 ollama serve 失败: {}", e))?;

                // 注册子进程到状态，供 stop 命令管理
                {
                    let mut engines = state.lock();
                    let inner = engines.entry(self.identifier().to_string()).or_default();
                    inner.child_process = Some(child);
                }

                // 等待 ollama 就绪（轮询 /api/version）
                let deadline = std::time::Instant::now() + Duration::from_secs(30);
                loop {
                    if std::time::Instant::now() > deadline {
                        return Err("Ollama 服务启动超时".to_string());
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

            // 2. 导入 GGUF 模型（若尚未导入）
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

    fn parse_progress_from_log(&self, line: &str) -> Option<f64> {
        if line.contains("listening") || line.contains("Listening") {
            Some(0.3)
        } else if line.contains("creating") || line.contains("importing") {
            Some(0.5)
        } else if line.contains("success") || line.contains("ready") {
            Some(0.9)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归测试：安装包环境可能持有旧 PATH 快照（不含 Ollama 目录），
    /// 检测必须通过常见安装目录兜底命中，而不是只依赖 PATH。
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

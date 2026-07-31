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

/// 检查指定 Python 解释器中是否已安装 vllm 包
fn check_vllm_installed(python: &str) -> bool {
    let mut cmd = create_progress_cmd(python, &["-c", "import vllm; print(vllm.__version__)"]);
    cmd.spawn()
        .and_then(|mut c| c.wait())
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 获取 vLLM 版本号（使用解析出的 Python 解释器）
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

/// 执行 `python --version` 验证解释器可运行
fn python_runs(python: &str) -> bool {
    create_progress_cmd(python, &["--version"])
        .spawn()
        .and_then(|mut c| c.wait())
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 收集所有候选 Python 解释器路径列表。
///
/// 搜索策略：PATH 名称 → VIRTUAL_ENV → 系统绝对路径 → CONDA_PREFIX → HOME 目录（conda/venv/pyenv/pip --user）。
/// 安装包/桌面启动器拉起的进程可能持有旧 PATH 快照（不含用户自定义 Python 安装），
/// 因此不能只依赖 PATH 上的 `python` / `python3`。
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
        // ~/.local/bin (pip install --user) — 关键：桌面启动器 PATH 不含用户安装路径
        // Ollama 插件同样兜底此路径，见 plugins/engine/ollama.rs
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

/// 使用 `verify` 闭包过滤候选列表，返回第一个匹配的 Python 路径。
fn find_python_impl(verify: impl Fn(&str) -> bool) -> Result<String, String> {
    for python in python_candidates() {
        if verify(&python) {
            return Ok(python);
        }
    }
    Err("未找到 Python 运行时。请安装 Python 3.8+ 并确保已加入 PATH。".to_string())
}

/// 返回第一个能运行的 Python 解释器路径。
fn find_python() -> Result<String, String> {
    find_python_impl(python_runs)
}

/// 扫描所有候选 Python，返回第一个**同时满足**「能运行」且「已安装 vllm」的解释器。
///
/// 与 `find_python()` 的区别：后者找到第一个能跑的解释器就返回（可能是系统 Python 不含 vllm），
/// 而本函数继续尝试后续候选（如 `~/.local/bin/python3`、conda env、pyenv shims），
/// 解决桌面启动器环境下 PATH 不完整导致的 "dev 能识别、打包后识别不到" 问题。
fn find_python_with_vllm() -> Result<String, String> {
    for python in python_candidates() {
        if python_runs(&python) && check_vllm_installed(&python) {
            return Ok(python);
        }
    }
    Err("未找到已安装 vllm 的 Python 解释器。请执行: pip install --user vllm".to_string())
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

    #[test]
    fn find_python_prefers_path_name_when_working() {
        let result = find_python_impl(|c| c == "python3");
        assert_eq!(result, Ok("python3".to_string()));
    }

    #[test]
    fn find_python_falls_back_to_home_venv() {
        let tmp = std::env::temp_dir().join(format!("aio-vllm-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let old_home = std::env::var_os("HOME");
        std::env::set_var("HOME", &tmp);
        // Remove other vars to isolate
        let old_conda = std::env::var_os("CONDA_PREFIX");
        let old_venv = std::env::var_os("VIRTUAL_ENV");
        std::env::remove_var("CONDA_PREFIX");
        std::env::remove_var("VIRTUAL_ENV");

        let expected = tmp.join(".venv").join("bin").join("python").display().to_string();
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
        // 验证 ~/.local/bin/python3 在候选列表中（HOME 设置了才生效）
        // 使用当前进程的 HOME 而非设置临时 HOME，避免并行测试的 env 竞争
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
        // HOME 未设置时，此候选不加入列表，跳过断言
    }

    #[test]
    fn python_candidates_starts_with_path_names() {
        // 验证候选列表的前两个条目是 PATH 名称
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
        // 当没有任何 Python 安装了 vllm 时，应返回 Err（而非 panic）
        // 使用一个不存在的路径作为 HOME 来隔离系统环境干扰
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

/// 检查 vLLM 服务是否已在默认端口 8000 运行（HTTP 兜底，与 ollama 插件一致：
/// 服务在跑即视为引擎可用，即使进程环境探测不到 Python）
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
        args.push(
            w.to_str()
                .ok_or_else(|| format!("无效的 wheel 路径: {:?}", w))?,
        );
    }

    debug!("[vLLM] 正在从 bundled .whl 安装 vllm...");
    let mut cmd = create_progress_cmd(python, &args);
    let status = cmd
        .spawn()
        .map_err(|e| format!("pip install 启动失败: {}", e))?
        .wait()
        .map_err(|e| format!("pip install 执行失败: {}", e))?;

    if !status.success() {
        return Err(
            "pip install vllm 失败。请检查 Python 环境和 CUDA 工具链是否正确安装。".to_string(),
        );
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
        // 扫描所有候选 Python 解释器，只要有一个已安装 vllm 即视为已就绪。
        // 相比旧版 find_python() + check_vllm_installed，try_all 解决桌面启动器 PATH 不完整问题。
        // 兜底：8000 端口已有 vLLM 服务在运行（用户手动启动的情况）。
        find_python_with_vllm().is_ok() || vllm_http_reachable()
    }

    fn detect_installation(&self, _app: &AppHandle) -> crate::core::models::EngineInstallInfo {
        // 优先使用 find_python_with_vllm：扫描所有候选，返回同时能运行且已安装 vllm 的解释器。
        // 若找不到（解释器未安装 vllm 或 HTTP 兜底成功），退回 fallback 检测。
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
        trust_remote_code: bool,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + 'a>>
    {
        Box::pin(async move {
            debug!(
                "启动参数 - 引擎: vLLM, 模型: {}, 端口: {}, GPU层数: {}",
                model_path, port, gpu_layers
            );

            if !Path::new(model_path).exists() {
                return Err(format!("模型文件/目录不存在: {}", model_path));
            }
            let _ = app.emit(self.progress_event_name(), 0.02);

            // Step 1: 优先尝试 find_python_with_vllm() — 扫描所有候选，找到已安装 vllm 的解释器。
            // 这比先 find_python() 再 check_vllm_installed 更可靠：桌面启动器环境下 PATH 不完整，
            // 系统 Python 可能不含 vllm，但 ~/.local/bin/python3、conda env、pyenv shims 等仍有。
            let python = match tokio::task::spawn_blocking(find_python_with_vllm)
                .await
                .map_err(|_| "python 检测线程 panic".to_string())?
            {
                Ok(p) => {
                    debug!("[vLLM] 找到已安装 vllm 的 Python: {}", p);
                    p
                }
                Err(_) => {
                    // Step 2: fallback — 找任何一个能跑的 Python + 从 bundled .whl 安装
                    debug!("[vLLM] 未找到已安装 vllm 的 Python，尝试从 bundled .whl 安装");
                    let fallback_python = tokio::task::spawn_blocking(find_python)
                        .await
                        .map_err(|_| "python 检测线程 panic".to_string())??;

                    let resource_dir = app
                        .path()
                        .resolve("resources/engines/vllm", BaseDirectory::Resource)
                        .map_err(|e| format!("无法解析资源路径: {}", e))?;

                    let wheels = find_bundled_wheels(&resource_dir);

                    if wheels.is_empty() {
                        return Err(
                            "未找到已安装 vllm 的 Python 解释器，且无 bundled .whl 可供自动安装。\n\
                             请执行: pip install --user vllm\n\
                             或: pip install vllm\n\n\
                             注意：vLLM 仅支持 Linux/macOS，需要 CUDA 工具链支持。"
                                .to_string(),
                        );
                    }

                    let _ = app.emit(self.progress_event_name(), 0.1);
                    let wheels_clone = wheels.clone();
                    let python_clone = fallback_python.clone();
                    tokio::task::spawn_blocking(move || {
                        install_from_wheels(&python_clone, &wheels_clone)
                    })
                    .await
                    .map_err(|_| "pip install 线程 panic".to_string())??;

                    fallback_python
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
                format!(
                    "启动 vLLM 失败: {}。\n请确保 Python 和 vllm 已正确安装 (pip install --user vllm)。",
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

            match client.get(&health_url).send().await {
                Ok(_) => {
                    let _ = app.emit(self.progress_event_name(), 1.0);
                }
                Err(_) => {
                    let _ = child.kill();
                    return Err("vLLM 服务未响应健康检查，可能启动失败。\n请检查：1) CUDA 工具链是否正确安装 2) 显存是否充足 3) 模型路径是否有效".to_string());
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

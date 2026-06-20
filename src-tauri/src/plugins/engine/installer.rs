/// 引擎安装管理器
/// 负责自动从 GitHub Releases 下载并安装 llama.cpp 引擎
///
/// 策略：
/// 1. 查询 GitHub API 获取最新 release 信息
/// 2. 根据当前平台 + GPU 情况自动选择对应的 asset
/// 3. 流式下载（支持进度回调）
/// 4. 解压到 app data 目录
/// 5. 记录版本信息，支持版本对比和更新

use std::io::Write;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// GitHub release 信息
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub published_at: String,
    pub assets: Vec<AssetInfo>,
}

/// Release 中的单个 asset
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct AssetInfo {
    pub name: String,
    pub size: u64,
    pub browser_download_url: String,
}

/// 引擎安装状态（返回给前端）
#[derive(serde::Serialize, Clone, Debug)]
pub struct EngineStatus {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub version: Option<String>,
    pub latest_version: Option<String>,
    pub platform_supported: bool,
    pub error: Option<String>,
}

/// 引擎更新信息
#[derive(serde::Serialize, Clone, Debug)]
pub struct EngineUpdateInfo {
    pub id: String,
    pub current_version: Option<String>,
    pub latest_version: String,
    pub has_update: bool,
}

/// 本地版本记录（存储在 app data 中）
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
struct VersionInfo {
    tag: String,
    installed_at: String,
}

const LLAMA_CPP_OWNER: &str = "ggml-org";
const LLAMA_CPP_REPO: &str = "llama.cpp";

/// 引擎安装管理器
pub struct EngineInstaller;

impl EngineInstaller {
    /// 获取 llama.cpp 引擎的安装目录
    pub fn get_engine_dir(app: &AppHandle) -> PathBuf {
        let mut path = app
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| PathBuf::from("."));
        path.push("engines");
        path.push("llama-cpp");
        path
    }

    /// 获取引擎可执行文件路径
    pub fn get_exe_path(app: &AppHandle) -> PathBuf {
        #[cfg(target_os = "windows")]
        {
            Self::get_engine_dir(app).join("llama-server.exe")
        }
        #[cfg(target_os = "linux")]
        {
            Self::get_engine_dir(app).join("llama-server")
        }
        #[cfg(target_os = "macos")]
        {
            Self::get_engine_dir(app).join("llama-server")
        }
    }

    /// 查询 GitHub 最新 release
    pub async fn fetch_latest_release() -> Result<ReleaseInfo, String> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/releases/latest",
            LLAMA_CPP_OWNER, LLAMA_CPP_REPO
        );

        let client = reqwest::Client::builder()
            .user_agent("AIO-App/0.3.1")
            .build()
            .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("请求 GitHub API 失败: {} (可能是网络问题)", e))?;

        if !resp.status().is_success() {
            return Err(format!(
                "GitHub API 返回异常: HTTP {} (请求频率限制: 1小时内最多60次)",
                resp.status()
            ));
        }

        let raw: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("解析 GitHub 响应失败: {}", e))?;

        let tag_name = raw["tag_name"]
            .as_str()
            .ok_or("无法获取 release tag")?
            .to_string();

        let published_at = raw["published_at"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();

        let mut assets = Vec::new();
        if let Some(arr) = raw["assets"].as_array() {
            for a in arr {
                assets.push(AssetInfo {
                    name: a["name"].as_str().unwrap_or("").to_string(),
                    size: a["size"].as_u64().unwrap_or(0),
                    browser_download_url: a["browser_download_url"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                });
            }
        }

        Ok(ReleaseInfo {
            tag_name,
            published_at,
            assets,
        })
    }

    /// 根据当前平台 + GPU 选择最合适的 asset
    pub fn select_asset(release: &ReleaseInfo) -> Option<&AssetInfo> {
        let assets = &release.assets;

        #[cfg(target_os = "windows")]
        {
            // 检查是否有 NVIDIA GPU
            let has_nvidia = Self::check_nvidia_gpu();

            if has_nvidia {
                // 优先 CUDA 12.4（最稳定兼容），回退到 13.3
                assets
                    .iter()
                    .find(|a| {
                        a.name.contains("cudart")
                            && a.name.contains("win")
                            && a.name.contains("cuda-12.4")
                    })
                    .or_else(|| {
                        assets.iter().find(|a| {
                            a.name.contains("cudart")
                                && a.name.contains("win")
                                && a.name.contains("cuda-13.3")
                        })
                    })
                    .or_else(|| {
                        assets
                            .iter()
                            .find(|a| a.name.contains("cudart") && a.name.contains("win"))
                    })
            } else {
                // 无 NVIDIA GPU → 选 AVX2 版本（纯 CPU）
                assets
                    .iter()
                    .find(|a| a.name.contains("avx2") && a.name.contains("win"))
                    .or_else(|| {
                        assets
                            .iter()
                            .find(|a| a.name.contains("win") && a.name.ends_with(".zip"))
                    })
            }
        }

        #[cfg(target_os = "linux")]
        {
            let has_nvidia = Self::check_nvidia_gpu();
            if has_nvidia {
                assets
                    .iter()
                    .find(|a| a.name.contains("cudart") && a.name.contains("ubuntu"))
            } else {
                assets
                    .iter()
                    .find(|a| a.name.contains("linux") && a.name.contains("avx2"))
                    .or_else(|| {
                        assets
                            .iter()
                            .find(|a| a.name.contains("linux") && a.name.ends_with(".zip"))
                    })
            }
        }

        #[cfg(target_os = "macos")]
        {
            assets
                .iter()
                .find(|a| a.name.contains("macos") && a.name.contains("arm64"))
                .or_else(|| {
                    assets
                        .iter()
                        .find(|a| a.name.contains("macos") && a.name.ends_with(".zip"))
                })
        }
    }

    /// 检测 NVIDIA GPU 是否可用
    fn check_nvidia_gpu() -> bool {
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("nvidia-smi")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .and_then(|mut c| c.wait())
                .map(|s| s.success())
                .unwrap_or(false)
        }
        #[cfg(not(target_os = "windows"))]
        {
            std::process::Command::new("nvidia-smi")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .and_then(|mut c| c.wait())
                .map(|s| s.success())
                .unwrap_or(false)
        }
    }

    /// 获取已安装的版本
    pub fn get_installed_version(app: &AppHandle) -> Option<String> {
        let version_file = Self::get_engine_dir(app).join("version.json");
        if !version_file.exists() {
            return None;
        }
        std::fs::read_to_string(version_file)
            .ok()
            .and_then(|s| serde_json::from_str::<VersionInfo>(&s).ok())
            .map(|v| v.tag)
    }

    /// 检查引擎是否已安装（exe 和 version.json 都存在）
    pub fn is_installed(app: &AppHandle) -> bool {
        Self::get_exe_path(app).exists() && Self::get_installed_version(app).is_some()
    }

    /// 下载并安装引擎
    /// 通过 `on_progress` 回调报告进度 (0.0 ~ 1.0)
    pub async fn install(
        app: &AppHandle,
        on_progress: impl Fn(f64) + Send + 'static,
    ) -> Result<String, String> {
        // 1. 查询最新 release
        on_progress(0.01);
        let release = Self::fetch_latest_release().await?;
        let tag = release.tag_name.clone();
        on_progress(0.05);

        // 2. 选择 asset
        let asset = Self::select_asset(&release).ok_or_else(|| {
            format!(
                "未找到当前平台 ({}) 对应的下载文件",
                std::env::consts::OS
            )
        })?;
        on_progress(0.1);

        // 3. 创建目标目录
        let engine_dir = Self::get_engine_dir(app);
        std::fs::create_dir_all(&engine_dir)
            .map_err(|e| format!("创建引擎目录失败: {}", e))?;

        // 4. 下载到临时文件
        let temp_dir = std::env::temp_dir().join(format!("aio-llama-{}", tag));
        if temp_dir.exists() {
            let _ = std::fs::remove_dir_all(&temp_dir);
        }
        std::fs::create_dir_all(&temp_dir)
            .map_err(|e| format!("创建临时目录失败: {}", e))?;

        let zip_path = temp_dir.join(&asset.name);
        let fallback_progress = on_progress;

        // 流式下载
        let client = reqwest::Client::builder()
            .user_agent("AIO-App/0.3.1")
            .build()
            .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

        let resp = client
            .get(&asset.browser_download_url)
            .send()
            .await
            .map_err(|e| format!("下载请求失败: {} (请检查网络连接)", e))?;

        if !resp.status().is_success() {
            return Err(format!("下载失败: HTTP {}", resp.status()));
        }

        let total_size = resp.content_length().unwrap_or(0);
        let mut downloaded: u64 = 0;

        let mut file = std::fs::File::create(&zip_path)
            .map_err(|e| format!("创建临时文件失败: {}", e))?;

        let mut stream = resp.bytes_stream();
        use futures_util::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("下载数据流中断: {}", e))?;
            file.write_all(&chunk)
                .map_err(|e| format!("写入临时文件失败: {}", e))?;
            downloaded += chunk.len() as u64;

            // 报告下载进度 (10% ~ 70%)
            if total_size > 0 {
                let p = 0.1 + (downloaded.min(total_size) as f64 / total_size as f64) * 0.6;
                fallback_progress(p.min(0.7));
            }
        }
        drop(file);
        fallback_progress(0.72);

        // 5. 解压到引擎目录
        let zip_file =
            std::fs::File::open(&zip_path).map_err(|e| format!("打开下载的 ZIP 失败: {}", e))?;
        let mut archive = zip::ZipArchive::new(zip_file)
            .map_err(|e| format!("解压 ZIP 失败 (文件可能损坏): {}", e))?;

        // 计算需要提取的文件总数（用于解压进度）
        let total_files = archive.len();
        // 先清空旧文件（如有）
        if engine_dir.exists() {
            let _ = std::fs::remove_dir_all(&engine_dir);
        }
        std::fs::create_dir_all(&engine_dir)
            .map_err(|e| format!("创建引擎目录失败: {}", e))?;

        for i in 0..total_files {
            let mut entry = archive.by_index(i).map_err(|e| {
                format!("读取 ZIP 条目失败: {}", e)
            })?;

            // 安全路径检查：使用 zip crate 的 enclosed_name() 自动拒绝绝对路径和 .. 段
            let safe_name = match entry.enclosed_name() {
                Some(p) => p.to_path_buf(),
                None => {
                    return Err(format!(
                        "ZIP 条目包含非法路径 (绝对路径或路径逃逸): {:?}",
                        entry.name()
                    ));
                }
            };

            // 去除顶层目录前缀（与原逻辑保持一致）
            let relative_name = match safe_name.iter().next() {
                Some(_first) => safe_name.iter().skip(1).collect::<std::path::PathBuf>(),
                None => continue,
            };

            if relative_name.as_os_str().is_empty() {
                continue; // 目录条目
            }

            // 二次校验：解析后路径必须落在 engine_dir 内
            let dest_path = engine_dir.join(&relative_name);
            let canonical_engine = std::fs::canonicalize(&engine_dir)
                .unwrap_or_else(|_| engine_dir.clone());
            if let Ok(canonical_dest) = std::fs::canonicalize(&dest_path) {
                if !canonical_dest.starts_with(&canonical_engine) {
                    return Err(format!(
                        "ZIP 条目路径逃逸被拦截: {:?}",
                        relative_name
                    ));
                }
            } else {
                // 目标文件尚不存在，通过 components 检查防 .. 段
                let mut normalized = std::path::PathBuf::new();
                for comp in relative_name.components() {
                    match comp {
                        std::path::Component::Normal(c) => normalized.push(c),
                        std::path::Component::ParentDir => {
                            return Err(format!(
                                "ZIP 条目含 ParentDir 段: {:?}",
                                relative_name
                            ));
                        }
                        _ => {
                            return Err(format!(
                                "ZIP 条目含非 Normal 段: {:?}",
                                relative_name
                            ));
                        }
                    }
                }
                let dest_path = engine_dir.join(&normalized);
                let dest_str = dest_path.to_string_lossy().to_string();

                if let Some(parent) = dest_path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("创建目录 {} 失败: {}", parent.display(), e))?;
                }

                let mut outfile = std::fs::File::create(&dest_path)
                    .map_err(|e| format!("创建文件 {} 失败: {}", dest_str, e))?;
                std::io::copy(&mut entry, &mut outfile)
                    .map_err(|e| format!("写入文件 {} 失败: {}", dest_str, e))?;

                #[cfg(not(target_os = "windows"))]
                {
                    if dest_path.extension().is_none() || dest_path.extension().unwrap() == "" {
                        let _ = std::fs::set_permissions(
                            &dest_path,
                            std::fs::Permissions::from_mode(0o755),
                        );
                    }
                }

                let p = 0.72 + (i as f64 / total_files as f64) * 0.18;
                fallback_progress(p.min(0.9));
                continue;
            }

            let dest_str = dest_path.to_string_lossy().to_string();

            if let Some(parent) = dest_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("创建目录 {} 失败: {}", parent.display(), e))?;
            }

            let mut outfile = std::fs::File::create(&dest_path)
                .map_err(|e| format!("创建文件 {} 失败: {}", dest_str, e))?;
            std::io::copy(&mut entry, &mut outfile)
                .map_err(|e| format!("写入文件 {} 失败: {}", dest_str, e))?;

            #[cfg(not(target_os = "windows"))]
            {
                if dest_path.extension().is_none() || dest_path.extension().unwrap() == "" {
                    let _ = std::fs::set_permissions(
                        &dest_path,
                        std::fs::Permissions::from_mode(0o755),
                    );
                }
            }

            // 解压进度 (72% ~ 90%)
            let p = 0.72 + (i as f64 / total_files as f64) * 0.18;
            fallback_progress(p.min(0.9));
        }

        fallback_progress(0.92);

        // 6. 写入版本信息
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let version_info = VersionInfo {
            tag: tag.clone(),
            installed_at: now.to_string(),
        };
        let version_json =
            serde_json::to_string_pretty(&version_info).map_err(|e| format!("序列化版本信息失败: {}", e))?;
        std::fs::write(engine_dir.join("version.json"), version_json)
            .map_err(|e| format!("写入版本信息失败: {}", e))?;

        // 7. 清理临时文件
        let _ = std::fs::remove_dir_all(&temp_dir);

        fallback_progress(1.0);

        Ok(tag)
    }

    /// 检查是否有新版本
    pub async fn check_update(app: &AppHandle) -> Result<EngineUpdateInfo, String> {
        let current = Self::get_installed_version(app);
        let release = Self::fetch_latest_release().await?;
        let latest = release.tag_name;

        let has_update = match &current {
            Some(cur) => {
                // 解析版本号比较 (b9673 vs b9600)
                Self::compare_versions(cur, &latest)
            }
            None => true,
        };

        Ok(EngineUpdateInfo {
            id: "llama_cpp".to_string(),
            current_version: current,
            latest_version: latest,
            has_update,
        })
    }

    /// 比较两个版本号 (bXXXX)
    fn compare_versions(current: &str, latest: &str) -> bool {
        let cur_num = current
            .trim_start_matches('b')
            .parse::<u64>()
            .unwrap_or(0);
        let lat_num = latest
            .trim_start_matches('b')
            .parse::<u64>()
            .unwrap_or(0);
        lat_num > cur_num
    }
}

// 为非 Windows 平台添加 PermissionsExt
#[cfg(not(target_os = "windows"))]
use std::os::unix::fs::PermissionsExt;

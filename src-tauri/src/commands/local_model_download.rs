//! Local GGUF model downloader (local_model_* download family): fetches model files from
//! ModelScope / HuggingFace into a user-configurable install directory, with pause /
//! resume (HTTP Range) / cancel, progress events, disk-space pre-check and automatic
//! migration (files + provider-config path rewrite) when the directory changes.
//!
//! Unlike the embedding downloader (static compile-time manifest), the file list arrives
//! at runtime from the market UI. Source fallback per file: ModelScope -> hf-mirror.com
//! -> huggingface.co (the optional HF token from the secure store is attached to both HF
//! endpoints; reqwest drops it on cross-host redirects, which is fine because the signed
//! CDN targets need no auth).
//!
//! Downloaded models are registered by the FRONTEND (provider localModelPaths +
//! enabledModels) once the command returns the entry file path; this module owns files only.

use crate::core::events as ev;
use crate::ierr;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, Window};
use tokio::io::AsyncWriteExt;

use crate::commands::local_model_market::MarketFile;

/// One file to download (repo-relative path; name may contain subdirectories).
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelFileSpec {
    pub name: String,
    /// Exact byte size from the market API (0 = unknown; skipped in the space pre-check)
    pub size: u64,
}

/// Download request: one quant-family (all shards of one quant) of one model.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelDownloadRequest {
    /// Install folder name (quant family, e.g. "Qwen3-14B-Q4_K_M")
    pub id: String,
    pub hf_repo: Option<String>,
    pub ms_repo: Option<String>,
    /// Files in order; the first is the engine entry (first shard for split GGUFs)
    pub files: Vec<LocalModelFileSpec>,
    /// Optional per-provider proxy (e.g. `http://127.0.0.1:7890`) for hard-to-reach sources
    #[serde(default)]
    pub proxy_url: Option<String>,
    /// Download source preference: "auto" (default: ModelScope -> hf-mirror -> huggingface.co),
    /// "modelscope" (ModelScope only) or "huggingface" (hf-mirror -> huggingface.co)
    #[serde(default)]
    pub source: Option<String>,
}
/// Download outcome: status = ok (finished) | paused (checkpoint kept) | cancelled
/// (checkpoint deleted). `entryPath` is the first file's absolute path on ok.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelDownloadOutcome {
    pub status: String,
    pub id: String,
    pub entry_path: Option<String>,
}

/// One installed model found under the install directory.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InstalledLocalModel {
    pub id: String,
    pub files: Vec<MarketFile>,
    /// Entry file (first shard) for the engine / provider registration
    pub entry_path: String,
    pub size_bytes: u64,
}

/// Control flags for an in-flight download (registered per id).
#[derive(Clone)]
struct DownloadControl {
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
}

fn download_controls() -> &'static Mutex<std::collections::HashMap<String, DownloadControl>> {
    static CONTROLS: OnceLock<Mutex<std::collections::HashMap<String, DownloadControl>>> =
        OnceLock::new();
    CONTROLS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

fn signal_download(id: &str, set: impl Fn(&DownloadControl)) -> bool {
    match download_controls().lock() {
        Ok(map) => match map.get(id) {
            Some(c) => {
                set(c);
                true
            }
            None => false,
        },
        Err(_) => false,
    }
}

/// Default install directory for local models: `<app_data_dir>/models/local`.
pub fn default_local_models_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("models")
        .join("local")
}

/// Actual install directory (customizable in settings; empty config falls back to default).
pub fn local_models_dir(app: &AppHandle) -> PathBuf {
    let cfg = crate::core::config::load_app_config(app).unwrap_or_default();
    let dir = cfg.local_models_dir.trim();
    if dir.is_empty() {
        default_local_models_dir(app)
    } else {
        PathBuf::from(dir)
    }
}

/// Builds a download client (streaming downloads: no total timeout, 30s connect timeout,
/// optional proxy).
fn download_client(proxy_url: Option<&str>) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(30));
    if let Some(p) = proxy_url.filter(|s| !s.trim().is_empty()) {
        let proxy = reqwest::Proxy::all(p.trim())
            .map_err(|e| ierr!("代理 URL 非法: {}", "Invalid proxy URL: {}", e))?;
        builder = builder.proxy(proxy);
    }
    builder.build().map_err(|e| e.to_string())
}

/// One candidate source for a file.
struct SourceCandidate {
    name: &'static str,
    url: String,
    token: Option<String>,
}

fn source_candidates(
    req: &LocalModelDownloadRequest,
    file: &str,
    hf_token: Option<String>,
) -> Vec<SourceCandidate> {
    let encoded = crate::commands::local_model_market::encode_repo_path(file);
    let source = req
        .source
        .as_deref()
        .unwrap_or("auto")
        .trim()
        .to_lowercase();
    let mut out = Vec::new();
    if source != "huggingface" {
        if let Some(repo) = req.ms_repo.as_deref().filter(|r| !r.trim().is_empty()) {
            let repo = crate::commands::local_model_market::encode_repo_path(repo.trim());
            out.push(SourceCandidate {
                name: "modelscope",
                url: format!("https://modelscope.cn/models/{repo}/resolve/master/{encoded}"),
                token: None,
            });
        }
    }
    if source != "modelscope" {
        if let Some(repo) = req.hf_repo.as_deref().filter(|r| !r.trim().is_empty()) {
            let repo = crate::commands::local_model_market::encode_repo_path(repo.trim());
            for (name, base) in [
                ("hf-mirror", "https://hf-mirror.com"),
                ("huggingface", "https://huggingface.co"),
            ] {
                out.push(SourceCandidate {
                    name,
                    url: format!("{base}/{repo}/resolve/main/{encoded}"),
                    token: hf_token.clone(),
                });
            }
        }
    }
    out
}

/// Available space (bytes) on the filesystem containing `dir` (None = unknown).
/// Windows canonicalize returns `\\?\` extended-length paths; strip it before matching
/// against sysinfo mount points ("C:\").
fn available_space(dir: &Path) -> Option<u64> {
    use sysinfo::Disks;
    let dir_str = std::fs::canonicalize(dir)
        .map(|p| p.to_string_lossy().to_lowercase())
        .unwrap_or_else(|_| dir.to_string_lossy().to_lowercase())
        .replace("\\\\?\\", "");
    let disks = Disks::new_with_refreshed_list();
    let mut best: Option<(usize, u64)> = None;
    for disk in disks.list() {
        let mount = disk
            .mount_point()
            .to_string_lossy()
            .to_lowercase()
            .trim_end_matches('\\')
            .to_string();
        if dir_str == mount || dir_str.starts_with(&format!("{mount}\\")) {
            let len = mount.len();
            if best.as_ref().is_none_or(|(l, _)| len > *l) {
                best = Some((len, disk.available_space()));
            }
        }
    }
    best.map(|(_, space)| space)
}

/// Downloads all requested files. Ends in ok / paused / cancelled; network and write
/// errors keep the `.part` checkpoint (resumed next time). Progress events:
/// `local-model-download-progress` `{id, file, downloaded, total, source}` + `{id, done}`
/// at the end.
#[tauri::command]
pub async fn local_model_download(
    window: Window,
    app: AppHandle,
    request: LocalModelDownloadRequest,
) -> Result<LocalModelDownloadOutcome, String> {
    let id = request.id.trim().to_string();
    if id.is_empty() {
        return Err(ierr!("下载 ID 不能为空", "Download id must not be empty"));
    }
    if request.files.is_empty() {
        return Err(ierr!("文件列表为空", "File list is empty"));
    }
    let mut request = request;
    request.id = id.clone();

    let dir = local_models_dir(&app).join(&request.id);
    std::fs::create_dir_all(&dir).map_err(|e| {
        ierr!(
            "创建模型目录失败: {}",
            "Failed to create model directory: {}",
            e
        )
    })?;

    // Disk-space pre-check (only for known-size, not-yet-downloaded files)
    let pending: u64 = request
        .files
        .iter()
        .filter(|f| !dir.join(&f.name).exists())
        .map(|f| f.size)
        .sum();
    if pending > 0 {
        if let Some(avail) = available_space(&dir) {
            if pending > avail {
                return Err(ierr!(
                    "磁盘空间不足：需要 {}，{} 仅剩 {}（更换目录或清理空间）",
                    "Insufficient disk space: need {}, only {} left at {} (change the directory or free space)",
                    format_bytes(pending),
                    format_bytes(avail),
                    dir.display()
                ));
            }
        }
    }

    let control = DownloadControl {
        cancel: Arc::new(AtomicBool::new(false)),
        pause: Arc::new(AtomicBool::new(false)),
    };
    if let Ok(mut map) = download_controls().lock() {
        map.insert(id.clone(), control.clone());
    }
    struct UnregisterGuard(String);
    impl Drop for UnregisterGuard {
        fn drop(&mut self) {
            if let Ok(mut map) = download_controls().lock() {
                map.remove(&self.0);
            }
        }
    }
    let _guard = UnregisterGuard(id.clone());

    let client = download_client(request.proxy_url.as_deref())?;
    let hf_token =
        crate::core::secure_store::get(&app, &crate::core::secure_store::accounts::hf_token())
            .ok()
            .flatten();

    for file in &request.files {
        let dest = dir.join(&file.name);
        if dest.exists() {
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                ierr!(
                    "创建模型目录失败: {}",
                    "Failed to create model directory: {}",
                    e
                )
            })?;
        }
        let tmp = dest.with_extension("gguf.part");
        let mut resume_from = tokio::fs::metadata(&tmp)
            .await
            .map(|m| m.len())
            .unwrap_or(0);

        // Source fallback: try every candidate; connect errors / 404 / 5xx move to the next.
        let mut response: Option<(String, reqwest::Response)> = None;
        let mut last_err = String::new();
        for cand in source_candidates(&request, &file.name, hf_token.clone()) {
            let mut attempt_from = resume_from;
            let mut tries = 0;
            while tries < 2 {
                tries += 1;
                let mut req = client.get(&cand.url);
                if attempt_from > 0 {
                    req = req.header(reqwest::header::RANGE, format!("bytes={attempt_from}-"));
                }
                if let Some(t) = cand.token.as_deref() {
                    req = req.bearer_auth(t);
                }
                match req.send().await {
                    Ok(resp) => {
                        let status = resp.status();
                        if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
                            // Corrupt checkpoint larger than the remote file: restart clean
                            let _ = tokio::fs::remove_file(&tmp).await;
                            attempt_from = 0;
                            resume_from = 0;
                            continue; // retry this source from scratch
                        }
                        if status == reqwest::StatusCode::UNAUTHORIZED {
                            last_err = ierr!(
                                "{} 需要授权：受限仓库需先在网页上同意协议并在设置中填入 HuggingFace Token",
                                "{} requires authorization: accept the license on the website and configure a HuggingFace token for gated repos",
                                cand.name
                            );
                            break;
                        }
                        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                            last_err = ierr!(
                                "{} 触发限流，请稍后重试或更换下载源",
                                "{} rate-limited the request, retry later or switch the source",
                                cand.name
                            );
                            break;
                        }
                        if !status.is_success() {
                            last_err = format!("HTTP {status}");
                            break; // next candidate
                        }
                        if status != reqwest::StatusCode::PARTIAL_CONTENT {
                            resume_from = 0; // server ignored Range -> full restart
                        }
                        response = Some((cand.name.to_string(), resp));
                        break;
                    }
                    Err(e) => {
                        last_err = e.to_string();
                        break; // next candidate
                    }
                }
            }
            if response.is_some() {
                break;
            }
        }
        let (source, resp) = response.ok_or_else(|| {
            ierr!(
                "下载 {} 失败（已尝试所有来源）: {}",
                "Failed to download {} (all sources tried): {}",
                file.name,
                last_err
            )
        })?;

        let total = resume_from + resp.content_length().unwrap_or(0);
        let mut stream = resp.bytes_stream();
        let mut out = if resume_from > 0 {
            tokio::fs::OpenOptions::new().append(true).open(&tmp).await
        } else {
            tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp)
                .await
        }
        .map_err(|e| {
            ierr!(
                "创建临时文件失败: {}",
                "Failed to create temporary file: {}",
                e
            )
        })?;
        let mut downloaded: u64 = resume_from;
        let mut interrupted: Option<&'static str> = None;
        let mut last_emit: Option<Instant> = None;
        use futures_util::StreamExt as _;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| ierr!("下载中断: {}", "Download interrupted: {}", e))?;
            out.write_all(&chunk)
                .await
                .map_err(|e| ierr!("写入失败: {}", "Write failed: {}", e))?;
            downloaded += chunk.len() as u64;
            if last_emit.is_none_or(|t| t.elapsed() >= Duration::from_millis(200)) {
                last_emit = Some(Instant::now());
                let _ = window.emit(
                    ev::LOCAL_MODEL_DOWNLOAD_PROGRESS,
                    serde_json::json!({
                        "id": request.id,
                        "file": file.name,
                        "downloaded": downloaded,
                        "total": total,
                        "source": source,
                    }),
                );
            }
            if control.cancel.load(Ordering::Relaxed) {
                interrupted = Some("cancelled");
                break;
            }
            if control.pause.load(Ordering::Relaxed) {
                interrupted = Some("paused");
                break;
            }
        }
        out.flush()
            .await
            .map_err(|e| ierr!("落盘失败: {}", "Failed to flush to disk: {}", e))?;
        drop(out);
        match interrupted {
            Some("cancelled") => {
                let _ = tokio::fs::remove_file(&tmp).await;
                return Ok(LocalModelDownloadOutcome {
                    status: "cancelled".into(),
                    id: request.id,
                    entry_path: None,
                });
            }
            Some("paused") => {
                return Ok(LocalModelDownloadOutcome {
                    status: "paused".into(),
                    id: request.id,
                    entry_path: None,
                });
            }
            _ => {
                std::fs::rename(&tmp, &dest)
                    .map_err(|e| ierr!("重命名失败: {}", "Failed to rename: {}", e))?;
            }
        }
    }
    let entry = dir
        .join(request.files[0].name.as_str())
        .to_string_lossy()
        .to_string();
    // Record the market marker so the installed-list scan recognizes this directory as a
    // market-managed model (recursive scan + exact entry file, even for nested layouts)
    write_model_marker(&dir, &request.files[0].name);
    let _ = window.emit(
        ev::LOCAL_MODEL_DOWNLOAD_PROGRESS,
        serde_json::json!({ "id": request.id, "done": true }),
    );
    Ok(LocalModelDownloadOutcome {
        status: "ok".into(),
        id: request.id,
        entry_path: Some(entry),
    })
}

/// Pauses an in-flight local model download. `id`: install folder name. Returns whether a
/// download was in flight.
#[tauri::command]
pub fn local_model_pause_download(id: String) -> bool {
    signal_download(&id, |c| c.pause.store(true, Ordering::Relaxed))
}

/// Cancels a local model download (deletes the checkpoint; with no active download,
/// cleans up `.part` files under the model's directory directly).
#[tauri::command]
pub fn local_model_cancel_download(app: AppHandle, id: String) -> Result<bool, String> {
    let active = signal_download(&id, |c| c.cancel.store(true, Ordering::Relaxed));
    if !active {
        let dir = local_models_dir(&app).join(id.trim());
        if dir.is_dir() {
            remove_part_files(&dir);
        }
    }
    Ok(active)
}

/// Marker file written on download completion: distinguishes market-managed model
/// directories from foreign subdirectories during the installed-list scan (a foreign
/// dir qualifies only when it directly contains `*.gguf` files, never via deep nesting).
const MODEL_MARKER: &str = ".aio-model.json";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelMarker {
    /// Repo-relative entry file (first shard) recorded at download time
    entry_file: String,
}

fn write_model_marker(dir: &Path, entry_file: &str) {
    let marker = ModelMarker {
        entry_file: entry_file.to_string(),
    };
    if let Ok(bytes) = serde_json::to_vec(&marker) {
        let _ = std::fs::write(dir.join(MODEL_MARKER), bytes);
    }
}

/// Reads a model directory's marker (None when absent or unreadable).
fn read_model_marker(dir: &Path) -> Option<ModelMarker> {
    let bytes = std::fs::read(dir.join(MODEL_MARKER)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Deletes an installed local model (removes its install directory, or the loose
/// `.gguf` file itself when the id refers to a top-level file).
#[tauri::command]
pub fn local_model_delete(app: AppHandle, id: String) -> Result<(), String> {
    let path = local_models_dir(&app).join(id.trim());
    if path.is_dir() {
        std::fs::remove_dir_all(&path)
            .map_err(|e| ierr!("删除模型失败: {}", "Failed to delete model: {}", e))?;
    } else if path.is_file() {
        std::fs::remove_file(&path)
            .map_err(|e| ierr!("删除模型失败: {}", "Failed to delete model: {}", e))?;
    }
    Ok(())
}

/// Lists installed local models by scanning the install directory for GGUF content:
/// - a top-level loose `*.gguf` file counts as a single-file model;
/// - a subdirectory counts only when the market wrote a marker into it (recursive
///   scan, marker's entry file) or when it DIRECTLY contains `*.gguf` files (no
///   recursion) — foreign directories (caches, tool folders with deeply nested
///   models) are never mistaken for downloaded models.
#[tauri::command]
pub fn local_model_installed_list(app: AppHandle) -> Result<Vec<InstalledLocalModel>, String> {
    let root = local_models_dir(&app);
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(&root) {
        Ok(e) => e,
        Err(_) => return Ok(out),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_file() {
            // Top-level loose GGUF: a single-file model
            if !name.to_lowercase().ends_with(".gguf") {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let id = name
                .strip_suffix(".gguf")
                .or_else(|| name.strip_suffix(".GGUF"))
                .map(str::to_string)
                .unwrap_or_else(|| name.clone());
            out.push(InstalledLocalModel {
                id,
                files: vec![MarketFile {
                    name: name.clone(),
                    size,
                }],
                entry_path: path.to_string_lossy().to_string(),
                size_bytes: size,
            });
            continue;
        }
        if !path.is_dir() {
            continue;
        }
        // Model directory: marker (market install, recursive) or direct GGUF files only
        let marker = read_model_marker(&path);
        let mut files: Vec<MarketFile> = Vec::new();
        if marker.is_some() {
            collect_gguf_files(&path, &path, &mut files);
        } else {
            collect_gguf_files_shallow(&path, &mut files);
        }
        if files.is_empty() {
            continue;
        }
        files.sort_by(|a, b| a.name.cmp(&b.name));
        let entry_rel = marker
            .and_then(|m| {
                let rel = m.entry_file.replace('\\', "/");
                files.iter().any(|f| f.name == rel).then_some(rel)
            })
            .unwrap_or_else(|| files[0].name.clone());
        let size_bytes = files.iter().map(|f| f.size).sum();
        out.push(InstalledLocalModel {
            id: name,
            files,
            entry_path: path.join(entry_rel).to_string_lossy().to_string(),
            size_bytes,
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// Current local model install directory.
#[tauri::command]
pub fn local_models_get_dir(app: AppHandle) -> Result<String, String> {
    Ok(local_models_dir(&app).to_string_lossy().to_string())
}

/// Sets the local model install directory (empty `dir` restores the default). Downloaded
/// models are migrated to the new directory, and every provider's localModelPaths entry
/// pointing into the old directory is rewritten (best-effort, failures are logged only).
#[tauri::command]
pub fn local_models_set_dir(app: AppHandle, dir: String) -> Result<String, String> {
    let old = local_models_dir(&app);
    let mut cfg = crate::core::config::load_app_config(&app)
        .map_err(|e| ierr!("读取配置失败: {}", "Failed to read config: {}", e))?;
    cfg.local_models_dir = dir.trim().to_string();
    crate::commands::config::save_app_config(app.clone(), cfg)
        .map_err(|e| ierr!("保存配置失败: {}", "Failed to save config: {}", e))?;
    let new = local_models_dir(&app);
    if !same_path(&old, &new) {
        migrate_local_models(&app, &old, &new);
    }
    Ok(new.to_string_lossy().to_string())
}

// ===== helpers =====

fn format_bytes(n: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    if n as f64 >= GB {
        format!("{:.1} GB", n as f64 / GB)
    } else {
        format!("{:.0} MB", n as f64 / MB)
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(a) == canon(b)
}

/// Case-insensitive prefix match (`p` under `base`); returns the path below base.
fn path_under(p: &str, base: &Path) -> Option<String> {
    let pl = p.replace('/', "\\");
    let bl = base
        .to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_string();
    let plb = pl.as_bytes();
    let blb = bl.as_bytes();
    if plb.len() > blb.len()
        && plb[..blb.len()].eq_ignore_ascii_case(blb)
        && plb[blb.len()] == b'\\'
    {
        Some(pl[blb.len() + 1..].to_string())
    } else {
        None
    }
}

/// Collects `*.gguf` files DIRECTLY inside `dir` (no recursion). Foreign directories
/// qualify as models only through this shallow scan, so caches/tool folders with
/// deeply nested models are never mistaken for downloaded models.
fn collect_gguf_files_shallow(dir: &Path, out: &mut Vec<MarketFile>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false)
            && name.to_lowercase().ends_with(".gguf")
        {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            out.push(MarketFile { name, size });
        }
    }
}

/// Recursively collects `*.gguf` files under `root`, returning repo-relative names + sizes.
/// Only used for market-managed directories (marker present).
fn collect_gguf_files(base: &Path, dir: &Path, out: &mut Vec<MarketFile>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_gguf_files(base, &path, out);
        } else if entry
            .file_name()
            .to_string_lossy()
            .to_lowercase()
            .ends_with(".gguf")
        {
            let name = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            out.push(MarketFile { name, size });
        }
    }
}

/// Removes `*.part` checkpoint files under `dir` (recursive).
fn remove_part_files(dir: &Path) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            remove_part_files(&path);
        } else if entry
            .file_name()
            .to_string_lossy()
            .to_lowercase()
            .ends_with(".part")
        {
            let _ = std::fs::remove_file(&path);
        }
    }
}
/// Moves every model directory from `old` to `new` (rename first, cross-volume copy
/// fallback), then rewrites provider localModelPaths entries under `old` to `new`.
fn migrate_local_models(app: &AppHandle, old: &Path, new: &Path) {
    if old.is_dir() {
        if let Err(e) = std::fs::create_dir_all(new) {
            tracing::warn!(
                "[local-models] 迁移目标目录创建失败（{}）: {e}",
                new.display()
            );
        } else {
            for entry in std::fs::read_dir(old).ok().into_iter().flatten().flatten() {
                let src = entry.path();
                if src.is_file() {
                    // Top-level loose GGUF: migrate like a model directory
                    if !entry
                        .file_name()
                        .to_string_lossy()
                        .to_lowercase()
                        .ends_with(".gguf")
                    {
                        continue;
                    }
                } else if !src.is_dir() {
                    continue;
                } else {
                    // Only directories that qualify as models move (marker or direct
                    // GGUF files); foreign subdirectories stay where they are
                    let is_model = read_model_marker(&src).is_some() || {
                        let mut probe = Vec::new();
                        collect_gguf_files_shallow(&src, &mut probe);
                        !probe.is_empty()
                    };
                    if !is_model {
                        continue;
                    }
                }
                let dst = new.join(entry.file_name());
                if dst.exists() {
                    tracing::warn!(
                        "[local-models] 迁移跳过 {}: 目标已存在 {}",
                        entry.file_name().to_string_lossy(),
                        dst.display()
                    );
                    continue;
                }
                if std::fs::rename(&src, &dst).is_err() {
                    if let Err(e) = copy_dir_recursive(&src, &dst) {
                        tracing::warn!(
                            "[local-models] 迁移 {} 失败: {e}",
                            entry.file_name().to_string_lossy()
                        );
                        continue;
                    }
                    let _ = std::fs::remove_dir_all(&src);
                }
                tracing::info!(
                    "[local-models] 已迁移本地模型 {} → {}",
                    entry.file_name().to_string_lossy(),
                    dst.display()
                );
            }
        }
    }
    // Rewrite provider localModelPaths entries pointing into the old directory
    if let Ok(mut file) = crate::commands::provider_config::load_provider_configs(app.clone()) {
        let mut changed = false;
        for cfg in file.providers.values_mut() {
            if let Some(map) = cfg.local_model_paths.as_mut() {
                for path in map.values_mut() {
                    if let Some(rest) = path_under(path, old) {
                        let next = new.join(rest).to_string_lossy().to_string();
                        tracing::info!("[local-models] 路径重写 {} → {next}", path);
                        *path = next;
                        changed = true;
                    }
                }
            }
        }
        if changed {
            if let Err(e) =
                crate::commands::provider_config::save_provider_configs(app.clone(), file)
            {
                tracing::warn!("[local-models] 迁移后保存供应商配置失败: {e}");
            }
        }
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for item in std::fs::read_dir(src)? {
        let item = item?;
        let from = item.path();
        let to = dst.join(item.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_under_matches_case_insensitive_windows_prefix() {
        let base = Path::new("C:\\Data\\Models");
        assert_eq!(
            path_under("C:\\data\\models\\qwen\\a.gguf", base),
            Some("qwen\\a.gguf".into())
        );
        assert_eq!(path_under("C:\\Data\\Models", base), None);
        assert_eq!(path_under("C:\\Data\\ModelsOther\\a.gguf", base), None);
        assert_eq!(path_under("D:\\Elsewhere\\a.gguf", base), None);
    }

    #[test]
    fn format_bytes_units() {
        assert_eq!(format_bytes(500 * 1024 * 1024), "500 MB");
        assert_eq!(
            format_bytes((5.5 * 1024.0 * 1024.0 * 1024.0) as u64),
            "5.5 GB"
        );
    }

    #[test]
    fn collect_gguf_files_nested_and_sorted_entry() {
        let tmp = std::env::temp_dir().join(format!("aio-test-gguf-{}", uuid::Uuid::new_v4()));
        let sub = tmp.join("model-a").join("nested");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("b-00002-of-00002.gguf"), "x").unwrap();
        std::fs::write(sub.join("b-00001-of-00002.gguf"), "xy").unwrap();
        std::fs::write(sub.join("notes.txt"), "x").unwrap();
        let mut files = Vec::new();
        collect_gguf_files(&tmp.join("model-a"), &tmp.join("model-a"), &mut files);
        files.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].name, "nested/b-00001-of-00002.gguf");
        assert_eq!(files[0].size, 2);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn shallow_scan_ignores_nested_ggufs() {
        let tmp = std::env::temp_dir().join(format!("aio-test-shallow-{}", uuid::Uuid::new_v4()));
        let nested = tmp.join("cache").join("deep");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("hidden.gguf"), "x").unwrap();
        std::fs::write(tmp.join("direct.gguf"), "xyz").unwrap();
        std::fs::write(tmp.join("notes.txt"), "x").unwrap();

        // A foreign dir with only deeply nested GGUFs yields nothing on the shallow scan
        let mut files = Vec::new();
        collect_gguf_files_shallow(&tmp.join("cache"), &mut files);
        assert!(files.is_empty());

        // Direct files are picked up (name + size)
        let mut files = Vec::new();
        collect_gguf_files_shallow(&tmp, &mut files);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "direct.gguf");
        assert_eq!(files[0].size, 3);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn model_marker_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("aio-test-marker-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        assert!(read_model_marker(&tmp).is_none());
        write_model_marker(&tmp, "nested/entry-00001-of-00002.gguf");
        let marker = read_model_marker(&tmp).unwrap();
        assert_eq!(marker.entry_file, "nested/entry-00001-of-00002.gguf");
        std::fs::remove_dir_all(&tmp).ok();
    }
}

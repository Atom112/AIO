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

/// ENG-11: repo-wide concurrent download cap — unbounded parallel model downloads
/// thrash the disk and the network; two at a time keeps large models from starving
/// everything else.
static DOWNLOAD_SLOTS: OnceLock<tokio::sync::Semaphore> = OnceLock::new();

fn download_slots() -> &'static tokio::sync::Semaphore {
    DOWNLOAD_SLOTS.get_or_init(|| tokio::sync::Semaphore::new(2))
}

/// ENG-5: removes only the registering task's own control entry on drop
/// (identity-matched by the cancel flag's Arc pointer) — a successor's entry is
/// never dropped by a predecessor's exit.
struct UnregisterGuard {
    id: String,
    cancel: Arc<AtomicBool>,
}

impl Drop for UnregisterGuard {
    fn drop(&mut self) {
        if let Ok(mut map) = download_controls().lock() {
            if let Some(current) = map.get(&self.id) {
                if Arc::ptr_eq(&current.cancel, &self.cancel) {
                    map.remove(&self.id);
                }
            }
        }
    }
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
                    // ENG-02: Token 仅附于 huggingface.co 官方域；镜像候选匿名访问
                    token: hf_token.clone().filter(|_| name == "huggingface"),
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

    // P0-2：前端传入的 id / file.name 一律经 safe_join 校验——拒绝绝对路径（含
    // Windows 盘符 / UNC）、`..` 回溯与保留字符，防止任意目录写入与递归删除
    let dir = crate::core::path_safety::safe_join(&local_models_dir(&app), &id)?;
    let dests: Vec<PathBuf> = request
        .files
        .iter()
        .map(|f| crate::core::path_safety::safe_join(&dir, &f.name))
        .collect::<Result<_, _>>()?;

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
        .zip(dests.iter())
        .filter(|(_, dest)| !dest.exists())
        .map(|(f, _)| f.size)
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
    // ENG-5: same-id downloads must not overlap — the old unconditional insert let a
    // second task overwrite the first's control (making the first unstoppable) and
    // the first task's Drop guard then removed the SECOND's entry on exit.
    {
        let mut map = download_controls().lock().map_err(|e| e.to_string())?;
        if map.contains_key(&id) {
            return Err(ierr!(
                "该模型已有下载任务在进行中（不可并行下载同一模型）",
                "A download for this model is already in progress (parallel downloads of the same model are not allowed)"
            ));
        }
        map.insert(id.clone(), control.clone());
    }
    // ENG-5: the unregister guard removes only OUR entry (identity-matched by the
    // cancel flag's Arc pointer), never a successor's.
    let _guard = UnregisterGuard {
        id: id.clone(),
        cancel: control.cancel.clone(),
    };

    // ENG-11: wait for a global download slot; a queued task still honors cancel
    // (the flags are polled while waiting instead of blocking uninteruptibly).
    {
        let sem = download_slots();
        loop {
            tokio::select! {
                permit = sem.acquire() => {
                    let _permit = permit.map_err(|_| "下载并发闸门已关闭".to_string())?;
                    break;
                }
                _ = tokio::time::sleep(Duration::from_millis(400)) => {
                    if control.cancel.load(Ordering::Relaxed) {
                        return Err("cancelled".into());
                    }
                }
            }
        }
    }

    let client = download_client(request.proxy_url.as_deref())?;
    let hf_token =
        crate::core::secure_store::get(&app, &crate::core::secure_store::accounts::hf_token())
            .ok()
            .flatten();

    for (file, dest) in request.files.iter().zip(dests.iter()) {
        if dest.exists() {
            // ENG-04: 盲信跳过使半截文件永久视为完成——已知大小时核对长度，
            // 并校验 GGUF 魔数；不符则删除重下
            let size_ok = file.size == 0
                || std::fs::metadata(dest)
                    .map(|m| m.len() == file.size)
                    .unwrap_or(false);
            if size_ok && verify_gguf_magic(dest).is_ok() {
                continue;
            }
            tracing::warn!(
                "[download] 已存在的 {} 未通过完整性校验，删除重下",
                dest.display()
            );
            let _ = std::fs::remove_file(dest);
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
                    // ENG-04: 续传附 If-Range（上次响应的 ETag）——跨源/远端版本变更时
                    // 服务器改回 200，上面"server ignored Range"分支会清零重下，
                    // 不再把不同版本的字节拼接成静默损坏的文件
                    if let Ok(etag) = std::fs::read_to_string(etag_path(&tmp)) {
                        let etag = etag.trim().to_string();
                        if !etag.is_empty() {
                            req = req.header(reqwest::header::IF_RANGE, etag);
                        }
                    }
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
                            let _ = tokio::fs::remove_file(etag_path(&tmp)).await;
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
                        // ENG-04: 206 的 Content-Range 起点必须与续传起点一致——
                        // 起点不符仍按 206 追加会拼接出错位的损坏文件
                        if status == reqwest::StatusCode::PARTIAL_CONTENT && attempt_from > 0 {
                            let range_ok = resp
                                .headers()
                                .get(reqwest::header::CONTENT_RANGE)
                                .and_then(|v| v.to_str().ok())
                                .and_then(|cr| cr.split_once(' '))
                                .map(|(_, rest)| rest.starts_with(&format!("{attempt_from}-")))
                                .unwrap_or(false);
                            if !range_ok {
                                let _ = tokio::fs::remove_file(&tmp).await;
                                let _ = tokio::fs::remove_file(etag_path(&tmp)).await;
                                attempt_from = 0;
                                resume_from = 0;
                                continue; // restart this source from scratch
                            }
                        }
                        if status != reqwest::StatusCode::PARTIAL_CONTENT {
                            resume_from = 0; // server ignored Range -> full restart
                        }
                        // ENG-04: 记录本次响应的 ETag 供下次续传 If-Range 使用
                        if let Some(etag) = resp
                            .headers()
                            .get(reqwest::header::ETAG)
                            .and_then(|v| v.to_str().ok())
                        {
                            let _ = std::fs::write(etag_path(&tmp), etag);
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
        // ENG-11: per-file space check at the point where the true size is known.
        // The aggregate pre-check at task start is advisory only (TOCTOU: other
        // processes consume space during a multi-GB download; and metadata-less
        // files with size=0 bypassed it entirely — Content-Length here is
        // authoritative).
        let file_needed = total.saturating_sub(resume_from);
        if file_needed > 0 {
            if let Some(parent) = dest.parent() {
                if let Some(avail) = available_space(parent) {
                    if file_needed > avail {
                        return Err(ierr!(
                            "磁盘空间不足：{} 还需 {}，仅剩 {}（清理空间后重试）",
                            "Insufficient disk space: {} needs {} more, only {} left (free space and retry)",
                            file.name,
                            format_bytes(file_needed),
                            format_bytes(avail)
                        ));
                    }
                }
            }
        }
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
                let _ = tokio::fs::remove_file(etag_path(&tmp)).await;
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
                // ENG-04: 终长核对——截断流此前被当完成直接 rename，
                // 之后 dest.exists() 盲信跳过使损坏成为"永久完成"
                if total > 0 && downloaded != total {
                    return Err(ierr!(
                        "下载不完整（{}/{} 字节），检查点已保留可续传",
                        "Download incomplete ({}/{} bytes); checkpoint kept, resumable",
                        downloaded,
                        total
                    ));
                }
                std::fs::rename(&tmp, dest)
                    .map_err(|e| ierr!("重命名失败: {}", "Failed to rename: {}", e))?;
                let _ = std::fs::remove_file(etag_path(&tmp));
                // ENG-04: GGUF 魔数校验（跨源续传/截断的最后一道兜底防线）
                if let Err(e) = verify_gguf_magic(dest) {
                    let _ = std::fs::remove_file(dest);
                    return Err(e);
                }
            }
        }
    }
    let entry = dests[0].to_string_lossy().to_string();
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
/// HOR-4: the no-download cleanup walks the model dir — off the main thread.
#[tauri::command]
pub async fn local_model_cancel_download(app: AppHandle, id: String) -> Result<bool, String> {
    let active = signal_download(&id, |c| c.cancel.store(true, Ordering::Relaxed));
    if !active {
        // P0-2：id 经 safe_join 校验后再定位目录
        let dir = crate::core::path_safety::safe_join(&local_models_dir(&app), id.trim())?;
        tokio::task::spawn_blocking(move || {
            if dir.is_dir() {
                remove_part_files(&dir);
            }
        })
        .await
        .map_err(|e| e.to_string())?;
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
        let _ = crate::core::fs_util::atomic_write(&dir.join(MODEL_MARKER), &bytes);
    }
}

/// Reads a model directory's marker (None when absent or unreadable).
fn read_model_marker(dir: &Path) -> Option<ModelMarker> {
    let bytes = std::fs::read(dir.join(MODEL_MARKER)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Deletes an installed local model (removes its install directory, or the loose
/// `.gguf` file itself when the id refers to a top-level file).
/// HOR-4: recursive removal of a multi-GB model dir must not run on the main
/// thread (Tauri executes sync commands there) — the removal runs on the
/// blocking pool.
#[tauri::command]
pub async fn local_model_delete(app: AppHandle, id: String) -> Result<(), String> {
    // P0-2：id 经 safe_join 校验——`Path::join` 对绝对路径参数会整体替换基路径，
    // `..` 组件在 syscall 时逃逸；这里杜绝「任意目录递归删除」原语
    let path = crate::core::path_safety::safe_join(&local_models_dir(&app), id.trim())?;
    tokio::task::spawn_blocking(move || {
        if path.is_dir() {
            std::fs::remove_dir_all(&path)
                .map_err(|e| ierr!("删除模型失败: {}", "Failed to delete model: {}", e))?;
        } else if path.is_file() {
            std::fs::remove_file(&path)
                .map_err(|e| ierr!("删除模型失败: {}", "Failed to delete model: {}", e))?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Lists installed local models by scanning the install directory for GGUF content:
/// - a top-level loose `*.gguf` file counts as a single-file model;
/// - a subdirectory counts only when the market wrote a marker into it (recursive
///   scan, marker's entry file) or when it DIRECTLY contains `*.gguf` files (no
///   recursion) — foreign directories (caches, tool folders with deeply nested
///   models) are never mistaken for downloaded models.
/// HOR-4: the recursive disk scan runs on the blocking pool (sync commands would
/// execute it on the main thread).
#[tauri::command]
pub async fn local_model_installed_list(
    app: AppHandle,
) -> Result<Vec<InstalledLocalModel>, String> {
    let root = local_models_dir(&app);
    tokio::task::spawn_blocking(move || Ok(installed_list_inner(root)))
        .await
        .map_err(|e| e.to_string())?
}

/// Synchronous body of [`local_model_installed_list`] (runs on the blocking pool).
fn installed_list_inner(root: PathBuf) -> Vec<InstalledLocalModel> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(&root) {
        Ok(e) => e,
        Err(_) => return out,
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
    out
}

/// Current local model install directory.
#[tauri::command]
pub fn local_models_get_dir(app: AppHandle) -> Result<String, String> {
    Ok(local_models_dir(&app).to_string_lossy().to_string())
}

/// Sets the local model install directory (empty `dir` restores the default). Downloaded
/// models are migrated to the new directory, and every provider's localModelPaths entry
/// pointing into the old directory is rewritten (best-effort, failures are logged only).
/// HOR-4: the migration renames/copies multi-GB model dirs — it runs on the blocking
/// pool so the main thread (and the UI) stays responsive.
#[tauri::command]
pub async fn local_models_set_dir(app: AppHandle, dir: String) -> Result<String, String> {
    let old = local_models_dir(&app);
    let mut cfg = crate::core::config::load_app_config(&app)
        .map_err(|e| ierr!("读取配置失败: {}", "Failed to read config: {}", e))?;
    cfg.local_models_dir = dir.trim().to_string();
    crate::commands::config::save_app_config(app.clone(), cfg)
        .map_err(|e| ierr!("保存配置失败: {}", "Failed to save config: {}", e))?;
    let new = local_models_dir(&app);
    if !same_path(&old, &new) {
        let app_c = app.clone();
        let new_c = new.clone();
        tokio::task::spawn_blocking(move || migrate_local_models(&app_c, &old, &new_c))
            .await
            .map_err(|e| e.to_string())?;
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

/// Removes `*.part` checkpoint files (and their `.etag` sidecars) under `dir` (recursive).
fn remove_part_files(dir: &Path) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            remove_part_files(&path);
        } else {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name.ends_with(".part") || name.ends_with(".part.etag") {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

/// ENG-04: 续传检查点旁的 ETag sidecar 路径（`<tmp>.etag`）。
fn etag_path(tmp: &Path) -> PathBuf {
    PathBuf::from(format!("{}.etag", tmp.display()))
}

/// ENG-04: 校验文件头 4 字节为 "GGUF"——下载完成/已存在跳过前的完整性兜底
/// （跨源续传拼接或截断的文件在此被拦下，成本近零）。
fn verify_gguf_magic(path: &Path) -> Result<(), String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)
        .map_err(|e| ierr!("校验模型文件失败: {}", "Failed to verify model file: {}", e))?;
    let mut magic = [0u8; 4];
    f.read_exact(&mut magic).map_err(|e| {
        ierr!(
            "读取模型文件头失败: {}",
            "Failed to read model file header: {}",
            e
        )
    })?;
    if &magic != b"GGUF" {
        return Err(ierr!(
            "模型文件校验失败：文件头不是 GGUF 格式（下载可能损坏，已删除请重试）",
            "Model file verification failed: header is not GGUF format (download may be corrupt; deleted, please retry)"
        ));
    }
    Ok(())
}

/// Moves every model directory from `old` to `new` (rename first, cross-volume copy
/// fallback), then rewrites provider localModelPaths entries under `old` to `new` —
/// ENG-10: only for entries that actually migrated (failed/skipped copies keep their
/// old-path references; rewriting them would create dangling pointers).
fn migrate_local_models(app: &AppHandle, old: &Path, new: &Path) {
    // ENG-10: names that actually landed under `new` (drive the path rewrite below)
    let mut migrated: std::collections::HashSet<String> = std::collections::HashSet::new();
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
                    match copy_dir_recursive(&src, &dst) {
                        Ok(()) => {
                            // Copy succeeded only if the source is fully removable
                            // afterwards; a failed removal leaves duplicates, which
                            // the provider rewrite still treats as migrated (the
                            // file exists at the new path).
                            let _ = std::fs::remove_dir_all(&src);
                        }
                        Err(e) => {
                            tracing::warn!(
                                "[local-models] 迁移 {} 失败: {e}",
                                entry.file_name().to_string_lossy()
                            );
                            continue;
                        }
                    }
                }
                migrated.insert(entry.file_name().to_string_lossy().into_owned());
                tracing::info!(
                    "[local-models] 已迁移本地模型 {} → {}",
                    entry.file_name().to_string_lossy(),
                    dst.display()
                );
            }
        }
    }
    // Rewrite provider localModelPaths entries pointing into the old directory —
    // ENG-10: only entries whose model actually migrated; the old unconditional
    // rewrite stranded references to models that stayed behind at the old path.
    if let Ok(mut file) = crate::commands::provider_config::load_provider_configs(app.clone()) {
        let mut changed = false;
        for cfg in file.providers.values_mut() {
            if let Some(map) = cfg.local_model_paths.as_mut() {
                for path in map.values_mut() {
                    if let Some(rest) = path_under(path, old) {
                        // The first path component under `old` identifies the entry
                        // that had to migrate (model directory or loose GGUF name).
                        let top = std::path::Path::new(&rest)
                            .components()
                            .next()
                            .map(|c| c.as_os_str().to_string_lossy().into_owned())
                            .unwrap_or_default();
                        if !migrated.contains(&top) {
                            continue;
                        }
                        let next = new.join(&rest).to_string_lossy().to_string();
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

    /// ENG-04 回归：GGUF 魔数校验拦下非 GGUF 文件，正确文件放行。
    #[test]
    fn gguf_magic_verification() {
        let dir = std::env::temp_dir().join(format!("aio-magic-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("创建临时目录失败");
        let good = dir.join("good.gguf");
        std::fs::write(&good, b"GGUF\x03\x00\x00\x00rest").unwrap();
        assert!(verify_gguf_magic(&good).is_ok());
        let bad = dir.join("bad.gguf");
        std::fs::write(&bad, b"HTML error page payload").unwrap();
        assert!(verify_gguf_magic(&bad).is_err());
        // ETag sidecar 路径与 .part 检查点成对
        let tmp = dir.join("model.gguf.part");
        assert_eq!(etag_path(&tmp).file_name().unwrap(), "model.gguf.part.etag");
        let _ = std::fs::remove_dir_all(dir);
    }

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

    /// ENG-5: the unregister guard is identity-matched — a predecessor's exit must
    /// never remove a successor's control entry (the historic id-keyed remove did).
    #[test]
    fn unregister_guard_only_removes_own_entry() {
        const KEY: &str = "eng5-test-model";
        let a = DownloadControl {
            cancel: Arc::new(AtomicBool::new(false)),
            pause: Arc::new(AtomicBool::new(false)),
        };
        download_controls()
            .lock()
            .unwrap()
            .insert(KEY.into(), a.clone());
        let guard = UnregisterGuard {
            id: KEY.into(),
            cancel: a.cancel.clone(),
        };
        // 模拟后续任务覆盖控制表（同 id 并发现在已被拒，但守卫必须保持安全）
        let b = DownloadControl {
            cancel: Arc::new(AtomicBool::new(false)),
            pause: Arc::new(AtomicBool::new(false)),
        };
        download_controls().lock().unwrap().insert(KEY.into(), b);
        drop(guard);
        assert!(
            download_controls().lock().unwrap().contains_key(KEY),
            "旧任务退出不得移除新任务的控制项"
        );
        download_controls().lock().unwrap().remove(KEY);
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

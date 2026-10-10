//! On-demand model downloader (generic): the manifest describes remote files, which are
//! downloaded into the matching install directory; progress is reported via the
//! `model-download-progress` event.
//!
//! Current entries:
//! - kind=embed: mainstream open-source local embedding models (ONNX, from HF export
//!   repos like Xenova / Qdrant / Alibaba-NLP; all take the self-contained quantized
//!   weights at `onnx/model_quantized.onnx` for fast CPU inference and a small footprint);
//!   the install dir is user-configurable in settings (default `<app_data_dir>/models/embed/<id>/`).
//! - kind=rerank: local rerank models (bge cross-encoder family + mxbai, fixed dir `<app_data_dir>/models/rerank/<id>/`).
//!
//! The bundled model all-MiniLM-L6-v2 (kind=embed, bundled=true) ships with the app
//! (include_bytes) and needs no download; it is always listed as installed, and delete
//! is a no-op.
//!
//! Download control: pause (keeps the `.part` checkpoint, resume via HTTP Range) /
//! cancel (deletes the checkpoint). Flags are registered per model id in a
//! process-level control table, set by dedicated commands and checked per chunk.

use crate::core::events as ev;
use crate::core::models::EmbeddingModelSetting;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Window};
use tokio::io::AsyncWriteExt;

use crate::ierr;

// The manifest registry and directory resolution live in core::model_registry
// (needed by the plugin layer; this command layer only re-exports them).
pub(crate) use crate::core::model_registry::entry_dir;
pub use crate::core::model_registry::*;

/// Model info for frontend display (id/kind/name/dims/bundled/enabled/files/downloaded/size on disk).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub dims: usize,
    pub pooled: String,
    pub bundled: bool,
    pub enabled: bool,
    pub files: Vec<ModelFileInfo>,
    pub downloaded: bool,
    pub size_bytes: u64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ModelFileInfo {
    pub name: String,
    pub downloaded: bool,
}

/// Download outcome: status = ok (finished) | paused (paused, checkpoint kept) | cancelled (cancelled, checkpoint deleted).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DownloadOutcome {
    pub status: String,
    pub info: ModelInfo,
}

/// Control flags for an in-flight download (registered per model id).
#[derive(Clone)]
struct DownloadControl {
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
}

fn download_controls() -> &'static Mutex<HashMap<String, DownloadControl>> {
    static CONTROLS: OnceLock<Mutex<HashMap<String, DownloadControl>>> = OnceLock::new();
    CONTROLS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Sets a download flag for the given model; returns whether a download is in flight.
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

fn model_info(app: &AppHandle, entry: &'static ManifestEntry) -> ModelInfo {
    let dir = entry_dir(app, entry);
    let files = entry
        .files
        .iter()
        .map(|f| ModelFileInfo {
            name: f.name.to_string(),
            downloaded: dir.join(f.name).exists(),
        })
        .collect::<Vec<_>>();
    let size_bytes = files
        .iter()
        .filter(|f| f.downloaded)
        .map(|f| {
            std::fs::metadata(dir.join(&f.name))
                .map(|m| m.len())
                .unwrap_or(0)
        })
        .sum::<u64>();
    ModelInfo {
        id: entry.id.to_string(),
        kind: entry.kind.to_string(),
        label: entry.label.to_string(),
        dims: entry.dims,
        pooled: entry.pooling.to_string(),
        bundled: entry.bundled,
        enabled: is_model_enabled(app, entry.id),
        downloaded: entry.bundled || files.iter().all(|f| f.downloaded),
        files,
        size_bytes,
    }
}

/// Lists downloadable/installed models with their status (local rerank + local embedding models).
#[tauri::command]
pub fn embedding_model_list(app: AppHandle) -> Result<Vec<ModelInfo>, String> {
    Ok(MANIFEST.iter().map(|e| model_info(&app, e)).collect())
}

/// Queries one model's download/enabled status (with per-file detail). `id`: manifest entry id.
#[tauri::command]
pub fn embedding_model_status(app: AppHandle, id: String) -> Result<Option<ModelInfo>, String> {
    Ok(find_entry(&id).map(|e| model_info(&app, e)))
}

/// Current install directory for embed models (shown on the settings page).
#[tauri::command]
pub fn embedding_models_get_dir(app: AppHandle) -> Result<String, String> {
    Ok(embed_models_dir(&app).to_string_lossy().to_string())
}

/// Sets the embed model install directory (persisted to the app config; an empty `dir`
/// restores the default directory). Downloaded embed models under the old directory are
/// migrated to the new one (freeing space there); migration is best-effort: per-item
/// failures are only logged and never block the directory switch.
#[tauri::command]
pub fn embedding_models_set_dir(app: AppHandle, dir: String) -> Result<String, String> {
    let old = embed_models_dir(&app);
    let mut cfg = crate::core::config::load_app_config(&app)
        .map_err(|e| ierr!("读取配置失败: {}", "Failed to read config: {}", e))?;
    cfg.embedding_models_dir = dir.trim().to_string();
    crate::commands::config::save_app_config(app.clone(), cfg)
        .map_err(|e| ierr!("保存配置失败: {}", "Failed to save config: {}", e))?;
    let new = embed_models_dir(&app);
    if !same_path(&old, &new) {
        migrate_embed_models(&old, &new);
    }
    Ok(new.to_string_lossy().to_string())
}

/// Globally enables/disables a model (shared by embed/rerank; once disabled, the model is unavailable in all projects).
#[tauri::command]
pub fn embedding_model_set_enabled(
    app: AppHandle,
    id: String,
    enabled: bool,
) -> Result<ModelInfo, String> {
    let entry = find_entry(&id).ok_or_else(|| ierr!("未知模型: {}", "Unknown model: {}", id))?;
    let mut cfg = crate::core::config::load_app_config(&app)
        .map_err(|e| ierr!("读取配置失败: {}", "Failed to read config: {}", e))?;
    let mut list: Vec<EmbeddingModelSetting> = cfg
        .embedding_models
        .iter()
        .filter(|s| s.id != id)
        .cloned()
        .collect();
    list.push(EmbeddingModelSetting {
        id: id.clone(),
        enabled,
    });
    cfg.embedding_models = list;
    crate::commands::config::save_app_config(app.clone(), cfg)
        .map_err(|e| ierr!("保存配置失败: {}", "Failed to save config: {}", e))?;
    Ok(model_info(&app, entry))
}

/// Downloads a model (streamed to disk + model-download-progress events; existing files
/// are skipped). `id`: manifest entry id. Ends in one of ok / paused / cancelled:
/// - paused: keeps the `.part` checkpoint, resumed with Range on the next call;
/// - cancelled: deletes the `.part` checkpoint;
/// - network/write errors also keep the checkpoint (resumed next time).
#[tauri::command]
pub async fn embedding_model_download(
    window: Window,
    app: AppHandle,
    id: String,
) -> Result<DownloadOutcome, String> {
    let entry = find_entry(&id).ok_or_else(|| ierr!("未知模型: {}", "Unknown model: {}", id))?;
    if entry.bundled {
        return Ok(DownloadOutcome {
            status: "ok".into(),
            info: model_info(&app, entry),
        });
    }
    // Register the control flags (a duplicate download of the same id overwrites the old flags)
    let control = DownloadControl {
        cancel: Arc::new(AtomicBool::new(false)),
        pause: Arc::new(AtomicBool::new(false)),
    };
    if let Ok(mut map) = download_controls().lock() {
        map.insert(id.clone(), control.clone());
    }
    // Unregister the control flags when the command ends (including if the frontend drops it)
    struct UnregisterGuard(String);
    impl Drop for UnregisterGuard {
        fn drop(&mut self) {
            if let Ok(mut map) = download_controls().lock() {
                map.remove(&self.0);
            }
        }
    }
    let _guard = UnregisterGuard(id.clone());

    let dir = entry_dir(&app, entry);
    std::fs::create_dir_all(&dir).map_err(|e| {
        ierr!(
            "创建模型目录失败: {}",
            "Failed to create model directory: {}",
            e
        )
    })?;
    let client = reqwest::Client::new();
    for file in entry.files {
        let dest = dir.join(file.name);
        if dest.exists() {
            continue;
        }
        // Manifest file names may contain subdirectories (onnx/model_quantized.onnx); create the parent directory first
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                ierr!(
                    "创建模型目录失败: {}",
                    "Failed to create model directory: {}",
                    e
                )
            })?;
        }
        let tmp = dest.with_extension("part");
        // Resume: when a .part file remains, continue from the checkpoint with Range
        let mut resume_from = tokio::fs::metadata(&tmp)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        let mut resp = if resume_from > 0 {
            client
                .get(file.url)
                .header(reqwest::header::RANGE, format!("bytes={resume_from}-"))
                .send()
                .await
        } else {
            client.get(file.url).send().await
        }
        .map_err(|e| {
            ierr!(
                "下载 {} 失败: {}",
                "Failed to download {}: {}",
                file.name,
                e
            )
        })?;
        // .part larger than the remote file (corrupt) -> delete the checkpoint and restart the download
        if resp.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            let _ = tokio::fs::remove_file(&tmp).await;
            resume_from = 0;
            resp = client.get(file.url).send().await.map_err(|e| {
                ierr!(
                    "下载 {} 失败: {}",
                    "Failed to download {}: {}",
                    file.name,
                    e
                )
            })?;
        }
        let partial = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        if !resp.status().is_success() {
            let status = resp.status();
            return Err(ierr!(
                "下载 {} 失败（{}）",
                "Failed to download {} ({})",
                file.name,
                status
            ));
        }
        if !partial {
            resume_from = 0; // server ignored Range -> restart the whole download
        }
        let total = resume_from + resp.content_length().unwrap_or(0);
        let mut stream = resp.bytes_stream();
        let mut out = if partial {
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
            // Throttle progress events (the first chunk is emitted immediately)
            if last_emit.is_none_or(|t| t.elapsed() >= Duration::from_millis(200)) {
                last_emit = Some(Instant::now());
                let _ = window.emit(
                    ev::MODEL_DOWNLOAD_PROGRESS,
                    serde_json::json!({
                        "id": entry.id,
                        "file": file.name,
                        "downloaded": downloaded,
                        "total": total,
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
                return Ok(DownloadOutcome {
                    status: "cancelled".into(),
                    info: model_info(&app, entry),
                });
            }
            Some("paused") => {
                return Ok(DownloadOutcome {
                    status: "paused".into(),
                    info: model_info(&app, entry),
                });
            }
            _ => {
                // ENG-04（同族修复）：终长核对——截断流不再被当完成 rename 成"永久完成"
                if total > 0 && downloaded != total {
                    return Err(ierr!(
                        "下载不完整（{}/{} 字节），检查点已保留可续传",
                        "Download incomplete ({}/{} bytes); checkpoint kept, resumable",
                        downloaded,
                        total
                    ));
                }
                std::fs::rename(&tmp, &dest)
                    .map_err(|e| ierr!("重命名失败: {}", "Failed to rename: {}", e))?;
            }
        }
    }
    let _ = window.emit(
        ev::MODEL_DOWNLOAD_PROGRESS,
        serde_json::json!({ "id": entry.id, "done": true }),
    );
    Ok(DownloadOutcome {
        status: "ok".into(),
        info: model_info(&app, entry),
    })
}

/// Pauses an in-flight model download. `id`: manifest entry id. Returns whether a download was in flight.
#[tauri::command]
pub fn embedding_model_pause_download(id: String) -> bool {
    signal_download(&id, |c| c.pause.store(true, Ordering::Relaxed))
}

/// Cancels a model download. `id`: manifest entry id. Returns whether a download was in
/// flight; when none is (e.g. already paused), it cleans up all `.part` checkpoint files
/// for the model directly.
#[tauri::command]
pub fn embedding_model_cancel_download(app: AppHandle, id: String) -> Result<bool, String> {
    let active = signal_download(&id, |c| c.cancel.store(true, Ordering::Relaxed));
    if !active {
        if let Some(entry) = find_entry(&id) {
            let dir = entry_dir(&app, entry);
            if dir.is_dir() {
                for f in entry.files {
                    let _ = std::fs::remove_file(dir.join(f.name).with_extension("part"));
                }
            }
        }
    }
    Ok(active)
}

/// Deletes a downloaded model (frees disk space + evicts the in-process loaded instance cache; no-op for bundled models).
#[tauri::command]
pub fn embedding_model_delete(app: AppHandle, id: String) -> Result<(), String> {
    let entry = find_entry(&id).ok_or_else(|| ierr!("未知模型: {}", "Unknown model: {}", id))?;
    if entry.bundled {
        return Ok(());
    }
    if entry.kind == "embed" {
        crate::plugins::embed::local_onnx::evict_model(&id);
    } else if entry.kind == "rerank" {
        crate::plugins::rerank::local_onnx::evict_model(&id);
    }
    let dir = entry_dir(&app, entry);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .map_err(|e| ierr!("删除模型失败: {}", "Failed to delete model: {}", e))?;
    }
    Ok(())
}

/// Compares two directory paths in normalized form (canonicalize when they exist, literal comparison otherwise).
fn same_path(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(a) == canon(b)
}

/// Migrates downloaded embed models from the old directory to the new one (rename first;
/// falls back to a recursive copy across volumes). Best-effort: existing targets or
/// per-item failures are only logged and skipped.
fn migrate_embed_models(old: &Path, new: &Path) {
    if !old.is_dir() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(new) {
        tracing::warn!("[models] 迁移目标目录创建失败（{}）: {e}", new.display());
        return;
    }
    for entry in MANIFEST.iter().filter(|e| e.kind == "embed" && !e.bundled) {
        let src = old.join(entry.id);
        if !src.is_dir() {
            continue;
        }
        let dst = new.join(entry.id);
        if dst.exists() {
            tracing::warn!(
                "[models] 迁移跳过 {}: 目标已存在 {}",
                entry.id,
                dst.display()
            );
            continue;
        }
        if std::fs::rename(&src, &dst).is_err() {
            // Cross-volume rename failed -> recursively copy, then delete the source directory
            if let Err(e) = copy_dir_recursive(&src, &dst) {
                tracing::warn!("[models] 迁移 {} 失败: {e}", entry.id);
                continue;
            }
            let _ = std::fs::remove_dir_all(&src);
        }
        tracing::info!(
            "[models] 已迁移 embed 模型 {} → {}",
            entry.id,
            dst.display()
        );
    }
}

/// Recursively copies a directory (fallback path for cross-volume migration).
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

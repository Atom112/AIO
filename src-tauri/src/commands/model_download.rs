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
//! 阶段4: the control table and the streamed copy tail live in
//! `services::download` — the single implementation shared with the local GGUF
//! downloader (the historic two private copies had already drifted once).

use crate::core::events as ev;
use crate::core::models::EmbeddingModelSetting;
use crate::ierr;
// 阶段4: 控制表/注销守卫/流复制尾段——与本地 GGUF 下载器共享（services::download）
use crate::services::download::{
    signal_download, stream_to_checkpoint, DownloadControl, DownloadInterrupt, UnregisterGuard,
};
use serde::Serialize;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Window};

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
/// HOR-4: the migration moves multi-GB model dirs — it runs on the blocking pool
/// instead of Tauri's main thread.
#[tauri::command]
pub async fn embedding_models_set_dir(app: AppHandle, dir: String) -> Result<String, String> {
    let old = embed_models_dir(&app);
    let mut cfg = crate::core::config::load_app_config(&app)
        .map_err(|e| ierr!("读取配置失败: {}", "Failed to read config: {}", e))?;
    cfg.embedding_models_dir = dir.trim().to_string();
    crate::core::config::save_app_config(&app, &cfg)
        .map_err(|e| ierr!("保存配置失败: {}", "Failed to save config: {}", e))?;
    let new = embed_models_dir(&app);
    if !same_path(&old, &new) {
        let new_c = new.clone();
        tokio::task::spawn_blocking(move || migrate_embed_models(&old, &new_c))
            .await
            .map_err(|e| e.to_string())?;
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
    crate::core::config::save_app_config(&app, &cfg)
        .map_err(|e| ierr!("保存配置失败: {}", "Failed to save config: {}", e))?;
    Ok(model_info(&app, entry))
}

/// Downloads a model (streamed to disk + model-download-progress events; existing files
/// are skipped). `id`: manifest entry id. Ends in one of ok / paused / cancelled:
/// - paused: keeps the `.part` checkpoint, resumed with Range on the next call;
/// - cancelled: deletes the `.part` checkpoint;
/// - network/write errors also keep the checkpoint (resumed next time).
///
/// Terminal states (ok / paused / cancelled / error) are additionally announced with a
/// `{id, done: true, status}` event so the background-task menu can close its entry;
/// `done: true` alone remains the legacy contract the settings page clears rows on.
#[tauri::command]
pub async fn embedding_model_download(
    window: Window,
    app: AppHandle,
    id: String,
) -> Result<DownloadOutcome, String> {
    let result = embedding_model_download_inner(window, app.clone(), id.clone()).await;
    let _ = app.emit(
        ev::MODEL_DOWNLOAD_PROGRESS,
        match &result {
            Ok(outcome) => serde_json::json!({ "id": id, "done": true, "status": outcome.status }),
            Err(e) => {
                serde_json::json!({ "id": id, "done": true, "status": "error", "error": e })
            }
        },
    );
    result
}

/// Body of [`embedding_model_download`] (the command layer wraps it to emit the
/// terminal lifecycle event on every exit path).
async fn embedding_model_download_inner(
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
    // Register the control flags.
    // ENG-05 (embedding side, parity with the GGUF downloader): a concurrent
    // download of the same id is rejected instead of overwriting the old flags —
    // the overwrite let the second task cancel/pause the first's state and the
    // first's exit-guard could drop the successor's entry.
    let control = DownloadControl {
        cancel: Arc::new(AtomicBool::new(false)),
        pause: Arc::new(AtomicBool::new(false)),
    };
    {
        let mut map = crate::services::download::download_controls()
            .lock()
            .map_err(|e| e.to_string())?;
        if map.contains_key(&id) {
            return Err(ierr!(
                "模型 {} 已有下载任务在进行中",
                "A download is already in progress for model {}",
                id
            ));
        }
        map.insert(id.clone(), control.clone());
    }
    // Unregister the control flags when the command ends (including if the frontend drops it).
    // 阶段4: the shared guard is ENG-5 identity-matched — a predecessor's exit can
    // no longer drop a successor's entry (the historic local copy of this guard
    // removed unconditionally).
    let _guard = UnregisterGuard::new(&id, &control);

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
        // 阶段4: streamed copy tail — shared implementation (services::download):
        // throttled progress events, per-chunk pause/cancel polling, ENG-04
        // final-length check and the atomic rename. Embedding/rerank models carry
        // no ETag sidecar.
        match stream_to_checkpoint(
            // 阶段5: the engine takes an emit closure (testable without a Tauri app);
            // the command layer wraps its window here.
            |event, payload| {
                let _ = window.emit(event, payload);
            },
            ev::MODEL_DOWNLOAD_PROGRESS,
            &serde_json::json!({ "id": entry.id }),
            file.name,
            resp,
            &tmp,
            &dest,
            resume_from,
            &control,
            None,
        )
        .await?
        {
            Some(DownloadInterrupt::Cancelled) => {
                return Ok(DownloadOutcome {
                    status: "cancelled".into(),
                    info: model_info(&app, entry),
                });
            }
            Some(DownloadInterrupt::Paused) => {
                return Ok(DownloadOutcome {
                    status: "paused".into(),
                    info: model_info(&app, entry),
                });
            }
            None => {}
        }
    }
    // Terminal event moved to the command-layer wrapper (embedding_model_download):
    // it fires on every exit path with the outcome status.
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
/// HOR-4: the cleanup is disk I/O — off the main thread.
#[tauri::command]
pub async fn embedding_model_cancel_download(app: AppHandle, id: String) -> Result<bool, String> {
    let active = signal_download(&id, |c| c.cancel.store(true, Ordering::Relaxed));
    if !active {
        if let Some(entry) = find_entry(&id) {
            let dir = entry_dir(&app, entry);
            // Preserve the historic checkpoint naming: <file>.<ext> -> <file>.part
            let names: Vec<String> = entry.files.iter().map(|f| f.name.to_string()).collect();
            tokio::task::spawn_blocking(move || {
                if dir.is_dir() {
                    for name in names {
                        let _ = std::fs::remove_file(dir.join(name).with_extension("part"));
                    }
                }
            })
            .await
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(active)
}

/// Deletes a downloaded model (frees disk space + evicts the in-process loaded instance cache; no-op for bundled models).
/// HOR-4: recursive removal of a multi-GB model dir — off the main thread.
#[tauri::command]
pub async fn embedding_model_delete(app: AppHandle, id: String) -> Result<(), String> {
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
    tokio::task::spawn_blocking(move || {
        if dir.exists() {
            std::fs::remove_dir_all(&dir)
                .map_err(|e| ierr!("删除模型失败: {}", "Failed to delete model: {}", e))?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
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

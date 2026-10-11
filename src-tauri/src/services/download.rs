//! Shared resumable-download engine (阶段4 第4项: `commands/local_model_download` 与
//! `commands/model_download` 的同构实现收敛为单一来源).
//!
//! Both downloaders historically carried private copies of the same machinery:
//! the per-id control table (pause/cancel flags), the control-registration guard,
//! and the streamed copy tail (append/truncate by resume semantics → chunk loop
//! with throttled progress events → pause/cancel polling → flush → final-length
//! check → atomic rename). The copies had already drifted once (ENG-5's
//! identity-matched unregister existed only in one of the two). This module is
//! the single implementation; the commands keep only their domain specifics
//! (multi-source fallback, If-Range/ETag, space pre-check, GGUF magic verify).

use crate::ierr;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::Emitter;

/// Control flags for an in-flight download (registered per id).
#[derive(Clone)]
pub(crate) struct DownloadControl {
    pub cancel: Arc<AtomicBool>,
    pub pause: Arc<AtomicBool>,
}

/// Global control table. Shared by every downloader domain — embedding/rerank
/// models and local GGUF market downloads key on disjoint id namespaces (manifest
/// slugs vs full repo ids), so one table is sufficient.
pub(crate) fn download_controls() -> &'static Mutex<HashMap<String, DownloadControl>> {
    static CONTROLS: OnceLock<Mutex<HashMap<String, DownloadControl>>> = OnceLock::new();
    CONTROLS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// ENG-5: removes only the registering task's own control entry on drop
/// (identity-matched by the cancel flag's Arc pointer) — a successor's entry is
/// never dropped by a predecessor's exit.
pub(crate) struct UnregisterGuard {
    id: String,
    cancel: Arc<AtomicBool>,
}

impl UnregisterGuard {
    /// Creates the guard; the caller registers the control BEFORE constructing it.
    pub(crate) fn new(id: &str, control: &DownloadControl) -> Self {
        Self {
            id: id.to_string(),
            cancel: control.cancel.clone(),
        }
    }
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

/// Sets a download flag for the given id; returns whether a download is in flight.
pub(crate) fn signal_download(id: &str, set: impl Fn(&DownloadControl)) -> bool {
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

/// Why the streamed copy ended early (None = the file completed and was renamed).
pub(crate) enum DownloadInterrupt {
    Cancelled,
    Paused,
}

/// Streamed copy tail shared by both downloaders: opens the checkpoint file
/// (append when `resume_from > 0`, truncate otherwise), copies the response
/// stream with throttled progress events merged with `progress_extras`
/// (`{"id":…}` / `{"id":…, "source":…}` — `file`/`downloaded`/`total` are added
/// by the engine), polls pause/cancel, flushes, verifies the final length
/// (ENG-04: a truncated stream is never renamed into a "permanent completion")
/// and atomically renames to `dest`. `etag_sidecar` (the `.etag` path paired
/// with the `.part` checkpoint) is removed on cancel and after a successful
/// rename.
///
/// The caller has already validated the resume semantics (206 + Content-Range
/// start match / Range-ignored reset) and keeps its domain specifics: the local
/// GGUF downloader does the ENG-11 space pre-check before calling and the GGUF
/// magic verify after `Ok(None)`.
#[allow(clippy::too_many_arguments)] // one streamed-copy engine, parameterized by event/payload/cleanup
pub(crate) async fn stream_to_checkpoint(
    window: &tauri::Window,
    event: &str,
    progress_extras: &Value,
    file_name: &str,
    resp: reqwest::Response,
    tmp: &Path,
    dest: &Path,
    resume_from: u64,
    control: &DownloadControl,
    etag_sidecar: Option<&Path>,
) -> Result<Option<DownloadInterrupt>, String> {
    let total = resume_from + resp.content_length().unwrap_or(0);
    let mut stream = resp.bytes_stream();
    let mut out = if resume_from > 0 {
        tokio::fs::OpenOptions::new().append(true).open(tmp).await
    } else {
        tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(tmp)
            .await
    }
    .map_err(|e| {
        ierr!(
            "创建临时文件失败: {}",
            "Failed to create temporary file: {}",
            e
        )
    })?;
    use tokio::io::AsyncWriteExt as _;
    let mut downloaded: u64 = resume_from;
    let mut interrupted: Option<DownloadInterrupt> = None;
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
            let mut payload = progress_extras.clone();
            if let Value::Object(map) = &mut payload {
                map.insert("file".into(), Value::String(file_name.to_string()));
                map.insert("downloaded".into(), Value::from(downloaded));
                map.insert("total".into(), Value::from(total));
            }
            let _ = window.emit(event, payload);
        }
        if control.cancel.load(Ordering::Relaxed) {
            interrupted = Some(DownloadInterrupt::Cancelled);
            break;
        }
        if control.pause.load(Ordering::Relaxed) {
            interrupted = Some(DownloadInterrupt::Paused);
            break;
        }
    }
    out.flush()
        .await
        .map_err(|e| ierr!("落盘失败: {}", "Failed to flush to disk: {}", e))?;
    drop(out);
    if let Some(DownloadInterrupt::Cancelled) = interrupted {
        let _ = tokio::fs::remove_file(tmp).await;
        if let Some(etag) = etag_sidecar {
            let _ = tokio::fs::remove_file(etag).await;
        }
        return Ok(Some(DownloadInterrupt::Cancelled));
    }
    if interrupted.is_some() {
        return Ok(Some(DownloadInterrupt::Paused));
    }
    // ENG-04: 终长核对——截断流不再被当完成 rename 成"永久完成"
    if total > 0 && downloaded != total {
        return Err(ierr!(
            "下载不完整（{}/{} 字节），检查点已保留可续传",
            "Download incomplete ({}/{} bytes); checkpoint kept, resumable",
            downloaded,
            total
        ));
    }
    std::fs::rename(tmp, dest).map_err(|e| ierr!("重命名失败: {}", "Failed to rename: {}", e))?;
    if let Some(etag) = etag_sidecar {
        let _ = std::fs::remove_file(etag);
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 阶段4 回归：控制表注册 + ENG-5 身份匹配注销——前任退出不得删掉继任的条目。
    #[test]
    fn control_unregister_is_identity_matched() {
        let id = "test-model-x";
        let first = DownloadControl {
            cancel: Arc::new(AtomicBool::new(false)),
            pause: Arc::new(AtomicBool::new(false)),
        };
        download_controls()
            .lock()
            .unwrap()
            .insert(id.to_string(), first.clone());
        {
            let _guard = UnregisterGuard::new(id, &first);
            // 前任还在场时继任注册（覆盖表项，但 first 仍持有自己的 Arc）
            let second = DownloadControl {
                cancel: Arc::new(AtomicBool::new(false)),
                pause: Arc::new(AtomicBool::new(false)),
            };
            download_controls()
                .lock()
                .unwrap()
                .insert(id.to_string(), second.clone());
            // 前任退出：ptr_eq 不匹配 → 继任条目必须保留
            drop(_guard);
            assert!(
                download_controls().lock().unwrap().contains_key(id),
                "前任退出不得移除继任的控制条目"
            );
            // 继任的旗标可被设置（signal 命中继任）
            assert!(signal_download(id, |c| c
                .pause
                .store(true, Ordering::Relaxed)));
            let second_flag = second.pause.load(Ordering::Relaxed);
            assert!(second_flag);
            // 继任退出：此时表项指向继任 → 正常移除
            let _guard2 = UnregisterGuard::new(id, &second);
        }
        assert!(!download_controls().lock().unwrap().contains_key(id));
        assert!(!signal_download(id, |c| c
            .cancel
            .store(true, Ordering::Relaxed)));
    }
}

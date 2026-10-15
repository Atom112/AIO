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
#[derive(Debug)]
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
///
/// `emit` receives `(event, payload)` — the caller wraps its window; taking a
/// closure instead of `&Window` keeps the engine testable without a Tauri app
/// (the 阶段5 kill-recovery integration tests below drive it with synthetic 206
/// responses).
#[allow(clippy::too_many_arguments)] // one streamed-copy engine, parameterized by event/payload/cleanup
pub(crate) async fn stream_to_checkpoint(
    emit: impl Fn(&str, Value),
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
            emit(event, payload);
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

#[cfg(test)]
mod kill_recovery_tests {
    //! 下载中途 kill 恢复（报告阶段5第3项）。
    //!
    //! 用 `http::Response` 合成 206/200 响应（`reqwest::Response: From<http::Response>`，
    //! http 已在依赖树内）直接驱动共享下载引擎，无需真实 HTTP 服务器。覆盖：
    //! - A. 进程 kill 后重试：`.part` 检查点续传 → 追加 → 终长核对 → 原子 rename → 完整文件
    //! - B. 服务器中途断流（Content-Length 虚高）：报"不完整"且检查点保留已收字节
    //! - C. 取消：`.part` 与 `.etag` sidecar 一并清理
    //! - D. 暂停：检查点保留（前缀+已收字节），下次继续
    //! - E. 全新下载（Range 被忽略/无检查点）：truncate 分支，陈旧检查点被整体覆盖

    use super::*;
    use std::path::PathBuf;

    /// Kill 发生时已落盘的字节（GGUF 魔数开头，贴近真实产物）。
    const PREFIX: &[u8] = b"GGUF\x03\x00\x00\x00\x12\x34\x56\x78\x9a\xbc";
    /// 续传响应携带的剩余字节。
    const SUFFIX: &[u8] = b"0123456789abcdefghijklmnopqrstuv";

    /// 合成 (status, headers, body) 响应供引擎流式消费。
    fn synth_response(status: u16, headers: &[(&str, &str)], body: Vec<u8>) -> reqwest::Response {
        let mut builder = http::Response::builder().status(status);
        for (k, v) in headers {
            builder = builder.header(*k, *v);
        }
        reqwest::Response::from(builder.body(body).expect("静态响应构造必然成功"))
    }

    fn fresh_control() -> DownloadControl {
        DownloadControl {
            cancel: Arc::new(AtomicBool::new(false)),
            pause: Arc::new(AtomicBool::new(false)),
        }
    }

    struct KillDir {
        root: PathBuf,
    }

    impl KillDir {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("aio-kill-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            Self { root }
        }
        fn part(&self) -> PathBuf {
            self.root.join("model.gguf.part")
        }
        fn etag(&self) -> PathBuf {
            self.root.join("model.gguf.part.etag")
        }
        fn dest(&self) -> PathBuf {
            self.root.join("model.gguf")
        }
        /// 模拟下完 PREFIX 字节后被 kill：检查点 + ETag sidecar 留在磁盘。
        fn killed_state(&self) {
            std::fs::write(self.part(), PREFIX).unwrap();
            std::fs::write(self.etag(), "\"etag-v1\"").unwrap();
        }
    }

    impl Drop for KillDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn noop_emit() -> impl Fn(&str, Value) {
        |_: &str, _: Value| {}
    }

    /// A. 主恢复路径：kill 后重试拿到 206 → 追加 → rename 出完整文件，sidecar 清理。
    #[tokio::test]
    async fn kill_then_resume_appends_and_renames_complete_file() {
        let dir = KillDir::new();
        dir.killed_state();
        let total_len = (PREFIX.len() + SUFFIX.len() - 1).to_string();
        let resp = synth_response(
            206,
            &[
                ("Content-Type", "application/octet-stream"),
                ("Content-Length", &SUFFIX.len().to_string()),
                (
                    "Content-Range",
                    &format!(
                        "bytes {}-{}/{}",
                        PREFIX.len(),
                        total_len,
                        PREFIX.len() + SUFFIX.len()
                    ),
                ),
            ],
            SUFFIX.to_vec(),
        );
        let control = fresh_control();
        let out = stream_to_checkpoint(
            noop_emit(),
            "local-model-download-progress",
            &serde_json::json!({ "id": "test-model" }),
            "model.gguf",
            resp,
            &dir.part(),
            &dir.dest(),
            PREFIX.len() as u64,
            &control,
            Some(&dir.etag()),
        )
        .await;
        assert!(
            matches!(out, Ok(None)),
            "完整续传应以 Ok(None) 完成: {out:?}"
        );
        // 终文件 = 前缀 + 续传字节（kill 的字节一分不丢）
        let content = std::fs::read(dir.dest()).unwrap();
        assert_eq!(content, [PREFIX, SUFFIX].concat());
        // .part 与 .etag 均已清理
        assert!(!dir.part().exists());
        assert!(!dir.etag().exists());
    }

    /// B. 服务器中途断流：声明 32 字节只送到 10 字节——终长核对报"不完整"，
    /// 检查点保留前缀+已收字节（下次 Range 从这里续传），绝不 rename 半截文件。
    /// 真实断流是"头已声明、body 未送完"：用 wrap_stream（size_hint 未知）构造，
    /// 使 content_length 走 Content-Length 头而非完整 body 的精确长度。
    #[tokio::test]
    async fn server_kill_midstream_keeps_resumable_checkpoint() {
        let dir = KillDir::new();
        dir.killed_state();
        let truncated = &SUFFIX[..10];
        // wrap_stream: unknown body length → content_length() falls back to the header
        let body = reqwest::Body::wrap_stream(futures_util::stream::once(async move {
            Ok::<Vec<u8>, std::io::Error>(truncated.to_vec())
        }));
        let http_resp = http::Response::builder()
            .status(206)
            .header("Content-Length", SUFFIX.len().to_string())
            .header(
                "Content-Range",
                format!(
                    "bytes {}-{}/{}",
                    PREFIX.len(),
                    PREFIX.len() + SUFFIX.len() - 1,
                    PREFIX.len() + SUFFIX.len()
                ),
            )
            .body(body)
            .expect("静态响应构造必然成功");
        let resp = reqwest::Response::from(http_resp);
        let control = fresh_control();
        let out = stream_to_checkpoint(
            noop_emit(),
            "local-model-download-progress",
            &serde_json::json!({ "id": "test-model" }),
            "model.gguf",
            resp,
            &dir.part(),
            &dir.dest(),
            PREFIX.len() as u64,
            &control,
            Some(&dir.etag()),
        )
        .await;
        let err = out.expect_err("截断流必须报错");
        assert!(err.contains("下载不完整"), "终长核对必须报不完整: {err}");
        // 检查点 = 前缀 + 已收到的 10 字节
        let part = std::fs::read(dir.part()).unwrap();
        assert_eq!(part, [PREFIX, truncated].concat());
        assert!(dir.etag().exists(), "ETag sidecar 保留供下次 If-Range");
        assert!(!dir.dest().exists(), "不得 rename 半截文件");
    }

    /// C. 取消：`.part` 检查点与 `.etag` sidecar 一并清理。
    #[tokio::test]
    async fn cancel_midstream_removes_checkpoint_and_sidecar() {
        let dir = KillDir::new();
        dir.killed_state();
        let control = fresh_control();
        control.cancel.store(true, Ordering::Relaxed);
        let resp = synth_response(
            206,
            &[("Content-Length", &SUFFIX.len().to_string())],
            SUFFIX.to_vec(),
        );
        let out = stream_to_checkpoint(
            noop_emit(),
            "local-model-download-progress",
            &serde_json::json!({ "id": "test-model" }),
            "model.gguf",
            resp,
            &dir.part(),
            &dir.dest(),
            PREFIX.len() as u64,
            &control,
            Some(&dir.etag()),
        )
        .await;
        assert!(matches!(out, Ok(Some(DownloadInterrupt::Cancelled))));
        assert!(!dir.part().exists(), "取消应删除检查点");
        assert!(!dir.etag().exists(), "取消应删除 sidecar");
        assert!(!dir.dest().exists());
    }

    /// D. 暂停：检查点保留前缀+本次已收字节，sidecar 保留——下次 Range 续传。
    #[tokio::test]
    async fn pause_midstream_preserves_checkpoint_for_next_resume() {
        let dir = KillDir::new();
        dir.killed_state();
        let control = fresh_control();
        control.pause.store(true, Ordering::Relaxed);
        let resp = synth_response(
            206,
            &[("Content-Length", &SUFFIX.len().to_string())],
            SUFFIX.to_vec(),
        );
        let out = stream_to_checkpoint(
            noop_emit(),
            "local-model-download-progress",
            &serde_json::json!({ "id": "test-model" }),
            "model.gguf",
            resp,
            &dir.part(),
            &dir.dest(),
            PREFIX.len() as u64,
            &control,
            Some(&dir.etag()),
        )
        .await;
        assert!(matches!(out, Ok(Some(DownloadInterrupt::Paused))));
        let part = std::fs::read(dir.part()).unwrap();
        assert_eq!(part, [PREFIX, SUFFIX].concat(), "暂停保留全部已收字节");
        assert!(dir.etag().exists());
        assert!(!dir.dest().exists());
    }

    /// E. 全新下载（服务器忽略 Range 回 200 / 无检查点）：truncate 分支——
    /// 磁盘上的陈旧检查点被整体覆盖，终文件只含新响应的字节。
    #[tokio::test]
    async fn fresh_download_truncates_stale_checkpoint() {
        let dir = KillDir::new();
        // 上一次中断留下的陈旧检查点（若无 truncate 会与新响应拼接成损坏文件）
        std::fs::write(dir.part(), b"stale-bytes-from-previous-attempt").unwrap();
        let resp = synth_response(
            200,
            &[("Content-Length", &SUFFIX.len().to_string())],
            SUFFIX.to_vec(),
        );
        let control = fresh_control();
        let out = stream_to_checkpoint(
            noop_emit(),
            "local-model-download-progress",
            &serde_json::json!({ "id": "test-model" }),
            "model.gguf",
            resp,
            &dir.part(),
            &dir.dest(),
            0,
            &control,
            Some(&dir.etag()),
        )
        .await;
        assert!(matches!(out, Ok(None)));
        let content = std::fs::read(dir.dest()).unwrap();
        assert_eq!(content, SUFFIX, "陈旧检查点必须被 truncate 覆盖");
        assert!(!dir.part().exists());
    }
}

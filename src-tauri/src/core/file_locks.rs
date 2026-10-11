//! In-process per-file exclusive locks for multi-agent write coordination.
//!
//! All built-in mutating file tools (`write_file` / `replace_in_file` / `delete_file` /
//! `make_directory`) funnel through `commands::llm::execute_builtin_tool`, making it the
//! single choke point where same-file writes are serialized across the main agent and all
//! concurrently running subagents (`delegate_tasks` batches, fire-and-forget background
//! subagents). This registry provides that serialization.
//!
//! Beyond mutual exclusion, the registry tracks the **last successful writer** per file.
//! When a different writer (another agent) is about to modify the same file, the hashline
//! protocol becomes mandatory: `replace_in_file` must carry a fresh `anchor_hash` and
//! `write_file` (overwrite) must carry `expect_hash`, turning silent lost updates into
//! detectable conflicts the model resolves by re-reading the file (see
//! `cross_writer_gate`).
//!
//! Scope: locks are in-process only. The legacy stdio fs MCP server (`mcp_fs_server.rs`)
//! runs in a separate process and cannot participate — its agent-facing tool variants are
//! therefore filtered out of tool-set assembly (`FsServerPolicy` in
//! `commands/llm/tool_dispatch.rs`) so every mutating file tool routes through this choke
//! point; the server itself remains for external MCP clients.

use crate::ierr;
use dashmap::DashMap;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

/// Default per-file lock wait timeout. Writers that cannot acquire within this window get
/// an actionable error naming the current holder instead of blocking the agent forever.
pub const LOCK_WAIT_TIMEOUT: Duration = Duration::from_secs(120);

/// Waits at least this long are logged (tracing) as coordination friction; timeouts always
/// error to the model with the holder name.
pub const CONTENTION_REPORT_AFTER: Duration = Duration::from_secs(3);

/// Writer id recorded for files changed by `execute_command` (SEC-09): shell commands
/// modify files outside the hashline protocol, so the last-writer entry must force a
/// re-read (anchor_hash / expect_hash) before any agent write to those files.
pub const COMMAND_WRITER_ID: &str = "command";

/// Directories the post-command touch scan never descends into: agent commands
/// regenerate their contents at scale (builds/installs), an agent writing there by
/// hand is pathological, and skipping keeps the walk bounded.
const SCAN_SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    ".cache",
    ".gradle",
    ".m2",
];

/// Max filesystem entries the post-command scan visits (runaway-tree guard).
const SCAN_MAX_ENTRIES: usize = 100_000;
/// Max files the scan marks per command; beyond that the walk stops early (the
/// re-read guidance still reaches the model through the tool result itself).
const SCAN_MAX_MARKS: usize = 5_000;

struct LockEntry {
    sem: Arc<Semaphore>,
    /// Label of the agent currently holding (or last holding) the lock — informational,
    /// shown in timeout errors.
    holder: Mutex<String>,
}

impl LockEntry {
    fn new() -> Self {
        Self {
            sem: Arc::new(Semaphore::new(1)),
            holder: Mutex::new(String::new()),
        }
    }
}

/// Failure modes of lock acquisition.
#[derive(Debug)]
pub enum FileLockError {
    /// The wait budget expired while another agent held the lock.
    Timeout {
        /// Label of the agent holding the lock at timeout.
        holder: String,
        /// How long this task waited before giving up.
        waited: Duration,
    },
    /// The agent turn was cancelled while waiting.
    Cancelled,
    /// The semaphore was closed (registry shutting down; not expected in practice).
    Closed,
}

/// Owned lock guard: released when dropped (including inside a `spawn_blocking` task).
pub struct FileLockGuard {
    _permit: OwnedSemaphorePermit,
    /// How long this task waited before acquiring.
    pub waited_for: Duration,
    /// The holder label at acquire time — `Some` only when the lock was contended.
    pub previous_holder: Option<String>,
}

/// Per-path exclusive lock registry + last-writer tracking.
///
/// Entries are retained for the session (bounded by distinct files touched; each entry is
/// a tiny Arc) — removal would race with concurrent acquire and gains nothing.
pub struct FileLockRegistry {
    locks: DashMap<String, Arc<LockEntry>>,
    /// Canonical path → last successful writer id.
    last_writers: DashMap<String, String>,
}

impl Default for FileLockRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FileLockRegistry {
    pub fn new() -> Self {
        Self {
            locks: DashMap::new(),
            last_writers: DashMap::new(),
        }
    }

    /// Normalized lock key: on Windows the filesystem is case-insensitive, so two
    /// spellings of one file (Foo.txt / foo.txt) must share one lock and one
    /// last-writer entry. Keys come from `safe_path`, whose canonicalization only
    /// covers existing components — the final spelling of not-yet-existing files is
    /// caller-provided, hence the fold here.
    fn key(path: &str) -> String {
        #[cfg(windows)]
        {
            path.to_ascii_lowercase()
        }
        #[cfg(not(windows))]
        {
            path.to_string()
        }
    }

    /// Acquires the exclusive per-path lock, waiting up to `timeout` for the current
    /// holder. Cancel-safe: the wait aborts when `token` fires (the semaphore acquire is
    /// dropped and any queued waiter moves on).
    pub async fn acquire(
        &self,
        path: &str,
        holder: &str,
        token: Option<&CancellationToken>,
        timeout: Duration,
    ) -> Result<FileLockGuard, FileLockError> {
        let key = Self::key(path);
        // `entry()`'s RefMut guard derefs to the stored Arc and is dropped at the end of
        // the statement, so awaiting below never holds a DashMap shard lock.
        let entry: Arc<LockEntry> = self
            .locks
            .entry(key)
            .or_insert_with(|| Arc::new(LockEntry::new()))
            .clone();

        let started = Instant::now();
        let was_free = entry.sem.available_permits() > 0;
        let previous_holder =
            (!was_free).then(|| entry.holder.lock().map(|h| h.clone()).unwrap_or_default());

        let cancelled_fut = async {
            match token {
                Some(t) => t.cancelled().await,
                None => std::future::pending::<()>().await,
            }
        };

        let permit = tokio::select! {
            _ = cancelled_fut => {
                return Err(FileLockError::Cancelled);
            }
            _ = tokio::time::sleep(timeout) => {
                let current = entry.holder.lock().map(|h| h.clone()).unwrap_or_default();
                return Err(FileLockError::Timeout {
                    holder: current,
                    waited: started.elapsed(),
                });
            }
            p = entry.sem.clone().acquire_owned() => match p {
                Ok(p) => p,
                Err(_) => {
                    return Err(FileLockError::Closed);
                }
            },
        };

        if let Ok(mut h) = entry.holder.lock() {
            *h = holder.to_string();
        }
        Ok(FileLockGuard {
            _permit: permit,
            waited_for: started.elapsed(),
            previous_holder: previous_holder.filter(|h| !h.is_empty()),
        })
    }

    /// The id of the writer that last successfully modified `path` (`None` when the file
    /// was never written through the lock choke point).
    pub fn last_writer(&self, path: &str) -> Option<String> {
        self.last_writers
            .get(&Self::key(path))
            .map(|v| v.value().clone())
    }

    /// Records a successful write to `path` by `writer_id`.
    pub fn mark_writer(&self, path: &str, writer_id: &str) {
        self.last_writers
            .insert(Self::key(path), writer_id.to_string());
    }

    /// SEC-09: single bounded walk under `root` marking every file whose mtime falls at
    /// or after `since` as written by [`COMMAND_WRITER_ID`], so any subsequent agent
    /// write to those files is forced through the hashline gate (re-read before edit).
    ///
    /// `since` is the command's start time; a 2-second mtime-granularity margin is
    /// subtracted, which can false-positive on files modified just before the command —
    /// safe (it only triggers an extra re-read). Files modified *while* the command ran
    /// always fall inside the window. Returns the number of files marked.
    pub fn mark_command_touched_files(&self, root: &std::path::Path, since: SystemTime) -> usize {
        let since = since.checked_sub(Duration::from_secs(2)).unwrap_or(since);
        let mut marked = 0usize;
        let mut visited = 0usize;
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                visited += 1;
                if visited > SCAN_MAX_ENTRIES {
                    tracing::warn!(
                        "[sec-09] 命令触碰扫描超过 {SCAN_MAX_ENTRIES} 个条目，提前中止（仅标记已发现的 {} 个文件）",
                        marked
                    );
                    return marked;
                }
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if file_type.is_dir() {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if !SCAN_SKIP_DIRS.contains(&name.as_ref()) {
                        stack.push(entry.path());
                    }
                } else if file_type.is_file() {
                    let Ok(metadata) = entry.metadata() else {
                        continue;
                    };
                    let Ok(modified) = metadata.modified() else {
                        continue;
                    };
                    if modified >= since {
                        if marked >= SCAN_MAX_MARKS {
                            tracing::warn!(
                                "[sec-09] 单条命令触碰的文件超过 {SCAN_MAX_MARKS} 个，停止标记（后续写入请自行 read_file 校准）"
                            );
                            return marked;
                        }
                        marked += 1;
                        self.mark_writer(&entry.path().to_string_lossy(), COMMAND_WRITER_ID);
                    }
                }
            }
        }
        marked
    }

    /// Number of lock entries currently tracked (test assertions).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.locks.len()
    }

    /// Whether no lock entries exist (test assertions).
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.locks.is_empty()
    }
}

/// Pure hashline gate: decides whether a write still passes after **another** writer
/// touched the target file. When the last writer differs from the current one, the edit
/// must be anchored to fresh content — `replace_in_file` carries `anchor_hash` and
/// `write_file` (overwrite) carries `expect_hash` — otherwise the call is rejected with a
/// re-read instruction. Same-writer and never-written files pass unchanged (single-agent
/// behavior is fully backward compatible).
///
/// `target_exists` matters only for `write_file`: overwriting an existing file needs the
/// hash anchor, creating a new file does not.
pub fn cross_writer_gate(
    tool_name: &str,
    arguments: &Value,
    target_exists: bool,
    last_writer: Option<&str>,
    writer_id: &str,
) -> Result<(), String> {
    let Some(last) = last_writer else {
        return Ok(());
    };
    if last == writer_id {
        return Ok(());
    }
    match tool_name {
        "replace_in_file" => {
            if arguments["anchor_hash"].as_str().unwrap_or("").is_empty() {
                Err(ierr!(
                    "文件此前由另一个智能体（{}）修改过，为防止丢失更新，本次替换必须携带 anchor_hash：请先 read_file(with_hash=true) 重新读取最新内容，然后携带返回的 [HASH sha256:...] 值重试",
                    "The file was last modified by another agent ({}). To prevent lost updates this replacement requires anchor_hash: re-run read_file(with_hash=true) for the latest content, then retry with its [HASH sha256:...] value",
                    last
                ))
            } else {
                Ok(())
            }
        }
        "write_file" => {
            if target_exists && arguments["expect_hash"].as_str().unwrap_or("").is_empty() {
                Err(ierr!(
                    "文件此前由另一个智能体（{}）修改过，直接覆盖会丢弃对方改动：请先 read_file(with_hash=true) 重新读取，并在 write_file 中携带 expect_hash=最新哈希；请基于最新内容重写",
                    "The file was last modified by another agent ({}); overwriting it directly would discard their changes: re-run read_file(with_hash=true) for the latest content and pass expect_hash=<its sha256> in write_file; write based on the fresh content",
                    last
                ))
            } else {
                Ok(())
            }
        }
        _ => Ok(()),
    }
}

/// Lock-timeout error text (bilingual), naming the holder so the model can adapt.
pub fn lock_timeout_error(path: &str, holder: &str, waited: Duration) -> String {
    ierr!(
        "文件锁等待超时（{}，已等 {} 秒）：该文件正被智能体「{}」修改。请稍后重试，或协调任务分工避免多个智能体同时修改同一文件",
        "File lock wait timed out ({}; waited {}s): the file is being modified by agent '{}'. Retry later, or partition the tasks so multiple agents do not modify the same file",
        path,
        waited.as_secs(),
        holder
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn registry() -> FileLockRegistry {
        FileLockRegistry::new()
    }

    /// Locks are exclusive: a second acquirer with a short budget times out while the
    /// first holder keeps the lock, and succeeds after it is released.
    #[tokio::test]
    async fn second_writer_times_out_until_first_releases() {
        let r = registry();
        let g1 = r
            .acquire("/p/a.txt", "agent-a", None, Duration::from_secs(1))
            .await
            .expect("首次获取必须成功");
        assert!(g1.previous_holder.is_none(), "无竞争时不应报持有者");

        let err = r
            .acquire("/p/a.txt", "agent-b", None, Duration::from_millis(50))
            .await
            .err()
            .expect("持有期间第二次获取应超时");
        match err {
            FileLockError::Timeout { holder, waited } => {
                assert_eq!(holder, "agent-a", "超时应携带当前持有者");
                assert!(waited >= Duration::from_millis(40), "应记录等待时长");
            }
            other => panic!("应为 Timeout，实际 {other:?}"),
        }

        drop(g1);
        let g2 = r
            .acquire("/p/a.txt", "agent-b", None, Duration::from_secs(1))
            .await
            .expect("释放后应立即获取");
        // Free acquire (no contention at entry): no previous holder is reported.
        assert!(g2.previous_holder.is_none());
        assert!(g2.waited_for < Duration::from_millis(300));
    }

    /// A waiter that acquires after the holder releases mid-wait observes the previous
    /// holder (contended-at-entry semantics).
    #[tokio::test]
    async fn waiter_sees_previous_holder_when_released_midwait() {
        let r = std::sync::Arc::new(FileLockRegistry::new());
        let g1 = r
            .acquire("/p/mid.txt", "agent-a", None, Duration::from_secs(5))
            .await
            .unwrap();

        let r2 = std::sync::Arc::clone(&r);
        let waiter = tokio::spawn(async move {
            r2.acquire("/p/mid.txt", "agent-b", None, Duration::from_secs(5))
                .await
                .expect("释放后应能获取")
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(g1);
        let g2 = waiter.await.expect("等待任务不应 panic");
        assert_eq!(
            g2.previous_holder.as_deref(),
            Some("agent-a"),
            "竞争获取应携带前一持有者"
        );
        assert!(
            g2.waited_for >= Duration::from_millis(40),
            "应记录实际等待时长"
        );
    }

    /// Cancellation while waiting aborts the acquire without poisoning the lock.
    #[tokio::test]
    async fn cancellation_aborts_wait() {
        let r = registry();
        let token = CancellationToken::new();
        let _g1 = r
            .acquire("/p/b.txt", "agent-a", None, Duration::from_secs(1))
            .await
            .unwrap();

        let t = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            t.cancel();
        });
        let err = r
            .acquire("/p/b.txt", "agent-b", Some(&token), Duration::from_secs(10))
            .await
            .err()
            .expect("等待中取消应返回错误");
        assert!(matches!(err, FileLockError::Cancelled));

        // The lock still works for the next writer after cancellation.
        drop(_g1);
        r.acquire("/p/b.txt", "agent-c", None, Duration::from_millis(200))
            .await
            .expect("取消后锁应仍可用");
    }

    /// Last-writer tracking roundtrip.
    #[tokio::test]
    async fn last_writer_tracking() {
        let r = registry();
        assert_eq!(r.last_writer("/p/c.txt"), None);
        r.mark_writer("/p/c.txt", "agent-a");
        assert_eq!(r.last_writer("/p/c.txt").as_deref(), Some("agent-a"));
        r.mark_writer("/p/c.txt", "agent-b");
        assert_eq!(r.last_writer("/p/c.txt").as_deref(), Some("agent-b"));
    }

    /// SEC-09: the post-command scan marks files modified inside the command window as
    /// written by "command", skips untouched files and skipped dirs (node_modules),
    /// and the cross-writer gate then demands an anchor from any agent writer.
    #[test]
    fn command_touch_scan_marks_modified_files_only() {
        let r = registry();
        let dir =
            std::env::temp_dir().join(format!("aio-cmdscan-{}", uuid::Uuid::new_v4().simple()));
        let sub = dir.join("sub");
        let nm = dir.join("node_modules");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir_all(&nm).unwrap();

        let old_path = dir.join("old.txt");
        std::fs::write(&old_path, "old").unwrap();
        let hour_ago = SystemTime::now()
            .checked_sub(Duration::from_secs(3600))
            .unwrap();
        let f = std::fs::File::options()
            .write(true)
            .open(&old_path)
            .unwrap();
        f.set_times(std::fs::FileTimes::new().set_modified(hour_ago))
            .unwrap();
        drop(f);

        let new_path = sub.join("new.txt");
        std::fs::write(&new_path, "new").unwrap();
        let nm_path = nm.join("gen.txt");
        std::fs::write(&nm_path, "gen").unwrap();

        let marked = r.mark_command_touched_files(&dir, SystemTime::now());
        assert_eq!(marked, 1, "只有窗口内修改的文件被标记");
        assert_eq!(
            r.last_writer(&new_path.to_string_lossy()).as_deref(),
            Some(COMMAND_WRITER_ID),
            "命令窗口内新增/修改的文件应标记为 command 写入"
        );
        assert!(
            r.last_writer(&old_path.to_string_lossy()).is_none(),
            "窗口前修改的文件不应被标记"
        );
        assert!(
            r.last_writer(&nm_path.to_string_lossy()).is_none(),
            "node_modules 应被扫描跳过"
        );
        // The gate then forces a re-read for any agent writing the touched file.
        assert!(cross_writer_gate(
            "replace_in_file",
            &json!({"path": "x", "new_string": "y"}),
            true,
            Some(COMMAND_WRITER_ID),
            "agent-a"
        )
        .is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Cross-writer gate: fresh files and same-writer sequences pass; a different last
    /// writer forces the hashline anchor for replace/overwrite but not for delete or
    /// new-file creation.
    #[test]
    fn cross_writer_gate_rules() {
        let replace_args_no_hash = json!({"path": "a", "new_string": "x"});
        let replace_args_hash = json!({"path": "a", "new_string": "x", "anchor_hash": "h"});
        let write_new = json!({"path": "a", "content": "x"});
        let write_overwrite_no_hash = json!({"path": "a", "content": "x"});
        let write_overwrite_hash = json!({"path": "a", "content": "x", "expect_hash": "h"});
        let delete_args = json!({"path": "a"});

        // Never written by anyone: everything passes
        for (tool, args, exists) in [
            ("replace_in_file", &replace_args_no_hash, true),
            ("write_file", &write_new, false),
            ("write_file", &write_overwrite_no_hash, true),
            ("delete_file", &delete_args, true),
        ] {
            assert!(
                cross_writer_gate(tool, args, exists, None, "agent-b").is_ok(),
                "无 last_writer 时 {tool} 应放行"
            );
        }

        // Same writer as last: passes without anchors (backward compatible)
        assert!(cross_writer_gate(
            "replace_in_file",
            &replace_args_no_hash,
            true,
            Some("agent-b"),
            "agent-b"
        )
        .is_ok());
        assert!(cross_writer_gate(
            "write_file",
            &write_overwrite_no_hash,
            true,
            Some("agent-b"),
            "agent-b"
        )
        .is_ok());

        // Different writer: replace requires anchor_hash
        assert!(cross_writer_gate(
            "replace_in_file",
            &replace_args_no_hash,
            true,
            Some("agent-a"),
            "agent-b"
        )
        .is_err());
        assert!(cross_writer_gate(
            "replace_in_file",
            &replace_args_hash,
            true,
            Some("agent-a"),
            "agent-b"
        )
        .is_ok());

        // Different writer: overwrite requires expect_hash, new file does not
        assert!(cross_writer_gate(
            "write_file",
            &write_overwrite_no_hash,
            true,
            Some("agent-a"),
            "agent-b"
        )
        .is_err());
        assert!(cross_writer_gate(
            "write_file",
            &write_overwrite_hash,
            true,
            Some("agent-a"),
            "agent-b"
        )
        .is_ok());
        assert!(
            cross_writer_gate("write_file", &write_new, false, Some("agent-a"), "agent-b").is_ok()
        );

        // Different writer: delete passes (lock still serializes; content merge N/A)
        assert!(cross_writer_gate(
            "delete_file",
            &delete_args,
            true,
            Some("agent-a"),
            "agent-b"
        )
        .is_ok());
    }

    /// The gate error text names the other agent (actionable for the model).
    #[test]
    fn gate_error_names_other_writer() {
        let err = cross_writer_gate(
            "replace_in_file",
            &json!({"path": "a", "new_string": "x"}),
            true,
            Some("sub-coder-123"),
            "main-agent",
        )
        .unwrap_err();
        assert!(
            err.contains("sub-coder-123"),
            "错误文本应包含对方 id: {err}"
        );
    }

    /// Different paths never contend with each other.
    #[tokio::test]
    async fn different_paths_do_not_contend() {
        let r = registry();
        let _g1 = r
            .acquire("/p/x.txt", "agent-a", None, Duration::from_secs(1))
            .await
            .unwrap();
        let _g2 = r
            .acquire("/p/y.txt", "agent-b", None, Duration::from_millis(200))
            .await
            .expect("不同路径互不阻塞");
        assert_eq!(r.len(), 2);
        assert!(!r.is_empty());
    }

    /// On Windows (case-insensitive filesystem) two spellings of one file must share a
    /// single lock and last-writer entry; on case-sensitive platforms they are distinct
    /// files by definition and stay independent.
    #[tokio::test]
    async fn case_fold_keys_match_filesystem_semantics() {
        let r = registry();
        let upper = "/p/Foo.TXT";
        let lower = "/p/foo.txt";
        let _g1 = r
            .acquire(upper, "agent-a", None, Duration::from_secs(1))
            .await
            .unwrap();
        let second = r
            .acquire(lower, "agent-b", None, Duration::from_millis(80))
            .await;

        r.mark_writer(upper, "agent-a");
        let writer_via_lower = r.last_writer(lower);

        #[cfg(windows)]
        {
            assert!(second.is_err(), "大小写变体必须是同一把锁");
            drop(_g1);
            let w = writer_via_lower.expect("同一 last-writer 条目");
            assert_eq!(w, "agent-a", "Windows 下大小写折叠必须共享 last-writer");
        }
        #[cfg(not(windows))]
        {
            assert!(second.is_ok(), "大小写敏感文件系统应为不同文件");
            assert!(writer_via_lower.is_none(), "不同键不应串写");
        }
    }
}

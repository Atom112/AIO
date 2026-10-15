//! Locked execution of the built-in file tools.
//!
//! Mutating single-path file tools (`write_file` / `replace_in_file` / `delete_file` /
//! `make_directory`) take the per-file exclusive lock (`core::file_locks::FileLockRegistry`)
//! so same-file writes serialize across the main agent and all concurrently running
//! subagents, and a different last writer forces a fresh hashline anchor (gate) so lost
//! updates become detectable re-read errors. Read-only file tools pass straight through.
//!
//! Ordering invariants: the gate runs while holding the lock (last_writer is only updated
//! by lock holders, so the read is fresh) and both the lock guard and `mark_writer` live
//! INSIDE the blocking closure — even if the surrounding future is dropped mid-await, the
//! write completes under the lock and the writer bookkeeping is recorded before release.

use crate::core::file_locks::FileLockRegistry;
use crate::core::models::ToolResult;
use crate::ierr;
use crate::utils::file_tools;
use serde_json::Value;
use std::sync::Arc;
use tauri::Manager;
use tokio_util::sync::CancellationToken;

/// Executes one built-in file tool (read or mutating) under the write-coordination policy
/// (thin wrapper: fetches the managed lock registry, then delegates to `run_locked_file_tool`).
///
/// `project_root` is the root the tool operates on (the project root, or a workspace root
/// when the caller runs isolated); `writer_id` labels the calling agent for lock holder
/// tracking and cross-writer gate decisions.
pub(crate) async fn execute_locked_file_tool<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    tool_name: &str,
    arguments: &Value,
    project_root: &str,
    token: &CancellationToken,
    writer_id: &str,
) -> Result<ToolResult, String> {
    let registry: Arc<FileLockRegistry> = app.state::<Arc<FileLockRegistry>>().inner().clone();
    run_locked_file_tool(
        &registry,
        tool_name,
        arguments,
        project_root,
        token,
        writer_id,
    )
    .await
}

/// Core locked execution, decoupled from Tauri state so tests can drive it with a plain
/// registry (the AppHandle wrapper is a one-line state fetch). All ordering invariants
/// live here: the gate runs while holding the lock (last_writer is only updated by lock
/// holders, so the read is fresh) and both the lock guard and `mark_writer` live INSIDE
/// the blocking closure — even if the surrounding future is dropped mid-await, the write
/// completes under the lock and the writer bookkeeping is recorded before release.
pub(crate) async fn run_locked_file_tool(
    registry: &Arc<FileLockRegistry>,
    tool_name: &str,
    arguments: &Value,
    project_root: &str,
    token: &CancellationToken,
    writer_id: &str,
) -> Result<ToolResult, String> {
    let lock_target = file_tools::mutating_tool_target(project_root, tool_name, arguments);
    let mut lock_guard: Option<crate::core::file_locks::FileLockGuard> = None;

    if let Some((path, _exists)) = &lock_target {
        let path_s = path.to_string_lossy().to_string();
        let display_path = file_tools::strip_windows_extended_prefix(&path_s);

        match registry
            .acquire(
                &path_s,
                writer_id,
                Some(token),
                crate::core::file_locks::LOCK_WAIT_TIMEOUT,
            )
            .await
        {
            Ok(g) => {
                if g.waited_for >= crate::core::file_locks::CONTENTION_REPORT_AFTER {
                    tracing::warn!(
                        "[file-lock] 写协调等待 {}ms：{} 等待 {}（此前由 {} 持有）",
                        g.waited_for.as_millis(),
                        writer_id,
                        display_path,
                        g.previous_holder.as_deref().unwrap_or("?")
                    );
                }
                lock_guard = Some(g);
            }
            Err(crate::core::file_locks::FileLockError::Timeout { holder, waited }) => {
                tracing::warn!(
                    "[file-lock] 写协调超时：{} 等待 {} 超过 {}ms（持有者 {}）",
                    writer_id,
                    display_path,
                    waited.as_millis(),
                    holder
                );
                return Ok(file_tools::tool_err(
                    &crate::core::file_locks::lock_timeout_error(&display_path, &holder, waited),
                ));
            }
            Err(crate::core::file_locks::FileLockError::Cancelled) => {
                return Err("cancelled".into());
            }
            Err(_) => {
                return Ok(file_tools::tool_err(&ierr!(
                    "文件锁通道已关闭",
                    "File lock channel closed"
                )));
            }
        }

        // Cross-writer gate under the lock: re-read last_writer + existence here (both
        // may have changed while waiting for the lock)
        let gate = crate::core::file_locks::cross_writer_gate(
            tool_name,
            arguments,
            path.exists(),
            registry.last_writer(&path_s).as_deref(),
            writer_id,
        );
        if let Err(msg) = gate {
            return Ok(file_tools::tool_err(&msg)); // guard drops -> lock released
        }
    }

    let tool_name_c = tool_name.to_string();
    let arguments_c = arguments.clone();
    let root_c = project_root.to_string();
    let writer_c = writer_id.to_string();
    let registry_c = Arc::clone(registry);
    let result = tokio::task::spawn_blocking(move || {
        // Guard + mark_writer live inside the blocking task: the lock is held for exactly
        // the duration of the write and the writer bookkeeping is recorded before
        // release — even if the awaiting future is dropped mid-write.
        let _guard = lock_guard;
        let result = file_tools::execute_file_tool(&tool_name_c, &arguments_c, &root_c);
        if let (Some((path, _)), false) = (&lock_target, result.is_error) {
            registry_c.mark_writer(&path.to_string_lossy(), &writer_c);
        }
        result
    })
    .await
    .unwrap_or_else(|_| file_tools::tool_err("文件工具执行线程 panic"));

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::file_locks::FileLockRegistry;
    use crate::core::tool_names;
    use serde_json::json;
    use std::sync::Arc;

    /// Extracts the text of a ToolResult (mirrors the subagent helper).
    fn text_of(r: &ToolResult) -> String {
        r.content
            .first()
            .and_then(|c| c.data.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }

    fn fresh_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("aio_lock_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn registry() -> Arc<FileLockRegistry> {
        Arc::new(FileLockRegistry::new())
    }

    /// Convenience: run one tool through the locked core path.
    async fn run(
        reg: &Arc<FileLockRegistry>,
        tool: &str,
        args: &serde_json::Value,
        root: &str,
        writer: &str,
    ) -> ToolResult {
        let token = tokio_util::sync::CancellationToken::new();
        run_locked_file_tool(reg, tool, args, root, &token, writer)
            .await
            .unwrap()
    }

    /// Two writers onto one file, end to end: B's BLIND overwrite is gate-rejected (lost
    /// update prevention); after re-reading and carrying expect_hash B's write lands,
    /// the content is never torn, and the bookkeeping records B as the last writer.
    #[tokio::test]
    async fn concurrent_writers_serialize_on_one_file() {
        let reg = registry();
        let dir = fresh_dir("serial");
        let root = dir.to_string_lossy().to_string();

        let a = run(
            &reg,
            tool_names::WRITE_FILE,
            &json!({"path": "same.txt", "content": "from writer A"}),
            &root,
            "sub-coder-a",
        )
        .await;
        assert!(text_of(&a).contains("✓ 写入"));

        // B: blind overwrite of A's file -> cross-writer gate rejection
        let b = run(
            &reg,
            tool_names::WRITE_FILE,
            &json!({"path": "same.txt", "content": "from writer B"}),
            &root,
            "sub-coder-b",
        )
        .await;
        assert!(b.is_error, "跨写者盲覆盖必须被拦截");
        assert!(text_of(&b).contains("expect_hash"));

        // B re-reads and retries with the fresh digest -> serialized write lands
        let read = run(
            &reg,
            tool_names::READ_FILE,
            &json!({"path": "same.txt", "with_hash": true}),
            &root,
            "sub-coder-b",
        )
        .await;
        let hash = text_of(&read)
            .split("[HASH sha256:")
            .nth(1)
            .and_then(|s| s.split(']').next())
            .expect("read_file 应返回哈希")
            .to_string();
        let b = run(
            &reg,
            tool_names::WRITE_FILE,
            &json!({"path": "same.txt", "content": "from writer B", "expect_hash": hash}),
            &root,
            "sub-coder-b",
        )
        .await;
        assert!(text_of(&b).contains("✓ 写入"));

        // Full content, never torn; last-writer recorded under the (case-folded) key
        let content = std::fs::read_to_string(dir.join("same.txt")).unwrap();
        assert_eq!(content, "from writer B", "内容不得撕裂: {content:?}");
        let canonical = std::fs::canonicalize(dir.join("same.txt")).unwrap();
        assert_eq!(
            reg.last_writer(&canonical.to_string_lossy()).as_deref(),
            Some("sub-coder-b")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Cross-writer gate over the full locked path: after writer A touched a file, writer
    /// B's unanchored replace is rejected with a re-read instruction; anchored with the
    /// fresh hash it succeeds. Same rule for an overwriting write_file (expect_hash).
    #[tokio::test]
    async fn cross_writer_gate_enforced_through_the_locked_path() {
        let reg = registry();
        let dir = fresh_dir("gate");
        let root = dir.to_string_lossy().to_string();

        // Writer A creates the file
        run(
            &reg,
            tool_names::WRITE_FILE,
            &json!({"path": "gate.txt", "content": "alpha\n"}),
            &root,
            "sub-coder-a",
        )
        .await;

        // Writer B: unanchored replace -> gate error naming A
        let r = run(
            &reg,
            tool_names::REPLACE_IN_FILE,
            &json!({"path": "gate.txt", "old_string": "alpha", "new_string": "ALPHA"}),
            &root,
            "sub-coder-b",
        )
        .await;
        let text = text_of(&r);
        assert!(r.is_error, "跨写者无锚替换必须被拦截");
        assert!(text.contains("sub-coder-a"), "错误应点名另一写者: {text}");
        assert!(text.contains("anchor_hash"), "应指引重读: {text}");

        // Writer B: unanchored overwrite write_file -> gate error (expect_hash)
        let r = run(
            &reg,
            tool_names::WRITE_FILE,
            &json!({"path": "gate.txt", "content": "overwrite"}),
            &root,
            "sub-coder-b",
        )
        .await;
        assert!(r.is_error, "跨写者无锚覆盖必须被拦截");
        assert!(text_of(&r).contains("expect_hash"));

        // Writer B re-reads with hash and retries the replace with the anchor
        let read = run(
            &reg,
            tool_names::READ_FILE,
            &json!({"path": "gate.txt", "with_hash": true}),
            &root,
            "sub-coder-b",
        )
        .await;
        let read_text = text_of(&read);
        let hash = read_text
            .split("[HASH sha256:")
            .nth(1)
            .and_then(|s| s.split(']').next())
            .expect("read_file 应返回哈希")
            .to_string();
        let r = run(
            &reg,
            tool_names::REPLACE_IN_FILE,
            &json!({"path": "gate.txt", "old_string": "alpha", "new_string": "ALPHA", "anchor_hash": hash}),
            &root,
            "sub-coder-b",
        )
        .await;
        assert!(!r.is_error, "携带新鲜锚的替换应成功: {}", text_of(&r));

        // Writer B overwrites write_file with expect_hash = the CURRENT digest (post-replace)
        let read = run(
            &reg,
            tool_names::READ_FILE,
            &json!({"path": "gate.txt", "with_hash": true}),
            &root,
            "sub-coder-b",
        )
        .await;
        let fresh_hash = text_of(&read)
            .split("[HASH sha256:")
            .nth(1)
            .and_then(|s| s.split(']').next())
            .expect("re-read 应返回新哈希")
            .to_string();
        let r = run(
            &reg,
            tool_names::WRITE_FILE,
            &json!({"path": "gate.txt", "content": "final", "expect_hash": fresh_hash}),
            &root,
            "sub-coder-b",
        )
        .await;
        assert!(!r.is_error, "expect_hash 与锚一致时应成功: {}", text_of(&r));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Read-only tools pass through untouched (no lock bookkeeping, normal result).
    #[tokio::test]
    async fn read_only_tools_pass_through() {
        let reg = registry();
        let dir = fresh_dir("readonly");
        let root = dir.to_string_lossy().to_string();
        std::fs::write(dir.join("note.txt"), "hello").unwrap();

        let r = run(
            &reg,
            tool_names::READ_FILE,
            &json!({"path": "note.txt"}),
            &root,
            "main-agent",
        )
        .await;
        assert!(
            text_of(&r).contains("hello"),
            "只读工具应正常返回: {}",
            text_of(&r)
        );
        // No last-writer entry for read-only access
        let canonical = std::fs::canonicalize(dir.join("note.txt")).unwrap();
        assert!(reg.last_writer(&canonical.to_string_lossy()).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// TRUE concurrency: the same writer issuing two simultaneous writes (e.g. two
    /// in-flight calls of one agent) — the per-file lock serializes them; the file ends
    /// with one writer's full content (never interleaved), no gate interference.
    #[tokio::test]
    async fn parallel_tasks_never_tear_the_file() {
        let reg = registry();
        let dir = fresh_dir("parallel");
        let root = dir.to_string_lossy().to_string();
        let body_a = "A".repeat(50_000);
        let body_b = "B".repeat(50_000);
        let tok_a = tokio_util::sync::CancellationToken::new();
        let tok_b = tokio_util::sync::CancellationToken::new();
        let args_a = json!({"path": "par.txt", "content": body_a});
        let args_b = json!({"path": "par.txt", "content": body_b});

        let (ra, rb) = tokio::join!(
            run_locked_file_tool(
                &reg,
                tool_names::WRITE_FILE,
                &args_a,
                &root,
                &tok_a,
                "main-agent"
            ),
            run_locked_file_tool(
                &reg,
                tool_names::WRITE_FILE,
                &args_b,
                &root,
                &tok_b,
                "main-agent"
            ),
        );
        assert!(text_of(&ra.unwrap()).contains("✓ 写入"));
        assert!(text_of(&rb.unwrap()).contains("✓ 写入"));
        let content = std::fs::read_to_string(dir.join("par.txt")).unwrap();
        assert!(
            content.len() == 50_000
                && (content.chars().all(|c| c == 'A') || content.chars().all(|c| c == 'B')),
            "并发写后文件必须是某一方完整内容，不得交错: len={}",
            content.len()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

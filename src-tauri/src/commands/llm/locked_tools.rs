//! Locked execution of the built-in file tools.
//!
//! Mutating single-path file tools (`write_file` / `replace_in_file` / `delete_file` /
//! `make_directory`) take the per-file exclusive lock (`core::file_locks::FileLockRegistry`)
//! so same-file writes serialize across the main agent and all concurrently running
//! subagents, and a different last writer forces a fresh hashline anchor (gate) so lost
//! updates become detectable re-read errors. Read-only file tools pass straight through.
//!
//! Ordering invariants: the gate runs while holding the lock (last_writer is only updated
//! by lock holders, so the read is fresh) and mark_writer runs before the guard drops, so
//! the next writer always observes the latest completed writer.

use crate::core::events as ev;
use crate::core::models::ToolResult;
use crate::ierr;
use crate::utils::file_tools;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio_util::sync::CancellationToken;

/// Executes one built-in file tool (read or mutating) under the write-coordination policy.
///
/// `project_root` is the root the tool operates on (the project root, or a workspace root
/// when the caller runs isolated); `writer_id` labels the calling agent for lock holder
/// tracking and cross-writer gate decisions.
#[allow(clippy::too_many_arguments, reason = "explicit execution context")]
pub(crate) async fn execute_locked_file_tool(
    app: &AppHandle,
    tool_name: &str,
    arguments: &Value,
    project_root: &str,
    token: &CancellationToken,
    writer_id: &str,
) -> Result<ToolResult, String> {
    let lock_target = file_tools::mutating_tool_target(project_root, tool_name, arguments);
    let lock_registry = app.state::<crate::core::file_locks::FileLockRegistry>();
    let mut lock_guard: Option<crate::core::file_locks::FileLockGuard> = None;

    if let Some((path, _exists)) = &lock_target {
        let path_s = path.to_string_lossy().to_string();
        let display_path = file_tools::strip_windows_extended_prefix(&path_s);

        match lock_registry
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
                    let _ = app.emit(
                        ev::FILE_LOCK_CONTENTION,
                        json!({
                            "path": display_path,
                            "holder": g.previous_holder.clone().unwrap_or_default(),
                            "waiter": writer_id,
                            "waitedMs": g.waited_for.as_millis() as u64,
                            "timedOut": false,
                        }),
                    );
                }
                lock_guard = Some(g);
            }
            Err(crate::core::file_locks::FileLockError::Timeout { holder, waited }) => {
                let _ = app.emit(
                    ev::FILE_LOCK_CONTENTION,
                    json!({
                        "path": display_path,
                        "holder": holder,
                        "waiter": writer_id,
                        "waitedMs": waited.as_millis() as u64,
                        "timedOut": true,
                    }),
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
            lock_registry.last_writer(&path_s).as_deref(),
            writer_id,
        );
        if let Err(msg) = gate {
            return Ok(file_tools::tool_err(&msg)); // guard drops -> lock released
        }
    }

    let tool_name_c = tool_name.to_string();
    let arguments_c = arguments.clone();
    let root_c = project_root.to_string();
    let result = tokio::task::spawn_blocking(move || {
        file_tools::execute_file_tool(&tool_name_c, &arguments_c, &root_c)
    })
    .await
    .unwrap_or_else(|_| file_tools::tool_err("文件工具执行线程 panic"));

    // Record the successful writer before releasing the lock (ordering guarantee for the
    // next lock holder's gate read)
    if let (Some((path, _)), false) = (&lock_target, result.is_error) {
        lock_registry.mark_writer(&path.to_string_lossy(), writer_id);
    }
    drop(lock_guard); // released after the write completes
    Ok(result)
}

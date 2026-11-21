//! Applies pulled server changes to the local SQLite database.
//!
//! Two invariants keep local-only data intact:
//! 1. Remote writes only touch columns the sync DTO actually carries: `agent_steps_json`,
//!    `images_json`, `interim_content`, `agent_start_time` and other local-only columns
//!    are preserved on update.
//! 2. Deletes are physical (AIO has no tombstone column), so the existing cleanup helpers
//!    run in the same transaction to avoid orphaned checkpoints/attachment files.
use rusqlite::{params, Connection};
use serde_json::Value;
use std::collections::BTreeSet;

use super::dto::{RemoteAssistant, RemoteChange, RemoteMessage, RemoteThread, SnapshotEntity};
use super::outbox;

/// Outcome of one apply pass.
#[derive(Debug, Default, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyOutcome {
    /// Topics whose messages changed (the frontend refreshes only these).
    pub changed_topic_ids: Vec<String>,
    /// Entities written or removed.
    pub applied: usize,
    /// Changes skipped because a parent is missing locally.
    pub skipped: usize,
}

/// Server timestamps are ISO-8601 UTC; SQLite `CURRENT_TIMESTAMP` is
/// `YYYY-MM-DD HH:MM:SS`, and the whole local ordering (topic history, branch
/// ordering) depends on that one comparable format.
fn sqlite_timestamp(iso: &str) -> String {
    let trimmed = iso.trim();
    let core = trimmed.get(0..19).unwrap_or(trimmed).replace('T', " ");
    if core.len() == 19 {
        core
    } else {
        trimmed.to_string()
    }
}

fn json_string(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

/// Inserts or updates an assistant row, keeping local-only columns untouched.
fn apply_assistant(conn: &Connection, payload: &Value) -> Result<(), String> {
    let remote: RemoteAssistant = serde_json::from_value(payload.clone())
        .map_err(|e| format!("invalid assistant payload: {e}"))?;
    if remote.deleted_at.is_some() {
        return delete_assistant(conn, &remote.id);
    }
    let mcp_ids = Value::Array(
        remote
            .mcp_server_ids
            .iter()
            .map(|id| Value::String(id.clone()))
            .collect(),
    );
    let skill_ids = Value::Array(
        remote
            .skill_ids
            .iter()
            .map(|id| Value::String(id.clone()))
            .collect(),
    );
    conn.execute(
        "INSERT INTO assistants (id, name, prompt, model_id, mcp_server_ids, skill_ids, project_id, agent_mode, assistant_type, cloud_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, COALESCE(?8, 'off'), COALESCE(?9, 'project'), ?10)
         ON CONFLICT(id) DO UPDATE SET
           name = excluded.name,
           prompt = excluded.prompt,
           model_id = excluded.model_id,
           mcp_server_ids = excluded.mcp_server_ids,
           skill_ids = excluded.skill_ids,
           project_id = excluded.project_id,
           agent_mode = COALESCE(excluded.agent_mode, assistants.agent_mode),
           assistant_type = COALESCE(excluded.assistant_type, assistants.assistant_type),
           cloud_version = excluded.cloud_version",
        params![
            remote.id,
            remote.name,
            remote.prompt,
            remote.model_id,
            json_string(&mcp_ids),
            json_string(&skill_ids),
            remote.project_id,
            remote.agent_mode,
            remote.assistant_type,
            remote.version
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Inserts or updates a topic (server "thread") row.
fn apply_thread(conn: &Connection, payload: &Value) -> Result<Option<String>, String> {
    let remote: RemoteThread = serde_json::from_value(payload.clone())
        .map_err(|e| format!("invalid thread payload: {e}"))?;
    if remote.deleted_at.is_some() {
        delete_topic(conn, &remote.id)?;
        return Ok(Some(remote.id));
    }
    // The assistant must exist locally: the FK would reject the row and the whole
    // batch would roll back, hiding the real ordering problem.
    let assistant_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM assistants WHERE id = ?1",
            [&remote.assistant_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if assistant_exists == 0 {
        return Ok(None);
    }
    conn.execute(
        "INSERT INTO topics (id, assistant_id, name, summary, renamed, branched_from_message_id, parent_topic_id, context_trim_count, cloud_version)
         VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
           name = excluded.name,
           summary = excluded.summary,
           branched_from_message_id = excluded.branched_from_message_id,
           parent_topic_id = excluded.parent_topic_id,
           context_trim_count = excluded.context_trim_count,
           cloud_version = excluded.cloud_version",
        params![
            remote.id,
            remote.assistant_id,
            remote.title.unwrap_or_else(|| remote.summary.clone().unwrap_or_default()),
            remote.summary,
            remote.branched_from_message_id,
            remote.parent_thread_id,
            remote.context_trim_count,
            remote.version
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(Some(remote.id))
}

/// Applies one remote message.
///
/// Deliberately not `upsert_message_row`: that writer overwrites every column from the
/// IPC payload and would erase local-only fields (`agent_steps_json`, `images_json`,
/// `interim_content`, `agent_start_time`) that the sync DTO does not carry.
fn apply_message(conn: &Connection, payload: &Value) -> Result<Option<String>, String> {
    let remote: RemoteMessage = serde_json::from_value(payload.clone())
        .map_err(|e| format!("invalid message payload: {e}"))?;

    let topic_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM topics WHERE id = ?1",
            [&remote.thread_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if topic_exists == 0 {
        return Ok(None);
    }

    let timestamp = remote
        .created_at
        .as_deref()
        .map(sqlite_timestamp)
        .unwrap_or_else(local_timestamp_now);
    let tool_calls_json = remote.tool_calls.as_ref().map(json_string);

    conn.execute(
        "INSERT INTO messages
           (id, topic_id, role, content, model_id, display_text, reasoning, tool_calls_json,
            parent_message_id, branch_index, input_tokens, output_tokens, cached_input_tokens,
            context_tokens, ttft_ms, timestamp, cloud_seq, cloud_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
         ON CONFLICT(id) DO UPDATE SET
           content = excluded.content,
           role = excluded.role,
           model_id = excluded.model_id,
           display_text = excluded.display_text,
           reasoning = excluded.reasoning,
           tool_calls_json = excluded.tool_calls_json,
           parent_message_id = excluded.parent_message_id,
           branch_index = excluded.branch_index,
           input_tokens = excluded.input_tokens,
           output_tokens = excluded.output_tokens,
           cached_input_tokens = excluded.cached_input_tokens,
           context_tokens = excluded.context_tokens,
           ttft_ms = excluded.ttft_ms,
           timestamp = excluded.timestamp,
           cloud_seq = excluded.cloud_seq,
           cloud_version = excluded.cloud_version",
        params![
            remote.id,
            remote.thread_id,
            remote.role,
            json_string(&remote.content),
            remote.model_id,
            remote.display_text,
            remote.reasoning,
            tool_calls_json,
            remote.parent_message_id,
            remote.branch_index,
            remote.input_tokens,
            remote.output_tokens,
            remote.cached_input_tokens,
            remote.context_tokens,
            remote.ttft_ms,
            timestamp,
            remote.seq,
            remote.version
        ],
    )
    .map_err(|e| e.to_string())?;

    // Map server attachment ids to local ids. An attachment the device has not
    // downloaded yet keeps the server ref in `cloud_sync_state`-independent form: the
    // message is still written, and the file arrives through `cloud_attachment_download`.
    for (index, attachment) in remote.attachments.iter().enumerate() {
        let local_id: Option<String> = conn
            .query_row(
                "SELECT id FROM attachments WHERE cloud_id = ?1",
                [&attachment.id],
                |row| row.get(0),
            )
            .ok();
        if let Some(local_id) = local_id {
            conn.execute(
                "INSERT OR IGNORE INTO message_attachments (message_id, attachment_id, sort_order)
                 VALUES (?1, ?2, ?3)",
                params![remote.id, local_id, index as i64],
            )
            .map_err(|e| e.to_string())?;
        }
    }

    Ok(Some(remote.thread_id))
}

/// Wall clock in the SQLite `timestamp` format (`YYYY-MM-DD HH:MM:SS`), reusing the
/// shared time helper so local and remote rows stay directly comparable.
fn local_timestamp_now() -> String {
    crate::utils::time::now_iso().replace('T', " ")
}

fn delete_assistant(conn: &Connection, assistant_id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM assistants WHERE id = ?1", [assistant_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Deletes a topic together with its checkpoints and unreferenced attachment files.
fn delete_topic(conn: &Connection, topic_id: &str) -> Result<(), String> {
    let attachment_ids: Vec<String> = conn
        .prepare(
            "SELECT DISTINCT ma.attachment_id
             FROM message_attachments ma
             JOIN messages m ON m.id = ma.message_id
             WHERE m.topic_id = ?1",
        )
        .and_then(|mut stmt| {
            stmt.query_map([topic_id], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|e| e.to_string())?;
    crate::commands::llm::checkpoints::delete_topic_checkpoints(conn, topic_id);
    conn.execute("DELETE FROM topics WHERE id = ?1", [topic_id])
        .map_err(|e| e.to_string())?;
    crate::commands::attachment::cleanup_attachment_ids(conn, &attachment_ids)?;
    Ok(())
}

fn delete_message(conn: &Connection, message_id: &str) -> Result<(), String> {
    let attachment_ids: Vec<String> = conn
        .prepare("SELECT attachment_id FROM message_attachments WHERE message_id = ?1")
        .and_then(|mut stmt| {
            stmt.query_map([message_id], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM messages WHERE id = ?1", [message_id])
        .map_err(|e| e.to_string())?;
    crate::commands::attachment::cleanup_attachment_ids(conn, &attachment_ids)?;
    Ok(())
}

/// Applies one change (upsert or delete) and reports the touched topic.
///
/// Applied changes are also written to `cloud_sync_state` so conflict resolution and
/// diagnostics can tell "this row came from the server at version N".
pub(crate) fn apply_change(
    conn: &Connection,
    change_type: &str,
    entity_id: &str,
    version: i64,
    payload: Option<&Value>,
) -> Result<Option<String>, String> {
    let (entity, op) = change_type
        .split_once('.')
        .unwrap_or((change_type, "upsert"));

    if op == "delete" {
        match entity {
            "assistant" => delete_assistant(conn, entity_id)?,
            "thread" => delete_topic(conn, entity_id)?,
            "message" => delete_message(conn, entity_id)?,
            _ => {}
        }
        outbox::record_synced(conn, entity, entity_id, version, None)?;
        return Ok(None);
    }

    let Some(payload) = payload else {
        return Ok(None);
    };

    match entity {
        "assistant" => {
            apply_assistant(conn, payload)?;
            outbox::record_synced(conn, "assistant", entity_id, version, None)?;
            Ok(None)
        }
        "thread" => {
            let topic = apply_thread(conn, payload)?;
            outbox::record_synced(conn, "thread", entity_id, version, None)?;
            Ok(topic)
        }
        "message" => {
            let topic = apply_message(conn, payload)?;
            let seq = payload.get("seq").and_then(Value::as_i64);
            outbox::record_synced(conn, "message", entity_id, version, seq)?;
            Ok(topic)
        }
        _ => Ok(None),
    }
}

/// Applies a pulled page (or one snapshot page) inside one transaction.
///
/// A missing parent is not an error: push order guarantees parents travel first, so the
/// next page/tick retries the child. Changes are applied in cursor order.
pub(crate) fn apply_changes(
    conn: &Connection,
    changes: &[RemoteChange],
) -> Result<ApplyOutcome, String> {
    let mut outcome = ApplyOutcome::default();
    let mut touched: BTreeSet<String> = BTreeSet::new();
    for change in changes {
        // Pull reports the bare entity type plus `op` (`message` + `delete`), while push
        // and conflict payloads use the dotted form (`message.delete`). Compose once here
        // so a delete can never be mistaken for an upsert with a missing payload.
        let change_type = if change.change_type.contains('.') {
            change.change_type.clone()
        } else if change.op.is_empty() {
            format!("{}.upsert", change.change_type)
        } else {
            format!("{}.{}", change.change_type, change.op)
        };
        match apply_change(
            conn,
            &change_type,
            &change.id,
            change.version,
            change.payload.as_ref(),
        )? {
            Some(topic) => {
                outcome.applied += 1;
                touched.insert(topic);
            }
            None => outcome.applied += 1,
        }
    }
    outcome.changed_topic_ids = touched.into_iter().collect();
    Ok(outcome)
}

/// Applies one snapshot page; `deleted` entities are removed locally.
pub(crate) fn apply_snapshot(
    conn: &Connection,
    entities: &[SnapshotEntity],
) -> Result<ApplyOutcome, String> {
    let mut outcome = ApplyOutcome::default();
    let mut touched: BTreeSet<String> = BTreeSet::new();
    for entity in entities {
        if entity.deleted {
            match entity.entity_type.as_str() {
                "assistant" => delete_assistant(conn, &entity.id)?,
                "thread" => delete_topic(conn, &entity.id)?,
                "message" => delete_message(conn, &entity.id)?,
                _ => {}
            }
            outbox::record_synced(conn, &entity.entity_type, &entity.id, entity.version, None)?;
            outcome.applied += 1;
            continue;
        }
        match apply_change(
            conn,
            &format!("{}.upsert", entity.entity_type),
            &entity.id,
            entity.version,
            entity.payload.as_ref(),
        )? {
            Some(topic) => {
                touched.insert(topic);
                outcome.applied += 1;
            }
            None => outcome.applied += 1,
        }
    }
    outcome.changed_topic_ids = touched.into_iter().collect();
    Ok(outcome)
}

/// Applies the current server payload returned with a `409 version_conflict`.
///
/// Server-wins in Phase 1: the stale local edit is replaced, the queue row is dropped,
/// and the local row records the server version so the next edit compares against it.
pub(crate) fn apply_conflict(
    conn: &Connection,
    entity_id: &str,
    change_type: &str,
    server_version: i64,
    payload: Option<&Value>,
) -> Result<(), String> {
    let entity = change_type
        .split_once('.')
        .map(|(entity, _)| entity)
        .unwrap_or(change_type);
    if let Some(payload) = payload {
        apply_change(
            conn,
            &format!("{entity}.upsert"),
            entity_id,
            server_version,
            Some(payload),
        )?;
    } else {
        outbox::record_synced(conn, entity, entity_id, server_version, None)?;
    }
    outbox::remove(conn, entity, entity_id)?;
    Ok(())
}

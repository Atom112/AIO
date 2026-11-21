//! Local change queue (outbox) for cloud sync.
//!
//! Every local mutation that must reach other devices appends one row per entity to
//! `cloud_outbox` in the same transaction as the write. Payloads are built here from
//! the persisted row (never from IPC input), so what leaves the machine is exactly what
//! the database holds, with the two exclusions the protocol requires: agent steps and
//! full tool-result bodies stay local in Phase 1.
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use super::dto::OutboundChange;

/// One batch may not exceed these limits (mirrors the server contract).
pub(crate) const MAX_CHANGES_PER_BATCH: usize = 200;
pub(crate) const MAX_BATCH_BYTES: usize = 5 * 1024 * 1024;

/// The signed-in account binding (one workspace per installation).
#[derive(Debug, Clone)]
pub(crate) struct Account {
    pub server_url: String,
    pub workspace_id: String,
    pub user_id: String,
    pub user_login: String,
    pub device_id: String,
    pub last_cursor: Option<String>,
}

/// Reads the single `cloud_account` row, if the installation is linked.
pub(crate) fn account(conn: &Connection) -> Result<Option<Account>, String> {
    conn.query_row(
        "SELECT server_url, workspace_id, user_id, user_login, device_id, last_cursor
         FROM cloud_account WHERE id = 1",
        [],
        |row| {
            Ok(Account {
                server_url: row.get(0)?,
                workspace_id: row.get(1)?,
                user_id: row.get(2)?,
                user_login: row.get(3)?,
                device_id: row.get(4)?,
                last_cursor: row.get(5)?,
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// Persists the account binding, resetting the cursor when the workspace changes.
pub(crate) fn bind_account(conn: &Connection, account: &Account) -> Result<(), String> {
    conn.execute(
        "INSERT INTO cloud_account (id, server_url, workspace_id, user_id, user_login, device_id, last_cursor, created_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, CURRENT_TIMESTAMP)
         ON CONFLICT(id) DO UPDATE SET
           server_url = excluded.server_url,
           workspace_id = excluded.workspace_id,
           user_id = excluded.user_id,
           user_login = excluded.user_login,
           device_id = excluded.device_id,
           last_cursor = excluded.last_cursor",
        params![
            account.server_url,
            account.workspace_id,
            account.user_id,
            account.user_login,
            account.device_id,
            account.last_cursor
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// One queued change to write into the outbox.
pub(crate) struct EnqueueChange<'a> {
    pub entity_type: &'a str,
    pub entity_id: &'a str,
    pub op: &'a str,
    pub payload: Option<&'a Value>,
    pub base_version: Option<i64>,
    pub local_order: Option<&'a str>,
    pub priority: i64,
}

/// Queues a change. No-op when the installation is not linked to a workspace.
///
/// Mapping by `(entity_type, entity_id)` means a repeated edit of the same row
/// collapses into one queue entry instead of growing the queue.
pub(crate) fn enqueue(conn: &Connection, change: &EnqueueChange<'_>) -> Result<(), String> {
    if account(conn)?.is_none() {
        return Ok(());
    }
    let payload_json = match change.payload {
        Some(value) => serde_json::to_string(value).map_err(|e| e.to_string())?,
        None => "null".to_string(),
    };
    conn.execute(
        "INSERT INTO cloud_outbox
           (entity_type, entity_id, op, payload_json, base_version, local_order, priority, enqueued_at, attempts, last_error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, CURRENT_TIMESTAMP, 0, NULL)
         ON CONFLICT(entity_type, entity_id) DO UPDATE SET
           op = excluded.op,
           payload_json = excluded.payload_json,
           base_version = excluded.base_version,
           local_order = excluded.local_order,
           priority = excluded.priority,
           enqueued_at = CURRENT_TIMESTAMP,
           last_error = NULL",
        params![
            change.entity_type,
            change.entity_id,
            change.op,
            payload_json,
            change.base_version,
            change.local_order,
            change.priority
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Queues a change without letting bookkeeping failures break a local write.
///
/// The local database is the source of truth: a failed enqueue is logged and the row
/// is picked up by the next full save instead of failing the user's edit.
pub(crate) fn enqueue_best_effort(conn: &Connection, change: &EnqueueChange<'_>) {
    if let Err(error) = enqueue(conn, change) {
        tracing::warn!(
            "[cloud] 无法入队同步变更 {}.{} ({}): {error}",
            change.entity_type,
            change.entity_id,
            change.op
        );
    }
}

/// Reads the local cloud version of a row (`assistants` / `topics`).
pub(crate) fn local_version(conn: &Connection, table: &str, id: &str) -> Option<i64> {
    if table != "assistants" && table != "topics" {
        return None;
    }
    conn.query_row(
        &format!("SELECT cloud_version FROM {table} WHERE id = ?1"),
        [id],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .ok()
    .flatten()
}

fn split_json_list(raw: Option<String>) -> Value {
    raw.and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(Value::is_array)
        .unwrap_or_else(|| json!([]))
}

/// Builds the assistant payload from the persisted row.
pub(crate) fn assistant_payload(conn: &Connection, assistant_id: &str) -> Option<Value> {
    conn.query_row(
        "SELECT id, name, prompt, model_id, mcp_server_ids, skill_ids, project_id, agent_mode, assistant_type
         FROM assistants WHERE id = ?1",
        [assistant_id],
        |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "name": row.get::<_, String>(1)?,
                "prompt": row.get::<_, Option<String>>(2)?,
                "modelId": row.get::<_, Option<String>>(3)?,
                "mcpServerIds": split_json_list(row.get(4)?),
                "skillIds": split_json_list(row.get(5)?),
                "projectId": row.get::<_, Option<String>>(6)?,
                "agentMode": row.get::<_, Option<String>>(7)?,
                "assistantType": row.get::<_, Option<String>>(8)?,
            }))
        },
    )
    .optional()
    .ok()
    .flatten()
}

/// Builds the thread (local topic) payload from the persisted row.
pub(crate) fn thread_payload(conn: &Connection, topic_id: &str) -> Option<Value> {
    conn.query_row(
        "SELECT id, assistant_id, name, summary, context_trim_count, branched_from_message_id, parent_topic_id
         FROM topics WHERE id = ?1",
        [topic_id],
        |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "assistantId": row.get::<_, String>(1)?,
                "title": row.get::<_, String>(2)?,
                "summary": row.get::<_, Option<String>>(3)?,
                "contextTrimCount": row.get::<_, i64>(4)?,
                "branchedFromMessageId": row.get::<_, Option<String>>(5)?,
                "parentThreadId": row.get::<_, Option<String>>(6)?,
            }))
        },
    )
    .optional()
    .ok()
    .flatten()
}

/// Copies tool calls without the local-only full result bodies.
fn strip_tool_calls(raw: Option<String>) -> Option<Value> {
    let parsed: Value = serde_json::from_str(&raw?).ok()?;
    let Value::Array(items) = parsed else {
        return None;
    };
    let stripped: Vec<Value> = items
        .into_iter()
        .map(|mut item| {
            if let Value::Object(map) = &mut item {
                map.remove("fullContent");
                map.remove("full_content");
            }
            item
        })
        .collect();
    Some(Value::Array(stripped))
}

/// Attachment references for one message, in display order, already uploaded.
///
/// `None` means at least one attachment has no server id yet: the message must wait
/// so it never references a file other devices cannot fetch.
fn message_attachment_refs(conn: &Connection, message_id: &str) -> Result<Option<Value>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.cloud_id, a.file_name, a.mime_type, a.sha256, a.size
             FROM message_attachments ma
             JOIN attachments a ON a.id = ma.attachment_id
             WHERE ma.message_id = ?1
             ORDER BY ma.sort_order ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([message_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mut refs = Vec::with_capacity(rows.len());
    for (_local_id, cloud_id, file_name, mime_type, sha256, size_bytes) in rows {
        let Some(cloud_id) = cloud_id else {
            return Ok(None);
        };
        refs.push(json!({
            "id": cloud_id,
            "fileName": file_name,
            "mimeType": mime_type,
            "sha256": sha256,
            "sizeBytes": size_bytes,
        }));
    }
    Ok(Some(Value::Array(refs)))
}

/// Builds the message payload from the persisted row, or `None` while a referenced
/// attachment is still waiting for its server id.
pub(crate) fn message_payload(
    conn: &Connection,
    message_id: &str,
) -> Result<Option<Value>, String> {
    let Some(refs) = message_attachment_refs(conn, message_id)? else {
        return Ok(None);
    };
    let row = conn
        .query_row(
            "SELECT id, topic_id, role, content, display_text, model_id, reasoning,
                    tool_calls_json, parent_message_id, branch_index, input_tokens,
                    output_tokens, cached_input_tokens, context_tokens, ttft_ms
             FROM messages WHERE id = ?1",
            [message_id],
            |row| {
                let content_text: String = row.get(3)?;
                let content_value = serde_json::from_str::<Value>(&content_text)
                    .unwrap_or(Value::String(content_text));
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "threadId": row.get::<_, String>(1)?,
                    "role": row.get::<_, String>(2)?,
                    "content": content_value,
                    "displayText": row.get::<_, Option<String>>(4)?,
                    "modelId": row.get::<_, Option<String>>(5)?,
                    "reasoning": row.get::<_, Option<String>>(6)?,
                    "toolCalls": strip_tool_calls(row.get::<_, Option<String>>(7)?),
                    "parentMessageId": row.get::<_, Option<String>>(8)?,
                    "branchIndex": row.get::<_, i64>(9)?,
                    "inputTokens": row.get::<_, Option<i64>>(10)?,
                    "outputTokens": row.get::<_, Option<i64>>(11)?,
                    "cachedInputTokens": row.get::<_, Option<i64>>(12)?,
                    "contextTokens": row.get::<_, Option<i64>>(13)?,
                    "ttftMs": row.get::<_, Option<i64>>(14)?,
                }))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;

    let Some(mut payload) = row else {
        return Ok(None);
    };
    if let Value::Object(map) = &mut payload {
        map.insert("attachments".to_string(), refs);
    }
    Ok(Some(payload))
}

/// Local sort key for messages: the SQLite timestamp keeps chronological order.
fn message_order(conn: &Connection, message_id: &str) -> Option<String> {
    conn.query_row(
        "SELECT COALESCE(timestamp, CURRENT_TIMESTAMP) FROM messages WHERE id = ?1",
        [message_id],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
}

/// Enqueues one assistant row (priority 0).
pub(crate) fn enqueue_assistant(conn: &Connection, assistant_id: &str) {
    let Some(payload) = assistant_payload(conn, assistant_id) else {
        return;
    };
    let base = local_version(conn, "assistants", assistant_id).unwrap_or(0);
    enqueue_best_effort(
        conn,
        &EnqueueChange {
            entity_type: "assistant",
            entity_id: assistant_id,
            op: "upsert",
            payload: Some(&payload),
            base_version: (base > 0).then_some(base),
            local_order: None,
            priority: 0,
        },
    );
}

/// Enqueues one topic (thread) at priority 1.
pub(crate) fn enqueue_topic(conn: &Connection, topic_id: &str) {
    let Some(payload) = thread_payload(conn, topic_id) else {
        return;
    };
    let base = local_version(conn, "topics", topic_id).unwrap_or(0);
    enqueue_best_effort(
        conn,
        &EnqueueChange {
            entity_type: "thread",
            entity_id: topic_id,
            op: "upsert",
            payload: Some(&payload),
            base_version: (base > 0).then_some(base),
            local_order: None,
            priority: 1,
        },
    );
}

/// Enqueues every message of a topic at priority 3 in chronological order.
pub(crate) fn enqueue_messages_of(conn: &Connection, topic_id: &str) {
    let ids: Vec<String> = conn
        .prepare("SELECT id FROM messages WHERE topic_id = ?1 ORDER BY timestamp ASC")
        .and_then(|mut stmt| {
            stmt.query_map([topic_id], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()
        })
        .unwrap_or_default();
    for id in ids {
        enqueue_message(conn, &id);
    }
}

/// Queues the message's large payload artifacts (agent steps, full tool results).
///
/// Priority 4 and a separate entity type keep them out of the message push: the server
/// must accept the message first so the payload flags always reference an existing row.
pub(crate) fn enqueue_message_payloads(conn: &Connection, message_id: &str) {
    if account(conn).ok().flatten().is_none() {
        return;
    }
    let agent_steps: Option<String> = conn
        .query_row(
            "SELECT agent_steps_json FROM messages WHERE id = ?1",
            [message_id],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten();
    let tool_calls: Option<String> = conn
        .query_row(
            "SELECT tool_calls_json FROM messages WHERE id = ?1",
            [message_id],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten();

    let mut kinds: Vec<&str> = Vec::new();
    if agent_steps
        .as_deref()
        .is_some_and(|value| value != "null" && !value.trim().is_empty())
    {
        kinds.push("agent_steps");
    }
    if tool_calls
        .as_deref()
        .is_some_and(|value| value.contains("fullContent"))
    {
        kinds.push("tool_full_result");
    }
    for kind in kinds {
        let entity_id = format!("{message_id}:{kind}");
        enqueue_best_effort(
            conn,
            &EnqueueChange {
                entity_type: "message_payload",
                entity_id: &entity_id,
                op: "upsert",
                payload: None,
                base_version: None,
                local_order: None,
                priority: 4,
            },
        );
    }
}

/// Enqueues one message at priority 3.
pub(crate) fn enqueue_message(conn: &Connection, message_id: &str) {
    let Ok(Some(payload)) = message_payload(conn, message_id) else {
        return;
    };
    let base = conn
        .query_row(
            "SELECT cloud_version FROM messages WHERE id = ?1",
            [message_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .ok()
        .flatten()
        .unwrap_or(0);
    let order = message_order(conn, message_id);
    enqueue_best_effort(
        conn,
        &EnqueueChange {
            entity_type: "message",
            entity_id: message_id,
            op: "upsert",
            payload: Some(&payload),
            base_version: (base > 0).then_some(base),
            local_order: order.as_deref(),
            priority: 3,
        },
    );
    enqueue_message_payloads(conn, message_id);
}

/// Queues one attachment for upload (priority 2, before the messages referencing it).
///
/// No-op when signed out, when the attachment already has a server id, or when the row
/// no longer exists (a message can reference a just-cleaned file).
pub(crate) fn enqueue_attachment(conn: &Connection, attachment_id: &str) {
    if account(conn).ok().flatten().is_none() {
        return;
    }
    let needs_upload: Option<Option<String>> = conn
        .query_row(
            "SELECT cloud_id FROM attachments WHERE id = ?1",
            [attachment_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .ok()
        .flatten();
    match needs_upload {
        Some(None) => {}
        _ => return,
    }
    let payload = serde_json::json!({ "id": attachment_id });
    enqueue_best_effort(
        conn,
        &EnqueueChange {
            entity_type: "attachment",
            entity_id: attachment_id,
            op: "upload",
            payload: Some(&payload),
            base_version: None,
            local_order: None,
            priority: 2,
        },
    );
}

/// Enqueues a tombstone for a row that was just deleted locally.
pub(crate) fn enqueue_tombstone(
    conn: &Connection,
    entity_type: &str,
    entity_id: &str,
    base_version: Option<i64>,
    priority: i64,
) {
    enqueue_best_effort(
        conn,
        &EnqueueChange {
            entity_type,
            entity_id,
            op: "delete",
            payload: None,
            base_version: base_version.filter(|v| *v > 0),
            local_order: None,
            priority,
        },
    );
}

/// One queued row ready to be sent.
struct PendingRow {
    entity_type: String,
    entity_id: String,
    op: String,
    payload: Option<Value>,
    base_version: Option<i64>,
    local_order: Option<String>,
}

/// Reads the next batch from the outbox in `(priority, local_order)` order.
///
/// Rows whose payload can no longer be built (the entity vanished between the write
/// and the flush) are dropped instead of blocking the queue. Attachment rows are
/// excluded: they are uploaded by [`super::attachments`] before the batch.
pub(crate) fn next_batch(conn: &Connection, limit: usize) -> Result<Vec<OutboundChange>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT entity_type, entity_id, op, payload_json, base_version, local_order
             FROM cloud_outbox
             WHERE entity_type NOT IN ('attachment', 'message_payload')
             ORDER BY priority ASC, COALESCE(local_order, '') ASC, enqueued_at ASC
             LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([limit as i64], |row| {
            Ok(PendingRow {
                entity_type: row.get(0)?,
                entity_id: row.get(1)?,
                op: row.get(2)?,
                payload: row.get::<_, String>(3).ok().and_then(|text| {
                    if text == "null" {
                        None
                    } else {
                        serde_json::from_str(&text).ok()
                    }
                }),
                base_version: row.get(4)?,
                local_order: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mut batch = Vec::new();
    let mut bytes = 0usize;
    for row in rows {
        // Rebuild message payloads at flush time: attachment uploads may have completed
        // since the row was queued, and the payload must carry fresh server ids.
        let (payload, ready) = if row.op == "upsert" && row.entity_type == "message" {
            match message_payload(conn, &row.entity_id)? {
                Some(payload) => (Some(payload), true),
                None => (None, false),
            }
        } else {
            (row.payload.clone(), true)
        };
        if !ready {
            continue;
        }
        if batch.len() >= MAX_CHANGES_PER_BATCH {
            break;
        }
        let change = OutboundChange {
            change_id: uuid::Uuid::new_v4().to_string(),
            change_type: format!("{}.{}", row.entity_type, row.op),
            id: row.entity_id.clone(),
            base_version: row.base_version,
            local_order: row.local_order.clone(),
            data: if row.op == "delete" { None } else { payload },
        };
        let size = serde_json::to_string(&change).map(|s| s.len()).unwrap_or(0);
        if bytes + size > MAX_BATCH_BYTES && !batch.is_empty() {
            break;
        }
        bytes += size;
        batch.push(change);
    }
    Ok(batch)
}

/// Removes a queued row after the server accepted (or superseded) it.
pub(crate) fn remove(conn: &Connection, entity_type: &str, entity_id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM cloud_outbox WHERE entity_type = ?1 AND entity_id = ?2",
        params![entity_type, entity_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Records the version/seq the server assigned to a row.
pub(crate) fn record_synced(
    conn: &Connection,
    entity_type: &str,
    entity_id: &str,
    version: i64,
    seq: Option<i64>,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO cloud_sync_state (entity_type, entity_id, remote_version, remote_seq, synced_at)
         VALUES (?1, ?2, ?3, ?4, CURRENT_TIMESTAMP)
         ON CONFLICT(entity_type, entity_id) DO UPDATE SET
           remote_version = excluded.remote_version,
           remote_seq = excluded.remote_seq,
           synced_at = CURRENT_TIMESTAMP",
        params![entity_type, entity_id, version, seq],
    )
    .map_err(|e| e.to_string())?;
    match entity_type {
        "assistant" => {
            conn.execute(
                "UPDATE assistants SET cloud_version = ?2 WHERE id = ?1",
                params![entity_id, version],
            )
            .map_err(|e| e.to_string())?;
        }
        "thread" => {
            conn.execute(
                "UPDATE topics SET cloud_version = ?2 WHERE id = ?1",
                params![entity_id, version],
            )
            .map_err(|e| e.to_string())?;
        }
        "message" => {
            conn.execute(
                "UPDATE messages SET cloud_version = ?2, cloud_seq = COALESCE(?3, cloud_seq) WHERE id = ?1",
                params![entity_id, version, seq],
            )
            .map_err(|e| e.to_string())?;
        }
        _ => {}
    }
    Ok(())
}

/// Number of queued changes (drives the "pending" counter in the UI).
pub(crate) fn pending_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM cloud_outbox", [], |row| row.get(0))
        .unwrap_or(0)
}

/// Clears every sync bookkeeping table (sign-out).
pub(crate) fn reset(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "DELETE FROM cloud_outbox; DELETE FROM cloud_sync_state; DELETE FROM cloud_account;",
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded_connection() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch(
            "CREATE TABLE cloud_account (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                server_url TEXT NOT NULL,
                workspace_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                user_login TEXT NOT NULL,
                device_id TEXT NOT NULL,
                last_cursor TEXT,
                created_at TEXT NOT NULL
            );
            CREATE TABLE cloud_outbox (
                entity_type TEXT NOT NULL,
                entity_id TEXT NOT NULL,
                op TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                base_version INTEGER,
                local_order TEXT,
                priority INTEGER NOT NULL DEFAULT 1,
                enqueued_at TEXT NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0,
                last_error TEXT,
                PRIMARY KEY(entity_type, entity_id)
            );
            INSERT INTO cloud_account
                (id, server_url, workspace_id, user_id, user_login, device_id, last_cursor, created_at)
                VALUES (1, 'https://cloud.test', 'ws1', 'u1', 'octocat', 'dev1', NULL, CURRENT_TIMESTAMP);",
        )
        .expect("create cloud tables");
        conn
    }

    /// Repeated edits of one row collapse into a single queue entry (last edit wins).
    #[test]
    fn outbox_upsert_collapses_repeated_edits() {
        let conn = seeded_connection();
        for attempt in 0..3 {
            enqueue_best_effort(
                &conn,
                &EnqueueChange {
                    entity_type: "thread",
                    entity_id: "t1",
                    op: "upsert",
                    payload: Some(&serde_json::json!({ "title": format!("edit {attempt}") })),
                    base_version: None,
                    local_order: None,
                    priority: 1,
                },
            );
        }

        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM cloud_outbox", [], |row| row.get(0))
            .expect("count outbox");
        assert_eq!(rows, 1, "one row per entity");
        let payload: String = conn
            .query_row("SELECT payload_json FROM cloud_outbox", [], |row| {
                row.get(0)
            })
            .expect("read payload");
        assert!(payload.contains("edit 2"), "last edit wins: {payload}");
    }

    /// Without a linked account every enqueue is a silent no-op, so local editing keeps
    /// working for signed-out users.
    #[test]
    fn enqueue_is_noop_when_signed_out() {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch(
            "CREATE TABLE cloud_account (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                server_url TEXT NOT NULL,
                workspace_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                user_login TEXT NOT NULL,
                device_id TEXT NOT NULL,
                last_cursor TEXT,
                created_at TEXT NOT NULL
            );
            CREATE TABLE cloud_outbox (
                entity_type TEXT NOT NULL,
                entity_id TEXT NOT NULL,
                op TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                base_version INTEGER,
                local_order TEXT,
                priority INTEGER NOT NULL DEFAULT 1,
                enqueued_at TEXT NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0,
                last_error TEXT,
                PRIMARY KEY(entity_type, entity_id)
            );",
        )
        .expect("create cloud tables");
        enqueue_best_effort(
            &conn,
            &EnqueueChange {
                entity_type: "thread",
                entity_id: "t1",
                op: "upsert",
                payload: Some(&serde_json::json!({ "title": "x" })),
                base_version: None,
                local_order: None,
                priority: 1,
            },
        );
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM cloud_outbox", [], |row| row.get(0))
            .expect("count outbox");
        assert_eq!(rows, 0);
    }
}

//! Non-command sync helpers: short-lived DB lock scopes around queue, apply and
//! bookkeeping work. Kept out of `mod.rs` so commands stay readable and no lock is ever
//! held across an `.await` point.
use rusqlite::Connection;
use tauri::{AppHandle, Manager};

use super::apply::{self, ApplyOutcome};
use super::dto::{OutboundChange, PushResponse, RemoteChange, SnapshotEntity};
use super::outbox::{self, Account};

/// Locks the chat database and runs `f` with it. The lock is released when the closure
/// returns, which is why every network call lives outside these helpers.
pub(crate) fn with_conn<R>(
    app: &AppHandle,
    f: impl FnOnce(&Connection) -> Result<R, String>,
) -> Result<R, String> {
    let state = app.state::<crate::core::state::DbState>();
    let conn = state.0.lock();
    f(&conn)
}

/// Current account binding, if the installation is linked to a workspace.
pub(crate) fn account(app: &AppHandle) -> Result<Option<Account>, String> {
    with_conn(app, outbox::account)
}

/// Next push batch (bounded by the server's change/size caps).
pub(crate) fn queued_batch(app: &AppHandle) -> Result<Vec<OutboundChange>, String> {
    with_conn(app, |conn| {
        outbox::next_batch(conn, outbox::MAX_CHANGES_PER_BATCH)
    })
}

/// Number of queued changes plus deferred attachment uploads.
pub(crate) fn pending_count(app: &AppHandle) -> i64 {
    with_conn(app, |conn| Ok(outbox::pending_count(conn))).unwrap_or(0)
}

/// Marks every change the server accepted, applies server-wins conflicts and clears
/// their queue rows. Returns `(applied, conflicts)`.
pub(crate) fn record_push_result(
    app: &AppHandle,
    response: &PushResponse,
) -> Result<(usize, usize), String> {
    with_conn(app, |conn| {
        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| e.to_string())?;
        let result = (|| -> Result<(usize, usize), String> {
            for applied in &response.applied {
                let entity = applied
                    .change_type
                    .split_once('.')
                    .map(|(entity, _)| entity)
                    .unwrap_or(applied.change_type.as_str());
                outbox::record_synced(conn, entity, &applied.id, applied.version, applied.seq)?;
                outbox::remove(conn, entity, &applied.id)?;
            }
            for conflict in &response.conflicts {
                // Conflict entries carry no entity type, so read it from the queue row
                // that produced the change (ids are unique per entity type locally).
                let entity_type = queued_entity_type(conn, &conflict.id)?
                    .unwrap_or_else(|| "message".to_string());
                apply::apply_conflict(
                    conn,
                    &conflict.id,
                    &format!("{entity_type}.upsert"),
                    conflict.server_version,
                    conflict.payload.as_ref(),
                )?;
            }
            Ok((response.applied.len(), response.conflicts.len()))
        })();
        match result {
            Ok(value) => {
                conn.execute_batch("COMMIT").map_err(|e| e.to_string())?;
                Ok(value)
            }
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    })
}

/// Entity type still queued for `entity_id` (conflict entries do not carry it).
pub(crate) fn queued_entity_type(
    conn: &Connection,
    entity_id: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT entity_type FROM cloud_outbox WHERE entity_id = ?1 LIMIT 1",
        [entity_id],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .map_or(Ok(None), |value| Ok(Some(value)))
}

/// Applies one pulled page and advances the cursor in the same transaction.
///
/// Keeping both in one transaction is what makes the loop crash-safe: a process death
/// either applies the page and moves the cursor, or does neither.
pub(crate) fn apply_pulled(
    app: &AppHandle,
    changes: &[RemoteChange],
    cursor: Option<&str>,
) -> Result<ApplyOutcome, String> {
    with_conn(app, |conn| {
        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| e.to_string())?;
        let outcome = apply::apply_changes(conn, changes).and_then(|outcome| {
            if let Some(cursor) = cursor {
                conn.execute(
                    "UPDATE cloud_account SET last_cursor = ?1 WHERE id = 1",
                    [cursor],
                )
                .map_err(|e| e.to_string())?;
            }
            Ok(outcome)
        });
        match outcome {
            Ok(outcome) => {
                conn.execute_batch("COMMIT").map_err(|e| e.to_string())?;
                Ok(outcome)
            }
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    })
}

/// Applies one snapshot page inside a transaction.
pub(crate) fn apply_snapshot_page(
    app: &AppHandle,
    entities: &[SnapshotEntity],
) -> Result<ApplyOutcome, String> {
    with_conn(app, |conn| {
        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| e.to_string())?;
        match apply::apply_snapshot(conn, entities) {
            Ok(outcome) => {
                conn.execute_batch("COMMIT").map_err(|e| e.to_string())?;
                Ok(outcome)
            }
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    })
}

/// Removes everything that must not outlive a sign-out. Local conversations stay.
pub(crate) fn reset_local(app: &AppHandle) -> Result<(), String> {
    with_conn(app, |conn| {
        outbox::reset(conn)?;
        // Attachments keep the file on disk but lose the server link, so a later link to
        // another workspace re-uploads them instead of referencing foreign ids.
        conn.execute("UPDATE attachments SET cloud_id = NULL", [])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
}

/// Persists the account binding (device id is generated once per installation).
pub(crate) fn bind(app: &AppHandle, account: &Account) -> Result<(), String> {
    with_conn(app, |conn| outbox::bind_account(conn, account))
}

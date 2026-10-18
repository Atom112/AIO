//! Agent turn checkpoints: persist a context snapshot at the end of each round so a crashed process can resume losslessly.
//!
//! Semantics:
//! - Snapshot = full JSON of messages_for_api (the exact form the loop consumes; ready to use on restore);
//! - status: active (epilogue not reached = crash leftover, recoverable) / completed / cancelled;
//!   both user cancellation and normal completion reach the epilogue and mark it, so active is the "recoverable" signal;
//! - Retention: per turn keep the latest 1 + one per 10 rounds (guards against bloat);
//! - Restore: run_agent_turn takes resume_turn_id -> loads the latest active snapshot to replace the initial messages.
//!
//! Known limitation (v1): the resumed turn's event stream starts from the recovery point, so frontend history lacks
//! pre-crash intermediate steps (model context stays complete; the UI timeline shows from the recovery point).

use rusqlite::Connection;
use serde::Serialize;

/// Summary of an active checkpoint (used by the frontend "resume unfinished turn" entry point).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ActiveCheckpoint {
    pub turn_id: String,
    pub topic_id: String,
    pub round: i64,
    pub created_at: i64,
}

/// Persist a round snapshot (older snapshots of the same turn are pruned per the retention policy).
/// Test/back-compat convenience: serializes then delegates to [`save_checkpoint_json`].
/// Production call sites hold a global SQLite lock and use `save_checkpoint_json`
/// directly (LOOP-7: serialize outside the lock).
#[cfg(test)]
pub(crate) fn save_checkpoint(
    conn: &Connection,
    topic_id: &str,
    turn_id: &str,
    round: u32,
    messages: &[serde_json::Value],
) {
    let messages_json = serde_json::to_string(messages).unwrap_or_default();
    save_checkpoint_json(conn, topic_id, turn_id, round, &messages_json);
}

/// LOOP-7: checkpoint write taking a PRE-SERIALIZED context.
/// Callers that hold a global SQLite lock must serialize outside the lock —
/// serializing a multi-MB agent context while holding `DbState` blocks every
/// other database user (memory tools, usage tracking, checkpoint reads) for the
/// whole duration. The two SQL statements here are the only lock-held work.
pub(crate) fn save_checkpoint_json(
    conn: &Connection,
    topic_id: &str,
    turn_id: &str,
    round: u32,
    messages_json: &str,
) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // (审查修复 L-3) 崩溃恢复快照是本功能的核心语义，写失败必须可见——
    // 静默吞错会让用户以为"崩溃可无损续跑"而实际快照早已没在写。
    if let Err(e) = conn.execute(
        "INSERT OR REPLACE INTO agent_turn_checkpoints
            (topic_id, turn_id, round, messages_json, status, created_at)
         VALUES (?1, ?2, ?3, ?4, 'active', ?5)",
        rusqlite::params![topic_id, turn_id, round, messages_json, now],
    ) {
        tracing::warn!(
            "[checkpoint] 写入回合快照失败 (topic={topic_id} turn={turn_id} round={round}): {e}"
        );
    }
    // Retention policy: per turn keep only the latest 1 + one per 10 rounds
    // (retention failure is non-fatal: worst case is extra snapshot rows)
    let _ = conn.execute(
        "DELETE FROM agent_turn_checkpoints
         WHERE topic_id = ?1 AND turn_id = ?2 AND status = 'active'
           AND round < ?3 AND round % 10 != 0",
        rusqlite::params![topic_id, turn_id, round.saturating_sub(1)],
    );
}

/// Load the latest active snapshot messages for a turn (for recovery). Returns None when no snapshot exists.
pub(crate) fn load_checkpoint_messages(
    conn: &Connection,
    topic_id: &str,
    turn_id: &str,
) -> Option<Vec<serde_json::Value>> {
    let mut stmt = conn
        .prepare(
            "SELECT messages_json FROM agent_turn_checkpoints
             WHERE topic_id = ?1 AND turn_id = ?2 AND status = 'active'
             ORDER BY round DESC LIMIT 1",
        )
        .ok()?;
    let json: String = stmt
        .query_row(rusqlite::params![topic_id, turn_id], |r| r.get(0))
        .ok()?;
    serde_json::from_str(&json).ok()
}

/// Mark all snapshots of a turn as terminal (completed / cancelled). Called from the epilogue.
/// Terminal state keeps only the last round (earlier rounds deleted), uniformly for both completed and cancelled.
pub(crate) fn finalize_checkpoints(conn: &Connection, topic_id: &str, turn_id: &str, status: &str) {
    // (审查修复 L-3) 同上：终态标记失败会让快照永远停留为 active，下次启动被
    // 误判为"可恢复的崩溃残留"。
    if let Err(e) = conn.execute(
        "UPDATE agent_turn_checkpoints SET status = ?3
         WHERE topic_id = ?1 AND turn_id = ?2",
        rusqlite::params![topic_id, turn_id, status],
    ) {
        tracing::warn!(
            "[checkpoint] 终态标记失败 (topic={topic_id} turn={turn_id} status={status}): {e}"
        );
    }
    // Terminal state keeps only the last round (the latest snapshot stays queryable for audit/debugging)
    let _ = conn.execute(
        "DELETE FROM agent_turn_checkpoints
         WHERE topic_id = ?1 AND turn_id = ?2 AND round < (
             SELECT MAX(round) FROM agent_turn_checkpoints
             WHERE topic_id = ?1 AND turn_id = ?2
         )",
        rusqlite::params![topic_id, turn_id],
    );
}

/// Query a topic's latest active checkpoint (crash leftover -> the frontend offers a "resume" entry).
pub fn get_active_checkpoint_impl(conn: &Connection, topic_id: &str) -> Option<ActiveCheckpoint> {
    conn.query_row(
        "SELECT turn_id, topic_id, round, created_at FROM agent_turn_checkpoints
         WHERE topic_id = ?1 AND status = 'active'
         ORDER BY created_at DESC, round DESC LIMIT 1",
        rusqlite::params![topic_id],
        |r| {
            Ok(ActiveCheckpoint {
                turn_id: r.get(0)?,
                topic_id: r.get(1)?,
                round: r.get(2)?,
                created_at: r.get(3)?,
            })
        },
    )
    .ok()
}

/// Clean up checkpoints when a topic is deleted (foreign keys do not cover this table, so deletion must be explicit).
pub fn delete_topic_checkpoints(conn: &Connection, topic_id: &str) {
    let _ = conn.execute(
        "DELETE FROM agent_turn_checkpoints WHERE topic_id = ?1",
        rusqlite::params![topic_id],
    );
}

/// Query a topic's active checkpoint (a crash-leftover turn; the frontend offers a "resume unfinished turn" entry).
///
/// # Parameters
/// - `topic_id`: topic id
///
/// # Returns
/// Option<ActiveCheckpoint>: { turnId, topicId, round, createdAt }; None = no recoverable turn.
#[tauri::command(async)]
pub fn get_active_checkpoint(
    db_state: tauri::State<'_, crate::core::state::DbState>,
    topic_id: String,
) -> Result<Option<ActiveCheckpoint>, String> {
    let conn = db_state.0.lock();
    Ok(get_active_checkpoint_impl(&conn, &topic_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE agent_turn_checkpoints (
                topic_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                round INTEGER NOT NULL,
                messages_json TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                created_at INTEGER NOT NULL,
                PRIMARY KEY (topic_id, turn_id, round)
            );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn save_load_finalize_cycle() {
        let conn = mem_db();
        let msgs = vec![serde_json::json!({"role": "user", "content": "hi"})];
        save_checkpoint(&conn, "t1", "turn1", 1, &msgs);
        save_checkpoint(&conn, "t1", "turn1", 2, &msgs);
        // Retention policy: round 1 (< 2 and not a multiple of 10) is pruned
        let loaded = load_checkpoint_messages(&conn, "t1", "turn1").unwrap();
        assert_eq!(loaded.len(), 1);
        let active = get_active_checkpoint_impl(&conn, "t1").unwrap();
        assert_eq!(active.round, 2);

        finalize_checkpoints(&conn, "t1", "turn1", "completed");
        assert!(load_checkpoint_messages(&conn, "t1", "turn1").is_none());
        assert!(get_active_checkpoint_impl(&conn, "t1").is_none());
    }

    #[test]
    fn every_10th_round_is_kept() {
        let conn = mem_db();
        let msgs = vec![];
        for r in 1..=25u32 {
            save_checkpoint(&conn, "t2", "turn2", r, &msgs);
        }
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_turn_checkpoints WHERE topic_id='t2' AND turn_id='turn2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        // latest (25) + previous backup (24) + the two at rounds 10/20 = 4
        assert_eq!(count, 4, "应保留最新+前一轮备份+每10轮一条");
    }

    #[test]
    fn missing_turn_returns_none() {
        let conn = mem_db();
        assert!(load_checkpoint_messages(&conn, "t", "nope").is_none());
        assert!(get_active_checkpoint_impl(&conn, "t").is_none());
    }
}

//! Agent 回合检查点（P4.4）：每轮结束持久化上下文快照，进程崩溃后可无损续跑。
//!
//! 语义：
//! - 快照 = messages_for_api 全量 JSON（循环消费的精确形态，恢复即用）；
//! - status: active（epilogue 未到达 = 崩溃残留，可恢复）/ completed / cancelled；
//!   用户取消与正常结束都会到达 epilogue 并标记，因此 active 即"可恢复"信号；
//! - 保留策略：同 turn 保留最新 1 条 + 每 10 轮一条（防膨胀）；
//! - 恢复：run_agent_turn 传 resume_turn_id → 加载最新 active 快照替换初始消息。
//!
//! 已知限制（v1）：恢复后的回合事件流从恢复点开始，前端历史不含崩溃前的
//! 中间步骤（模型上下文完整，UI 时间线从恢复点展示）。

use rusqlite::Connection;
use serde::Serialize;

/// 活跃检查点的摘要信息（前端"继续未完成的回合"入口用）。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ActiveCheckpoint {
    pub turn_id: String,
    pub topic_id: String,
    pub round: i64,
    pub created_at: i64,
}

/// 写入一轮快照（同 turn 旧快照按保留策略清理）。
pub(crate) fn save_checkpoint(
    conn: &Connection,
    topic_id: &str,
    turn_id: &str,
    round: u32,
    messages: &[serde_json::Value],
) {
    let messages_json = serde_json::to_string(messages).unwrap_or_default();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let _ = conn.execute(
        "INSERT OR REPLACE INTO agent_turn_checkpoints
            (topic_id, turn_id, round, messages_json, status, created_at)
         VALUES (?1, ?2, ?3, ?4, 'active', ?5)",
        rusqlite::params![topic_id, turn_id, round, messages_json, now],
    );
    // 保留策略：同 turn 只留最新 1 条 + 每 10 轮一条
    let _ = conn.execute(
        "DELETE FROM agent_turn_checkpoints
         WHERE topic_id = ?1 AND turn_id = ?2 AND status = 'active'
           AND round < ?3 AND round % 10 != 0",
        rusqlite::params![topic_id, turn_id, round.saturating_sub(1)],
    );
}

/// 加载指定 turn 的最新 active 快照消息（恢复用）。无快照返回 None。
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

/// 标记 turn 的全部快照为终态（completed / cancelled）。epilogue 调用。
/// 完成态只保留最后一轮（历史轮删除）， cancelled 全保留？——统一只留最后一轮。
pub(crate) fn finalize_checkpoints(conn: &Connection, topic_id: &str, turn_id: &str, status: &str) {
    let _ = conn.execute(
        "UPDATE agent_turn_checkpoints SET status = ?3
         WHERE topic_id = ?1 AND turn_id = ?2",
        rusqlite::params![topic_id, turn_id, status],
    );
    // 终态只留最后一轮（审计/排障可查最新快照）
    let _ = conn.execute(
        "DELETE FROM agent_turn_checkpoints
         WHERE topic_id = ?1 AND turn_id = ?2 AND round < (
             SELECT MAX(round) FROM agent_turn_checkpoints
             WHERE topic_id = ?1 AND turn_id = ?2
         )",
        rusqlite::params![topic_id, turn_id],
    );
}

/// 查询话题最新的 active 检查点（崩溃残留 → 前端提供"继续"入口）。
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

/// 话题删除时清理检查点（外键不含此表，需显式）。
pub fn delete_topic_checkpoints(conn: &Connection, topic_id: &str) {
    let _ = conn.execute(
        "DELETE FROM agent_turn_checkpoints WHERE topic_id = ?1",
        rusqlite::params![topic_id],
    );
}

/// 查询话题的活跃检查点（P4.4：崩溃残留的回合，前端提供"继续未完成的回合"入口）。
///
/// # 参数
/// - `topic_id`：话题 id
///
/// # 返回
/// Option<ActiveCheckpoint>：{ turnId, topicId, round, createdAt }；None = 无可恢复回合。
#[tauri::command]
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
        // 保留策略：round 1（< 2 且非 10 倍数）被清理
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
        // 最新(25) + 前一备份(24) + 10/20 两条 = 4
        assert_eq!(count, 4, "应保留最新+前一轮备份+每10轮一条");
    }

    #[test]
    fn missing_turn_returns_none() {
        let conn = mem_db();
        assert!(load_checkpoint_messages(&conn, "t", "nope").is_none());
        assert!(get_active_checkpoint_impl(&conn, "t").is_none());
    }
}

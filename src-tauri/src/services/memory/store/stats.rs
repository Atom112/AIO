//! 统计域：用量统计、7 日滚动计数、访问计数（从 store.rs 拆出）。
use super::*;

impl MemoryStore {
    /// 记忆库统计。
    pub fn stats(&self) -> Result<MemoryStats, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM facts", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let active: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM facts WHERE status = ?1",
                [STATUS_ACTIVE],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let archived: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM facts WHERE status = ?1",
                [STATUS_ARCHIVED],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let superseded: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM facts WHERE status = ?1",
                [STATUS_SUPERSEDED],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let embedded: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM facts WHERE embedding IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let last_reindex: Option<String> = conn
            .query_row(
                "SELECT v FROM meta WHERE k = ?1",
                [META_LAST_REINDEX],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        drop(conn);
        let db_size = std::fs::metadata(
            Path::new(&self.project_root)
                .join(".aio")
                .join("memory")
                .join("memory.sqlite"),
        )
        .map(|m| m.len())
        .unwrap_or(0);
        // 近 7 天质量探针
        let queries = self.rolling_get(STAT_QUERIES);
        let hits = self.rolling_get(STAT_HITS);
        let dedup = self.rolling_get(STAT_DEDUP);
        let extracted = self.rolling_get(STAT_EXTRACTED);
        let judge_merge = self.rolling_get(STAT_JUDGE_MERGE);
        let judge_update = self.rolling_get(STAT_JUDGE_UPDATE);
        let judge_supersede = self.rolling_get(STAT_JUDGE_SUPERSEDE);
        let judge_keep = self.rolling_get(STAT_JUDGE_KEEP);
        let mut judge_distribution = std::collections::HashMap::new();
        judge_distribution.insert("merge".to_string(), judge_merge);
        judge_distribution.insert("update".to_string(), judge_update);
        judge_distribution.insert("supersede".to_string(), judge_supersede);
        judge_distribution.insert("keep_separate".to_string(), judge_keep);
        Ok(MemoryStats {
            total_facts: total,
            active_facts: active,
            archived_facts: archived,
            superseded_facts: superseded,
            embedded_facts: embedded,
            db_size_bytes: db_size,
            last_reindex_at: last_reindex,
            retrieval_queries_7d: queries,
            retrieval_hits_7d: hits,
            dedup_triggers_7d: dedup,
            extracted_7d: extracted,
            judge_distribution_7d: judge_distribution,
        })
    }

    /// 近 7 天滚动计数 +delta（窗口锚定于首事件，超 7 天自动重置）。用于质量探针，不阻塞主流程。
    pub fn rolling_bump(&self, key: &str, delta: i64) {
        if delta == 0 {
            return;
        }
        let Ok(conn) = self.conn.lock() else { return };
        let now = now_iso();
        let val_key = format!("{key}_val");
        let ts_key = format!("{key}_ts");
        let expired = match conn
            .query_row("SELECT v FROM meta WHERE k = ?1", [&ts_key], |r| {
                r.get::<_, String>(0)
            })
            .optional()
        {
            Ok(Some(ts)) => match (iso_to_secs(&ts), iso_to_secs(&now)) {
                (Some(t0), Some(t1)) => t1.saturating_sub(t0) >= 7 * 86400,
                _ => false,
            },
            _ => true,
        };
        if expired {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO meta (k, v) VALUES (?1, ?2)",
                params![&val_key, "0"],
            );
            let _ = conn.execute(
                "INSERT OR REPLACE INTO meta (k, v) VALUES (?1, ?2)",
                params![&ts_key, &now],
            );
        }
        let _ = conn.execute(
            "INSERT INTO meta (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = CAST(v AS INTEGER) + ?3",
            params![&val_key, delta.to_string(), delta],
        );
    }

    /// 读取近 7 天滚动计数。
    pub fn rolling_get(&self, key: &str) -> i64 {
        let Ok(conn) = self.conn.lock() else { return 0 };
        let sql_key = format!("{key}_val");
        // meta.v 为 TEXT 存储类，须经字符串解析（直接 get::<_, i64> 会因 InvalidColumnType 报错）
        let v: String = conn
            .query_row("SELECT v FROM meta WHERE k = ?1", [&sql_key], |r| r.get(0))
            .unwrap_or_default();
        v.trim().parse::<i64>().unwrap_or(0)
    }

    /// 检索计数累加（供使用统计）。
    pub fn bump_access(&self, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        let now = now_iso();
        if let Ok(conn) = self.conn.lock() {
            for id in ids {
                let _ = conn.execute(
                    "UPDATE facts SET access_count = access_count + 1, last_access_at = ?1 WHERE id = ?2",
                    params![now, id],
                );
            }
        }
    }
}

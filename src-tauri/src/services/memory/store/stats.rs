//! Stats domain: usage statistics, 7-day rolling counters, access counts.
use super::*;

impl MemoryStore {
    /// Memory store statistics.
    /// MEM-14: single conditional-aggregation COUNT (was 5 separate queries) and
    /// one lock hold for the rolling-counter reads (was 8 separate acquisitions).
    pub fn stats(&self) -> Result<MemoryStats, String> {
        let (total, active, archived, superseded, embedded) = {
            let conn = self.conn.lock().map_err(|e| e.to_string())?;
            conn.query_row(
                "SELECT COUNT(*), COALESCE(SUM(status = ?1), 0), COALESCE(SUM(status = ?2), 0), \
                        COALESCE(SUM(status = ?3), 0), COALESCE(SUM(embedding IS NOT NULL), 0) FROM facts",
                params![STATUS_ACTIVE, STATUS_ARCHIVED, STATUS_SUPERSEDED],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                },
            )
            .map_err(|e| e.to_string())?
        };
        let last_reindex: Option<String> = {
            let conn = self.conn.lock().map_err(|e| e.to_string())?;
            conn.query_row(
                "SELECT v FROM meta WHERE k = ?1",
                [META_LAST_REINDEX],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
        };
        let db_size = std::fs::metadata(
            Path::new(&self.project_root)
                .join(".aio")
                .join("memory")
                .join("memory.sqlite"),
        )
        .map(|m| m.len())
        .unwrap_or(0);
        // 7-day quality probes: all rolling counters read under ONE lock hold
        let queries;
        let hits;
        let dedup;
        let extracted;
        let mut judge_distribution = std::collections::HashMap::new();
        {
            let conn = self.conn.lock().map_err(|e| e.to_string())?;
            queries = rolling_get_with(&conn, STAT_QUERIES);
            hits = rolling_get_with(&conn, STAT_HITS);
            dedup = rolling_get_with(&conn, STAT_DEDUP);
            extracted = rolling_get_with(&conn, STAT_EXTRACTED);
            judge_distribution.insert(
                "merge".to_string(),
                rolling_get_with(&conn, STAT_JUDGE_MERGE),
            );
            judge_distribution.insert(
                "update".to_string(),
                rolling_get_with(&conn, STAT_JUDGE_UPDATE),
            );
            judge_distribution.insert(
                "supersede".to_string(),
                rolling_get_with(&conn, STAT_JUDGE_SUPERSEDE),
            );
            judge_distribution.insert(
                "keep_separate".to_string(),
                rolling_get_with(&conn, STAT_JUDGE_KEEP),
            );
        }
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

    /// Adds +delta to the 7-day rolling counter (window anchored to the first event; auto-resets after 7 days). Used for quality probes; never blocks the main flow.
    pub fn rolling_bump(&self, key: &str, delta: i64) {
        self.rolling_bump_many(&[(key, delta)]);
    }

    /// M-8: batched variant for the retrieval hot path — all probe bumps of one
    /// search land in a single transaction instead of one autocommit per bump
    /// (each bump costs 1-3 statements: expiry read + window reset + increment,
    /// so a recall's 5-7 probe writes previously meant 5-7 independent fsyncs).
    /// Best-effort like rolling_bump: failures are logged, never block the flow.
    pub fn rolling_bump_many(&self, bumps: &[(&str, i64)]) {
        let bumps: Vec<(&str, i64)> = bumps.iter().copied().filter(|(_, d)| *d != 0).collect();
        if bumps.is_empty() {
            return;
        }
        let Ok(conn) = self.conn.lock() else { return };
        let Ok(tx) = conn.unchecked_transaction() else {
            return;
        };
        for (key, delta) in bumps {
            let now = now_iso();
            let val_key = format!("{key}_val");
            let ts_key = format!("{key}_ts");
            let expired = match tx
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
                let _ = tx.execute(
                    "INSERT OR REPLACE INTO meta (k, v) VALUES (?1, ?2)",
                    params![&val_key, "0"],
                );
                let _ = tx.execute(
                    "INSERT OR REPLACE INTO meta (k, v) VALUES (?1, ?2)",
                    params![&ts_key, &now],
                );
            }
            let _ = tx.execute(
                "INSERT INTO meta (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = CAST(v AS INTEGER) + ?3",
                params![&val_key, delta.to_string(), delta],
            );
        }
        if let Err(e) = tx.commit() {
            tracing::warn!("[memory] 检索质量探针批量写入失败（非阻塞）: {e}");
        }
    }

    /// Reads the 7-day rolling counter.
    /// MEM-14: now only kept for external single-key callers (UI panel); the
    /// stats() batch path uses [`rolling_get_with`] under one lock.
    #[allow(dead_code)] // retained as the public single-key API
    pub fn rolling_get(&self, key: &str) -> i64 {
        let Ok(conn) = self.conn.lock() else { return 0 };
        rolling_get_with(&conn, key)
    }

    /// Increments retrieval access counts (for usage statistics).
    /// MEM-14: one batched UPDATE with an IN list — the historic per-id UPDATE
    /// loop cost one statement (and one lock hold each round) per hit.
    pub fn bump_access(&self, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        let now = now_iso();
        if let Ok(conn) = self.conn.lock() {
            for chunk in ids.chunks(100) {
                let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                let sql = format!(
                    "UPDATE facts SET access_count = access_count + 1, last_access_at = ?1 WHERE id IN ({placeholders})"
                );
                // Same binding shape as the params! macro: &[&dyn ToSql]
                let mut bind: Vec<&dyn rusqlite::ToSql> = vec![&now];
                for id in chunk {
                    bind.push(id);
                }
                let _ = conn.execute(&sql, bind.as_slice());
            }
        }
    }
}

/// Reads a rolling counter through an already-held connection (MEM-14: lets
/// `stats()` batch all 8 probe reads under one lock).
/// meta.v uses the TEXT storage class; must parse via string (direct get::<_, i64>
/// fails with InvalidColumnType).
fn rolling_get_with(conn: &rusqlite::Connection, key: &str) -> i64 {
    let sql_key = format!("{key}_val");
    let v: String = conn
        .query_row("SELECT v FROM meta WHERE k = ?1", [&sql_key], |r| r.get(0))
        .unwrap_or_default();
    v.trim().parse::<i64>().unwrap_or(0)
}

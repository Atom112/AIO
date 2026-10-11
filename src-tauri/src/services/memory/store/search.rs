//! Retrieval domain: vector/FTS5/hybrid search, scoring, and prompt-block assembly (split from store.rs).
use super::*;
use crate::services::memory::CollectRowsWarn;

impl MemoryStore {
    /// Vector search: vec0 first, brute-force cosine fallback on failure (missing table, dimension change, etc.).
    fn vector_search(
        &self,
        conn: &Connection,
        query: &[f32],
        k: usize,
    ) -> Result<Vec<(String, f32)>, String> {
        if crate::services::memory::vec0::vec0_available() {
            if let Ok(rows) = crate::services::memory::vec0::vec0_search(conn, query, k) {
                if !rows.is_empty() || k == 0 {
                    return Ok(rows);
                }
            }
        }
        let mut stmt = conn
            .prepare("SELECT id, embedding FROM facts WHERE status = ?1 AND embedding IS NOT NULL")
            .map_err(|e| e.to_string())?;
        let rows: Vec<(String, Vec<u8>)> = stmt
            .query_map(params![STATUS_ACTIVE], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
            })
            .map_err(|e| e.to_string())?
            .collect_rows("search");
        drop(stmt);
        let decoded: Vec<(String, Vec<f32>)> = rows
            .iter()
            .map(|(id, b)| (id.clone(), bytes_to_f32(b)))
            .collect();
        Ok(brute_force_topk(&decoded, query, k))
    }

    /// Pure vector cosine search (for dedup/arbitration): returns (id, cosine_similarity) sorted by descending similarity.
    /// Only active facts are searched; a dimension mismatch returns an empty list (the caller falls back to keywords).
    pub fn search_similar(
        &self,
        query_vec: &[f32],
        k: usize,
    ) -> Result<Vec<(String, f32)>, String> {
        if query_vec.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.vector_search(&conn, query_vec, k)
    }

    /// Hybrid search: vector top-k (when query_vec is given) + FTS5 BM25 -> RRF fusion + recency boost.
    pub fn search_hybrid(
        &self,
        query_text: &str,
        query_vec: Option<&[f32]>,
        opts: &SearchOptions,
    ) -> Result<Vec<ScoredFact>, String> {
        let k = opts.k.clamp(1, 50);
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        // 1) Vector recall (vec0-accelerated, brute-force cosine when unavailable)
        let mut vector_ranks: Vec<(String, usize)> = Vec::new();
        if let Some(qv) = query_vec {
            let top = self.vector_search(&conn, qv, k * 2)?;
            for (i, (id, _)) in top.iter().enumerate() {
                vector_ranks.push((id.clone(), i));
            }
        }

        // 2) FTS5 recall (trigram, supports Chinese substrings; active facts only) ->
        //    short queries (<3 chars, which trigram cannot match) or failed/empty expressions fall back to LIKE
        let mut fts_ranks: Vec<(String, usize)> = Vec::new();
        let sanitized = sanitize_fts_query(query_text);
        let mut like_query = false;
        if sanitized.chars().count() < 3 {
            // trigram needs >=3 chars, so 1-2 char Chinese queries go straight to LIKE
            like_query = true;
        } else if !sanitized.trim().is_empty() {
            let sql = "SELECT f.fact_id FROM fts_facts f JOIN facts ON facts.rowid = f.rowid WHERE f.fts_facts MATCH ?1 AND facts.status = ?2 ORDER BY bm25(f.fts_facts) LIMIT ?3";
            let matched: Result<Vec<String>, String> = (|| {
                let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
                let rows = stmt
                    .query_map(params![sanitized, STATUS_ACTIVE, (k * 2) as i64], |r| {
                        r.get::<_, String>(0)
                    })
                    .map_err(|e| e.to_string())?;
                rows.collect_rows("search").pipe(Ok)
            })();
            match matched {
                Ok(ids) if !ids.is_empty() => {
                    for (i, id) in ids.iter().enumerate() {
                        fts_ranks.push((id.clone(), i));
                    }
                }
                _ => like_query = true,
            }
        } else {
            like_query = true;
        }
        if like_query {
            // LIKE fuzzy match (status-filtered, so substring hits do not recall non-active facts)
            let pattern = format!("%{}%", query_text.trim());
            let mut stmt = conn
                .prepare("SELECT id FROM facts WHERE status = ?1 AND (content LIKE ?2 OR category LIKE ?2) LIMIT ?3")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![STATUS_ACTIVE, pattern, (k * 2) as i64], |r| {
                    r.get::<_, String>(0)
                })
                .map_err(|e| e.to_string())?;
            for (i, id) in rows.collect_rows("search").into_iter().enumerate() {
                fts_ranks.push((id, i));
            }
        }

        // 3) RRF fusion
        let mut rrf: HashMap<String, f32> = HashMap::new();
        for (id, rank) in vector_ranks.iter().chain(fts_ranks.iter()) {
            let entry = rrf.entry(id.clone()).or_insert(0.0);
            *entry += 1.0 / (60.0 + *rank as f32 + 1.0);
        }
        if rrf.is_empty() {
            return Ok(Vec::new());
        }

        // 4) Fetch details + recency boost + sort (MEM-11: one batched IN (...) query
        //    per 100 candidates instead of a SELECT per id — the fused candidate set
        //    is up to 4k entries, so the per-id loop cost one round-trip each)
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut candidates: Vec<(String, f32)> = rrf.into_iter().collect();
        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let scored = self.fetch_scored_batch(&candidates, now_secs, &conn)?;
        let mut scored: Vec<ScoredFact> = scored
            .into_iter()
            .filter(|fact| {
                // Category filter (vector/FTS/LIKE paths all converge here; off-category facts never enter the result)
                opts.category
                    .as_ref()
                    .is_none_or(|cat| &fact.category == cat)
                    && opts.min_score.is_none_or(|min| fact.score >= min)
            })
            .collect();
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(k);
        Ok(scored)
    }

    // ===== Sparse-leg (FTS-only) search: for evaluation, same filtering criteria as hybrid search =====

    /// Keyword-only search (FTS5/LIKE + status/category/recency); the vector path is not used.
    /// Used by RAG evaluation (eval_rag) to compare the vector-only / FTS / hybrid paths.
    #[cfg(test)]
    pub fn search_fts_only(
        &self,
        query_text: &str,
        opts: &SearchOptions,
    ) -> Result<Vec<ScoredFact>, String> {
        let k = opts.k.clamp(1, 50);
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut ids: Vec<String> = Vec::new();
        let sanitized = sanitize_fts_query(query_text);
        if sanitized.chars().count() >= 3 && !sanitized.trim().is_empty() {
            let sql = "SELECT f.fact_id FROM fts_facts f JOIN facts ON facts.rowid = f.rowid WHERE f.fts_facts MATCH ?1 AND facts.status = ?2 ORDER BY bm25(f.fts_facts) LIMIT ?3";
            let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![sanitized, STATUS_ACTIVE, (k * 3) as i64], |r| {
                    r.get::<_, String>(0)
                })
                .map_err(|e| e.to_string())?;
            for id in rows.collect_rows("search") {
                ids.push(id);
            }
        }
        if ids.is_empty() {
            // Fall back to LIKE for short queries or when trigram is unavailable (status-filtered)
            let pattern = format!("%{}%", query_text.trim());
            let mut stmt = conn
                .prepare("SELECT id FROM facts WHERE status = ?1 AND (content LIKE ?2 OR category LIKE ?2) LIMIT ?3")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![STATUS_ACTIVE, pattern, (k * 3) as i64], |r| {
                    r.get::<_, String>(0)
                })
                .map_err(|e| e.to_string())?;
            for id in rows.collect_rows("search") {
                ids.push(id);
            }
        }
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // MEM-11: batched detail assembly (same as the hybrid path)
        let candidates: Vec<(String, f32)> = ids
            .into_iter()
            .enumerate()
            .map(|(i, id)| (id, 1.0 / (60.0 + i as f32 + 1.0)))
            .collect();
        let scored = self.fetch_scored_batch(&candidates, now_secs, &conn)?;
        let mut scored: Vec<ScoredFact> = scored
            .into_iter()
            .filter(|fact| {
                opts.category
                    .as_ref()
                    .is_none_or(|cat| &fact.category == cat)
                    && opts.min_score.is_none_or(|min| fact.score >= min)
            })
            .collect();
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(k);
        Ok(scored)
    }

    /// Batched detail assembly (MEM-11): one `IN (...)` query per 100 candidates
    /// replaces the per-id SELECT loop. Only rows with status = 'active' are
    /// returned; recent facts (<= 90 days) get the same 1.1 recency boost as before.
    fn fetch_scored_batch(
        &self,
        candidates: &[(String, f32)],
        now_secs: u64,
        conn: &Connection,
    ) -> Result<Vec<ScoredFact>, String> {
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::with_capacity(candidates.len());
        for chunk in candidates.chunks(100) {
            let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT id, content, category, source_type, updated_at FROM facts \
                 WHERE status = 'active' AND id IN ({placeholders})"
            );
            let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
            let ids: Vec<&str> = chunk.iter().map(|(id, _)| id.as_str()).collect();
            let rows = stmt
                .query_map(rusqlite::params_from_iter(ids), |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            for (id, content, category, source_type, updated_at) in
                rows.collect_rows("search_batch")
            {
                // Rows come back in SQLite order; re-associate the fused score by id.
                let Some((_, score)) = chunk.iter().find(|(cid, _)| *cid == id) else {
                    continue;
                };
                let mut final_score = *score;
                if let Some(upd) = &updated_at {
                    if let Some(secs) = iso_to_secs(upd) {
                        let age_days = now_secs.saturating_sub(secs) / 86400;
                        if age_days <= 90 {
                            final_score *= 1.1;
                        }
                    }
                }
                out.push(ScoredFact {
                    id,
                    content,
                    category,
                    score: final_score,
                    source_type,
                    updated_at,
                });
            }
        }
        Ok(out)
    }

    /// Read all active facts that have vectors (for consolidation/clustering).
    pub fn list_active_facts_with_vectors(&self) -> Result<Vec<FactWithVector>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, content, importance, access_count, embedding FROM facts WHERE status = ?1 AND embedding IS NOT NULL",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![STATUS_ACTIVE], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, f32>(2)?,
                    r.get::<_, i64>(3)?,
                    crate::services::memory::vector::bytes_to_f32(&r.get::<_, Vec<u8>>(4)?),
                ))
            })
            .map_err(|e| e.to_string())?;
        Ok(rows.collect_rows("search"))
    }

    // ===== Lightweight knowledge graph (facts_relations) =====

    /// Build the injection block from search results (trimmed to the token budget; stable output for identical input, which favors prompt caching).
    pub fn build_block_from(
        &self,
        results: Vec<ScoredFact>,
        budget_tokens: usize,
    ) -> Result<Option<String>, String> {
        if budget_tokens == 0 || results.is_empty() {
            return Ok(None);
        }
        let mut block = String::from("项目记忆（跨会话沉淀，仅作快速参考，以实际代码为准）：\n");
        let mut used = count_approx(&block);
        let mut count = 0usize;
        for f in results {
            let line = format!(
                "- [{}] {}（更新时间 {}）\n",
                f.category,
                f.content,
                f.updated_at.clone().unwrap_or_default(),
            );
            let t = count_approx(&line);
            if used + t > budget_tokens && count > 0 {
                break;
            }
            block.push_str(&line);
            used += t;
            count += 1;
        }
        if count == 0 {
            return Ok(None);
        }
        Ok(Some(block))
    }
}

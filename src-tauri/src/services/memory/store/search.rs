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

        // 4) Fetch details + recency boost + sort
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut scored: Vec<ScoredFact> = Vec::new();
        for (id, score) in rrf.iter() {
            if let Ok(fact) = self.fetch_scored(id, *score, now_secs, &conn) {
                // Category filter (vector/FTS/LIKE paths all converge here; off-category facts never enter the result)
                if let Some(cat) = &opts.category {
                    if &fact.category != cat {
                        continue;
                    }
                }
                if let Some(min) = opts.min_score {
                    if fact.score < min {
                        continue;
                    }
                }
                scored.push(fact);
            }
        }
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
        let mut scored: Vec<ScoredFact> = Vec::new();
        for (i, id) in ids.iter().enumerate() {
            if let Ok(fact) = self.fetch_scored(id, 1.0 / (60.0 + i as f32 + 1.0), now_secs, &conn)
            {
                if let Some(cat) = &opts.category {
                    if &fact.category != cat {
                        continue;
                    }
                }
                if let Some(min) = opts.min_score {
                    if fact.score < min {
                        continue;
                    }
                }
                scored.push(fact);
            }
        }
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(k);
        Ok(scored)
    }

    fn fetch_scored(
        &self,
        id: &str,
        score: f32,
        now_secs: u64,
        conn: &Connection,
    ) -> Result<ScoredFact, String> {
        let row: (String, String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT content, category, source_type, updated_at FROM facts WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .map_err(|e| e.to_string())?;
        let mut final_score = score;
        if let Some(upd) = &row.3 {
            if let Some(secs) = iso_to_secs(upd) {
                let age_days = now_secs.saturating_sub(secs) / 86400;
                if age_days <= 90 {
                    final_score *= 1.1;
                }
            }
        }
        Ok(ScoredFact {
            id: id.to_string(),
            content: row.0,
            category: row.1,
            score: final_score,
            source_type: row.2,
            updated_at: row.3,
        })
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

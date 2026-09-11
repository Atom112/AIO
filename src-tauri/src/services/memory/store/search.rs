//! 检索域：向量/FTS5/混合检索、打分与提示块装配（从 store.rs 拆出）。
use super::*;
use crate::services::memory::CollectRowsWarn;

impl MemoryStore {
    /// 向量检索：vec0 优先，失败回退暴力余弦（表缺失/维度变化等）。
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

    /// 纯向量余弦检索（判重/仲裁用）：返回 (id, cosine_similarity) 按相似度降序。
    /// 只检索 active 事实；维度不匹配时返回空表（由调用方降级为关键词）。
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

    /// 混合检索：向量 top-k（query_vec 提供时）+ FTS5 BM25 → RRF 融合 + 时新度加成。
    pub fn search_hybrid(
        &self,
        query_text: &str,
        query_vec: Option<&[f32]>,
        opts: &SearchOptions,
    ) -> Result<Vec<ScoredFact>, String> {
        let k = opts.k.clamp(1, 50);
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        // 1) 向量召回（vec0 加速，不可用时暴力余弦）
        let mut vector_ranks: Vec<(String, usize)> = Vec::new();
        if let Some(qv) = query_vec {
            let top = self.vector_search(&conn, qv, k * 2)?;
            for (i, (id, _)) in top.iter().enumerate() {
                vector_ranks.push((id.clone(), i));
            }
        }

        // 2) FTS5 召回（trigram，支持中文子串；仅 active 事实）→
        //    短查询（<3 字符，trigram 无法命中）或表达式失败/空结果时降级 LIKE
        let mut fts_ranks: Vec<(String, usize)> = Vec::new();
        let sanitized = sanitize_fts_query(query_text);
        let mut like_query = false;
        if sanitized.chars().count() < 3 {
            // trigram 需要 ≥3 字符，中文 1-2 字查询（重构/测试等）直接走 LIKE
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
            // LIKE 模糊匹配（status 过滤，避免同名子串把非 active 事实召回）
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

        // 3) RRF 融合
        let mut rrf: HashMap<String, f32> = HashMap::new();
        for (id, rank) in vector_ranks.iter().chain(fts_ranks.iter()) {
            let entry = rrf.entry(id.clone()).or_insert(0.0);
            *entry += 1.0 / (60.0 + *rank as f32 + 1.0);
        }
        if rrf.is_empty() {
            return Ok(Vec::new());
        }

        // 4) 取详情 + 时新度加成 + 排序
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut scored: Vec<ScoredFact> = Vec::new();
        for (id, score) in rrf.iter() {
            if let Ok(fact) = self.fetch_scored(id, *score, now_secs, &conn) {
                // 类别过滤（向量/FTS/LIKE 三路统一在此收敛；off-category 不再进入结果）
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

    // ===== P3-2/P3-5 稀疏腿（FTS-only）检索：评估用，与混合检索同过滤口径 =====

    /// 仅关键词检索（FTS5/LIKE + status/category/时新度），不参与向量路。
    /// 供 RAG 评测（eval_rag）对比"纯向量 / FTS / 混合"三路使用。
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
            // 短查询或不支持 trigram 时回退 LIKE（status 过滤）
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

    /// 读取全部带向量的 active 事实（合并整理/聚类用）。
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

    // ===== P3-3 轻量知识图谱（facts_relations）=====

    /// 由检索结果生成注入块（按 token 预算裁剪；同样素材下输出稳定，利于 prompt cache）。
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

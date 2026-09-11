//! 检索后处理：二阶段精排（rerank）、查询改写（可选）与多查询 RRF 融合。
//!
//! 供 Agent 循环自动注入与检索工具（recall / search_memory / memory_search）共用，
//! 避免各调用方重复实现同一套"检索 → 精排 → 裁剪"流程。
//! 全部失败/未启用时 fail-open：回退 RRF-only 行为，与引入重排前一致。

use super::store::{MemoryStore, ScoredFact, SearchOptions};
use crate::core::models::MemoryRerankConfig;
use std::collections::HashMap;
use tauri::AppHandle;

/// 项目级重排配置的一次性迁移（**显式命名的写操作**，便于审计）：
/// 无项目 meta 时把旧全局配置快照到项目（幂等）。
/// 调用点：记忆命令入口（memory_get_status / memory_update_rerank）。
pub fn migrate_project_rerank(app: &AppHandle, store: &MemoryStore) {
    let (_, set) = store.rerank_cfg();
    if !set {
        let legacy = crate::core::config::load_app_config(app)
            .unwrap_or_default()
            .memory_rerank;
        let _ = store.set_rerank_cfg(&legacy);
    }
}

/// 解析项目级重排配置（**纯读取**，不写盘：偏好项目 meta，无则回退全局旧配置）。
/// 落盘迁移见 [`migrate_project_rerank`]（在命令入口显式执行）。
pub fn resolve_project_rerank(app: &AppHandle, store: &MemoryStore) -> MemoryRerankConfig {
    let (cfg, set) = store.rerank_cfg();
    if set {
        cfg
    } else {
        crate::core::config::load_app_config(app)
            .unwrap_or_default()
            .memory_rerank
    }
}

/// 注入 API Key（在线 provider）。
fn inject_key(app: &AppHandle, cfg: &mut MemoryRerankConfig) {
    if cfg.provider == "online" && cfg.api_key.is_empty() {
        if let Ok(Some(key)) = crate::core::secure_store::get(app, "rerank_api_key") {
            cfg.api_key = key;
        }
    }
}

/// 对检索结果应用项目级重排；未启用/解析失败/调用失败时原顺序返回（fail-safe）。
pub async fn apply_rerank(
    app: &AppHandle,
    store: &MemoryStore,
    query: &str,
    hits: Vec<ScoredFact>,
) -> Vec<ScoredFact> {
    if hits.len() <= 1 {
        return hits;
    }
    let mut cfg = resolve_project_rerank(app, store);
    inject_key(app, &mut cfg);
    let Ok(reranker) = crate::plugins::rerank::resolve(app, &cfg) else {
        return hits;
    };
    let docs: Vec<(String, String)> = hits
        .iter()
        .map(|h| (h.id.clone(), h.content.clone()))
        .collect();
    match reranker.rerank(query, &docs).await {
        Ok(ranked) => {
            let score_map: HashMap<String, f32> = ranked.into_iter().collect();
            let mut out = hits;
            for h in out.iter_mut() {
                if let Some(s) = score_map.get(&h.id) {
                    h.score = *s;
                }
            }
            out.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            out
        }
        Err(_) => hits,
    }
}

/// 查询向量（项目级配置 + 维度守卫：维度与库不一致时返回 None，退化为关键词检索）。
pub async fn embed_query(app: &AppHandle, store: &MemoryStore, text: &str) -> Option<Vec<f32>> {
    let ecfg = super::resolve_project_embedding(app, store);
    let embedder = crate::plugins::embed::resolve(app, &ecfg).ok()?;
    let v = embedder
        .embed(&[text.to_string()])
        .await
        .ok()?
        .into_iter()
        .next()?;
    if let Some(lib_dim) = store.embedding_dim_meta() {
        if lib_dim != v.len() {
            tracing::warn!(
                "查询嵌入维度 {} 与记忆库维度 {} 不一致，本次仅关键词检索",
                v.len(),
                lib_dim
            );
            return None;
        }
    }
    Some(v)
}

/// 生成注入块（含重排）：混合检索 top-20 → 项目级重排 → token 预算裁剪。
/// 任何环节失败返回 None（本轮不注入，不打断任务）。
pub async fn inject_block(
    app: &AppHandle,
    store: &MemoryStore,
    query: &str,
    query_vec: Option<Vec<f32>>,
    budget: usize,
) -> Option<String> {
    let results = store
        .search_hybrid(
            query,
            query_vec.as_deref(),
            &SearchOptions {
                category: None,
                k: 20,
                min_score: None,
            },
        )
        .ok()?;
    let results = apply_rerank(app, store, query, results).await;
    let results = apply_graph_boost(store, query, results).await;
    store.build_block_from(results, budget).ok().flatten()
}

/// 查询改写（可选，默认关）：LLM 生成 1-2 个查询变体；LLM 不可用返回空。
pub async fn rewrite_query(app: &AppHandle, query: &str) -> Vec<String> {
    match super::llm::llm_config(app) {
        Ok(cfg) => super::llm::rewrite_query(&cfg, query)
            .await
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// 多查询检索 + RRF 融合：每个查询独立做混合检索（含项目级重排），
/// 结果按 reciprocal rank 融合排序后取 top k。单查询时等价于普通检索。
pub async fn merged_search(
    app: &AppHandle,
    store: &MemoryStore,
    queries: &[String],
    opts: &SearchOptions,
) -> Result<Vec<ScoredFact>, String> {
    if queries.is_empty() {
        return Ok(Vec::new());
    }
    // 各查询独立检索（查询向量按项目配置生成）
    let mut lists: Vec<Vec<ScoredFact>> = Vec::with_capacity(queries.len());
    for q in queries {
        let qv = embed_query(app, store, q).await;
        let hits = store.search_hybrid(q, qv.as_deref(), opts)?;
        let hits = apply_rerank(app, store, q, hits).await;
        let hits = apply_graph_boost(store, q, hits).await;
        lists.push(hits);
    }
    // RRF 融合
    let mut rrf: HashMap<String, f32> = HashMap::new();
    let mut first_seen: HashMap<String, ScoredFact> = HashMap::new();
    for list in lists {
        for (i, hit) in list.iter().enumerate() {
            let entry = rrf.entry(hit.id.clone()).or_insert(0.0);
            *entry += 1.0 / (60.0 + i as f32 + 1.0);
            first_seen
                .entry(hit.id.clone())
                .or_insert_with(|| hit.clone());
        }
    }
    let mut out: Vec<ScoredFact> = first_seen
        .into_values()
        .map(|mut f| {
            f.score = rrf.get(&f.id).copied().unwrap_or(0.0);
            f
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out.truncate(opts.k.clamp(1, 50));
    Ok(out)
}

/// P3-1 Agentic/迭代检索（默认关）：首轮取 top-3，以其内容摘要作为扩展查询再检索一轮，
/// 两轮结果 RRF 融合。对表述模糊/术语缺失的查询可提升召回；成本 ≈ 两次查询嵌入。
pub async fn iterative_search(
    app: &AppHandle,
    store: &MemoryStore,
    query: &str,
    opts: &SearchOptions,
) -> Result<Vec<ScoredFact>, String> {
    let first = merged_search(app, store, &[query.to_string()], opts).await?;
    if first.is_empty() {
        return Ok(first);
    }
    let snippets: Vec<String> = first
        .iter()
        .take(3)
        .map(|f| {
            let mut s: String = f.content.chars().take(240).collect();
            if s.chars().count() >= 240 {
                s.push('…');
            }
            s
        })
        .collect();
    let ext = format!("{}\n相关要点：{}", query, snippets.join("\n"));
    let second = merged_search(app, store, &[ext], opts).await?;
    // 两轮 RRF 融合（迭代轮权重与首轮相同）
    let mut rrf: HashMap<String, f32> = HashMap::new();
    let mut first_seen: HashMap<String, ScoredFact> = HashMap::new();
    for list in [&first, &second] {
        for (i, hit) in list.iter().enumerate() {
            *rrf.entry(hit.id.clone()).or_insert(0.0) += 1.0 / (60.0 + i as f32 + 1.0);
            first_seen
                .entry(hit.id.clone())
                .or_insert_with(|| hit.clone());
        }
    }
    let mut out: Vec<ScoredFact> = first_seen
        .into_values()
        .map(|mut f| {
            f.score = rrf.get(&f.id).copied().unwrap_or(0.0);
            f
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out.truncate(opts.k.clamp(1, 50));
    Ok(out)
}

/// P3-3 图谱加分纯函数：实体命中数 → 分数加成（每命中 +15%，封顶 +30%）。
/// 匹配规则与 store::relation_entity_hits 一致：查询包含实体或实体包含查询（长度 >= 2）。
/// 运行时实现见 [apply_graph_boost]（按命中计数内联同式）；本函数供单元测试回归公式。
#[cfg(test)]
pub fn graph_boost(fact_entities: &[String], query: &str, base: f32) -> f32 {
    let q = query.trim();
    if q.is_empty() || fact_entities.is_empty() {
        return base;
    }
    let hits = fact_entities
        .iter()
        .filter(|e| e.len() >= 2 && (q.contains(e.as_str()) || e.contains(q)))
        .count();
    base * (1.0 + 0.15 * hits.min(2) as f32)
}

/// 对检索结果应用图谱实体加分（实体来自 facts_relations；无命中保持原序）。
pub async fn apply_graph_boost(
    store: &MemoryStore,
    query: &str,
    hits: Vec<ScoredFact>,
) -> Vec<ScoredFact> {
    if hits.is_empty() {
        return hits;
    }
    let Ok(entity_hits) = store.relation_entity_hits(query) else {
        return hits;
    };
    if entity_hits.is_empty() {
        return hits;
    }
    let mut out = hits;
    for h in out.iter_mut() {
        if let Some(n) = entity_hits.get(&h.id) {
            h.score *= 1.0 + 0.15 * (*n).min(2) as f32;
        }
    }
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_graph_boost() {
        // 无实体/空查询 → 原分
        assert_eq!(graph_boost(&[], "查询", 1.0), 1.0);
        assert_eq!(graph_boost(&["实体".into()], "  ", 0.5), 0.5);
        // 查询包含实体 → +15%
        let boosted = graph_boost(&["rusqlite".into()], "数据库用 rusqlite", 1.0);
        assert!((boosted - 1.15).abs() < 1e-5, "命中实体应 +15%：{boosted}");
        // 实体包含查询（短查询）→ +15%
        let boosted2 = graph_boost(&["sqlite-vec".into()], "sqlite", 1.0);
        assert!(
            (boosted2 - 1.15).abs() < 1e-5,
            "实体包含查询应 +15%：{boosted2}"
        );
        // 封顶 +30%（两个及以上命中）
        let capped = graph_boost(&["存储".into(), "SQLite".into()], "存储 SQLite 方案", 1.0);
        assert!((capped - 1.3).abs() < 1e-5, "命中封顶应 ×1.3：{capped}");
    }

    #[test]
    fn test_rrf_merge() {
        // 仅验证融合函数逻辑（不触达 SQLite/网络）
        let a = vec![
            ScoredFact {
                id: "a".into(),
                content: "A".into(),
                category: "note".into(),
                score: 0.5,
                source_type: None,
                updated_at: None,
            },
            ScoredFact {
                id: "b".into(),
                content: "B".into(),
                category: "note".into(),
                score: 0.4,
                source_type: None,
                updated_at: None,
            },
        ];
        let b = vec![ScoredFact {
            id: "b".into(),
            content: "B".into(),
            category: "note".into(),
            score: 0.9,
            source_type: None,
            updated_at: None,
        }];
        let opts = SearchOptions {
            category: None,
            k: 5,
            min_score: None,
        };
        // 双查询均在列表：a 只出现一次（rank0 → 1/61），b 出现两次（1/61 + 1/62）
        let lists = vec![a.clone(), b];
        let mut rrf: HashMap<String, f32> = HashMap::new();
        let mut first: HashMap<String, ScoredFact> = HashMap::new();
        for list in lists {
            for (i, hit) in list.iter().enumerate() {
                *rrf.entry(hit.id.clone()).or_insert(0.0) += 1.0 / (60.0 + i as f32 + 1.0);
                first.entry(hit.id.clone()).or_insert_with(|| hit.clone());
            }
        }
        let mut out: Vec<ScoredFact> = first
            .into_values()
            .map(|mut f| {
                f.score = rrf.get(&f.id).copied().unwrap_or(0.0);
                f
            })
            .collect();
        out.sort_by(|x, y| y.score.partial_cmp(&x.score).unwrap());
        out.truncate(opts.k.clamp(1, 50));
        assert_eq!(out[0].id, "b", "双路命中的 b 融合分应高于单路 a");
        assert!(out[0].score > out[1].score);
    }
}

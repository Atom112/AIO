//! Retrieval post-processing: two-stage rerank, optional query rewrite, and multi-query RRF fusion.
//!
//! Shared by the agent loop's auto-injection and the retrieval tools (recall / search_memory / memory_search),
//! so no caller reimplements the same "retrieve -> rerank -> trim" pipeline.
//! Fails open when everything fails or features are disabled: falls back to RRF-only behavior, as before rerank existed.

use super::store::{MemoryStore, ScoredFact, SearchOptions};
use crate::core::models::MemoryRerankConfig;
use std::collections::HashMap;
use tauri::AppHandle;

/// One-time migration of the project-level rerank config (**an explicitly named write**, easy to audit):
/// snapshots the legacy global config into the project when no project meta exists (idempotent).
/// Call sites: memory command entry points (memory_get_status / memory_update_rerank).
pub fn migrate_project_rerank(app: &AppHandle, store: &MemoryStore) {
    let (_, set) = store.rerank_cfg();
    if !set {
        let legacy = crate::core::config::load_app_config(app)
            .unwrap_or_default()
            .memory_rerank;
        let _ = store.set_rerank_cfg(&legacy);
    }
}

/// Resolve the project-level rerank config (**pure read**, no disk writes: prefers project meta, falls back to the legacy global config).
/// The on-disk migration is [`migrate_project_rerank`] (run explicitly at command entry points).
/// Normalization: provider is always local; a model not in the manifest's rerank entries (e.g. the
/// legacy online model name rerank-multilingual-v3) falls back to the default bge-reranker-base.
pub fn resolve_project_rerank(app: &AppHandle, store: &MemoryStore) -> MemoryRerankConfig {
    let (mut cfg, set) = store.rerank_cfg();
    if !set {
        cfg = crate::core::config::load_app_config(app)
            .unwrap_or_default()
            .memory_rerank;
    }
    cfg.provider = "local".into();
    let model_valid = crate::core::model_registry::find_entry(cfg.model.trim())
        .map(|e| e.kind == "rerank")
        .unwrap_or(false);
    if !model_valid {
        cfg.model = crate::plugins::rerank::local_onnx::LOCAL_MODEL_ID.into();
    }
    cfg
}

/// Apply the project-level rerank to retrieval results; when disabled/resolve fails/call fails, returns the original order (fail-safe).
pub async fn apply_rerank(
    app: &AppHandle,
    store: &MemoryStore,
    query: &str,
    hits: Vec<ScoredFact>,
) -> Vec<ScoredFact> {
    if hits.len() <= 1 {
        return hits;
    }
    let cfg = resolve_project_rerank(app, store);
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

/// Query vector (project-level config + dimension guard: returns None when the dimension mismatches the store, degrading to keyword retrieval).
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

/// Build the injection block (with rerank): hybrid search top-20 -> project-level rerank -> token-budget trim.
/// Returns None if any stage fails (skip injection this round; never interrupts the task).
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

/// Query rewrite (optional, off by default): the LLM generates 1-2 query variants; returns empty when the LLM is unavailable.
pub async fn rewrite_query(app: &AppHandle, query: &str) -> Vec<String> {
    match super::llm::llm_config(app) {
        Ok(cfg) => super::llm::rewrite_query(&cfg, query)
            .await
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Multi-query retrieval + RRF fusion: each query runs its own hybrid search (with project-level rerank),
/// results are fused and ranked by reciprocal rank, then truncated to top k. A single query degrades to plain retrieval.
pub async fn merged_search(
    app: &AppHandle,
    store: &MemoryStore,
    queries: &[String],
    opts: &SearchOptions,
) -> Result<Vec<ScoredFact>, String> {
    if queries.is_empty() {
        return Ok(Vec::new());
    }
    // Independent search per query (query vectors generated per project config)
    let mut lists: Vec<Vec<ScoredFact>> = Vec::with_capacity(queries.len());
    for q in queries {
        let qv = embed_query(app, store, q).await;
        let hits = store.search_hybrid(q, qv.as_deref(), opts)?;
        let hits = apply_rerank(app, store, q, hits).await;
        let hits = apply_graph_boost(store, q, hits).await;
        lists.push(hits);
    }
    // RRF fusion
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

/// Agentic/iterative retrieval (off by default): take top-3, use their content summaries as an expanded
/// query for a second round, then RRF-fuse both rounds. Helps vague/terminology-poor queries; cost ~ two query embeddings.
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
    // RRF fusion of both rounds (the iterative round weighs the same as the first)
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

/// Pure graph-boost function: entity hit count -> score bonus (+15% per hit, capped at +30%).
/// Matching rule matches store::relation_entity_hits: the query contains the entity or vice versa (length >= 2).
/// Runtime implementation is [apply_graph_boost] (same formula inlined by hit count); this function exists for unit-test regression of the formula.
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

/// Apply the graph entity boost to retrieval results (entities from facts_relations; original order kept on no hits).
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
        // No entities / empty query -> unchanged score
        assert_eq!(graph_boost(&[], "查询", 1.0), 1.0);
        assert_eq!(graph_boost(&["实体".into()], "  ", 0.5), 0.5);
        // Query contains entity -> +15%
        let boosted = graph_boost(&["rusqlite".into()], "数据库用 rusqlite", 1.0);
        assert!((boosted - 1.15).abs() < 1e-5, "命中实体应 +15%：{boosted}");
        // Entity contains query (short query) -> +15%
        let boosted2 = graph_boost(&["sqlite-vec".into()], "sqlite", 1.0);
        assert!(
            (boosted2 - 1.15).abs() < 1e-5,
            "实体包含查询应 +15%：{boosted2}"
        );
        // Cap +30% (two or more hits)
        let capped = graph_boost(&["存储".into(), "SQLite".into()], "存储 SQLite 方案", 1.0);
        assert!((capped - 1.3).abs() < 1e-5, "命中封顶应 ×1.3：{capped}");
    }

    #[test]
    fn test_rrf_merge() {
        // Only verifies the fusion logic (no SQLite/network access)
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
        // Both queries present: a appears once (rank0 -> 1/61), b appears twice (1/61 + 1/62)
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

//! Lightweight knowledge graph (Graphiti-lite): the LLM extracts entity-relation triples from
//! fact text and writes them to `facts_relations` (delete-then-insert, idempotent). The retrieval
//! side boosts results on entity hits in `retrieve::apply_graph_boost` (light string matching, no multi-hop queries).

use crate::services::memory::llm::{chat_json, llm_config};
use crate::services::memory::store::MemoryStore;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

/// Graph extraction report (returned by the memory_graph_extract command).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GraphExtractReport {
    /// Number of facts processed (extraction attempts) this run
    pub facts_processed: usize,
    /// Number of relation triples inserted or overwritten
    pub relations: usize,
    /// Number of facts whose LLM extraction failed
    pub failed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Deserialize)]
struct RelationItem {
    #[serde(default)]
    entity1: String,
    #[serde(default)]
    relation: String,
    #[serde(default)]
    entity2: String,
}

/// Extract relation triples from a single fact (returns empty on LLM failure).
async fn extract_from_fact(app: &AppHandle, content: &str) -> Vec<(String, String, String)> {
    let cfg = match llm_config(app) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let system = concat!(
        "你是知识图谱抽取器。从单条项目事实中抽取 1-3 个实体-关系三元组：\n",
        "- 实体是具体名词（技术、模块、文件、工具、人），关系是简短动词/介词短语；\n",
        "- 例：事实「项目使用 rusqlite 存储聊天记录」→ [{\"entity1\":\"项目\",\"relation\":\"使用\",\"entity2\":\"rusqlite\"},{\"entity1\":\"rusqlite\",\"relation\":\"存储\",\"entity2\":\"聊天记录\"}]；\n",
        "- 宁缺毋滥：无明确关系输出空数组。\n",
        "只输出 JSON 数组：[{entity1, relation, entity2}]。",
    );
    let user = format!("事实：{content}\n\n输出 JSON 数组：");
    match chat_json(&cfg, system, &user, 400).await {
        Ok(v) => {
            let arr = match v {
                serde_json::Value::Array(a) => a,
                _ => return Vec::new(),
            };
            arr.into_iter()
                .filter_map(|item| serde_json::from_value::<RelationItem>(item).ok())
                .filter(|r| !r.entity1.trim().is_empty() && !r.entity2.trim().is_empty())
                .map(|r| (r.entity1, r.relation, r.entity2))
                .take(3)
                .collect()
        }
        Err(_) => Vec::new(),
    }
}

fn score(importance: f32, access: i64) -> f32 {
    importance * (1.0 + 0.1 * (access.min(20)) as f32)
}

/// Batch extraction: take the top limit active facts with no relations yet, sorted by importance x access-weighted score descending.
pub async fn extract_relations(
    app: &AppHandle,
    store: &MemoryStore,
    limit: u32,
) -> Result<GraphExtractReport, String> {
    let mut report = GraphExtractReport {
        facts_processed: 0,
        relations: 0,
        failed: 0,
        error: None,
    };
    let mut facts = store.list_active_facts_with_vectors()?;
    facts.sort_by(|a, b| {
        score(b.2, b.3)
            .partial_cmp(&score(a.2, a.3))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let limit = (limit as usize).min(facts.len());
    for fact in facts.into_iter().take(limit) {
        let (id, content, _, _, _) = fact;
        // Idempotent: skip facts that already have relations (re-running adds more incrementally)
        if !store.relations_for(&id)?.is_empty() {
            continue;
        }
        report.facts_processed += 1;
        let rels = extract_from_fact(app, &content).await;
        if rels.is_empty() {
            // Either the LLM found no extractable relations or the call failed — both count as failed (auditable)
            report.failed += 1;
            continue;
        }
        report.relations += rels.len();
        store.set_relations(&id, &rels)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score_test() {
        // Weighted score is monotonic: higher importance or access count yields a higher score
        assert!(score(0.8, 5) > score(0.5, 0));
        assert!(score(0.5, 30) > score(0.5, 0));
        // Access count is capped at 20
        assert!((score(0.5, 20) - score(0.5, 100)).abs() < 1e-6);
    }

    #[test]
    fn test_score_ordering() {
        score_test();
    }
}

//! P3-3 轻量知识图谱（Graphiti-lite）：LLM 从事实文本抽取实体-关系三元组，
//! 写入 `facts_relations`（先删后插，幂等）。检索侧在 `retrieve::apply_graph_boost`
//! 中以实体命中给结果加分（轻量字符串匹配，不做多跳图查询）。

use crate::services::memory::llm::{chat_json, llm_config};
use crate::services::memory::store::MemoryStore;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

/// 图谱抽取报告（memory_graph_extract 命令返回）。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GraphExtractReport {
    /// 本次处理（抽取/尝试）的事实数
    pub facts_processed: usize,
    /// 新增/覆写的关系三元组数
    pub relations: usize,
    /// LLM 抽取失败的事实数
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

/// 抽取单条事实的关系三元组（LLM 失败返回空）。
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

/// 批量抽取：按 importance × 访问加权分降序取前 limit 条尚无关系的 active 事实。
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
        // 幂等：已有关系则跳过（用户可重跑以增量补充）
        if !store.relations_for(&id)?.is_empty() {
            continue;
        }
        report.facts_processed += 1;
        let rels = extract_from_fact(app, &content).await;
        if rels.is_empty() {
            // LLM 无可抽取关系或调用失败——两者都计数为 failed（可审计）
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
        // 加权分单调：importance 与访问次数越高分越高
        assert!(score(0.8, 5) > score(0.5, 0));
        assert!(score(0.5, 30) > score(0.5, 0));
        // 访问次数封顶 20
        assert!((score(0.5, 20) - score(0.5, 100)).abs() < 1e-6);
    }

    #[test]
    fn test_score_ordering() {
        score_test();
    }
}

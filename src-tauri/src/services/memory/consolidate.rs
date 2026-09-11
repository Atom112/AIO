//! Sleep-time 合并整理：空闲/每日对高相似事实聚类，LLM 仲裁批量合并/订正/取代。
//!
//! - 输入：全部带向量的 active 事实（容量上限 MAX_CLUSTER_FACTS 控制 O(n²) 成本）；
//! - 聚类：余弦 >= CLUSTER_THRESHOLD 的连通分量；
//! - 每簇以「importance × 访问加权」最高者为锚点，对簇内其余成员逐条仲裁；
//! - 预算：每日仲裁调用上限（memory_consolidate_budget，默认 20），超限即止；
//! - 互斥：per-project meta 锁（带 10 分钟过期），与用户手动操作天然并发安全（单条事务）
//! - 失败：静默降级（tracing + 报告计数），绝不阻塞主流程。

use crate::services::memory::judge::JudgeAction;
use crate::services::memory::manager::MemoryStoreManager;
use crate::services::memory::store::{iso_to_secs_pub, MemoryStore};
use serde::Serialize;
use std::collections::HashMap;
use tauri::{AppHandle, Manager};

const CLUSTER_THRESHOLD: f32 = 0.85;
/// 参与聚类的最大事实数（控制 O(n²) 余弦扫描成本）。
const MAX_CLUSTER_FACTS: usize = 600;
const LOCK_TTL_SECS: u64 = 600;
const META_CONSOLIDATE_LOCK: &str = "consolidate_lock_ts";
const META_CONSOLIDATE_LAST: &str = "consolidate_last_run";

/// 合并整理报告（命令返回 + 每日调度日志）。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConsolidationReport {
    /// 参与整理的聚类数（每一簇 = 一组高相似事实）
    pub clusters: usize,
    /// 实际执行的仲裁调用数
    pub arbitrated: usize,
    pub merged: usize,
    pub updated: usize,
    pub superseded: usize,
    /// 仲裁判定 keep_separate 的条数
    pub kept: usize,
    /// 预算使用量（达到上限即提前终止）
    pub budget_used: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn score(importance: f32, access: i64) -> f32 {
    importance * (1.0 + 0.1 * (access.min(20)) as f32)
}

/// 余弦连通分量聚类（迭代并查集；返回索引组，每组 >1 成员）。
fn cluster_components(facts: &[(String, Vec<f32>)], threshold: f32) -> Vec<Vec<usize>> {
    let n = facts.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    for i in 0..n {
        for j in (i + 1)..n {
            let sim = crate::services::memory::vector::cosine(&facts[i].1, &facts[j].1);
            if sim >= threshold {
                let ri = root(&mut parent, i);
                let rj = root(&mut parent, j);
                if ri != rj {
                    parent[ri] = rj;
                }
            }
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        groups.entry(root(&mut parent, i)).or_default().push(i);
    }
    groups.into_values().filter(|g| g.len() > 1).collect()
}

/// 对单个项目执行一次合并整理（幂等、预算封顶、锁防重入）。
pub async fn consolidate(
    app: &AppHandle,
    store: &MemoryStore,
    budget: usize,
) -> Result<ConsolidationReport, String> {
    let mut report = ConsolidationReport {
        clusters: 0,
        arbitrated: 0,
        merged: 0,
        updated: 0,
        superseded: 0,
        kept: 0,
        budget_used: 0,
        error: None,
    };
    let now = crate::utils::time::now_iso();
    // 互斥锁（带过期）：10 分钟内不重入（每日调度与手动触发共用同一把锁）
    if let Some(ts) = store.meta_get(META_CONSOLIDATE_LOCK)? {
        if let (Some(t0), Some(t1)) = (iso_to_secs_pub(&ts), iso_to_secs_pub(&now)) {
            if t1.saturating_sub(t0) < LOCK_TTL_SECS {
                return Ok(report);
            }
        }
    }
    store.meta_put(META_CONSOLIDATE_LOCK, &now)?;
    let mut all = store.list_active_facts_with_vectors()?;
    if all.len() > MAX_CLUSTER_FACTS {
        all.sort_by(|a, b| {
            score(b.2, b.3)
                .partial_cmp(&score(a.2, a.3))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        all.truncate(MAX_CLUSTER_FACTS);
    }
    let id_text: Vec<(String, String, f32, i64)> = all
        .iter()
        .map(|(id, content, imp, acc, _)| (id.clone(), content.clone(), *imp, *acc))
        .collect();
    let pairs: Vec<(String, Vec<f32>)> = all
        .iter()
        .map(|(id, _, _, _, v)| (id.clone(), v.clone()))
        .collect();
    let components = cluster_components(&pairs, CLUSTER_THRESHOLD);
    report.clusters = components.len();
    let lcfg = match crate::services::memory::llm::llm_config(app) {
        Ok(c) => c,
        Err(e) => {
            report.error = Some(e);
            return Ok(report);
        }
    };
    'outer: for component in components {
        // 锚点 = 簇内加权分数最高者
        let mut anchor = component[0];
        for &idx in &component {
            if score(id_text[idx].2, id_text[idx].3) > score(id_text[anchor].2, id_text[anchor].3) {
                anchor = idx;
            }
        }
        let anchor_fact = id_text[anchor].clone();
        for &idx in &component {
            if idx == anchor {
                continue;
            }
            if report.budget_used >= budget {
                break 'outer;
            }
            let member = id_text[idx].clone();
            if member.0 == anchor_fact.0 {
                continue;
            }
            report.budget_used += 1;
            let (action, merged) =
                crate::services::memory::judge::judge(&lcfg, &anchor_fact.1, &member.1).await;
            crate::services::memory::judge::bump_judge_stat(store, &action);
            report.arbitrated += 1;
            match action {
                JudgeAction::KeepSeparate => report.kept += 1,
                JudgeAction::Merge => {
                    let merged_content = if merged.trim().is_empty() {
                        format!("{}；{}", anchor_fact.1, member.1)
                    } else {
                        merged
                    };
                    store.merge_facts(&anchor_fact.0, &member.0, &merged_content)?;
                    report.merged += 1;
                }
                JudgeAction::Update => {
                    store.update_content(&anchor_fact.0, &member.1, "update")?;
                    // 锚点内容更新后重新向量化（失败不阻塞）
                    if let Some(emb) = embed_one(app, store, &member.1).await {
                        let _ = store.set_embedding(&anchor_fact.0, Some(emb));
                    }
                    report.updated += 1;
                }
                JudgeAction::Supersede => {
                    store.supersede_fact(&member.0, "被合并整理取代")?;
                    report.superseded += 1;
                }
            }
        }
    }
    store.meta_put(META_CONSOLIDATE_LAST, &crate::utils::time::now_iso())?;
    Ok(report)
}

async fn embed_one(
    app: &AppHandle,
    store: &MemoryStore,
    text: &str,
) -> Option<(String, usize, Vec<f32>)> {
    let ecfg = crate::services::memory::resolve_project_embedding(app, store);
    let embedder = crate::plugins::embed::resolve(app, &ecfg).ok()?;
    let v = embedder
        .embed(&[text.to_string()])
        .await
        .ok()?
        .into_iter()
        .next()?;
    if let Some(lib_dim) = store.embedding_dim_meta() {
        if lib_dim != v.len() {
            return None;
        }
    }
    Some((embedder.model_key(), v.len(), v))
}

/// 每日调度入口：遍历启用记忆的项目逐项目整理（预算各项目独立）。
pub async fn run_daily_consolidation(app: &AppHandle) {
    let cfg = crate::core::config::load_app_config(app).unwrap_or_default();
    // 与自动提取同一开关：用户关闭自动提取即无需自动整理
    if !cfg.memory_auto_extract {
        return;
    }
    let budget = cfg.memory_consolidate_budget.max(1) as usize;
    // 项目索引读取统一在 utils::project_index
    let file = crate::utils::project_index::load_projects_file(app);
    for project in file.projects.values() {
        let Ok(store) = app.state::<MemoryStoreManager>().open(&project.path) else {
            continue;
        };
        let enabled = match store.project_enabled() {
            Ok(Some(b)) => b,
            _ => cfg.memory_enabled,
        };
        if !enabled {
            continue;
        }
        match consolidate(app, &store, budget).await {
            Ok(report) => {
                if report.arbitrated > 0 {
                    tracing::info!("[memory] 每日合并整理 {}：{:?}", project.path, report);
                }
            }
            Err(e) => tracing::warn!("[memory] 每日合并整理失败 {}: {e}", project.path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(axis: usize) -> Vec<f32> {
        let mut v = vec![0.0f32; 8];
        v[axis] = 1.0;
        v
    }

    #[test]
    fn test_cluster_components_finds_groups() {
        // 三条同轴（彼此相似）+ 两条异轴（不同主题）
        let facts = vec![
            ("a".into(), v(0)),
            ("b".into(), v(0)),
            ("c".into(), v(0)),
            ("d".into(), v(1)),
            ("e".into(), v(1)),
        ];
        let groups = cluster_components(&facts, 0.85);
        assert_eq!(groups.len(), 2, "应分为两组：{groups:?}");
        for g in &groups {
            assert!(g.len() >= 2);
        }
        let sizes: Vec<usize> = groups.iter().map(|g| g.len()).collect();
        sizes.iter().for_each(|s| assert!(*s >= 2));
    }

    #[test]
    fn test_cluster_components_orthogonal_ignored() {
        let facts = vec![("a".into(), v(0)), ("b".into(), v(1))];
        let groups = cluster_components(&facts, 0.85);
        assert!(groups.is_empty(), "正交向量不应成簇");
    }
}

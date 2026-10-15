//! Sleep-time consolidation: idle/daily clustering of highly similar facts, LLM arbitrates batch merges/corrections/supersessions.
//!
//! - Input: all active facts with vectors (capped at MAX_CLUSTER_FACTS to bound O(n²) cost);
//! - Clustering: connected components with cosine >= CLUSTER_THRESHOLD;
//! - Each cluster anchors on the member with the highest importance x access-weighted score and arbitrates the remaining members one by one;
//! - Budget: daily arbitration call cap (memory_consolidate_budget, default 20); stop once exceeded;
//! - Mutual exclusion: per-project meta lock (10-minute expiry) keeps daily scheduling and manual triggers concurrency-safe (single-statement transactions)
//! - Failure: silent degradation (tracing + report counters); never blocks the main flow.

// MEM-06: canonical single-text embedding (dimension-drift guard included)
use crate::services::memory::embed_one;
use crate::services::memory::judge::JudgeAction;
use crate::services::memory::manager::MemoryStoreManager;
use crate::services::memory::store::MemoryStore;
use serde::Serialize;
use std::collections::HashMap;
use tauri::{AppHandle, Emitter, Manager};

const CLUSTER_THRESHOLD: f32 = 0.85;
/// Maximum number of facts participating in clustering (bounds the O(n²) cosine scan cost).
const MAX_CLUSTER_FACTS: usize = 600;
const LOCK_TTL_SECS: u64 = 600;
const META_CONSOLIDATE_LOCK: &str = "consolidate_lock_ts";
const META_CONSOLIDATE_LAST: &str = "consolidate_last_run";

/// Consolidation report (command return value + daily scheduling log).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConsolidationReport {
    /// Number of clusters consolidated (each cluster = one group of highly similar facts)
    pub clusters: usize,
    /// Number of arbitration calls actually executed
    pub arbitrated: usize,
    pub merged: usize,
    pub updated: usize,
    pub superseded: usize,
    /// Number of facts the judge resolved as keep_separate
    pub kept: usize,
    /// Budget usage (terminates early once the cap is reached)
    pub budget_used: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// MEM-02: the DB mutation plan derived from one judge verdict. `judge` is called
/// as judge(existing=anchor, incoming=member), and this pure mapping makes the
/// direction contract testable WITHOUT the LLM — the daily auto-consolidation
/// once inverted Supersede (it archived the NEW fact and kept the stale one,
/// making the store WORSE every night).
#[derive(Debug)]
pub(crate) enum ConsolidatePlan {
    KeepSeparate,
    /// Merge the two rows into `anchor_id` with the (possibly joined) content.
    Merge {
        anchor_id: String,
        member_id: String,
        content: String,
    },
    /// Overwrite `anchor_id` with the member's (more accurate) content; the
    /// caller re-embeds the new content after `update_content`.
    Update {
        anchor_id: String,
        content: String,
    },
    /// The OLD fact (anchor) steps down; the member row stays as the winner.
    SupersedeLoser {
        loser_id: String,
    },
}

/// Map a judge verdict (existing=anchor, incoming=member) onto its execution
/// plan. Pure: no DB, no LLM, no clock — see the MEM-02 tests for the direction
/// symmetry against the tools.rs / extract.rs contract (both supersede existing).
pub(crate) fn plan_from_judge(
    action: JudgeAction,
    anchor_id: &str,
    anchor_content: &str,
    member_id: &str,
    member_content: &str,
    merged: String,
) -> ConsolidatePlan {
    match action {
        JudgeAction::KeepSeparate => ConsolidatePlan::KeepSeparate,
        JudgeAction::Merge => {
            let content = if merged.trim().is_empty() {
                format!("{}；{}", anchor_content, member_content)
            } else {
                merged
            };
            ConsolidatePlan::Merge {
                anchor_id: anchor_id.to_string(),
                member_id: member_id.to_string(),
                content,
            }
        }
        JudgeAction::Update => ConsolidatePlan::Update {
            anchor_id: anchor_id.to_string(),
            content: member_content.to_string(),
        },
        JudgeAction::Supersede => ConsolidatePlan::SupersedeLoser {
            // MEM-02: the loser is the EXISTING (anchor) fact — member wins.
            loser_id: anchor_id.to_string(),
        },
    }
}

fn score(importance: f32, access: i64) -> f32 {
    importance * (1.0 + 0.1 * (access.min(20)) as f32)
}

/// Cosine connected-component clustering (iterative union-find); returns index groups, each with more than one member.
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

/// Run one consolidation pass for a single project (idempotent, budget-capped, lock-guarded against re-entry).
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
    // MEM-9: the expiring mutex is claimed with ONE atomic check-and-set
    // (INSERT … ON CONFLICT … WHERE old <= cutoff). The previous read-then-write
    // sequence let two concurrent consolidations both pass the freshness check
    // before either had written the lock, double-spending the LLM budget.
    let cutoff = crate::utils::time::secs_to_iso(
        crate::utils::time::now_unix_secs().saturating_sub(LOCK_TTL_SECS),
    );
    if !store.meta_try_lock(META_CONSOLIDATE_LOCK, &cutoff, &now)? {
        return Ok(report); // 另一次整理在 10 分钟窗口内，直接让位
    }
    // A real pass starts here (the lock-skip above never touched the store): tell
    // the frontend's background-task menu the job is running.
    let _ = app.emit(
        crate::core::events::MEMORY_CONSOLIDATE_PROGRESS,
        serde_json::json!({ "status": "running", "project": store.project_root() }),
    );
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
        // Anchor = the member with the highest weighted score in the cluster
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
            match plan_from_judge(
                action,
                &anchor_fact.0,
                &anchor_fact.1,
                &member.0,
                &member.1,
                merged,
            ) {
                ConsolidatePlan::KeepSeparate => report.kept += 1,
                ConsolidatePlan::Merge {
                    anchor_id,
                    member_id,
                    content,
                } => {
                    store.merge_facts(&anchor_id, &member_id, &content)?;
                    report.merged += 1;
                }
                ConsolidatePlan::Update { anchor_id, content } => {
                    store.update_content(&anchor_id, &content, "update")?;
                    // Re-embed the anchor after its content is updated (failure does not block)
                    if let Some(emb) = embed_one(app, store, &content).await {
                        let _ = store.set_embedding(&anchor_id, Some(emb));
                    }
                    report.updated += 1;
                }
                ConsolidatePlan::SupersedeLoser { loser_id } => {
                    store.supersede_fact(&loser_id, "被更准确的事实取代")?;
                    report.superseded += 1;
                }
            }
        }
    }
    store.meta_put(META_CONSOLIDATE_LAST, &crate::utils::time::now_iso())?;
    Ok(report)
}

/// Emit one consolidation terminal event (background-task menu feed). Shared by the
/// daily scheduler loop and the manual command wrapper so both report identically.
pub(crate) fn emit_consolidate_terminal(
    app: &AppHandle,
    project: &str,
    outcome: Result<&ConsolidationReport, &str>,
) {
    let payload = match outcome {
        Ok(rep) => serde_json::json!({
            "status": "done",
            "project": project,
            "clusters": rep.clusters,
            "arbitrated": rep.arbitrated,
            "merged": rep.merged,
            "updated": rep.updated,
            "superseded": rep.superseded,
        }),
        Err(e) => serde_json::json!({ "status": "error", "project": project, "error": e }),
    };
    let _ = app.emit(crate::core::events::MEMORY_CONSOLIDATE_PROGRESS, payload);
}

/// Daily scheduling entry point: iterate memory-enabled projects and consolidate each one (per-project budget).
pub async fn run_daily_consolidation(app: &AppHandle) {
    let cfg = crate::core::config::load_app_config(app).unwrap_or_default();
    // Shares the switch with auto-extraction: if auto-extraction is disabled, auto-consolidation is unnecessary
    if !cfg.memory_auto_extract {
        return;
    }
    let budget = cfg.memory_consolidate_budget.max(1) as usize;
    // Project index reads are centralized in utils::project_index
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
        // MEM-15: per-project panic containment. A panic inside one project's
        // consolidation used to unwind through the whole loop, silently skipping
        // every remaining project's daily pass until the next day. Errors were
        // already contained per project; panics now are too.
        let outcome = futures_util::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(
            consolidate(app, &store, budget),
        ))
        .await;
        match outcome {
            Ok(Ok(report)) => {
                emit_consolidate_terminal(app, &project.path, Ok(&report));
                if report.arbitrated > 0 {
                    tracing::info!("[memory] 每日合并整理 {}：{:?}", project.path, report);
                }
            }
            Ok(Err(e)) => {
                emit_consolidate_terminal(app, &project.path, Err(e.as_str()));
                tracing::warn!("[memory] 每日合并整理失败 {}: {e}", project.path);
            }
            Err(panic) => {
                let reason = panic
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "unknown panic".to_string());
                emit_consolidate_terminal(app, &project.path, Err(reason.as_str()));
                tracing::error!(
                    "[memory] 每日合并整理 panic（已隔离） {}: {reason}",
                    project.path
                );
            }
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
        // Three facts on the same axis (mutually similar) + two on a different axis (different topic)
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

    /// MEM-02 对称性回归：judge(existing=anchor, incoming=member) 判 Supersede 时，
    /// 让位的必须是 **anchor（旧事实）**——与 tools.rs / extract.rs 的
    /// "supersede existing" 契约同向。历史实现反着归档 member（新事实），
    /// 每日自动整理持续把"更准确"的信息标记过时、保留过时内容。
    #[test]
    fn supersede_direction_matches_tools_and_extract_contract() {
        let plan = plan_from_judge(
            JudgeAction::Supersede,
            "fact-old",
            "旧：服务器是 A 型",
            "fact-new",
            "新：服务器已换为 B 型",
            String::new(),
        );
        match plan {
            ConsolidatePlan::SupersedeLoser { loser_id } => {
                assert_eq!(loser_id, "fact-old", "让位的应是旧事实 anchor");
            }
            other => panic!("Supersede 应映射为 SupersedeLoser，实际 {other:?} 不适用"),
        }
    }

    /// MEM-02 契约面（纯函数，无 LLM）：Update 采用 member 的内容改写 anchor；
    /// Merge 内容为空时回退拼接、非空保留模型产物；KeepSeparate 原样返回。
    #[test]
    fn judge_verdicts_map_to_their_db_plans() {
        let update = plan_from_judge(
            JudgeAction::Update,
            "fact-old",
            "旧内容",
            "fact-new",
            "更准确的内容",
            String::new(),
        );
        match update {
            ConsolidatePlan::Update {
                anchor_id, content, ..
            } => {
                assert_eq!(anchor_id, "fact-old");
                assert_eq!(
                    content, "更准确的内容",
                    "Update 应采用 member 内容改写 anchor"
                );
            }
            other => panic!("Update 映射错误: {other:?}"),
        }

        let joined = plan_from_judge(
            JudgeAction::Merge,
            "fact-old",
            "甲",
            "fact-new",
            "乙",
            String::new(),
        );
        match joined {
            ConsolidatePlan::Merge { content, .. } => {
                assert_eq!(content, "甲；乙", "模型未产出合并内容时回退拼接");
            }
            other => panic!("Merge 映射错误: {other:?}"),
        }

        let authored = plan_from_judge(
            JudgeAction::Merge,
            "fact-old",
            "甲",
            "fact-new",
            "乙",
            "模型合并稿".to_string(),
        );
        match authored {
            ConsolidatePlan::Merge { content, .. } => {
                assert_eq!(content, "模型合并稿", "模型产物优先于拼接");
            }
            other => panic!("Merge 映射错误: {other:?}"),
        }

        assert!(matches!(
            plan_from_judge(JudgeAction::KeepSeparate, "a", "x", "b", "y", String::new()),
            ConsolidatePlan::KeepSeparate
        ));
    }
}

//! 后台自动事实提取：把一轮对话（user + assistant + 工具结果）提炼为项目事实。
//!
//! 提取为 sleep-time compute：主 Agent 循环结束后异步执行，去抖，绝不阻塞交互。
//! 写入前与既有事实做向量查重，高相似时走 LLM 仲裁（merge/update/supersede/keep_separate），
//! 与显式 remember 同一判重链路（自进化闭环的自动侧）。

use crate::services::memory::judge::JudgeAction;
use crate::services::memory::llm::{chat_json, llm_config, LlmConfig};
use crate::services::memory::manager::MemoryStoreManager;
use crate::services::memory::store::{MemoryStore, NewFact};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

/// 单次提取产出的一条事实。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedFact {
    pub content: String,
    pub category: String,
    pub importance: f32,
    /// 证据：对话记录中的消息序号（形如 [user#0]/[assistant#1] 的 # 后数字），可为空
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<usize>,
}

/// 提取执行结果（供事件与日志）。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionReport {
    pub extracted: usize,
    pub skipped: usize,
    pub embedded: usize,
    pub archived_over_cap: usize,
    /// 判重命中但 LLM 仲裁不可用时的保守跳过条数（fail-safe：宁可少进，不误并误删）
    #[serde(default, skip_serializing_if = "is_zero")]
    pub deduped: usize,
    /// 仲裁动作：订正既有事实（update）
    #[serde(default, skip_serializing_if = "is_zero")]
    pub updated: usize,
    /// 仲裁动作：合并入既有事实（merge）
    #[serde(default, skip_serializing_if = "is_zero")]
    pub merged: usize,
    /// 仲裁动作：新事实取代旧事实（supersede），新事实照常入库
    #[serde(default, skip_serializing_if = "is_zero")]
    pub superseded: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

const MAX_PER_RUN: usize = 5;
const MIN_IMPORTANCE: f32 = 0.6;
const META_LAST_EXTRACT: &str = "last_extract_at";

/// 提取管线入口：校验开关/去抖 → 提取 → 判重 + LLM 仲裁 → 写入（向量化 + 容量治理）。
pub async fn extract_and_store(
    app: &AppHandle,
    project_root: &str,
    transcript: &str,
) -> Result<ExtractionReport, String> {
    let empty = ExtractionReport {
        extracted: 0,
        skipped: 0,
        embedded: 0,
        archived_over_cap: 0,
        deduped: 0,
        updated: 0,
        merged: 0,
        superseded: 0,
        error: None,
    };
    let cfg = crate::core::config::load_app_config(app).unwrap_or_default();
    if transcript.trim().is_empty() {
        return Ok(empty);
    }
    let store = app.state::<MemoryStoreManager>().open(project_root)?;
    // 每项目开关（覆盖优先）
    let project_override = store.project_enabled()?;
    if !project_override.unwrap_or(cfg.memory_enabled) || !cfg.memory_auto_extract {
        return Ok(empty);
    }
    // 去抖：距上次提取不足 debounce 秒则跳过
    let debounce = cfg.memory_extract_debounce_secs.max(1);
    if let Some(last) = store.meta_get(META_LAST_EXTRACT)? {
        if let Some(secs) = crate::services::memory::store::iso_to_secs_pub(&last) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if now.saturating_sub(secs) < debounce {
                return Ok(empty);
            }
        }
    }
    // 提取
    let lcfg = match llm_config(app) {
        Ok(c) => c,
        Err(e) => {
            let _ = app.emit(
                "memory-extract-progress",
                serde_json::json!({ "status": "error", "error": e }),
            );
            return Ok(ExtractionReport {
                error: Some(e),
                ..empty
            });
        }
    };
    let facts = extract_facts(&lcfg, transcript).await;
    let mut report = empty;
    // 判重阈值与写入侧（remember）一致
    let dedup_threshold = cfg.memory_dedup_threshold.clamp(0.0, 1.0);
    for f in facts {
        if f.content.trim().is_empty() || f.importance < MIN_IMPORTANCE {
            report.skipped += 1;
            continue;
        }
        // 证据链：消息序号写入 source_refs（JSON），可追溯、可审计
        let source_refs = if f.evidence.is_empty() {
            None
        } else {
            Some(serde_json::json!({ "messageIds": f.evidence }).to_string())
        };
        let fact = NewFact {
            key: None,
            content: f.content,
            category: f.category,
            importance: f.importance,
            source_type: Some("conversation".into()),
            source_refs,
            pinned: false,
        };
        let embedding = embed_one(app, &store, &fact.content).await;
        // 写入前向量查重 + LLM 仲裁（与 remember 同一判重链路）
        let mut resolved: Option<String> = None; // Some(既有事实 id) = 已订正/合并，不再新增
        if let Some((_, _, vec)) = &embedding {
            if let Ok(dups) = store.search_similar(vec, 3) {
                if let Some((dup_id, _)) = dups.iter().find(|(_, c)| *c >= dedup_threshold).cloned()
                {
                    store.rolling_bump(crate::services::memory::store::STAT_DEDUP, 1);
                    if let Ok(existing) = store.get_fact(&dup_id) {
                        match llm_config(app) {
                            Ok(jcfg) => {
                                let (action, merged) = crate::services::memory::judge::judge(
                                    &jcfg,
                                    &existing.content,
                                    &fact.content,
                                )
                                .await;
                                crate::services::memory::judge::bump_judge_stat(&store, &action);
                                match action {
                                    JudgeAction::KeepSeparate => {}
                                    JudgeAction::Update => {
                                        store.update_content(
                                            &existing.id,
                                            &fact.content,
                                            "update",
                                        )?;
                                        if let Some(emb) = &embedding {
                                            store.set_embedding(&existing.id, Some(emb.clone()))?;
                                        }
                                        report.updated += 1;
                                        resolved = Some(existing.id);
                                    }
                                    JudgeAction::Merge => {
                                        let merged_content = if merged.trim().is_empty() {
                                            format!("{}；{}", existing.content, fact.content)
                                        } else {
                                            merged
                                        };
                                        store.update_content(
                                            &existing.id,
                                            &merged_content,
                                            "merge",
                                        )?;
                                        if let Some((m, d, v)) =
                                            embed_one(app, &store, &merged_content).await
                                        {
                                            let _ =
                                                store.set_embedding(&existing.id, Some((m, d, v)));
                                        }
                                        report.merged += 1;
                                        resolved = Some(existing.id);
                                    }
                                    JudgeAction::Supersede => {
                                        store.supersede_fact(
                                            &existing.id,
                                            "被自动提取的新事实取代",
                                        )?;
                                        report.superseded += 1;
                                    }
                                }
                            }
                            // fail-safe：仲裁不可用时保守跳过（宁缺毋滥）
                            Err(_) => {
                                report.deduped += 1;
                                resolved = Some(existing.id);
                            }
                        }
                    } else {
                        report.deduped += 1;
                        resolved = Some(dup_id);
                    }
                }
            }
        }
        if resolved.is_some() {
            // 已订正/合并/跳过，无需新增
            continue;
        }
        if embedding.is_some() {
            report.embedded += 1;
        }
        match store.upsert_fact(&fact, embedding) {
            Ok(_) => report.extracted += 1,
            Err(_) => report.skipped += 1,
        }
    }
    store.rolling_bump(
        crate::services::memory::store::STAT_EXTRACTED,
        report.extracted as i64,
    );
    report.archived_over_cap = store.prune_to_capacity(cfg.memory_max_facts.max(100))?;
    store.meta_put(META_LAST_EXTRACT, &crate::utils::time::now_iso())?;
    let _ = app.emit(
        "memory-extract-progress",
        serde_json::json!({
            "status": "done",
            "extracted": report.extracted,
            "embedded": report.embedded,
            "skipped": report.skipped,
            "deduped": report.deduped,
            "updated": report.updated,
            "merged": report.merged,
            "superseded": report.superseded,
            "archived": report.archived_over_cap,
        }),
    );
    Ok(report)
}

/// 调用 LLM 提取事实（模型输出 JSON 数组；解析失败返回空）。
pub async fn extract_facts(cfg: &LlmConfig, transcript: &str) -> Vec<ExtractedFact> {
    let system = concat!(
        "你是项目记忆提取器。从对话中提取值得长期记住的项目事实（架构决策、代码约定、依赖变更、关键路径、重要发现）。\n",
        "只提取有明确依据的事实，宁缺毋滥；避免记录临时状态、客套话与无关讨论。\n",
        "类别限于 decision|pattern|convention|note|entity|architecture|task。\n",
        "每条输出 {content, category, importance(0到1), evidence}：evidence 为该事实依据的消息序号数组（对话记录中形如 [user#0]/[assistant#1] 的 # 后数字；无明确依据可留空）。\n",
        "输出 JSON 数组，最多 5 条。",
    );
    let user = format!("对话记录：\n{transcript}\n\n输出 JSON 数组：");
    match chat_json(cfg, system, &user, 900).await {
        Ok(v) => parse_extracted(&v),
        Err(_) => Vec::new(),
    }
}

/// 解析提取结果：接受数组或 {facts: [...]}；evidence 缺失/越界时置空。
pub fn parse_extracted(v: &serde_json::Value) -> Vec<ExtractedFact> {
    let arr: &Vec<serde_json::Value> = match v {
        serde_json::Value::Array(a) => a,
        serde_json::Value::Object(m) => match m.get("facts").and_then(|x| x.as_array()) {
            Some(a) => a,
            None => return Vec::new(),
        },
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    for item in arr {
        if let Ok(f) = serde_json::from_value::<ExtractedFact>(item.clone()) {
            if f.content.trim().is_empty() {
                continue;
            }
            let importance = f.importance.clamp(0.0, 1.0);
            // 证据序号消毒：仅保留 < 64 的合理序号（转录消息数上限 12）
            let evidence: Vec<usize> = f.evidence.into_iter().filter(|i| *i < 64).collect();
            out.push(ExtractedFact {
                content: f.content,
                category: if f.category.trim().is_empty() {
                    "note".into()
                } else {
                    f.category
                },
                importance,
                evidence,
            });
        }
    }
    out.truncate(MAX_PER_RUN);
    out
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
            tracing::warn!(
                "记忆库向量维度 {} 与当前嵌入模型维度 {} 不一致，跳过向量化",
                lib_dim,
                v.len()
            );
            return None;
        }
    }
    Some((embedder.model_key(), v.len(), v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_extracted_array() {
        let v = json!([
            { "content": "使用 rusqlite", "category": "decision", "importance": 0.8, "evidence": [0, 2] },
            { "content": "", "category": "note", "importance": 0.9 },
            { "content": "低优先级", "category": "note", "importance": 0.2 },
        ]);
        let facts = parse_extracted(&v);
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].content, "使用 rusqlite");
        assert_eq!(facts[0].category, "decision");
        assert_eq!(facts[0].evidence, vec![0, 2]);
    }

    #[test]
    fn test_parse_extracted_evidence_sanitized() {
        let v = json!([
            { "content": "A", "category": "note", "importance": 0.7, "evidence": [0, 128, 3] }
        ]);
        let facts = parse_extracted(&v);
        assert_eq!(facts[0].evidence, vec![0, 3], "越界序号应被过滤");

        let v2 = json!([{ "content": "B", "category": "note", "importance": 0.7 }]);
        let facts2 = parse_extracted(&v2);
        assert!(facts2[0].evidence.is_empty(), "缺失 evidence 应为空");
    }

    #[test]
    fn test_parse_extracted_object() {
        let v = json!({ "facts": [{ "content": "A", "category": "note", "importance": 0.7 }] });
        assert_eq!(parse_extracted(&v).len(), 1);
    }

    #[test]
    fn test_parse_extracted_garbage() {
        assert!(parse_extracted(&json!("nope")).is_empty());
        assert!(parse_extracted(&json!(null)).is_empty());
    }
}

//! Background automatic fact extraction: distills a conversation turn (user + assistant + tool results) into project facts.
//!
//! Extraction is sleep-time compute: runs asynchronously after the main agent loop ends, debounced, never blocking interaction.
//! Before writing, vector-deduplicates against existing facts; high similarity goes through LLM arbitration (merge/update/supersede/keep_separate),
//! the same dedup chain as explicit remember (the automatic side of the self-improving loop).

// MEM-06: canonical single-text embedding (dimension-drift guard included)
use crate::services::memory::embed_one;
use crate::services::memory::judge::JudgeAction;
use crate::services::memory::llm::{chat_json, llm_config, LlmConfig};
use crate::services::memory::manager::MemoryStoreManager;
use crate::services::memory::store::NewFact;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

/// One fact produced by a single extraction.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedFact {
    pub content: String,
    pub category: String,
    pub importance: f32,
    /// Evidence: message indices in the transcript (the number after # in [user#0]/[assistant#1]); may be empty
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<usize>,
}

/// Extraction run result (for events and logs).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionReport {
    pub extracted: usize,
    pub skipped: usize,
    pub embedded: usize,
    pub archived_over_cap: usize,
    /// Conservative skips when a dedup hit occurs but LLM arbitration is unavailable (fail-safe: fewer inserts over wrong merges/deletes)
    #[serde(default, skip_serializing_if = "is_zero")]
    pub deduped: usize,
    /// Arbitration action: correct an existing fact (update)
    #[serde(default, skip_serializing_if = "is_zero")]
    pub updated: usize,
    /// Arbitration action: merge into an existing fact (merge)
    #[serde(default, skip_serializing_if = "is_zero")]
    pub merged: usize,
    /// Arbitration action: the new fact supersedes the old one (supersede); the new fact is stored as usual
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

/// Extraction pipeline entry: check switches/debounce -> extract -> dedup + LLM arbitration -> write (vectorize + capacity governance).
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
    // Per-project switch (override takes precedence)
    let project_override = store.project_enabled()?;
    if !project_override.unwrap_or(cfg.memory_enabled) || !cfg.memory_auto_extract {
        return Ok(empty);
    }
    // Debounce: skip if less than debounce seconds since the last extraction
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
    // A real run starts here (skips above never extracted anything): tell the
    // frontend's background-task menu the job is running.
    let _ = app.emit(
        crate::core::events::MEMORY_EXTRACT_PROGRESS,
        serde_json::json!({ "status": "running", "project": project_root }),
    );
    let lcfg = match llm_config(app) {
        Ok(c) => c,
        Err(e) => {
            let _ = app.emit(
                crate::core::events::MEMORY_EXTRACT_PROGRESS,
                serde_json::json!({ "status": "error", "project": project_root, "error": e }),
            );
            return Ok(ExtractionReport {
                error: Some(e),
                ..empty
            });
        }
    };
    let facts = extract_facts(&lcfg, transcript).await;
    let mut report = empty;
    // Dedup threshold matches the write side (remember)
    let dedup_threshold = cfg.memory_dedup_threshold.clamp(0.0, 1.0);
    for f in facts {
        if f.content.trim().is_empty() || f.importance < MIN_IMPORTANCE {
            report.skipped += 1;
            continue;
        }
        // Evidence chain: message indices go into source_refs (JSON), traceable and auditable
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
        // Pre-write vector dedup + LLM arbitration (same dedup chain as remember)
        let mut resolved: Option<String> = None; // Some(existing fact id) = already corrected/merged, no new insert
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
                            // fail-safe: skip conservatively when arbitration is unavailable (quality over quantity)
                            Err(_) => {
                                report.deduped += 1;
                                resolved = Some(existing.id);
                            }
                        }
                    }
                    // MEM-05: get_fact 失败（幽灵向量——clear 残留等）视为"无重复"，
                    // 落到下方正常插入；此前"读取失败"被当作"已判重"，与旧幽灵
                    // 相似的新事实被静默丢弃
                }
            }
        }
        if resolved.is_some() {
            // Already corrected/merged/skipped; no insert needed
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
        crate::core::events::MEMORY_EXTRACT_PROGRESS,
        serde_json::json!({
            "status": "done",
            "project": project_root,
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

/// Call the LLM to extract facts (the model outputs a JSON array; returns empty on parse failure).
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

/// Parse extraction results: accepts an array or {facts: [...]}; missing/out-of-range evidence is emptied.
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
            // Sanitize evidence indices: keep only plausible indices < 64 (transcripts hold at most 12 messages)
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

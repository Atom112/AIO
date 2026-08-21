//! 后台自动事实提取：把一轮对话（user + assistant + 工具结果）提炼为项目事实。
//!
//! 提取为 sleep-time compute：主 Agent 循环结束后异步执行，去抖，绝不阻塞交互。

use crate::services::memory::llm::{chat_json, llm_config, LlmConfig};
use crate::services::memory::manager::MemoryStoreManager;
use crate::services::memory::store::NewFact;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

/// 单次提取产出的一条事实。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedFact {
    pub content: String,
    pub category: String,
    pub importance: f32,
}

/// 提取执行结果（供事件与日志）。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionReport {
    pub extracted: usize,
    pub skipped: usize,
    pub embedded: usize,
    pub archived_over_cap: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

const MAX_PER_RUN: usize = 5;
const MIN_IMPORTANCE: f32 = 0.6;
const META_LAST_EXTRACT: &str = "last_extract_at";

/// 提取管线入口：校验开关/去抖 → 提取 → 写入（向量化 + 容量治理）。
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
        error: None,
    };
    let cfg = crate::commands::config::load_app_config(app.clone()).unwrap_or_default();
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
    for f in facts {
        if f.content.trim().is_empty() || f.importance < MIN_IMPORTANCE {
            report.skipped += 1;
            continue;
        }
        let fact = NewFact {
            key: None,
            content: f.content,
            category: f.category,
            importance: f.importance,
            source_type: Some("conversation".into()),
            source_refs: None,
            pinned: false,
        };
        let embedding = embed_one(app, &fact.content).await;
        if embedding.is_some() {
            report.embedded += 1;
        }
        match store.upsert_fact(&fact, embedding) {
            Ok(_) => report.extracted += 1,
            Err(_) => report.skipped += 1,
        }
    }
    report.archived_over_cap = store.prune_to_capacity(cfg.memory_max_facts.max(100))?;
    store.meta_put(META_LAST_EXTRACT, &crate::utils::knowledge::now_iso())?;
    let _ = app.emit(
        "memory-extract-progress",
        serde_json::json!({
            "status": "done",
            "extracted": report.extracted,
            "embedded": report.embedded,
            "skipped": report.skipped,
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
        "输出 JSON 数组，每项 {content, category, importance(0到1)}，最多 5 条。",
    );
    let user = format!("对话记录：\n{transcript}\n\n输出 JSON 数组：");
    match chat_json(cfg, system, &user, 900).await {
        Ok(v) => parse_extracted(&v),
        Err(_) => Vec::new(),
    }
}

/// 解析提取结果：接受数组或 {facts: [...]}。
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
            out.push(ExtractedFact {
                content: f.content,
                category: if f.category.trim().is_empty() {
                    "note".into()
                } else {
                    f.category
                },
                importance,
            });
        }
    }
    out.truncate(MAX_PER_RUN);
    out
}

async fn embed_one(app: &AppHandle, text: &str) -> Option<(String, usize, Vec<f32>)> {
    let cfg = crate::commands::config::load_app_config(app.clone()).unwrap_or_default();
    let mut ecfg = cfg.memory_embedding;
    if ecfg.provider == "openai_compat" && ecfg.api_key.is_empty() {
        if let Ok(Some(key)) = crate::core::secure_store::get(app, "embedding_api_key") {
            ecfg.api_key = key;
        }
    }
    let embedder = crate::plugins::embed::resolve(&ecfg).ok()?;
    let v = embedder
        .embed(&[text.to_string()])
        .await
        .ok()?
        .into_iter()
        .next()?;
    Some((embedder.model_key(), v.len(), v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_extracted_array() {
        let v = json!([
            { "content": "使用 rusqlite", "category": "decision", "importance": 0.8 },
            { "content": "", "category": "note", "importance": 0.9 },
            { "content": "低优先级", "category": "note", "importance": 0.2 },
        ]);
        let facts = parse_extracted(&v);
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].content, "使用 rusqlite");
        assert_eq!(facts[0].category, "decision");
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

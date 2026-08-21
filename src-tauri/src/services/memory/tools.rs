//! 记忆工具（remember / recall / search_memory / update_memory / forget_memory）：
//! 模型可见的工具规格与执行实现。
//!
//! 记忆未启用时 remember / recall 回退到旧版 knowledge.json 实现，保持向后兼容。

use crate::core::models::{ToolFunctionSpec, ToolResult, ToolResultContent, ToolSpec};
use crate::services::memory::judge::JudgeAction;
use crate::services::memory::manager::MemoryStoreManager;
use crate::services::memory::store::{MemoryStore, NewFact, SearchOptions};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, Manager};

/// 返回记忆工具的五个 ToolSpec。
pub fn get_memory_tool_specs() -> Vec<ToolSpec> {
    vec![
        remember_spec(),
        recall_spec(),
        search_memory_spec(),
        update_memory_spec(),
        forget_memory_spec(),
        search_code_spec(),
    ]
}

fn tool_ok(text: String) -> ToolResult {
    ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({ "text": text }),
        }],
        is_error: false,
    }
}

fn remember_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".to_string(),
        function: ToolFunctionSpec {
            name: "remember".to_string(),
            description: "记住一条项目级事实（架构决策、代码约定、常用模式、重要发现），跨 session 持久化，支持语义检索".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "知识的简短标识（用于去重，可选）" },
                    "content": { "type": "string", "description": "事实内容" },
                    "category": {
                        "type": "string",
                        "enum": ["decision", "pattern", "convention", "note", "entity", "architecture", "task"],
                        "description": "事实类别"
                    }
                },
                "required": ["content", "category"],
            }),
        },
    }
}

fn recall_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".to_string(),
        function: ToolFunctionSpec {
            name: "recall".to_string(),
            description: "检索项目级记忆（语义 + 关键词混合，返回带相关度与来源的事实列表）"
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "检索意图描述" },
                    "category": { "type": "string", "description": "按类别过滤（可选）" }
                },
                "required": ["query"],
            }),
        },
    }
}

fn search_memory_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".to_string(),
        function: ToolFunctionSpec {
            name: "search_memory".to_string(),
            description: "精细检索项目记忆：支持类别过滤、返回条数与最低相关度阈值".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "检索意图描述" },
                    "category": { "type": "string", "description": "按类别过滤（可选）" },
                    "k": { "type": "integer", "description": "返回条数（默认 10，上限 50）" },
                    "min_score": { "type": "number", "description": "最低相关度（0-1，可选）" }
                },
                "required": ["query"],
            }),
        },
    }
}

fn update_memory_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".to_string(),
        function: ToolFunctionSpec {
            name: "update_memory".to_string(),
            description: "按 id 订正一条项目记忆事实（内容/类别）".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "事实 id（从 recall/search_memory 结果获取）" },
                    "content": { "type": "string", "description": "新内容（可选）" },
                    "category": { "type": "string", "description": "新类别（可选）" }
                },
                "required": ["id"],
            }),
        },
    }
}

fn forget_memory_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".to_string(),
        function: ToolFunctionSpec {
            name: "forget_memory".to_string(),
            description: "归档（软删除）一条项目记忆事实，不再参与检索".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "事实 id" },
                },
                "required": ["id"],
            }),
        },
    }
}

fn search_code_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".to_string(),
        function: ToolFunctionSpec {
            name: "search_code".to_string(),
            description: "检索项目代码（语义 + 关键词），返回相关文件路径与行号；需先索引代码库（memory_code_index）".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "检索意图，如某个函数/配置/实现的位置" },
                    "k": { "type": "integer", "description": "返回条数（默认 10）" }
                },
                "required": ["query"],
            }),
        },
    }
}

/// 只读记忆工具（供子智能体使用：recall + search_code）。
pub fn get_readonly_memory_specs() -> Vec<ToolSpec> {
    vec![recall_spec(), search_code_spec()]
}

/// 解析项目记忆是否启用（每项目覆盖优先，其次全局默认）。
pub fn memory_effective(app: &AppHandle, project_root: &str) -> bool {
    let cfg = crate::commands::config::load_app_config(app.clone()).unwrap_or_default();
    let override_enabled = app
        .state::<MemoryStoreManager>()
        .open(project_root)
        .ok()
        .and_then(|s| s.project_enabled().ok())
        .flatten();
    override_enabled.unwrap_or(cfg.memory_enabled)
}

/// 执行记忆工具（未启用时 remember/recall 回退旧 knowledge 实现）。
pub async fn execute_memory_tool(
    app: &AppHandle,
    tool_name: &str,
    arguments: &Value,
    project_root: &str,
) -> Result<ToolResult, String> {
    if !memory_effective(app, project_root) {
        let cfg = crate::commands::config::load_app_config(app.clone()).unwrap_or_default();
        if cfg.knowledge_enabled {
            match tool_name {
                "remember" => {
                    return crate::utils::knowledge::execute_remember(project_root, arguments)
                }
                "recall" => {
                    return crate::utils::knowledge::execute_recall(project_root, arguments)
                }
                _ => {}
            }
        }
        return Err("项目记忆未启用（请在项目设置中开启）".into());
    }
    let store = app
        .state::<MemoryStoreManager>()
        .open(project_root)
        .map_err(|e| format!("打开项目记忆库失败: {e}"))?;
    match tool_name {
        "remember" => execute_remember(app, &store, arguments).await,
        "recall" => execute_recall(app, &store, arguments).await,
        "search_memory" => execute_search(app, &store, arguments).await,
        "update_memory" => execute_update(app, &store, arguments).await,
        "forget_memory" => execute_forget(&store, arguments),
        "search_code" => execute_search_code(app, project_root, arguments),
        _ => Err(format!("未知记忆工具: {tool_name}")),
    }
}

fn execute_search_code(
    app: &AppHandle,
    project_root: &str,
    args: &Value,
) -> Result<ToolResult, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "缺少参数 query".to_string())?;
    let k = args
        .get("k")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(10);
    let hits = crate::services::memory::code::search_code(app, project_root, query, k)?;
    if hits.is_empty() {
        return Ok(tool_ok(
            "未找到相关代码。可先索引代码库（记忆面板 -> 索引代码库，或 memory_code_index 命令）"
                .to_string(),
        ));
    }
    let mut out = format!("找到 {} 个相关代码块:\n", hits.len());
    for (i, h) in hits.iter().enumerate() {
        out.push_str(&format!(
            "{}. {}:{}-{}  {}\n\n",
            i + 1,
            h.file_path,
            h.start_line,
            h.end_line,
            h.content,
        ));
    }
    Ok(tool_ok(out))
}

/// 嵌入单条文本；失败返回 None（降级为关键词）。
async fn embed_one(app: &AppHandle, text: &str) -> Option<(String, usize, Vec<f32>)> {
    let cfg = crate::commands::config::load_app_config(app.clone()).unwrap_or_default();
    let mut cfg = cfg.memory_embedding;
    if cfg.provider == "openai_compat" && cfg.api_key.is_empty() {
        if let Ok(Some(key)) = crate::core::secure_store::get(app, "embedding_api_key") {
            cfg.api_key = key;
        }
    }
    let embedder = crate::plugins::embed::resolve(&cfg).ok()?;
    let v = embedder
        .embed(&[text.to_string()])
        .await
        .ok()?
        .into_iter()
        .next()?;
    Some((embedder.model_key(), v.len(), v))
}

async fn execute_remember(
    app: &AppHandle,
    store: &Arc<MemoryStore>,
    args: &Value,
) -> Result<ToolResult, String> {
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "缺少参数 content".to_string())?;
    let category = args
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("note");
    let key = args
        .get("key")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let embedding = embed_one(app, content).await;

    // P2 仲裁：向量近邻高分（>=0.92）时由 LLM 判定 merge / update / supersede / keep_separate
    let mut resolved_id: Option<String> = None;
    if let Some((_, _, vec)) = &embedding {
        if let Ok(dups) = store.search_hybrid(
            content,
            Some(vec),
            &SearchOptions {
                category: None,
                k: 1,
                min_score: Some(0.92),
            },
        ) {
            if let Some(top) = dups.first() {
                if let Ok(existing) = store.get_fact(&top.id) {
                    let same_key =
                        key.as_deref().is_some() && existing.key.as_deref() == key.as_deref();
                    if !same_key {
                        let (action, merged) = match crate::services::memory::llm::llm_config(app) {
                            Ok(cfg) => {
                                crate::services::memory::judge::judge(
                                    &cfg,
                                    &existing.content,
                                    content,
                                )
                                .await
                            }
                            Err(_) => (JudgeAction::KeepSeparate, String::new()),
                        };
                        match action {
                            JudgeAction::KeepSeparate => {}
                            JudgeAction::Update => {
                                store.update_content(&existing.id, content, "update")?;
                                if let Some(emb) = &embedding {
                                    store.set_embedding(&existing.id, Some(emb.clone()))?;
                                }
                                resolved_id = Some(existing.id);
                            }
                            JudgeAction::Merge => {
                                let merged_content = if merged.trim().is_empty() {
                                    format!("{}；{}", existing.content, content)
                                } else {
                                    merged
                                };
                                store.update_content(&existing.id, &merged_content, "merge")?;
                                // 合并后的内容重新向量化（失败不阻塞）
                                if let Some((model, dim, vec)) =
                                    embed_one(app, &merged_content).await
                                {
                                    let _ =
                                        store.set_embedding(&existing.id, Some((model, dim, vec)));
                                }
                                resolved_id = Some(existing.id);
                            }
                            JudgeAction::Supersede => {
                                store.supersede_fact(&existing.id, "被新事实取代")?;
                            }
                        }
                    }
                }
            }
        }
    }
    let saved = match resolved_id {
        Some(id) => store.get_fact(&id)?,
        None => {
            let fact = NewFact {
                key,
                content: content.to_string(),
                category: category.to_string(),
                importance: 0.5,
                source_type: Some("agent".into()),
                source_refs: None,
                pinned: false,
            };
            store.upsert_fact(&fact, embedding)?
        }
    };
    Ok(tool_ok(format!(
        "已记住: {}（{}）",
        saved.content, saved.category
    )))
}

async fn execute_recall(
    app: &AppHandle,
    store: &Arc<MemoryStore>,
    args: &Value,
) -> Result<ToolResult, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "缺少参数 query".to_string())?;
    let category = args
        .get("category")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let results = run_search(app, store, query, category, 10, None).await?;
    if results.is_empty() {
        return Ok(tool_ok(format!("未找到与 '{query}' 相关的项目记忆")));
    }
    let mut out = format!("找到 {} 条相关项目记忆:\n\n", results.len());
    for (i, r) in results.iter().enumerate() {
        out.push_str(&format!(
            "{}. [{}] {}（相关度 {:.2}）\n",
            i + 1,
            r.category,
            r.content,
            r.score,
        ));
    }
    Ok(tool_ok(out))
}

async fn execute_search(
    app: &AppHandle,
    store: &Arc<MemoryStore>,
    args: &Value,
) -> Result<ToolResult, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "缺少参数 query".to_string())?;
    let category = args
        .get("category")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let k = args
        .get("k")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(10);
    let min_score = args
        .get("min_score")
        .and_then(|v| v.as_f64())
        .map(|v| v as f32);
    let results = run_search(app, store, query, category, k, min_score).await?;
    if results.is_empty() {
        return Ok(tool_ok(format!("未找到与 '{query}' 相关的项目记忆")));
    }
    let mut out = format!("找到 {} 条相关项目记忆:\n\n", results.len());
    for (i, r) in results.iter().enumerate() {
        out.push_str(&format!(
            "{}. [{}] {}（相关度 {:.2}，id={}）\n",
            i + 1,
            r.category,
            r.content,
            r.score,
            r.id,
        ));
    }
    Ok(tool_ok(out))
}

async fn run_search(
    app: &AppHandle,
    store: &Arc<MemoryStore>,
    query: &str,
    category: Option<String>,
    k: usize,
    min_score: Option<f32>,
) -> Result<Vec<crate::services::memory::store::ScoredFact>, String> {
    let cfg = crate::commands::config::load_app_config(app.clone()).unwrap_or_default();
    let mut ecfg = cfg.memory_embedding;
    if ecfg.provider == "openai_compat" && ecfg.api_key.is_empty() {
        if let Ok(Some(key)) = crate::core::secure_store::get(app, "embedding_api_key") {
            ecfg.api_key = key;
        }
    }
    let query_vec = match crate::plugins::embed::resolve(&ecfg) {
        Ok(e) => e
            .embed(&[query.to_string()])
            .await
            .ok()
            .and_then(|v| v.into_iter().next()),
        Err(_) => None,
    };
    let mut opts = SearchOptions {
        category,
        k,
        min_score,
    };
    if opts.category.as_deref() == Some("") {
        opts.category = None;
    }
    let results = store.search_hybrid(query, query_vec.as_deref(), &opts)?;
    let ids: Vec<String> = results.iter().map(|f| f.id.clone()).collect();
    store.bump_access(&ids);
    Ok(results)
}

async fn execute_update(
    app: &AppHandle,
    store: &Arc<MemoryStore>,
    args: &Value,
) -> Result<ToolResult, String> {
    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "缺少参数 id".to_string())?;
    let existing = store.get_fact(id)?;
    let new_content = args
        .get("content")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or(existing.content.clone());
    let new_category = args
        .get("category")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or(existing.category);
    let fact = NewFact {
        key: existing.key.clone(),
        content: new_content.clone(),
        category: new_category,
        importance: existing.importance,
        source_type: existing.source_type.clone(),
        source_refs: existing.source_refs.clone(),
        pinned: existing.pinned,
    };
    let embedding = embed_one(app, &new_content).await;
    let saved = store.upsert_fact(&fact, embedding)?;
    Ok(tool_ok(format!("已更新记忆: {}", saved.id)))
}

fn execute_forget(store: &Arc<MemoryStore>, args: &Value) -> Result<ToolResult, String> {
    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "缺少参数 id".to_string())?;
    store.set_status(id, crate::services::memory::store::STATUS_ARCHIVED, None)?;
    Ok(tool_ok(format!("已归档记忆: {id}")))
}

//! Memory tools (remember / recall / search_memory / update_memory / forget_memory / search_code):
//! model-visible tool specs and their execution implementations.
//!
//! When project memory is disabled, tool execution returns an error hint (the legacy knowledge.json fallback was removed along with RAG).

use crate::core::models::{ToolFunctionSpec, ToolResult, ToolSpec};
use crate::ierr;
use crate::services::memory::judge::JudgeAction;
use crate::services::memory::manager::MemoryStoreManager;
use crate::services::memory::store::{MemoryStore, NewFact, SearchOptions};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, Manager};

/// Memory tool name list - the single source of truth for both tool specs and agent dispatch.
/// New memory tools must be registered here too, otherwise the model gets a spec that dispatch
/// cannot route (guarded by the memory_dispatch_parity test).
pub const MEMORY_TOOL_NAMES: &[&str] = &[
    "remember",
    "recall",
    "search_memory",
    "update_memory",
    "forget_memory",
    "search_code",
];

/// Return the memory tool ToolSpecs (count constrained by MEMORY_TOOL_NAMES; consistency guarded by the memory_dispatch_parity test).
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

// tool_ok is provided by utils::tool_result (single source of truth repo-wide)
use crate::utils::tool_result::tool_ok;

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
                    "category": { "type": "string", "description": "按类别过滤（可选）" },
                    "expand": { "type": "boolean", "description": "迭代检索：基于首轮结果再检索一轮后 RRF 融合（可选，默认 false；查询表述模糊时建议开启）" }
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
                    "min_score": { "type": "number", "description": "最低相关度（0-1，可选）" },
                    "expand": { "type": "boolean", "description": "迭代检索：基于首轮结果再检索一轮后 RRF 融合（可选，默认 false）" }
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
            description: "检索项目代码（语义 + 关键词），返回相关文件路径与行号；需先索引代码库（memory_code_index）。可用 path/symbol 限定范围".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "检索意图，如某个函数/配置/实现的位置" },
                    "k": { "type": "integer", "description": "返回条数（默认 10）" },
                    "path": { "type": "string", "description": "限定文件/目录路径子串（可选）" },
                    "symbol": { "type": "string", "description": "限定符号名（函数/类名，可选；需 LSP 运行并重建代码索引）" }
                },
                "required": ["query"],
            }),
        },
    }
}

/// Read-only memory tools (for subagents: recall + search_code).
pub fn get_readonly_memory_specs() -> Vec<ToolSpec> {
    vec![recall_spec(), search_code_spec()]
}

/// Resolve whether project memory is enabled (per-project override first, then the global default).
pub fn memory_effective(app: &AppHandle, project_root: &str) -> bool {
    let cfg = crate::core::config::load_app_config(app).unwrap_or_default();
    let override_enabled = app
        .state::<MemoryStoreManager>()
        .open(project_root)
        .ok()
        .and_then(|s| s.project_enabled().ok())
        .flatten();
    override_enabled.unwrap_or(cfg.memory_enabled)
}

/// Execute a memory tool (returns an error hint when project memory is disabled).
pub async fn execute_memory_tool(
    app: &AppHandle,
    tool_name: &str,
    arguments: &Value,
    project_root: &str,
) -> Result<ToolResult, String> {
    if !memory_effective(app, project_root) {
        return Err(ierr!(
            "项目记忆未启用（请在项目设置中开启）",
            "Project memory is not enabled (enable it in project settings)"
        ));
    }
    let store = app
        .state::<MemoryStoreManager>()
        .open(project_root)
        .map_err(|e| {
            ierr!(
                "打开项目记忆库失败: {}",
                "Failed to open project memory store: {}",
                e
            )
        })?;
    match tool_name {
        "remember" => execute_remember(app, &store, arguments).await,
        "recall" => execute_recall(app, &store, arguments).await,
        "search_memory" => execute_search(app, &store, arguments).await,
        "update_memory" => execute_update(app, &store, arguments).await,
        "forget_memory" => execute_forget(&store, arguments),
        "search_code" => execute_search_code(app, project_root, arguments).await,
        _ => Err(ierr!(
            "未知记忆工具: {}",
            "Unknown memory tool: {}",
            tool_name
        )),
    }
}

async fn execute_search_code(
    app: &AppHandle,
    project_root: &str,
    args: &Value,
) -> Result<ToolResult, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ierr!("缺少参数 query", "Missing parameter: query"))?;
    let k = args
        .get("k")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(10);
    let path_filter = args.get("path").and_then(|v| v.as_str());
    let symbol_filter = args.get("symbol").and_then(|v| v.as_str());
    let hits = crate::services::memory::code::search_code(
        app,
        project_root,
        query,
        k,
        path_filter,
        symbol_filter,
    )
    .await?;
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

/// Embed a single text (per the project-level embedding config; None on failure degrades to keyword search).
/// Returns None with a warning when the dimension mismatches existing store vectors (avoids writing/searching mismatched-dimension vectors).
async fn embed_one(
    app: &AppHandle,
    store: &MemoryStore,
    text: &str,
) -> Option<(String, usize, Vec<f32>)> {
    let cfg = crate::services::memory::resolve_project_embedding(app, store);
    let embedder = crate::plugins::embed::resolve(app, &cfg).ok()?;
    let v = embedder
        .embed(&[text.to_string()])
        .await
        .ok()?
        .into_iter()
        .next()?;
    if let Some(lib_dim) = store.embedding_dim_meta() {
        if lib_dim != v.len() {
            tracing::warn!(
                "记忆库向量维度 {} 与当前嵌入模型维度 {} 不一致，跳过向量化（请检查嵌入模型配置）",
                lib_dim,
                v.len()
            );
            return None;
        }
    }
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
        .ok_or_else(|| ierr!("缺少参数 content", "Missing parameter: content"))?;
    let category = args
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("note");
    let key = args
        .get("key")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let embedding = embed_one(app, store, content).await;

    // Dedup/arbitration: at cosine-NN >= threshold the LLM decides merge/update/supersede/keep_separate.
    // Note the threshold compares **cosine similarity** (see store::search_similar), not the RRF fusion score.
    let dedup_threshold = crate::core::config::load_app_config(app)
        .map(|c| c.memory_dedup_threshold)
        .unwrap_or(0.92)
        .clamp(0.0, 1.0);
    let mut resolved_id: Option<String> = None;
    if let Some((_, _, vec)) = &embedding {
        if let Ok(dups) = store.search_similar(vec, 3) {
            for (dup_id, cosine) in dups {
                if cosine < dedup_threshold {
                    continue;
                }
                store.rolling_bump(crate::services::memory::store::STAT_DEDUP, 1);
                if let Ok(existing) = store.get_fact(&dup_id) {
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
                        crate::services::memory::judge::bump_judge_stat(store, &action);
                        match action {
                            JudgeAction::KeepSeparate => {}
                            JudgeAction::Update => {
                                store.update_content(&existing.id, content, "update")?;
                                if let Some(emb) = &embedding {
                                    store.set_embedding(&existing.id, Some(emb.clone()))?;
                                }
                                resolved_id = Some(existing.id);
                                break;
                            }
                            JudgeAction::Merge => {
                                let merged_content = if merged.trim().is_empty() {
                                    format!("{}；{}", existing.content, content)
                                } else {
                                    merged
                                };
                                store.update_content(&existing.id, &merged_content, "merge")?;
                                // Re-embed the merged content (failure does not block)
                                if let Some((model, dim, vec)) =
                                    embed_one(app, store, &merged_content).await
                                {
                                    let _ =
                                        store.set_embedding(&existing.id, Some((model, dim, vec)));
                                }
                                resolved_id = Some(existing.id);
                                break;
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
        .ok_or_else(|| ierr!("缺少参数 query", "Missing parameter: query"))?;
    let category = args
        .get("category")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let expand = args
        .get("expand")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let results = run_search(app, store, query, category, 10, None, expand).await?;
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
        .ok_or_else(|| ierr!("缺少参数 query", "Missing parameter: query"))?;
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
    let expand = args
        .get("expand")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let results = run_search(app, store, query, category, k, min_score, expand).await?;
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
    expand: bool,
) -> Result<Vec<crate::services::memory::store::ScoredFact>, String> {
    let mut opts = SearchOptions {
        category,
        k,
        min_score,
    };
    if opts.category.as_deref() == Some("") {
        opts.category = None;
    }
    // Query rewrite (optional, off by default): the LLM generates 1-2 variants -> multi-query RRF fusion + project-level rerank
    let rewrite_enabled = crate::core::config::load_app_config(app)
        .map(|c| c.memory_query_rewrite)
        .unwrap_or(false);
    let mut queries = vec![query.to_string()];
    if rewrite_enabled {
        queries.extend(crate::services::memory::retrieve::rewrite_query(app, query).await);
    }
    // Iterative retrieval (optional): first-round summaries serve as an expanded query for a second round, then both rounds fuse
    let results = if expand {
        crate::services::memory::retrieve::iterative_search(app, store, query, &opts).await?
    } else {
        crate::services::memory::retrieve::merged_search(app, store, &queries, &opts).await?
    };
    let ids: Vec<String> = results.iter().map(|f| f.id.clone()).collect();
    store.bump_access(&ids);
    // Quality probes: query count and hit count (7-day rolling window)
    store.rolling_bump(crate::services::memory::store::STAT_QUERIES, 1);
    store.rolling_bump(
        crate::services::memory::store::STAT_HITS,
        results.len() as i64,
    );
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
        .ok_or_else(|| ierr!("缺少参数 id", "Missing parameter: id"))?;
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
        .unwrap_or_else(|| existing.category.clone());
    // MEM-01: keyless 事实（自动提取的事实全部如此）必须**原地**更新——
    // upsert_fact(key: None) 必走 INSERT，每次"更新"都复制一条新事实、旧事实
    // 仍 active，工具还报告"已更新"（对齐 commands::memory::memory_update 的
    // keyless 分支语义）
    let saved = if existing.key.is_some() {
        let fact = NewFact {
            key: existing.key.clone(),
            content: new_content.clone(),
            category: new_category,
            importance: existing.importance,
            source_type: existing.source_type.clone(),
            source_refs: existing.source_refs.clone(),
            pinned: existing.pinned,
        };
        let embedding = embed_one(app, store, &new_content).await;
        store.upsert_fact(&fact, embedding)?
    } else {
        let mut updated = store.update_content(id, &new_content, "update")?;
        if new_category != existing.category {
            updated = store.set_category(id, &new_category)?;
        }
        if let Some(embedding) = embed_one(app, store, &new_content).await {
            if let Err(e) = store.set_embedding(id, Some(embedding)) {
                tracing::warn!("记忆 {id} 向量重写失败（保留旧向量）: {e}");
            }
        }
        updated
    };
    Ok(tool_ok(format!("已更新记忆: {}", saved.id)))
}

fn execute_forget(store: &Arc<MemoryStore>, args: &Value) -> Result<ToolResult, String> {
    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ierr!("缺少参数 id", "Missing parameter: id"))?;
    store.set_status(id, crate::services::memory::store::STATUS_ARCHIVED, None)?;
    Ok(tool_ok(format!("已归档记忆: {id}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// The spec list, the dispatch list (MEMORY_TOOL_NAMES), and execute_memory_tool's match arms
    /// must stay consistent: prevents handing the model a tool spec that dispatch cannot route
    /// (which would fall through to the file-tool fallback and return "unknown tool").
    #[test]
    fn memory_dispatch_parity() {
        let spec_names: HashSet<String> = get_memory_tool_specs()
            .into_iter()
            .map(|s| s.function.name)
            .collect();
        let const_names: HashSet<String> =
            MEMORY_TOOL_NAMES.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            spec_names, const_names,
            "工具规格与 MEMORY_TOOL_NAMES 不一致"
        );
        // All read-only specs must be dispatchable
        for s in get_readonly_memory_specs() {
            assert!(
                const_names.contains(&s.function.name),
                "只读工具 {} 不在派发清单中",
                s.function.name
            );
        }
    }
}

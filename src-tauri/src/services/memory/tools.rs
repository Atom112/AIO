//! 记忆工具（remember / recall / search_memory / update_memory / forget_memory / search_code）：
//! 模型可见的工具规格与执行实现。
//!
//! 项目记忆未启用时，工具执行返回错误提示（旧版 knowledge.json 回退路径已随 RAG 移除）。

use crate::core::models::{ToolFunctionSpec, ToolResult, ToolSpec};
use crate::services::memory::judge::JudgeAction;
use crate::services::memory::manager::MemoryStoreManager;
use crate::services::memory::store::{MemoryStore, NewFact, SearchOptions};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, Manager};

/// 记忆工具名清单 —— 工具规格与 Agent 派发的唯一真相来源。
/// 新增记忆工具时务必同时注册到这里，否则会出现"模型拿到规格但派发路由不到"
/// （history: search_code 曾缺失于派发清单而永久落到文件工具兜底，见 memory_dispatch_parity 测试）。
pub const MEMORY_TOOL_NAMES: &[&str] = &[
    "remember",
    "recall",
    "search_memory",
    "update_memory",
    "forget_memory",
    "search_code",
];

/// 返回记忆工具的 ToolSpec（数量由 MEMORY_TOOL_NAMES 约束，memory_dispatch_parity 测试保证一致）。
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

// tool_ok 统一由 utils::tool_result 提供（消除全仓副本）
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

/// 只读记忆工具（供子智能体使用：recall + search_code）。
pub fn get_readonly_memory_specs() -> Vec<ToolSpec> {
    vec![recall_spec(), search_code_spec()]
}

/// 解析项目记忆是否启用（每项目覆盖优先，其次全局默认）。
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

/// 执行记忆工具（项目记忆未启用时返回错误提示）。
pub async fn execute_memory_tool(
    app: &AppHandle,
    tool_name: &str,
    arguments: &Value,
    project_root: &str,
) -> Result<ToolResult, String> {
    if !memory_effective(app, project_root) {
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
        "search_code" => execute_search_code(app, project_root, arguments).await,
        _ => Err(format!("未知记忆工具: {tool_name}")),
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
        .ok_or_else(|| "缺少参数 query".to_string())?;
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

/// 嵌入单条文本（按项目级嵌入配置；失败返回 None 降级为关键词）。
/// 维度与库中已有向量不一致时返回 None 并告警（避免写入/检索错维向量）。
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
        .ok_or_else(|| "缺少参数 content".to_string())?;
    let category = args
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("note");
    let key = args
        .get("key")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let embedding = embed_one(app, store, content).await;

    // 判重/仲裁：纯向量余弦近邻 >= 阈值时由 LLM 判定 merge/update/supersede/keep_separate。
    // 注意阈值比较的是**余弦相似度**（见 store::search_similar），不是 RRF 融合分。
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
                                // 合并后的内容重新向量化（失败不阻塞）
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
        .ok_or_else(|| "缺少参数 query".to_string())?;
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
    // 查询改写（可选，默认关）：LLM 生成 1-2 个变体 → 多查询 RRF 融合 + 项目级重排
    let rewrite_enabled = crate::core::config::load_app_config(app)
        .map(|c| c.memory_query_rewrite)
        .unwrap_or(false);
    let mut queries = vec![query.to_string()];
    if rewrite_enabled {
        queries.extend(crate::services::memory::retrieve::rewrite_query(app, query).await);
    }
    // P3-1 迭代检索（可选）：首轮结果摘要作为扩展查询再检索一轮后融合
    let results = if expand {
        crate::services::memory::retrieve::iterative_search(app, store, query, &opts).await?
    } else {
        crate::services::memory::retrieve::merged_search(app, store, &queries, &opts).await?
    };
    let ids: Vec<String> = results.iter().map(|f| f.id.clone()).collect();
    store.bump_access(&ids);
    // 质量探针：检索次数与命中条数（近 7 天滚动）
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
    let embedding = embed_one(app, store, &new_content).await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// 规格清单、派发清单（MEMORY_TOOL_NAMES）与 execute_memory_tool 的 match 分支
    /// 必须三者一致：防止"给了模型工具规格但派发路由不到"（search_code 曾因此
    /// 永久落到文件工具兜底返回"未知工具"）。
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
        // 只读规格必须全部可派发
        for s in get_readonly_memory_specs() {
            assert!(
                const_names.contains(&s.function.name),
                "只读工具 {} 不在派发清单中",
                s.function.name
            );
        }
    }
}

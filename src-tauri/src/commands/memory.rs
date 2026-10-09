//! 项目记忆命令（memory_*）。
//!
//! 提供事实 CRUD、混合检索、状态与统计，供前端项目设置与记忆面板调用。

use crate::core::events as ev;
use crate::core::models::{AppConfig, MemoryEmbeddingConfig, MemoryRerankConfig};
use crate::services::memory::manager::MemoryStoreManager;
use crate::services::memory::store::{
    FactVersion, MemoryFact, MemoryStats, NewFact, ScoredFact, SearchOptions,
};
use crate::utils::file_tools;
use tauri::{AppHandle, Emitter, Manager};

use crate::ierr;

/// 嵌入器就绪状态（memory_get_status 返回）。
#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EmbedderStatus {
    pub provider: String,
    pub model: String,
    pub configured: bool,
    pub available: bool,
    pub dimensions: usize,
}

/// 项目记忆状态（memory_get_status 返回）。
#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStatus {
    pub enabled: bool,
    pub store_path: String,
    pub stats: MemoryStats,
    pub embedder: EmbedderStatus,
    pub embedding: MemoryEmbeddingConfig,
    /// 项目级重排配置（apiKey 恒为空；在线 Key 存 secure_store）
    pub rerank: MemoryRerankConfig,
}

/// 记忆列表分页结果。
#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MemoryListPage {
    pub facts: Vec<MemoryFact>,
    pub total: i64,
}

/// 加载应用配置（失败时使用默认值）。
fn load_cfg(app: &AppHandle) -> AppConfig {
    crate::core::config::load_app_config(app).unwrap_or_default()
}

/// 由 project_id 解析项目根路径。
fn resolve_root(app: &AppHandle, project_id: &str) -> Result<String, String> {
    file_tools::resolve_project_root(app, Some(project_id))
}

/// 嵌入单条文本；失败返回 None（检索/写入降级为关键词模式）。
async fn embed_one(
    app: &AppHandle,
    cfg: &MemoryEmbeddingConfig,
    text: &str,
) -> Option<(String, usize, Vec<f32>)> {
    let embedder = crate::plugins::embed::resolve(app, cfg).ok()?;
    let v = embedder
        .embed(&[text.to_string()])
        .await
        .ok()?
        .into_iter()
        .next()?;
    Some((embedder.model_key(), v.len(), v))
}

/// 计算嵌入器就绪状态（优先回显实际解析到的嵌入器，而非原始配置：
/// 配置里的 provider/model/dimensions 可能来自旧版全局配置快照，与实际使用的不一致）。
fn embedder_status(app: &AppHandle, cfg: &MemoryEmbeddingConfig) -> EmbedderStatus {
    let (provider, model, dims, configured, available) =
        match crate::plugins::embed::resolve(app, cfg) {
            Ok(e) => (
                e.id().to_string(),
                e.model_key(),
                e.dimensions(),
                true,
                e.is_available(),
            ),
            Err(_) => (
                cfg.provider.clone(),
                cfg.model.clone(),
                cfg.dimensions,
                cfg.enabled,
                false,
            ),
        };
    EmbedderStatus {
        provider,
        model,
        configured,
        available,
        dimensions: dims,
    }
}

/// 读取项目记忆状态（含统计与嵌入器就绪情况）。
#[tauri::command]
pub fn memory_get_status(app: AppHandle, project_id: String) -> Result<MemoryStatus, String> {
    let cfg = load_cfg(&app);
    let root = resolve_root(&app, &project_id)?;
    let store = app.state::<MemoryStoreManager>().open(&root)?;
    let stats = store.stats()?;
    // 惰性迁移：旧 knowledge.json 首次打开时并入事实库（幂等）
    let _ = store.migrate_from_knowledge_json();
    let project_override = store.project_enabled()?;
    // 显式迁移步骤（命名、可审计）：嵌入/重排配置的项目级快照与在线归一化。
    // 随后的 resolve_* 均为纯读取（resolve_project_rerank 内部归一化 provider/model）。
    crate::services::memory::migrate_project_embedding(&app, &store);
    crate::services::memory::retrieve::migrate_project_rerank(&app, &store);
    let embedding = crate::services::memory::resolve_project_embedding(&app, &store);
    let rerank = crate::services::memory::retrieve::resolve_project_rerank(&app, &store);
    Ok(MemoryStatus {
        enabled: project_override.unwrap_or(cfg.memory_enabled),
        store_path: format!("{root}/.aio/memory/memory.sqlite"),
        stats,
        embedder: embedder_status(&app, &embedding),
        embedding,
        rerank,
    })
}

/// 混合检索：向量（可用时）+ 关键词（FTS5）融合。
#[tauri::command]
pub async fn memory_search(
    app: AppHandle,
    project_id: String,
    query: String,
    category: Option<String>,
    k: Option<usize>,
    expand: Option<bool>,
) -> Result<Vec<ScoredFact>, String> {
    let root = resolve_root(&app, &project_id)?;
    let store = app.state::<MemoryStoreManager>().open(&root)?;
    let mut opts = SearchOptions {
        category,
        k: k.unwrap_or(10),
        min_score: None,
    };
    if opts.category.as_deref() == Some("") {
        opts.category = None;
    }
    // 查询改写（可选）+ 多查询 RRF 融合 + 项目级重排 + 图谱加分；expand=true 走迭代检索（P3-1）
    let rewrite_enabled = load_cfg(&app).memory_query_rewrite;
    let mut queries = vec![query.clone()];
    if rewrite_enabled {
        queries.extend(crate::services::memory::retrieve::rewrite_query(&app, &query).await);
    }
    let results = if expand.unwrap_or(false) {
        crate::services::memory::retrieve::iterative_search(&app, &store, &query, &opts).await?
    } else {
        crate::services::memory::retrieve::merged_search(&app, &store, &queries, &opts).await?
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

/// 新增一条事实（key 存在则更新；写入前尝试向量化）。
#[tauri::command]
pub async fn memory_add(
    app: AppHandle,
    project_id: String,
    key: Option<String>,
    content: String,
    category: Option<String>,
    importance: Option<f32>,
) -> Result<MemoryFact, String> {
    let root = resolve_root(&app, &project_id)?;
    let store = app.state::<MemoryStoreManager>().open(&root)?;
    let embedding_cfg = crate::services::memory::resolve_project_embedding(&app, &store);
    let fact = NewFact {
        key,
        content: content.clone(),
        category: category.unwrap_or_else(|| "note".to_string()),
        importance: importance.unwrap_or(0.5),
        source_type: Some("user".into()),
        source_refs: None,
        pinned: false,
    };
    let embedding = embed_one(&app, &embedding_cfg, &fact.content).await;
    store.upsert_fact(&fact, embedding)
}

/// 按 id 订正事实（内容/类别；重新向量化）。
#[tauri::command]
pub async fn memory_update(
    app: AppHandle,
    project_id: String,
    id: String,
    content: Option<String>,
    category: Option<String>,
) -> Result<MemoryFact, String> {
    let root = resolve_root(&app, &project_id)?;
    let store = app.state::<MemoryStoreManager>().open(&root)?;
    let embedding_cfg = crate::services::memory::resolve_project_embedding(&app, &store);
    let existing = store.get_fact(&id)?;
    let new_content = content.unwrap_or(existing.content.clone());
    // key 存在时走 upsert（同 key 原地更新 + 版本审计）；无 key 时直接订正内容
    let saved = if existing.key.is_some() {
        let fact = NewFact {
            key: existing.key.clone(),
            content: new_content.clone(),
            category: category.unwrap_or(existing.category),
            importance: existing.importance,
            source_type: existing.source_type.clone(),
            source_refs: existing.source_refs.clone(),
            pinned: existing.pinned,
        };
        let embedding = embed_one(&app, &embedding_cfg, &new_content).await;
        store.upsert_fact(&fact, embedding)?
    } else {
        // 无 key 事实：内容就地订正；向量原地重写（set_embedding）。
        // 禁止用 upsert_fact(key: None) 重写向量 —— key 为 None 时 upsert 会跳过
        // 查重直接 INSERT，导致每次编辑都插入一条重复事实。
        let mut updated = store.update_content(&id, &new_content, "update")?;
        if let Some(cat) = &category {
            // keyless 分支此前会静默丢弃 category 参数；现经由 set_category 生效
            updated = store.set_category(&id, cat)?;
        }
        if let Some(embedding) = embed_one(&app, &embedding_cfg, &new_content).await {
            if let Err(e) = store.set_embedding(&id, Some(embedding)) {
                tracing::warn!("记忆 {id} 向量重写失败（保留旧向量）: {e}");
            }
        }
        updated
    };
    Ok(saved)
}

/// 删除一条事实。
#[tauri::command]
pub fn memory_delete(app: AppHandle, project_id: String, id: String) -> Result<(), String> {
    let root = resolve_root(&app, &project_id)?;
    app.state::<MemoryStoreManager>()
        .open(&root)?
        .delete_fact(&id)
}

/// 按 id 读取单条事实。
#[tauri::command]
pub fn memory_get(app: AppHandle, project_id: String, id: String) -> Result<MemoryFact, String> {
    let root = resolve_root(&app, &project_id)?;
    app.state::<MemoryStoreManager>().open(&root)?.get_fact(&id)
}

/// 设置本项目记忆开关（写入项目 memory.sqlite 的 meta 覆盖；跟随全局默认请用 memory_reset_enabled）。
#[tauri::command]
pub fn memory_set_enabled(app: AppHandle, project_id: String, enabled: bool) -> Result<(), String> {
    let root = resolve_root(&app, &project_id)?;
    app.state::<MemoryStoreManager>()
        .open(&root)?
        .set_project_enabled(Some(enabled))
}

/// 清除项目记忆开关覆盖（跟随全局默认）。
#[tauri::command]
pub fn memory_reset_enabled(app: AppHandle, project_id: String) -> Result<(), String> {
    let root = resolve_root(&app, &project_id)?;
    app.state::<MemoryStoreManager>()
        .open(&root)?
        .set_project_enabled(None)
}

/// 读取事实的版本审计链。
#[tauri::command]
pub fn memory_get_versions(
    app: AppHandle,
    project_id: String,
    id: String,
) -> Result<Vec<FactVersion>, String> {
    let root = resolve_root(&app, &project_id)?;
    app.state::<MemoryStoreManager>()
        .open(&root)?
        .get_versions(&id)
}

/// 手工合并两条事实（target 保留，source 标记 superseded）。
#[tauri::command]
pub fn memory_merge_facts(
    app: AppHandle,
    project_id: String,
    target_id: String,
    source_id: String,
    merged_content: String,
) -> Result<MemoryFact, String> {
    let root = resolve_root(&app, &project_id)?;
    app.state::<MemoryStoreManager>().open(&root)?.merge_facts(
        &target_id,
        &source_id,
        &merged_content,
    )
}

/// 归档（软删除）一条事实。
#[tauri::command]
pub fn memory_archive(
    app: AppHandle,
    project_id: String,
    id: String,
) -> Result<MemoryFact, String> {
    let root = resolve_root(&app, &project_id)?;
    app.state::<MemoryStoreManager>().open(&root)?.set_status(
        &id,
        crate::services::memory::store::STATUS_ARCHIVED,
        None,
    )
}

/// 钉住 / 取消钉住一条事实。
#[tauri::command]
pub fn memory_set_pinned(
    app: AppHandle,
    project_id: String,
    id: String,
    pinned: bool,
) -> Result<MemoryFact, String> {
    let root = resolve_root(&app, &project_id)?;
    app.state::<MemoryStoreManager>()
        .open(&root)?
        .set_pinned(&id, pinned)
}

/// 容量治理：超出上限时归档最低分非 pinned 事实，返回归档数。
#[tauri::command]
pub fn memory_prune(app: AppHandle, project_id: String) -> Result<usize, String> {
    let cfg = load_cfg(&app);
    let root = resolve_root(&app, &project_id)?;
    app.state::<MemoryStoreManager>()
        .open(&root)?
        .prune_to_capacity(cfg.memory_max_facts.max(100))
}

/// 导出全部记忆事实为 JSON 文件（不含向量），写入用户指定的文件系统路径。
/// `path`：目标文件绝对路径（前端经 dialog save 选取）。
#[tauri::command]
pub fn memory_export_file(app: AppHandle, project_id: String, path: String) -> Result<(), String> {
    let root = resolve_root(&app, &project_id)?;
    let json = app
        .state::<MemoryStoreManager>()
        .open(&root)?
        .export_facts()?;
    std::fs::write(&path, json)
        .map_err(|e| ierr!("写入导出文件失败: {}", "Failed to write export file: {}", e))
}

/// 从用户指定路径的 JSON 文件导入记忆事实（按 key 去重；向量需重建）。
/// `path`：源文件绝对路径（前端经 dialog open 选取）。
#[tauri::command]
pub fn memory_import_file(
    app: AppHandle,
    project_id: String,
    path: String,
) -> Result<usize, String> {
    let root = resolve_root(&app, &project_id)?;
    let json = std::fs::read_to_string(&path)
        .map_err(|e| ierr!("读取导入文件失败: {}", "Failed to read import file: {}", e))?;
    app.state::<MemoryStoreManager>()
        .open(&root)?
        .import_facts(&json)
}

/// 索引项目代码（P4 代码级 RAG；增量跳过未变化文件）。
#[tauri::command]
pub async fn memory_code_index(
    window: tauri::Window,
    app: AppHandle,
    project_id: String,
    force: Option<bool>,
) -> Result<crate::services::memory::code::CodeIndexResult, String> {
    let root = resolve_root(&app, &project_id)?;
    crate::services::memory::code::index_code(
        &app,
        &root,
        force.unwrap_or(false),
        move |done, total| {
            let _ = window.emit(
                ev::CODE_INDEX_PROGRESS,
                serde_json::json!({ "done": done, "total": total }),
            );
        },
    )
    .await
}

/// 代码语义检索（search_code 工具的后端；path/symbol 过滤可选）。
#[tauri::command]
pub async fn memory_code_search(
    app: AppHandle,
    project_id: String,
    query: String,
    k: Option<usize>,
    path: Option<String>,
    symbol: Option<String>,
) -> Result<Vec<crate::services::memory::code::CodeChunkHit>, String> {
    let root = resolve_root(&app, &project_id)?;
    crate::services::memory::code::search_code(
        &app,
        &root,
        &query,
        k.unwrap_or(10),
        path.as_deref(),
        symbol.as_deref(),
    )
    .await
}

/// 代码索引状态。
#[tauri::command]
pub fn memory_code_status(
    app: AppHandle,
    project_id: String,
) -> Result<crate::services::memory::code::CodeIndexStatus, String> {
    let root = resolve_root(&app, &project_id)?;
    crate::services::memory::code::code_status(&app, &root)
}

/// 清空项目记忆。
#[tauri::command]
pub fn memory_clear(app: AppHandle, project_id: String) -> Result<(), String> {
    let root = resolve_root(&app, &project_id)?;
    app.state::<MemoryStoreManager>().open(&root)?.clear()
}

/// 记忆统计。
#[tauri::command]
pub fn memory_stats(app: AppHandle, project_id: String) -> Result<MemoryStats, String> {
    let root = resolve_root(&app, &project_id)?;
    app.state::<MemoryStoreManager>().open(&root)?.stats()
}

/// 列出事实（可选 status/category 过滤，分页）。
#[tauri::command]
pub fn memory_list(
    app: AppHandle,
    project_id: String,
    status: Option<String>,
    category: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<MemoryListPage, String> {
    let root = resolve_root(&app, &project_id)?;
    let store = app.state::<MemoryStoreManager>().open(&root)?;
    let (facts, total) = store.list_facts(
        status.as_deref(),
        category.as_deref(),
        limit.unwrap_or(100),
        offset.unwrap_or(0),
    )?;
    Ok(MemoryListPage { facts, total })
}

/// 重建/补建向量索引（为无 embedding 的 active 事实生成向量；经 reindex-progress 事件回报进度）。
#[tauri::command]
pub async fn memory_reindex(
    window: tauri::Window,
    app: AppHandle,
    project_id: String,
) -> Result<(usize, usize), String> {
    let root = resolve_root(&app, &project_id)?;
    let store = app.state::<MemoryStoreManager>().open(&root)?;
    let cfg = crate::services::memory::resolve_project_embedding(&app, &store);
    let embedder = crate::plugins::embed::resolve(&app, &cfg)?;
    store
        .reindex(
            |content| {
                let embedder = embedder.clone();
                async move {
                    match embedder.embed(&[content]).await {
                        Ok(v) => Ok(v
                            .into_iter()
                            .next()
                            .map(|vec| (embedder.model_key(), vec.len(), vec))),
                        Err(e) => Err(e),
                    }
                }
            },
            |done, total| {
                let _ = window.emit(
                    ev::REINDEX_PROGRESS,
                    serde_json::json!({ "done": done, "total": total }),
                );
            },
        )
        .await
}

/// 切换项目嵌入模型：校验 → 用所选模型探测实际维度 → 固化到项目 meta →
/// 清空该项目全部向量（事实文本保留，含代码向量）→ 自动重建索引（reindex-progress 事件回报）。
#[tauri::command]
pub async fn memory_update_embedding(
    window: tauri::Window,
    app: AppHandle,
    project_id: String,
    config: MemoryEmbeddingConfig,
) -> Result<MemoryStatus, String> {
    // 仅支持本地嵌入模型；旧配置中的 online/openai_compat 归一化为 local
    let mut config = config;
    config.provider = "local".into();
    config.api_url.clear();
    if config.model.trim().is_empty() {
        return Err(ierr!("请选择嵌入模型", "Please select an embedding model"));
    }
    if crate::commands::model_download::find_entry(config.model.trim()).is_none() {
        return Err(ierr!(
            "未知嵌入模型: {}",
            "Unknown embedding model: {}",
            config.model
        ));
    }
    let root = resolve_root(&app, &project_id)?;
    let store = app.state::<MemoryStoreManager>().open(&root)?;
    // 显式迁移（幂等）：先补齐项目级 meta，再做"是否变化"的判定，避免误判为首次切换
    crate::services::memory::migrate_project_embedding(&app, &store);
    let (current, _) = store.embedding_cfg();
    if current.provider == config.provider
        && current.model == config.model
        && current.dimensions == config.dimensions
    {
        // 配置未变化：直接返回当前状态，不做任何破坏性操作
        return memory_get_status(app, project_id);
    }
    // 1) 用所选模型探测实际维度与模型标识（比配置里手填的 dimensions/model 更可靠）
    let (model_key, dims, _) = embed_one(&app, &config, "embedding probe")
        .await
        .ok_or_else(|| {
            ierr!(
                "无法使用所选嵌入模型（请先下载并启用该模型）",
                "Unable to use the selected embedding model (download and enable it first)"
            )
        })?;
    // 2) 固化到项目 meta（model/维度按实测值；apiKey 不落盘）
    let mut resolved = config.clone();
    resolved.model = model_key;
    resolved.dimensions = dims;
    resolved.api_key.clear();
    store.set_embedding_cfg(&resolved)?;
    // 3) 清空全部向量（事实与代码块文本保留，仅弃用向量）
    store.reset_vectors()?;
    // 4) 自动重建事实索引（按新模型重新嵌入）
    let embedder = crate::plugins::embed::resolve(&app, &resolved)?;
    let (done, failed) = store
        .reindex(
            |content| {
                let embedder = embedder.clone();
                async move {
                    match embedder.embed(&[content]).await {
                        Ok(v) => Ok(v
                            .into_iter()
                            .next()
                            .map(|vec| (embedder.model_key(), vec.len(), vec))),
                        Err(e) => Err(e),
                    }
                }
            },
            |done, total| {
                let _ = window.emit(
                    ev::REINDEX_PROGRESS,
                    serde_json::json!({ "done": done, "total": total }),
                );
            },
        )
        .await?;
    let _ = (done, failed);
    memory_get_status(app, project_id)
}

/// 抽取事实间关系（P3-3 轻量知识图谱）：对按加权分排序的前 limit 条 active 事实，
/// 逐条调用 LLM 抽取实体-关系三元组并写 facts_relations（先删后插，幂等）。
#[tauri::command]
pub async fn memory_graph_extract(
    app: AppHandle,
    project_id: String,
    limit: Option<u32>,
) -> Result<crate::services::memory::graph::GraphExtractReport, String> {
    let root = resolve_root(&app, &project_id)?;
    let store = app.state::<MemoryStoreManager>().open(&root)?;
    crate::services::memory::graph::extract_relations(&app, &store, limit.unwrap_or(50)).await
}

/// 设置项目级重排配置（仅本地模型；rerank 不涉及向量，不触发重建）。
#[tauri::command]
pub fn memory_update_rerank(
    app: AppHandle,
    project_id: String,
    config: MemoryRerankConfig,
) -> Result<MemoryStatus, String> {
    let mut config = config;
    // 自 v0.8.1 起仅支持本地重排：旧 provider（online/openai_compat）归一化为 local
    config.provider = "local".into();
    let model = config.model.trim().to_string();
    if model.is_empty() {
        return Err(ierr!("请选择重排模型", "Please select a rerank model"));
    }
    match crate::commands::model_download::find_entry(&model) {
        Some(entry) if entry.kind == "rerank" => {}
        _ => return Err(ierr!("未知重排模型: {}", "Unknown rerank model: {}", model)),
    }
    config.model = model;
    let root = resolve_root(&app, &project_id)?;
    let store = app.state::<MemoryStoreManager>().open(&root)?;
    store.set_rerank_cfg(&config)?;
    memory_get_status(app, project_id)
}

/// 手动触发一次合并整理（sleep-time consolidation；预算默认取全局配置，可显式覆盖）。
/// 与每日后台调度共用 per-project 锁与预算口径。
#[tauri::command]
pub async fn memory_consolidate(
    app: AppHandle,
    project_id: String,
    budget: Option<u32>,
) -> Result<crate::services::memory::consolidate::ConsolidationReport, String> {
    let root = resolve_root(&app, &project_id)?;
    let store = app.state::<MemoryStoreManager>().open(&root)?;
    let budget = budget
        .map(|b| b.max(1) as usize)
        .unwrap_or_else(|| load_cfg(&app).memory_consolidate_budget.max(1) as usize);
    crate::services::memory::consolidate::consolidate(&app, &store, budget).await
}

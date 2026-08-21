//! 项目记忆命令（memory_*）。
//!
//! 提供事实 CRUD、混合检索、状态与统计，供前端项目设置与记忆面板调用。

use crate::core::models::{AppConfig, MemoryEmbeddingConfig};
use crate::services::memory::manager::MemoryStoreManager;
use crate::services::memory::store::{
    FactVersion, MemoryFact, MemoryStats, NewFact, ScoredFact, SearchOptions,
};
use crate::utils::file_tools;
use tauri::{AppHandle, Manager};

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
    crate::commands::config::load_app_config(app.clone()).unwrap_or_default()
}

/// 由 project_id 解析项目根路径。
fn resolve_root(app: &AppHandle, project_id: &str) -> Result<String, String> {
    file_tools::resolve_project_root(app, Some(project_id))
}

/// 为 openai_compat 从 secure_store 注入 API Key。
fn inject_key(app: &AppHandle, cfg: &mut MemoryEmbeddingConfig) {
    if cfg.provider == "openai_compat" && cfg.api_key.is_empty() {
        if let Ok(Some(key)) = crate::core::secure_store::get(app, "embedding_api_key") {
            cfg.api_key = key;
        }
    }
}

/// 嵌入单条文本；失败返回 None（检索/写入降级为关键词模式）。
async fn embed_one(
    app: &AppHandle,
    cfg: &MemoryEmbeddingConfig,
    text: &str,
) -> Option<(String, usize, Vec<f32>)> {
    let mut cfg = cfg.clone();
    inject_key(app, &mut cfg);
    let embedder = crate::plugins::embed::resolve(&cfg).ok()?;
    let v = embedder
        .embed(&[text.to_string()])
        .await
        .ok()?
        .into_iter()
        .next()?;
    Some((embedder.model_key(), v.len(), v))
}

/// 嵌入查询文本；失败返回 None。
async fn embed_query(app: &AppHandle, cfg: &MemoryEmbeddingConfig, text: &str) -> Option<Vec<f32>> {
    let mut cfg = cfg.clone();
    inject_key(app, &mut cfg);
    let embedder = crate::plugins::embed::resolve(&cfg).ok()?;
    embedder
        .embed(&[text.to_string()])
        .await
        .ok()?
        .into_iter()
        .next()
}

fn embedder_status(cfg: &MemoryEmbeddingConfig) -> EmbedderStatus {
    let (configured, available, dims) = match crate::plugins::embed::resolve(cfg) {
        Ok(e) => (true, e.is_available(), e.dimensions()),
        Err(_) => (cfg.enabled, false, cfg.dimensions),
    };
    EmbedderStatus {
        provider: cfg.provider.clone(),
        model: cfg.model.clone(),
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
    Ok(MemoryStatus {
        enabled: project_override.unwrap_or(cfg.memory_enabled),
        store_path: format!("{root}/.aio/memory/memory.sqlite"),
        stats,
        embedder: embedder_status(&cfg.memory_embedding),
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
) -> Result<Vec<ScoredFact>, String> {
    let cfg = load_cfg(&app);
    let root = resolve_root(&app, &project_id)?;
    let store = app.state::<MemoryStoreManager>().open(&root)?;
    let query_vec = embed_query(&app, &cfg.memory_embedding, &query).await;
    let mut opts = SearchOptions {
        category,
        k: k.unwrap_or(10),
        min_score: None,
    };
    if opts.category.as_deref() == Some("") {
        opts.category = None;
    }
    let results = store.search_hybrid(&query, query_vec.as_deref(), &opts)?;
    let ids: Vec<String> = results.iter().map(|f| f.id.clone()).collect();
    store.bump_access(&ids);
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
    let cfg = load_cfg(&app);
    let root = resolve_root(&app, &project_id)?;
    let store = app.state::<MemoryStoreManager>().open(&root)?;
    let fact = NewFact {
        key,
        content: content.clone(),
        category: category.unwrap_or_else(|| "note".to_string()),
        importance: importance.unwrap_or(0.5),
        source_type: Some("user".into()),
        source_refs: None,
        pinned: false,
    };
    let embedding = embed_one(&app, &cfg.memory_embedding, &fact.content).await;
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
    let cfg = load_cfg(&app);
    let root = resolve_root(&app, &project_id)?;
    let store = app.state::<MemoryStoreManager>().open(&root)?;
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
        let embedding = embed_one(&app, &cfg.memory_embedding, &new_content).await;
        store.upsert_fact(&fact, embedding)?
    } else {
        let updated = store.update_content(&id, &new_content, "update")?;
        if let Some(embedding) = embed_one(&app, &cfg.memory_embedding, &new_content).await {
            let _ = store.upsert_fact(
                &NewFact {
                    key: None,
                    content: new_content.clone(),
                    category: updated.category.clone(),
                    importance: updated.importance,
                    source_type: updated.source_type.clone(),
                    source_refs: updated.source_refs.clone(),
                    pinned: updated.pinned,
                },
                Some(embedding),
            );
        }
        store.get_fact(&id)?
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

/// 重建/补建向量索引（为无 embedding 的 active 事实生成向量）。
#[tauri::command]
pub async fn memory_reindex(app: AppHandle, project_id: String) -> Result<(usize, usize), String> {
    let cfg = load_cfg(&app);
    let root = resolve_root(&app, &project_id)?;
    let store = app.state::<MemoryStoreManager>().open(&root)?;
    let mut cfg = cfg.memory_embedding;
    inject_key(&app, &mut cfg);
    let embedder = crate::plugins::embed::resolve(&cfg)?;
    store
        .reindex(|content| {
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
        })
        .await
}

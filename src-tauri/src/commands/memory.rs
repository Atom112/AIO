//! Project memory commands (memory_*).
//!
//! Fact CRUD, hybrid retrieval, status and stats for the frontend project settings and memory panel.

use crate::core::events as ev;
use crate::core::models::{AppConfig, MemoryEmbeddingConfig, MemoryRerankConfig};
use crate::services::memory::manager::MemoryStoreManager;
use crate::services::memory::store::{
    FactVersion, MemoryFact, MemoryStats, NewFact, ScoredFact, SearchOptions,
};
use crate::utils::file_tools;
use tauri::{AppHandle, Emitter, Manager};

use crate::ierr;

/// Embedder readiness status (returned by memory_get_status).
#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EmbedderStatus {
    pub provider: String,
    pub model: String,
    pub configured: bool,
    pub available: bool,
    pub dimensions: usize,
}

/// Project memory status (returned by memory_get_status).
#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStatus {
    pub enabled: bool,
    pub store_path: String,
    pub stats: MemoryStats,
    pub embedder: EmbedderStatus,
    pub embedding: MemoryEmbeddingConfig,
    /// Project-level rerank config (apiKey is always empty; online keys live in secure_store)
    pub rerank: MemoryRerankConfig,
}

/// Paginated memory list result.
#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MemoryListPage {
    pub facts: Vec<MemoryFact>,
    pub total: i64,
}

/// Load app config (falls back to defaults on failure).
fn load_cfg(app: &AppHandle) -> AppConfig {
    crate::core::config::load_app_config(app).unwrap_or_default()
}

/// Resolve the project root path from project_id.
fn resolve_root(app: &AppHandle, project_id: &str) -> Result<String, String> {
    file_tools::resolve_project_root(app, Some(project_id))
}

/// Embed a single text; returns None on failure (retrieval/write falls back to keyword mode).
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

/// Compute embedder readiness (prefers the actually resolved embedder over the raw config:
/// provider/model/dimensions in the config may come from a legacy global config snapshot and not match actual usage).
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

/// Read project memory status (including stats and embedder readiness).
#[tauri::command]
pub fn memory_get_status(app: AppHandle, project_id: String) -> Result<MemoryStatus, String> {
    let cfg = load_cfg(&app);
    let root = resolve_root(&app, &project_id)?;
    let store = app.state::<MemoryStoreManager>().open(&root)?;
    let stats = store.stats()?;
    // Lazy migration: the legacy knowledge.json is merged into the fact store on first open (idempotent)
    let _ = store.migrate_from_knowledge_json();
    let project_override = store.project_enabled()?;
    // Explicit, named, auditable migration steps: project-level snapshots and online normalization of embedding/rerank config.
    // The resolve_* calls below are pure reads (resolve_project_rerank normalizes provider/model internally).
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

/// Hybrid retrieval: vector (when available) + keyword (FTS5) fusion.
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
    // Query rewrite (optional) + multi-query RRF fusion + project-level rerank + graph boost; expand=true uses iterative retrieval
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
    // Quality probes: query count and hit count (7-day rolling window)
    store.rolling_bump(crate::services::memory::store::STAT_QUERIES, 1);
    store.rolling_bump(
        crate::services::memory::store::STAT_HITS,
        results.len() as i64,
    );
    Ok(results)
}

/// Add a fact (updates when the key exists; attempts vectorization before write).
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

/// Amend a fact by id (content/category; re-vectorizes).
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
    // With a key, go through upsert (in-place update for the same key + version audit); without a key, amend the content directly
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
        // Keyless fact: amend the content in place; rewrite the vector in place (set_embedding).
        // Never rewrite the vector via upsert_fact(key: None) — with key=None, upsert skips
        // dedup and INSERTs directly, inserting a duplicate fact on every edit.
        let mut updated = store.update_content(&id, &new_content, "update")?;
        if let Some(cat) = &category {
            // The category param takes effect via set_category in the keyless branch
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

/// Delete a fact.
#[tauri::command]
pub fn memory_delete(app: AppHandle, project_id: String, id: String) -> Result<(), String> {
    let root = resolve_root(&app, &project_id)?;
    app.state::<MemoryStoreManager>()
        .open(&root)?
        .delete_fact(&id)
}

/// Read a single fact by id.
#[tauri::command]
pub fn memory_get(app: AppHandle, project_id: String, id: String) -> Result<MemoryFact, String> {
    let root = resolve_root(&app, &project_id)?;
    app.state::<MemoryStoreManager>().open(&root)?.get_fact(&id)
}

/// Set this project's memory toggle (writes a meta override into the project's memory.sqlite; use memory_reset_enabled to follow the global default).
#[tauri::command]
pub fn memory_set_enabled(app: AppHandle, project_id: String, enabled: bool) -> Result<(), String> {
    let root = resolve_root(&app, &project_id)?;
    app.state::<MemoryStoreManager>()
        .open(&root)?
        .set_project_enabled(Some(enabled))
}

/// Clear the project memory toggle override (follows the global default).
#[tauri::command]
pub fn memory_reset_enabled(app: AppHandle, project_id: String) -> Result<(), String> {
    let root = resolve_root(&app, &project_id)?;
    app.state::<MemoryStoreManager>()
        .open(&root)?
        .set_project_enabled(None)
}

/// Read a fact's version audit chain.
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

/// Manually merge two facts (target is kept, source is marked superseded).
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

/// Archive (soft-delete) a fact.
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

/// Pin / unpin a fact.
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

/// Capacity governance: when over the limit, archive the lowest-scoring non-pinned facts; returns the number archived.
#[tauri::command]
pub fn memory_prune(app: AppHandle, project_id: String) -> Result<usize, String> {
    let cfg = load_cfg(&app);
    let root = resolve_root(&app, &project_id)?;
    app.state::<MemoryStoreManager>()
        .open(&root)?
        .prune_to_capacity(cfg.memory_max_facts.max(100))
}

/// Export all memory facts as a JSON file (vectors excluded) to a user-chosen filesystem path.
/// `path`: absolute target file path (picked via the frontend dialog save).
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

/// Import memory facts from a user-chosen JSON file (dedup by key; vectors must be rebuilt).
/// `path`: absolute source file path (picked via the frontend dialog open).
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

/// Index project code for code-level RAG; incremental runs skip unchanged files.
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

/// Semantic code search (backend for the search_code tool; optional path/symbol filters).
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

/// Code index status.
#[tauri::command]
pub fn memory_code_status(
    app: AppHandle,
    project_id: String,
) -> Result<crate::services::memory::code::CodeIndexStatus, String> {
    let root = resolve_root(&app, &project_id)?;
    crate::services::memory::code::code_status(&app, &root)
}

/// Clear project memory.
#[tauri::command]
pub fn memory_clear(app: AppHandle, project_id: String) -> Result<(), String> {
    let root = resolve_root(&app, &project_id)?;
    app.state::<MemoryStoreManager>().open(&root)?.clear()
}

/// Memory stats.
#[tauri::command]
pub fn memory_stats(app: AppHandle, project_id: String) -> Result<MemoryStats, String> {
    let root = resolve_root(&app, &project_id)?;
    app.state::<MemoryStoreManager>().open(&root)?.stats()
}

/// List facts (optional status/category filters, paginated).
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

/// Rebuild/backfill the vector index (generates vectors for active facts without an embedding; progress is reported via the reindex-progress event).
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

/// Switch the project embedding model: validate → probe actual dimensions with the selected model → persist to project meta →
/// clear all project vectors (fact text is kept, including code vectors) → rebuild the index automatically (progress via the reindex-progress event).
#[tauri::command]
pub async fn memory_update_embedding(
    window: tauri::Window,
    app: AppHandle,
    project_id: String,
    config: MemoryEmbeddingConfig,
) -> Result<MemoryStatus, String> {
    // Only local embedding models are supported; legacy online/openai_compat in the config is normalized to local
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
    // Explicit idempotent migration: backfill project-level meta first, then decide "changed or not" to avoid a false first-time switch
    crate::services::memory::migrate_project_embedding(&app, &store);
    let (current, _) = store.embedding_cfg();
    if current.provider == config.provider
        && current.model == config.model
        && current.dimensions == config.dimensions
    {
        // Config unchanged: return the current status directly, no destructive operations
        return memory_get_status(app, project_id);
    }
    // 1) Probe actual dimensions and model key with the selected model (more reliable than hand-filled dimensions/model in the config)
    let (model_key, dims, _) = embed_one(&app, &config, "embedding probe")
        .await
        .ok_or_else(|| {
            ierr!(
                "无法使用所选嵌入模型（请先下载并启用该模型）",
                "Unable to use the selected embedding model (download and enable it first)"
            )
        })?;
    // 2) Persist to project meta (model/dimensions use measured values; apiKey is never written to disk)
    let mut resolved = config.clone();
    resolved.model = model_key;
    resolved.dimensions = dims;
    resolved.api_key.clear();
    store.set_embedding_cfg(&resolved)?;
    // 3) Clear all vectors (fact and code chunk text is kept; only vectors are discarded)
    store.reset_vectors()?;
    // 4) Rebuild the fact index automatically (re-embed with the new model)
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

/// Extract relations between facts (lightweight knowledge graph): for the top `limit` active facts by weighted score,
/// call the LLM per fact to extract entity-relation triples and write them to facts_relations (delete-then-insert, idempotent).
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

/// Set the project-level rerank config (local models only; rerank involves no vectors, so no rebuild is triggered).
#[tauri::command]
pub fn memory_update_rerank(
    app: AppHandle,
    project_id: String,
    config: MemoryRerankConfig,
) -> Result<MemoryStatus, String> {
    let mut config = config;
    // Since v0.8.1 only local rerank is supported: legacy providers (online/openai_compat) are normalized to local
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

/// Manually trigger one consolidation pass (sleep-time consolidation; budget defaults to the global config and can be overridden).
/// Shares the per-project lock and budget semantics with the daily background scheduler.
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

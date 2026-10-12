//! Project-scoped memory service: one SQLite memory store per project (facts + version audit + FTS5 full-text + vector scoring),
//! backing memory tools such as remember / recall / search_memory and automatic system-prompt injection.

pub mod code;
pub mod consolidate;
#[cfg(test)]
pub mod eval;
pub mod extract;
pub mod graph;
pub mod judge;
pub mod llm;
pub mod manager;
pub mod retrieve;
pub mod store;
pub mod tools;
pub mod vec0;
pub mod vector;

pub use manager::MemoryStoreManager;

use crate::core::models::MemoryEmbeddingConfig;
use store::MemoryStore;

/// Collector that surfaces per-row rusqlite read errors.
///
/// Replaces the silent error swallowing of `.filter_map(|r| r.ok())`:
/// row-level parse/type errors are logged as warn and skipped, so bad rows no longer vanish silently.
pub(crate) trait CollectRowsWarn<T>:
    Iterator<Item = Result<T, rusqlite::Error>> + Sized
{
    fn collect_rows(self, context: &str) -> Vec<T> {
        let mut out = Vec::new();
        for r in self {
            match r {
                Ok(v) => out.push(v),
                Err(e) => tracing::warn!("[memory:{context}] 行读取失败（已跳过）: {e}"),
            }
        }
        out
    }
}
impl<T, I: Iterator<Item = Result<T, rusqlite::Error>>> CollectRowsWarn<T> for I {}

/// One-time migration of project-level embedding config (**an explicitly named write**, easy to audit):
///
/// 1. If no project meta exists, snapshot the legacy global config into the project (idempotent);
/// 2. Normalize pre-v0.8 online embedding configs (online/openai_compat/ollama) to the local built-in default.
///
/// Call sites: memory command entry points (memory_get_status / memory_update_embedding).
pub fn migrate_project_embedding(app: &tauri::AppHandle, store: &MemoryStore) {
    let (_, set) = store.embedding_cfg();
    if !set {
        let legacy = crate::core::config::load_app_config(app)
            .unwrap_or_default()
            .memory_embedding;
        let _ = store.set_embedding_cfg(&legacy);
    }
    let (cfg, set) = store.embedding_cfg();
    if set && (cfg.provider != "local" || cfg.model.trim().is_empty()) {
        let fixed = MemoryEmbeddingConfig {
            enabled: cfg.enabled,
            ..MemoryEmbeddingConfig::default()
        };
        let _ = store.set_embedding_cfg(&fixed);
    }
}

/// Resolve the project-level embedding config (**pure read**, never writes to disk).
///
/// Prefers the config saved in project meta; falls back to the legacy global config when no
/// meta exists (without persisting — persisting the migration is an explicit step, see
/// [`migrate_project_embedding`]); online configs are normalized in memory to the local default
/// (all-MiniLM-L6-v2, 384 dims, works out of the box).
pub fn resolve_project_embedding(
    app: &tauri::AppHandle,
    store: &MemoryStore,
) -> MemoryEmbeddingConfig {
    let (cfg, set) = store.embedding_cfg();
    let cfg = if set {
        cfg
    } else {
        crate::core::config::load_app_config(app)
            .unwrap_or_default()
            .memory_embedding
    };
    let cfg = if cfg.provider != "local" || cfg.model.trim().is_empty() {
        MemoryEmbeddingConfig {
            enabled: cfg.enabled,
            ..MemoryEmbeddingConfig::default()
        }
    } else {
        cfg
    };
    // Harness round 6 (self-heal): a dead legacy config (unresolvable model name)
    // with an empty vector library auto-resets to the local default, so semantic
    // retrieval recovers on the next write instead of degrading forever.
    maybe_self_heal_embedding_cfg(&cfg, store)
}

/// Harness round 6 (self-heal): a project-level embedding config naming a model
/// that cannot resolve in the local registry (e.g. the legacy ollama model of the
/// pre-v0.8 era, "bge-m3:latest") silently degraded every write to NULL vectors
/// and every search to keyword RRF ranks — for five test rounds, because the fix
/// required a manual settings change that never happened. When such a dead config
/// is found AND the library holds no vectors at all, reset it to the local
/// default (bundled all-MiniLM-L6-v2) and persist the reset. A library WITH
/// vectors is never auto-reset (that is a user decision, not ours).
pub(crate) fn maybe_self_heal_embedding_cfg(
    cfg: &MemoryEmbeddingConfig,
    store: &MemoryStore,
) -> MemoryEmbeddingConfig {
    if !cfg.enabled
        || crate::core::model_registry::find_entry(cfg.model.trim()).is_some()
        || store.has_any_vectors()
    {
        return cfg.clone();
    }
    tracing::warn!(
        "[memory] 嵌入模型 '{}' 在本地注册表中不存在且库内无向量，自动回退默认本地模型（原配置导致语义检索静默降级为关键词匹配）",
        cfg.model
    );
    let def = MemoryEmbeddingConfig::default();
    if let Err(e) = store.set_embedding_cfg(&def) {
        tracing::warn!("[memory] 死配置自愈写入失败（本次进程内仍使用默认模型）: {e}");
    }
    def
}

/// Embed a single text under an explicit config, with no store guards (MEM-06).
/// Used by the embedding-model switch probe, which must measure the NEW model's
/// dimensions even while the store still holds old vectors (they get cleared next).
pub async fn embed_raw(
    app: &tauri::AppHandle,
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

/// Embed a single text per the project-level embedding config (MEM-06: the single
/// shared implementation — four copy-pasted versions previously disagreed on the
/// dimension-drift guard, so facts written through one path could silently carry
/// dead vectors). Returns None on failure (retrieval/write degrades to keyword mode)
/// and whenever the embedding dimension mismatches the store's existing vectors:
/// a switched embedding model never writes unsearchable vectors into the lib.
pub async fn embed_one(
    app: &tauri::AppHandle,
    store: &MemoryStore,
    text: &str,
) -> Option<(String, usize, Vec<f32>)> {
    let cfg = resolve_project_embedding(app, store);
    let (model, dim, v) = embed_raw(app, &cfg, text).await?;
    if let Some(lib_dim) = store.embedding_dim_meta() {
        if lib_dim != dim {
            tracing::warn!(
                "记忆库向量维度 {} 与当前嵌入模型维度 {} 不一致，跳过向量化（请检查嵌入模型配置）",
                lib_dim,
                dim
            );
            return None;
        }
    }
    Some((model, dim, v))
}

/// Best-effort reason the embedding pipeline is unavailable (None = it resolves
/// fine). Harness round 5: EVERY fact in the test project carried a NULL
/// embedding (a legacy ollama model name can never resolve in the local
/// registry) and neither the model nor the user could tell — recall scores had
/// silently degraded to RRF keyword ranks (~0.02, no discrimination). The
/// memory write/search tools surface this reason so the degradation is visible.
pub fn embedding_unavailable_reason(app: &tauri::AppHandle, store: &MemoryStore) -> Option<String> {
    let cfg = resolve_project_embedding(app, store);
    if !cfg.enabled {
        return Some(crate::ierr!("嵌入未启用", "Embedding is not enabled"));
    }
    crate::plugins::embed::resolve(app, &cfg).err()
}

#[cfg(test)]
mod self_heal_tests {
    use super::*;
    use crate::core::models::MemoryEmbeddingConfig;

    fn temp_store(tag: &str) -> (std::path::PathBuf, MemoryStore) {
        let dir = std::env::temp_dir().join(format!("aio-heal-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let dir_s = dir.to_string_lossy().to_string();
        let store = MemoryStore::open(&dir_s).expect("open store");
        (dir, store)
    }

    fn dead_legacy_cfg() -> MemoryEmbeddingConfig {
        MemoryEmbeddingConfig {
            provider: "ollama".into(),
            model: "bge-m3:latest".into(),
            api_url: String::new(),
            dimensions: 1024,
            enabled: true,
            api_key: String::new(),
        }
    }

    /// Harness round 6: the exact test-project situation — a legacy ollama model
    /// name that can never resolve + zero vectors — auto-resets to the local
    /// default and persists the reset (model, dims) so later opens stay healed.
    #[test]
    fn self_heal_resets_dead_legacy_model_when_no_vectors() {
        let (_dir, store) = temp_store("reset");
        store.set_embedding_cfg(&dead_legacy_cfg()).unwrap();
        let (cfg, set) = store.embedding_cfg();
        assert!(set, "meta 应已设置");
        assert_eq!(cfg.model, "bge-m3:latest");
        assert!(!store.has_any_vectors(), "新库应无向量");

        let healed = maybe_self_heal_embedding_cfg(&cfg, &store);
        assert_eq!(healed.model, "all-MiniLM-L6-v2");
        assert_eq!(healed.dimensions, 384);

        // Persisted: the next read sees the healed config and matching dim meta.
        let (cfg2, _) = store.embedding_cfg();
        assert_eq!(cfg2.model, "all-MiniLM-L6-v2");
        assert_eq!(store.embedding_dim_meta(), Some(384));
        let _ = std::fs::remove_dir_all(&_dir);
    }

    /// A library WITH vectors is never auto-reset (user decision, not ours), and
    /// a disabled config is never healed either (the user turned embedding off).
    #[test]
    fn self_heal_skips_when_vectors_exist_or_disabled() {
        let (_dir, store) = temp_store("skip-vec");
        store.set_embedding_cfg(&dead_legacy_cfg()).unwrap();
        // One fact WITH a vector → no heal
        store
            .upsert_fact(
                &crate::services::memory::store::NewFact {
                    key: Some("k".into()),
                    content: "c".into(),
                    category: "note".into(),
                    importance: 0.5,
                    source_type: None,
                    source_refs: None,
                    pinned: false,
                },
                Some(("m".into(), 4, vec![0.5f32; 4])),
            )
            .expect("upsert");
        let (cfg, _) = store.embedding_cfg();
        let out = maybe_self_heal_embedding_cfg(&cfg, &store);
        assert_eq!(out.model, "bge-m3:latest", "有向量的库不应自愈");
        let _ = std::fs::remove_dir_all(&_dir);

        // Disabled config (only reachable via the global fallback path — meta reads
        // always come back enabled=true) is never healed: the user turned it off.
        let (_dir2, store2) = temp_store("skip-off");
        let mut disabled = dead_legacy_cfg();
        disabled.enabled = false;
        let out2 = maybe_self_heal_embedding_cfg(&disabled, &store2);
        assert_eq!(out2.model, "bge-m3:latest", "禁用状态不应自愈");
        let _ = std::fs::remove_dir_all(&_dir2);
    }
}

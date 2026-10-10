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
    if cfg.provider != "local" || cfg.model.trim().is_empty() {
        MemoryEmbeddingConfig {
            enabled: cfg.enabled,
            ..MemoryEmbeddingConfig::default()
        }
    } else {
        cfg
    }
}

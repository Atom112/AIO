//! 项目级记忆服务：每项目一个 SQLite 记忆库（facts + 版本审计 + FTS5 全文 + 向量打分），
//! 支撑 remember / recall / search_memory 等记忆工具与系统提示词自动注入。

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

/// rusqlite 行级读取的可见化收集器。
///
/// 替代 `.filter_map(|r| r.ok())` 的静默吞错（本模块曾有 20 处）：
/// 行级解析/类型错误记录 warn 后跳过，坏行不再无痕消失。
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

/// 项目级嵌入配置的一次性迁移（**显式命名的写操作**，便于审计）：
///
/// 1. 无项目 meta 时把旧全局配置快照到项目（幂等）；
/// 2. v0.8 前的在线嵌入配置（online/openai_compat/ollama）归一化为本地内置默认。
///
/// 调用点：记忆命令入口（memory_get_status / memory_update_embedding）。
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

/// 解析项目级嵌入配置（**纯读取**，不写盘）。
///
/// 偏好项目 meta 中已保存的配置；无 meta 时回退全局旧配置（不落盘——落盘迁移是
/// 显式步骤，见 [`migrate_project_embedding`]）；在线配置在内存中归一化为本地默认
/// （all-MiniLM-L6-v2，384 维，开箱即用）。
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

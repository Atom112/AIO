//! 项目级记忆服务：每项目一个 SQLite 记忆库（facts + 版本审计 + FTS5 全文 + 向量打分），
//! 支撑 remember / recall / search_memory 等记忆工具与系统提示词自动注入。

pub mod code;
pub mod extract;
pub mod judge;
pub mod llm;
pub mod manager;
pub mod store;
pub mod tools;
pub mod vec0;
pub mod vector;

pub use manager::MemoryStoreManager;

use crate::core::models::MemoryEmbeddingConfig;
use store::MemoryStore;

/// 解析项目级嵌入配置（切换嵌入模型后按项目记忆库的 meta 持久化）。
///
/// 偏好项目 meta 中已保存的配置；首次使用时（无 meta）把旧全局配置快照到项目（幂等），
/// 没有旧配置则回退本地内置默认（all-MiniLM-L6-v2，384 维，开箱即用）。
pub fn resolve_project_embedding(
    app: &tauri::AppHandle,
    store: &MemoryStore,
) -> MemoryEmbeddingConfig {
    let (cfg, set) = store.embedding_cfg();
    if set {
        return cfg;
    }
    let legacy = crate::commands::config::load_app_config(app.clone())
        .unwrap_or_default()
        .memory_embedding;
    let _ = store.set_embedding_cfg(&legacy);
    legacy
}

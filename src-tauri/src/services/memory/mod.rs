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

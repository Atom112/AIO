//! 旧版跨会话项目知识（`.aio/knowledge.json`）。
//!
//! 已被项目级语义记忆（RAG）取代：新记忆写入 `.aio/memory/memory.sqlite`。
//! 本模块仅保留读取能力：首次打开项目记忆时把旧 knowledge.json 迁移为事实（幂等）。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// 单条项目级知识条目。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeEntry {
    /// 知识的简短标识（用于去重和检索）。
    pub key: String,
    /// 知识内容。
    pub content: String,
    /// 类别: "decision" | "pattern" | "convention" | "note"
    pub category: String,
    /// 创建时间（ISO 8601）。
    pub created_at: String,
    /// 最后更新时间（ISO 8601）。
    pub updated_at: String,
}

/// `.aio/knowledge.json` 的磁盘格式。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct KnowledgeFile {
    pub entries: Vec<KnowledgeEntry>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// 时间工具已统一收敛到 utils::time（now_iso / secs_to_iso / 公历换算）。
// 本模块为只读遗留代码，不应再成为新代码的依赖点。

// ---------------------------------------------------------------------------
// Core functions
// ---------------------------------------------------------------------------

/// 从 `{project_root}/.aio/knowledge.json` 加载知识文件。
/// 若文件不存在则返回空的 `KnowledgeFile`。
pub fn load_knowledge(project_root: &str) -> KnowledgeFile {
    let path = Path::new(project_root).join(".aio").join("knowledge.json");
    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => KnowledgeFile::default(),
    }
}

//! 旧版跨会话项目知识（`.aio/knowledge.json`）。
//!
//! 已被项目级语义记忆（RAG）取代：新记忆写入 `.aio/memory/memory.sqlite`。
//! 本模块仅保留读取能力：首次打开项目记忆时把旧 knowledge.json 迁移为事实（幂等）。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

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

/// 返回当前时间的 ISO 8601 字符串（精度到秒，不含时区后缀）。
pub(crate) fn now_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    secs_to_iso(secs)
}

/// 将 UNIX 时间戳（秒）转为 `YYYY-MM-DDTHH:MM:SS` 格式。
pub(crate) fn secs_to_iso(secs: u64) -> String {
    let (y, mo, d, h, m, s) = secs_to_date_parts(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}")
}

/// 简易公历换算 —— 来自 `config.rs:716-747` 的同款实现。
fn secs_to_date_parts(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    const SECS_PER_DAY: u64 = 86400;
    let days = secs / SECS_PER_DAY;
    let time_secs = secs % SECS_PER_DAY;
    let hours = time_secs / 3600;
    let mins = (time_secs % 3600) / 60;
    let s = time_secs % 60;

    let mut y = 1970u64;
    let mut remaining = days;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }
    let month_days: &[u64] = if is_leap(y) {
        &[31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        &[31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut mo = 1u64;
    for &md in month_days {
        if remaining < md {
            break;
        }
        remaining -= md;
        mo += 1;
    }
    (y, mo, remaining + 1, hours, mins, s)
}

fn is_leap(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

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

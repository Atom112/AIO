//! 项目级知识持久化 — 跨 session 记忆。
//!
//! 存储 Agent 在对话中学习到的项目关键信息，如架构决策、代码约定、常用模式等。
//! 数据持久化到 `{project_root}/.aio/knowledge.json`，每次 Agent 启动时自动注入到系统提示词中。
//!
//! 提供 `remember`（存储）和 `recall`（检索）两个内置工具。

use crate::core::models::{ToolFunctionSpec, ToolResult, ToolResultContent, ToolSpec};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
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
fn now_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    secs_to_iso(secs)
}

/// 将 UNIX 时间戳（秒）转为 `YYYY-MM-DDTHH:MM:SS` 格式。
fn secs_to_iso(secs: u64) -> String {
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

/// 快捷构造成功 `ToolResult`。
fn tool_ok(text: String) -> ToolResult {
    ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({"text": text}),
        }],
        is_error: false,
    }
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

/// 保存知识文件到 `{project_root}/.aio/knowledge.json`。
pub fn save_knowledge(project_root: &str, knowledge: &KnowledgeFile) -> Result<(), String> {
    let aio_dir = Path::new(project_root).join(".aio");
    fs::create_dir_all(&aio_dir).map_err(|e| format!("创建 .aio 目录失败: {e}"))?;

    let path = aio_dir.join("knowledge.json");
    let json = serde_json::to_string_pretty(knowledge).map_err(|e| format!("序列化失败: {e}"))?;

    let mut file = fs::File::create(&path).map_err(|e| format!("创建知识文件失败: {e}"))?;
    file.write_all(json.as_bytes())
        .map_err(|e| format!("写入知识文件失败: {e}"))?;
    Ok(())
}

/// 添加一条知识条目。
///
/// - 若已存在相同 `key` 的条目，则更新其 `content` / `category` / `updated_at`。
/// - 条目数上限 50；超出时移除最早的条目。
pub fn add_knowledge_entry(project_root: &str, entry: KnowledgeEntry) -> Result<(), String> {
    let mut knowledge = load_knowledge(project_root);

    // 去重：相同 key 的条目原地更新
    if let Some(existing) = knowledge.entries.iter_mut().find(|e| e.key == entry.key) {
        existing.content = entry.content;
        existing.category = entry.category;
        existing.updated_at = entry.updated_at;
    } else {
        knowledge.entries.push(entry);
    }

    // 上限裁剪：保留最新的 50 条
    if knowledge.entries.len() > 50 {
        // 按 created_at 升序排列，移除最早的
        knowledge
            .entries
            .sort_by(|a, b| a.created_at.cmp(&b.created_at));
        knowledge.entries = knowledge.entries.split_off(knowledge.entries.len() - 50);
    }

    save_knowledge(project_root, &knowledge)
}

/// 模糊搜索知识条目。
///
/// - `query` 对 `key` 和 `content` 做大小写不敏感的包含匹配。
/// - `category` 若提供，则只返回该分类的条目。
pub fn search_knowledge<'a>(
    knowledge: &'a KnowledgeFile,
    query: &str,
    category: Option<&str>,
) -> Vec<&'a KnowledgeEntry> {
    let query_lower = query.to_lowercase();
    knowledge
        .entries
        .iter()
        .filter(|e| {
            let cat_match = match category {
                Some(cat) => e.category == cat,
                None => true,
            };
            cat_match
                && (e.key.to_lowercase().contains(&query_lower)
                    || e.content.to_lowercase().contains(&query_lower))
        })
        .collect()
}

/// 将知识条目格式化为可注入系统提示词的 Markdown 文本。
///
/// 按 category 分组输出：
/// ```text
/// ## decision
/// - db-schema: Users 表包含 id, name, email
///
/// ## convention
/// - naming: 使用 camelCase 命名变量
/// ```
pub fn knowledge_to_prompt(knowledge: &KnowledgeFile) -> String {
    if knowledge.entries.is_empty() {
        return String::new();
    }

    // 按 category 分组
    let mut grouped: std::collections::BTreeMap<&str, Vec<&KnowledgeEntry>> =
        std::collections::BTreeMap::new();
    for entry in &knowledge.entries {
        grouped
            .entry(entry.category.as_str())
            .or_default()
            .push(entry);
    }

    let mut prompt = String::new();
    for (category, entries) in &grouped {
        prompt.push_str(&format!("## {category}\n"));
        for entry in entries {
            prompt.push_str(&format!("- {}: {}\n", entry.key, entry.content));
        }
        prompt.push('\n');
    }
    prompt
}

// ---------------------------------------------------------------------------
// Tool specs
// ---------------------------------------------------------------------------

/// 返回知识工具的两条 spec（`remember` + `recall`）。
pub fn get_knowledge_tool_specs() -> Vec<ToolSpec> {
    vec![remember_tool_spec(), recall_tool_spec()]
}

fn remember_tool_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".to_string(),
        function: ToolFunctionSpec {
            name: "remember".to_string(),
            description: "记住一条项目级知识（架构决策、代码约定、常用模式），跨 session 持久化到 .aio/knowledge.json".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "key": {
                        "type": "string",
                        "description": "知识的简短标识（用于去重和检索）"
                    },
                    "content": {
                        "type": "string",
                        "description": "知识内容"
                    },
                    "category": {
                        "type": "string",
                        "enum": ["decision", "pattern", "convention", "note"],
                        "description": "知识类别: decision（决策）, pattern（模式）, convention（约定）, note（笔记）"
                    }
                },
                "required": ["key", "content", "category"]
            }),
        },
    }
}

fn recall_tool_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".to_string(),
        function: ToolFunctionSpec {
            name: "recall".to_string(),
            description: "检索项目级知识（使用关键词模糊匹配 key 和 content）".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "搜索关键词（大小写不敏感，匹配 key 和 content）"
                    },
                    "category": {
                        "type": "string",
                        "enum": ["decision", "pattern", "convention", "note"],
                        "description": "按类别过滤（可选）"
                    }
                },
                "required": ["query"]
            }),
        },
    }
}

// ---------------------------------------------------------------------------
// Execute functions
// ---------------------------------------------------------------------------

/// 执行 `remember` 工具调用。
pub fn execute_remember(project_root: &str, args: &Value) -> Result<ToolResult, String> {
    let key = args
        .get("key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "缺少参数 key".to_string())?;

    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "缺少参数 content".to_string())?;

    let category = args
        .get("category")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "缺少参数 category".to_string())?;

    // 校验 category 合法值
    let valid_categories = ["decision", "pattern", "convention", "note"];
    if !valid_categories.contains(&category) {
        return Err(format!(
            "无效的 category '{}'，可选值: {}",
            category,
            valid_categories.join(", ")
        ));
    }

    let now = now_iso();
    let entry = KnowledgeEntry {
        key: key.to_string(),
        content: content.to_string(),
        category: category.to_string(),
        created_at: now.clone(),
        updated_at: now,
    };

    add_knowledge_entry(project_root, entry)?;

    Ok(tool_ok(format!("已记住: {key}")))
}

/// 执行 `recall` 工具调用。
pub fn execute_recall(project_root: &str, args: &Value) -> Result<ToolResult, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "缺少参数 query".to_string())?;

    let category = args.get("category").and_then(|v| v.as_str());

    let knowledge = load_knowledge(project_root);
    let results = search_knowledge(&knowledge, query, category);

    if results.is_empty() {
        return Ok(tool_ok(format!("未找到与 '{query}' 匹配的项目知识")));
    }

    let mut output = format!("找到 {} 条与 '{query}' 匹配的知识:\n\n", results.len());
    for (i, entry) in results.iter().enumerate() {
        output.push_str(&format!(
            "{}. [{}] {}: {}\n",
            i + 1,
            entry.category,
            entry.key,
            entry.content
        ));
    }

    Ok(tool_ok(output))
}

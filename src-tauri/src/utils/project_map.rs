//! 内置 Project Map 工具 — 项目结构概览。
//!
//! 提供一个只读工具，生成项目目录结构树状图，
//! 供 Plan 模式和编排 Agent 了解项目布局。
//! 不修改任何文件，无副作用。

use crate::core::models::{ToolResult, ToolResultContent, ToolSpec, ToolFunctionSpec};
use serde_json::json;
use std::fs;
use std::path::Path;

/// 项目结构树的显示深度上限
const MAX_DEPTH: usize = 4;
/// 每个目录最多展示的条目数
const MAX_ENTRIES_PER_DIR: usize = 50;

/// 返回 Project Map 工具的 ToolSpec。
pub fn tool_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".into(),
        function: ToolFunctionSpec {
            name: "project_map".into(),
            description: "获取项目目录结构概览（树状图）。用于快速了解项目布局、关键文件和模块组织。最大深度 4 层，每目录最多 50 条目。".into(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
    }
}

/// 执行 Project Map 工具 — 返回项目根目录的树状结构。
pub fn execute(project_root: &str) -> ToolResult {
    let root = Path::new(project_root);
    if !root.is_dir() {
        return ToolResult {
            content: vec![ToolResultContent {
                kind: "text".into(),
                data: json!({ "text": format!("[Error] 项目根目录不存在: {}", project_root) }),
            }],
            is_error: true,
        };
    }

    let mut output = String::new();
    output.push_str(&format!("项目结构 ({})\n", project_root));
    build_tree(root, &mut output, 0, "");

    if output.is_empty() {
        output.push_str("(空目录)");
    }

    ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({ "text": output }),
        }],
        is_error: false,
    }
}

/// 递归构建目录树字符串。
fn build_tree(dir: &Path, output: &mut String, depth: usize, prefix: &str) {
    if depth > MAX_DEPTH {
        output.push_str(&format!("{}... (超过最大深度)\n", prefix));
        return;
    }

    let mut entries: Vec<_> = match fs::read_dir(dir) {
        Ok(iter) => iter.filter_map(|e| e.ok()).collect(),
        Err(_) => return,
    };

    // 排序：目录在前，文件在后；字母序
    entries.sort_by(|a, b| {
        let a_is_dir = a.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let b_is_dir = b.file_type().map(|t| t.is_dir()).unwrap_or(false);
        b_is_dir
            .cmp(&a_is_dir)
            .then_with(|| a.file_name().cmp(&b.file_name()))
    });

    // 跳过隐藏文件和 node_modules / target 等常见忽略目录
    let ignore_dirs = ["node_modules", "target", ".git", "__pycache__", ".next", "dist", "build"];
    let entries: Vec<_> = entries
        .into_iter()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') && name != ".env" && name != ".gitignore" {
                return false;
            }
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) && ignore_dirs.contains(&name.as_str()) {
                return false;
            }
            true
        })
        .collect();

    let count = entries.len();
    let display_count = count.min(MAX_ENTRIES_PER_DIR);

    for (i, entry) in entries.iter().take(display_count).enumerate() {
        let is_last = i == display_count - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let child_prefix = if is_last {
            format!("{}    ", prefix)
        } else {
            format!("{}│   ", prefix)
        };

        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);

        if is_dir {
            output.push_str(&format!("{}{}{}/\n", prefix, connector, name));
            let child_path = entry.path();
            build_tree(&child_path, output, depth + 1, &child_prefix);
        } else {
            output.push_str(&format!("{}{}{}\n", prefix, connector, name));
        }
    }

    if count > display_count {
        output.push_str(&format!("{}... (+{} more entries)\n", prefix, count - display_count));
    }
}

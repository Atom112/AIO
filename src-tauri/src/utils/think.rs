//! 内置 Think 工具 — 结构化思考。
//!
//! 提供一个 no-op 工具，允许模型在工具调用中输出结构化思考。
//! 工具接收一个 `thought` 参数并返回确认文本，无副作用。
//!
//! 用于 Plan 模式和编排场景，让模型在调用执行工具前先整理思路。

use crate::core::models::{ToolResult, ToolResultContent, ToolSpec, ToolFunctionSpec};
use serde_json::json;

/// 返回 Think 工具的 ToolSpec。
pub fn tool_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".into(),
        function: ToolFunctionSpec {
            name: "think".into(),
            description: "记录结构化的思考过程。使用此工具来整理思路、分析问题、制定计划。此工具无副作用，仅返回确认。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "thought": {
                        "type": "string",
                        "description": "你的结构化思考内容"
                    }
                },
                "required": ["thought"]
            }),
        },
    }
}

/// 执行 Think 工具（no-op，返回确认文本）。
pub fn execute(arguments: &serde_json::Value) -> ToolResult {
    let thought = arguments["thought"]
        .as_str()
        .unwrap_or("");

    let preview: String = if thought.chars().count() > 100 {
        format!("{}...", thought.chars().take(100).collect::<String>())
    } else {
        thought.to_string()
    };

    ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({ "text": format!("[思考已记录] {}", preview) }),
        }],
        is_error: false,
    }
}

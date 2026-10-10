//! Built-in Think tool — structured thinking.
//!
//! Provides a no-op tool that lets the model emit structured thinking within tool calls.
//! The tool takes a `thought` parameter and returns a confirmation text; no side effects.
//!
//! Used in Plan mode and orchestration scenarios so the model can organize its thoughts before calling execution tools.

use crate::core::models::{ToolFunctionSpec, ToolResult, ToolSpec};
use crate::utils::tool_result::tool_ok;
use serde_json::json;

/// Return the ToolSpec of the Think tool.
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

/// Execute the Think tool (no-op; returns a confirmation text).
pub fn execute(arguments: &serde_json::Value) -> ToolResult {
    let thought = arguments["thought"].as_str().unwrap_or("");

    let preview: String = if thought.chars().count() > 100 {
        format!("{}...", thought.chars().take(100).collect::<String>())
    } else {
        thought.to_string()
    };

    tool_ok(format!("[思考已记录] {preview}"))
}

//! Tool result construction helpers — the single entry point for building the built-in tools' ToolResult.

use crate::core::models::{ToolResult, ToolResultContent};
use serde_json::json;

/// Construct a successful tool result (in-process tool shape).
pub fn tool_ok(text: String) -> ToolResult {
    ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({"text": text}),
        }],
        is_error: false,
    }
}

/// Construct a failed tool result (in-process tool shape); the text is automatically prefixed with `[Error] `.
pub fn tool_err(msg: &str) -> ToolResult {
    ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({"text": format!("[Error] {msg}")}),
        }],
        is_error: true,
    }
}

// The MCP-protocol-shaped tool_ok_mcp/tool_err_mcp were removed;
// fs-server now delegates to execute_file_tool and builds its own JSON-RPC results.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_are_stable() {
        let ok = tool_ok("hi".to_string());
        assert!(!ok.is_error);
        assert_eq!(ok.content[0].data["text"], "hi");
        let err = tool_err("boom");
        assert!(err.is_error);
        assert_eq!(err.content[0].data["text"], "[Error] boom");
    }
}

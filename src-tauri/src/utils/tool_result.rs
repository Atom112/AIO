//! 工具结果构造辅助 —— 内置工具 ToolResult 构造的唯一入口。
//!
//! 历史上 tool_ok/tool_err 在 8+5 个模块中各复制一份（project_map/think 还内联
//! 相同字面量），任何格式调整都要全仓同步极易漏改；现统一收敛于此。

use crate::core::models::{ToolResult, ToolResultContent};
use serde_json::{json, Value};

/// 构造成功的工具结果（进程内工具形态）。
pub fn tool_ok(text: String) -> ToolResult {
    ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({"text": text}),
        }],
        is_error: false,
    }
}

/// 构造失败的工具结果（进程内工具形态），文本自动加 `[Error] ` 前缀。
pub fn tool_err(msg: &str) -> ToolResult {
    ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({"text": format!("[Error] {msg}")}),
        }],
        is_error: true,
    }
}

/// MCP 协议形态的成功结果（`mcp_fs_server` 的 JSON-RPC 响应用）。
pub fn tool_ok_mcp(text: String) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": false})
}

/// MCP 协议形态的失败结果（`mcp_fs_server` 的 JSON-RPC 响应用）。
pub fn tool_err_mcp(msg: &str) -> Value {
    json!({"content": [{"type": "text", "text": format!("[Error] {msg}")}], "isError": true})
}

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

        let m_ok = tool_ok_mcp("hi".to_string());
        assert_eq!(m_ok["isError"], false);
        let m_err = tool_err_mcp("boom");
        assert_eq!(m_err["isError"], true);
        assert_eq!(m_err["content"][0]["text"], "[Error] boom");
    }
}

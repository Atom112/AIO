//! 工具结果构造辅助 —— 内置工具 ToolResult 构造的唯一入口。
//!
//! 历史上 tool_ok/tool_err 在 8+5 个模块中各复制一份（project_map/think 还内联
//! 相同字面量），任何格式调整都要全仓同步极易漏改；现统一收敛于此。

use crate::core::models::{ToolResult, ToolResultContent};
use serde_json::json;

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

// MCP 协议形态的 tool_ok_mcp/tool_err_mcp 已随 P4.3 fs-server 去分叉移除
//（fs-server 现委托 execute_file_tool 并自行构造 JSON-RPC 结果）。

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

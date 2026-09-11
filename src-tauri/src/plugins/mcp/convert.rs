//! MCP 响应 → 内部类型的转换辅助。
//!
//! stdio 与 http 两种传输此前各自复制了 tools/list、tools/call、resources/list、
//! prompts/list 四组映射逻辑（约 60 行），任一协议细节调整都要改两处。此处收敛为单实现。

use super::MAX_TOOLS_PER_SERVER;
use crate::core::models::{
    McpPrompt, McpResource, ToolFunctionSpec, ToolResult, ToolResultContent, ToolSpec,
};
use serde_json::Value;

/// `tools/list` 的 tools 数组 → OpenAI function calling 规格。
///
/// MCP 返回 `{ name, description, inputSchema }`，需转换为
/// `{ type: "function", function: { name, description, parameters } }`；
/// 无 name/description 的条目跳过，结果按 `MAX_TOOLS_PER_SERVER` 截断。
pub fn tools_to_specs(tools: Vec<Value>) -> Vec<ToolSpec> {
    tools
        .into_iter()
        .filter_map(|t| {
            let name = t.get("name")?.as_str()?.to_string();
            let description = t.get("description")?.as_str()?.to_string();
            let parameters = t
                .get("inputSchema")
                .cloned()
                .unwrap_or(serde_json::json!({}));
            Some(ToolSpec {
                kind: "function".to_string(),
                function: ToolFunctionSpec {
                    name,
                    description,
                    parameters,
                },
            })
        })
        .take(MAX_TOOLS_PER_SERVER)
        .collect()
}

/// `tools/call` 的 result → ToolResult（`{ content: [...], isError: bool }`）。
pub fn parse_call_tool_result(result: &Value) -> ToolResult {
    let content_val = result
        .get("content")
        .cloned()
        .unwrap_or(serde_json::json!([]));
    let content: Vec<ToolResultContent> = match content_val {
        Value::Array(arr) => arr
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect(),
        _ => vec![],
    };
    let is_error = result
        .get("isError")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    ToolResult { content, is_error }
}

/// `resources/list` 的 resources 数组 → McpResource 列表（解析失败条目跳过）。
pub fn parse_resources(resources: Vec<Value>) -> Vec<McpResource> {
    resources
        .into_iter()
        .filter_map(|r| serde_json::from_value(r).ok())
        .collect()
}

/// `prompts/list` 的 prompts 数组 → McpPrompt 列表（解析失败条目跳过）。
pub fn parse_prompts(prompts: Vec<Value>) -> Vec<McpPrompt> {
    prompts
        .into_iter()
        .filter_map(|p| serde_json::from_value(p).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tools_to_specs_maps_and_skips_invalid() {
        let specs = tools_to_specs(vec![
            json!({"name": "read", "description": "读文件", "inputSchema": {"type": "object"}}),
            json!({"description": "缺少 name，应跳过"}),
            json!({"name": "write"}), // 缺 description，跳过
        ]);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].function.name, "read");
        assert_eq!(specs[0].kind, "function");
        assert_eq!(specs[0].function.parameters["type"], "object");
    }

    #[test]
    fn parse_call_tool_result_reads_content_and_error() {
        let r = parse_call_tool_result(&json!({
            "content": [{"type": "text", "text": "hi"}],
            "isError": true
        }));
        assert!(r.is_error);
        assert_eq!(r.content.len(), 1);
        // 缺字段时的默认值
        let empty = parse_call_tool_result(&json!({}));
        assert!(!empty.is_error);
        assert!(empty.content.is_empty());
    }

    #[test]
    fn parse_resources_and_prompts_skip_invalid() {
        let res = parse_resources(vec![
            json!({"uri": "file:///a", "name": "A"}),
            json!("not-an-object"),
        ]);
        assert_eq!(res.len(), 1);
        let pr = parse_prompts(vec![json!({"name": "p1"}), json!(1)]);
        assert_eq!(pr.len(), 1);
    }
}

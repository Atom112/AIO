//! Helpers for converting MCP responses into internal types.
//!
//! Single implementation of the tools/list, tools/call, resources/list and
//! prompts/list mapping logic, shared by the stdio and http transports.

use super::MAX_TOOLS_PER_SERVER;
use crate::core::models::{
    McpPrompt, McpResource, ToolFunctionSpec, ToolResult, ToolResultContent, ToolSpec,
};
use serde_json::Value;

/// `tools/list` tools array → OpenAI function calling spec.
///
/// MCP returns `{ name, description, inputSchema }`, which must be converted to
/// `{ type: "function", function: { name, description, parameters } }`; entries
/// without a name are skipped. The result is truncated to `MAX_TOOLS_PER_SERVER`.
///
/// MCP-11: `description` is OPTIONAL in the spec — a valid tool without one is
/// no longer silently dropped (an empty description is emitted instead).
pub fn tools_to_specs(tools: Vec<Value>) -> Vec<ToolSpec> {
    tools
        .into_iter()
        .filter_map(|t| {
            let name = t.get("name")?.as_str()?.to_string();
            let description = t
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string();
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

/// `tools/call` result → ToolResult (`{ content: [...], isError: bool }`).
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

/// `resources/list` resources array → McpResource list (entries that fail to parse are skipped).
pub fn parse_resources(resources: Vec<Value>) -> Vec<McpResource> {
    resources
        .into_iter()
        .filter_map(|r| serde_json::from_value(r).ok())
        .collect()
}

/// `prompts/list` prompts array → McpPrompt list (entries that fail to parse are skipped).
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
            // MCP-11: description 是可选字段——无 description 的合法工具不再被丢弃
            json!({"name": "write"}),
        ]);
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].function.name, "read");
        assert_eq!(specs[0].kind, "function");
        assert_eq!(specs[0].function.parameters["type"], "object");
        assert_eq!(specs[1].function.name, "write");
        assert_eq!(specs[1].function.description, "");
    }

    #[test]
    fn parse_call_tool_result_reads_content_and_error() {
        let r = parse_call_tool_result(&json!({
            "content": [{"type": "text", "text": "hi"}],
            "isError": true
        }));
        assert!(r.is_error);
        assert_eq!(r.content.len(), 1);
        // Defaults when fields are missing
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

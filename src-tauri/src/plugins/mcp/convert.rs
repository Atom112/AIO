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
    // MCP-20: make the cap visible — tools beyond the cap used to vanish with no
    // signal at all, leaving the model and the user unaware the set was clipped.
    let total = tools.len();
    let out: Vec<ToolSpec> = tools
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
        .collect();
    if total > out.len() {
        tracing::warn!(
            "[mcp] tools/list 截断：服务器提供 {total} 个工具，仅保留前 {} 个",
            out.len()
        );
    }
    out
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

/// `initialize` result payload → McpServerInfo (阶段4: stdio/http previously
/// carried two diverging copies of this parsing — name/version/capabilities).
///
/// `result` is the UNWRAPPED initialize result object (the caller is responsible
/// for digging it out of the JSON-RPC envelope on transports that return one).
/// Absent fields degrade: name → "unknown", version → None, capabilities → None.
pub fn parse_server_info(result: &Value) -> crate::core::models::McpServerInfo {
    let info_val = result
        .get("serverInfo")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    let name = info_val
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let version = info_val
        .get("version")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let capabilities = result
        .get("capabilities")
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    crate::core::models::McpServerInfo {
        name,
        version,
        capabilities,
    }
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

    /// 阶段4 回归：共享 initialize 解析——缺字段的降级行为（unknown/None）
    /// 与 capabilities 反序列化保持一致。
    #[test]
    fn parse_server_info_degrades_missing_fields() {
        let full = json!({
            "serverInfo": { "name": "demo", "version": "1.2.3" },
            "capabilities": { "tools": {} }
        });
        let info = parse_server_info(&full);
        assert_eq!(info.name, "demo");
        assert_eq!(info.version.as_deref(), Some("1.2.3"));
        assert!(info.capabilities.is_some());

        // 完全缺失 → 降级
        let empty = parse_server_info(&json!({}));
        assert_eq!(empty.name, "unknown");
        assert!(empty.version.is_none());
        assert!(empty.capabilities.is_none());
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

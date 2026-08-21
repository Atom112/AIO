//! Anthropic provider plugin
//!
//! 端点规范:
//! - URL: ${base}/v1/models (base = https://api.anthropic.com, 不带版本号)
//! - Auth: x-api-key: <key> + anthropic-version: 2023-06-01 (不能用 Authorization: Bearer)
//! - Response: {data: [{id, display_name, created_at}]} —— 用 id 字段，和 OpenAI 兼容
//!
//! 参考 LobeHub packages/model-runtime/src/core/anthropicCompatibleFactory/index.ts:286-310
//!
//! 除模型列表外，本模块还提供 Anthropic Messages API 的原生协议转换层：
//! - to_anthropic_body：OpenAI 兼容请求体 → Anthropic Messages 请求体（含 cache_control 断点）
//! - handle_anthropic_stream_event：Anthropic SSE 事件 → 统一的 delta 累积（内容/思考/工具调用/用量）
//! - anthropic_text_content / anthropic_usage：非流式响应解析

use reqwest::{Client, RequestBuilder};
use serde_json::json;
use std::collections::BTreeMap;
use std::time::Duration;
use tauri::Emitter;

use super::ProviderPlugin;
use crate::core::models::LiveModel;

pub struct AnthropicProvider;

impl ProviderPlugin for AnthropicProvider {
    fn identifier(&self) -> &'static str {
        "anthropic"
    }

    fn name(&self) -> &'static str {
        "Anthropic"
    }

    fn matches(&self, api_url: &str) -> bool {
        is_anthropic_url(api_url)
    }

    fn build_client(&self, proxy_url: Option<&str>, timeout_secs: u64) -> Result<Client, String> {
        let mut b = Client::builder()
            .user_agent("AIO-Desktop/0.4 (anthropic-provider)")
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(timeout_secs));
        if let Some(p) = proxy_url {
            if !p.trim().is_empty() {
                let proxy = reqwest::Proxy::all(p).map_err(|e| format!("代理 URL 非法: {}", e))?;
                b = b.proxy(proxy);
            }
        }
        b.build()
            .map_err(|e| format!("构造 HTTP 客户端失败: {}", e))
    }

    fn models_url(&self, api_url: &str) -> String {
        let trimmed = api_url.trim_end_matches('/');
        if trimmed.ends_with("/v1") {
            format!("{}/models", trimmed)
        } else {
            format!("{}/v1/models", trimmed)
        }
    }

    fn chat_completions_url(&self, api_url: &str) -> String {
        // 真实 Anthropic API 的聊天端点是 POST /v1/messages（非 /chat/completions）
        let trimmed = api_url.trim_end_matches('/');
        let trimmed = trimmed
            .strip_suffix("/v1")
            .unwrap_or(trimmed)
            .trim_end_matches('/');
        format!("{}/v1/messages", trimmed)
    }

    fn apply_auth(&self, req: RequestBuilder, api_key: &str) -> RequestBuilder {
        // Anthropic 强制要求 x-api-key + anthropic-version 两个头
        if api_key.is_empty() {
            req.header("anthropic-version", "2023-06-01")
        } else {
            req.header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
        }
    }

    fn parse_models(&self, body: &serde_json::Value) -> Vec<LiveModel> {
        let arr = body
            .get("data")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        arr.iter()
            .filter_map(|m| {
                let id = m.get("id").and_then(|x| x.as_str())?.to_string();
                let display_name = m
                    .get("display_name")
                    .and_then(|x| x.as_str())
                    .map(String::from);
                let released_at = m
                    .get("created_at")
                    .and_then(|x| x.as_str())
                    .map(String::from);
                Some(LiveModel {
                    id,
                    owned_by: "Anthropic".to_string(),
                    display_name,
                    released_at,
                })
            })
            .collect()
    }
}

// ===== Anthropic Messages API 转换层 =====

/// 判断是否为真实 Anthropic API 端点（非兼容代理）。
pub fn is_anthropic_url(api_url: &str) -> bool {
    let u = api_url.to_lowercase();
    u.contains("api.anthropic.com")
}

/// 从 OpenAI 兼容请求体转换为 Anthropic Messages API 请求体。
///
/// 转换要点：
/// - system 消息合并到顶层 system（数组形式，最后一块加 cache_control 断点）；
/// - assistant.tool_calls → tool_use 内容块；role:tool 消息 → user 的 tool_result 内容块；
/// - 图片 data URL → image 内容块；tools → Anthropic 工具格式；
/// - max_tokens 必填（缺省 8192）；最后一个 tool 与最后一条消息加 cache_control 断点。
pub fn to_anthropic_body(openai: &serde_json::Value) -> Result<serde_json::Value, String> {
    let mut system_blocks: Vec<serde_json::Value> = Vec::new();
    let mut messages: Vec<serde_json::Value> = Vec::new();

    if let Some(arr) = openai.get("messages").and_then(|m| m.as_array()) {
        for msg in arr {
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            match role {
                "system" => {
                    let text = msg_text(msg);
                    if !text.is_empty() {
                        system_blocks.push(json!({"type": "text", "text": text}));
                    }
                }
                "user" => {
                    let mut blocks = Vec::new();
                    if let Some(content) = msg.get("content") {
                        openai_content_to_blocks(content, &mut blocks);
                    }
                    if blocks.is_empty() {
                        continue;
                    }
                    push_anthropic_message(&mut messages, "user", blocks);
                }
                "assistant" => {
                    let mut blocks = Vec::new();
                    if let Some(content) = msg.get("content") {
                        if let Some(s) = content.as_str() {
                            if !s.is_empty() {
                                blocks.push(json!({"type": "text", "text": s}));
                            }
                        } else {
                            openai_content_to_blocks(content, &mut blocks);
                        }
                    }
                    if let Some(tcs) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                        for tc in tcs {
                            let id = tc
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let name = tc
                                .get("function")
                                .and_then(|f| f.get("name"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let args_str = tc
                                .get("function")
                                .and_then(|f| f.get("arguments"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let input =
                                serde_json::from_str(args_str).unwrap_or_else(|_| json!({}));
                            blocks.push(json!({
                                "type": "tool_use",
                                "id": id,
                                "name": name,
                                "input": input
                            }));
                        }
                    }
                    if blocks.is_empty() {
                        blocks.push(json!({"type": "text", "text": ""}));
                    }
                    push_anthropic_message(&mut messages, "assistant", blocks);
                }
                "tool" => {
                    let tool_use_id = msg
                        .get("tool_call_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let text = match msg.get("content") {
                        Some(serde_json::Value::String(s)) => s.clone(),
                        Some(other) => other.to_string(),
                        None => String::new(),
                    };
                    let block = json!({
                        "type": "tool_result",
                        "tool_use_id": tool_use_id,
                        "content": text
                    });
                    push_anthropic_message(&mut messages, "user", vec![block]);
                }
                _ => {}
            }
        }
    }

    // 最后一条消息的内容块加 cache_control 断点（让下一轮请求缓存到该前缀）
    if let Some(last) = messages.last_mut() {
        if let Some(blocks) = last.get_mut("content").and_then(|c| c.as_array_mut()) {
            if let Some(block) = blocks.last_mut() {
                if let Some(obj) = block.as_object_mut() {
                    obj.insert("cache_control".into(), json!({"type": "ephemeral"}));
                }
            }
        }
    }

    // tools → Anthropic 格式；最后一个工具加 cache_control 断点
    let mut tools_out: Vec<serde_json::Value> = Vec::new();
    if let Some(tools) = openai.get("tools").and_then(|v| v.as_array()) {
        for t in tools {
            let f = t.get("function");
            let name = f
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let description = f
                .and_then(|f| f.get("description"))
                .and_then(|v| v.as_str())
                .map(String::from);
            let input_schema = f
                .and_then(|f| f.get("parameters"))
                .cloned()
                .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
            let mut obj = serde_json::Map::new();
            obj.insert("name".into(), json!(name));
            if let Some(d) = description {
                obj.insert("description".into(), json!(d));
            }
            obj.insert("input_schema".into(), input_schema);
            tools_out.push(serde_json::Value::Object(obj));
        }
        if let Some(last) = tools_out.last_mut() {
            if let Some(obj) = last.as_object_mut() {
                obj.insert("cache_control".into(), json!({"type": "ephemeral"}));
            }
        }
    }

    let mut body = serde_json::Map::new();
    body.insert(
        "model".into(),
        openai.get("model").cloned().unwrap_or(json!("")),
    );
    body.insert("messages".into(), json!(messages));
    // Anthropic 强制要求 max_tokens
    body.insert(
        "max_tokens".into(),
        json!(openai
            .get("max_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(8192)),
    );
    // system 最后一块加 cache_control 断点（缓存完整系统提示词）
    if let Some(last) = system_blocks.last_mut() {
        if let Some(obj) = last.as_object_mut() {
            obj.insert("cache_control".into(), json!({"type": "ephemeral"}));
        }
    }
    if !system_blocks.is_empty() {
        body.insert("system".into(), json!(system_blocks));
    }
    if !tools_out.is_empty() {
        body.insert("tools".into(), json!(tools_out));
    }
    if openai
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        body.insert("stream".into(), json!(true));
    }
    // tool_choice：auto（默认）省略；none → {type:"none"}；指定函数 → {type:"tool", name}
    if let Some(tc) = openai.get("tool_choice") {
        let tc_str = tc.as_str().unwrap_or("");
        if tc_str == "none" {
            body.insert("tool_choice".into(), json!({"type": "none"}));
        } else if tc_str.is_empty() {
            if let Some(name) = tc
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
            {
                body.insert("tool_choice".into(), json!({"type": "tool", "name": name}));
            }
        }
        // "auto" → 不设置（Anthropic 默认即为 auto）
    }
    Ok(serde_json::Value::Object(body))
}

/// 提取消息文本（string 或数组中的 text 部分）。
fn msg_text(msg: &serde_json::Value) -> String {
    match msg.get("content") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(other) => {
            if let Some(arr) = other.as_array() {
                let mut out = String::new();
                for part in arr {
                    if part.get("type").and_then(|v| v.as_str()) == Some("text") {
                        if let Some(s) = part.get("text").and_then(|v| v.as_str()) {
                            out.push_str(s);
                        }
                    }
                }
                out
            } else {
                other.to_string()
            }
        }
        None => String::new(),
    }
}

/// 把 OpenAI 的 content（string 或数组）转换为 Anthropic 内容块。
fn openai_content_to_blocks(content: &serde_json::Value, out: &mut Vec<serde_json::Value>) {
    match content {
        serde_json::Value::String(s) => {
            if !s.is_empty() {
                out.push(json!({"type": "text", "text": s}));
            }
        }
        serde_json::Value::Array(parts) => {
            for part in parts {
                let ptype = part.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match ptype {
                    "text" => {
                        if let Some(s) = part.get("text").and_then(|v| v.as_str()) {
                            out.push(json!({"type": "text", "text": s}));
                        }
                    }
                    "image_url" => {
                        if let Some(url) = part
                            .get("image_url")
                            .and_then(|v| v.get("url"))
                            .and_then(|v| v.as_str())
                        {
                            if let Some(rest) = url.strip_prefix("data:") {
                                let (meta, b64) = match rest.split_once(',') {
                                    Some((m, b)) => (m.to_string(), b.to_string()),
                                    None => ("image/png".to_string(), url.to_string()),
                                };
                                let media_type =
                                    meta.split(';').next().unwrap_or("image/png").to_string();
                                out.push(json!({
                                    "type": "image",
                                    "source": {
                                        "type": "base64",
                                        "media_type": media_type,
                                        "data": b64
                                    }
                                }));
                            } else {
                                out.push(json!({
                                    "type": "image",
                                    "source": {"type": "url", "url": url}
                                }));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// 追加一条 Anthropic 消息；相邻同角色消息合并（如连续多个 tool_result 合并为一个 user 消息）。
fn push_anthropic_message(
    messages: &mut Vec<serde_json::Value>,
    role: &str,
    blocks: Vec<serde_json::Value>,
) {
    if let Some(last) = messages.last_mut() {
        if last.get("role").and_then(|v| v.as_str()) == Some(role) {
            if let Some(existing) = last.get_mut("content").and_then(|c| c.as_array_mut()) {
                existing.extend(blocks);
                return;
            }
        }
    }
    messages.push(json!({"role": role, "content": blocks}));
}

/// 从非流式响应提取首个文本块。
pub fn anthropic_text_content(body: &serde_json::Value) -> String {
    body.get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| {
            arr.iter()
                .find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
        })
        .and_then(|b| b.get("text"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// 从响应提取用量 (input_tokens, output_tokens, cache_read_input_tokens)。
/// 当前供非流式路径的后续用量统计使用（已有单测覆盖）。
#[allow(dead_code)]
pub fn anthropic_usage(body: &serde_json::Value) -> (u32, u32, u32) {
    let u = body.get("usage");
    let input = u
        .and_then(|x| x.get("input_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let output = u
        .and_then(|x| x.get("output_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let cached = u
        .and_then(|x| x.get("cache_read_input_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    (input, output, cached)
}

/// 处理一条 Anthropic 流式事件，累积内容/思考/工具调用/用量并向前端发射增量事件。
#[allow(clippy::too_many_arguments)]
pub fn handle_anthropic_stream_event(
    val: &serde_json::Value,
    window: &tauri::Window,
    assistant_id: &str,
    topic_id: &str,
    content_buf: &mut String,
    reasoning_buf: &mut String,
    tc_accum: &mut BTreeMap<usize, (String, String, String)>,
    finish_reason: &mut Option<String>,
    input_tokens: &mut u32,
    output_tokens: &mut u32,
    cached_input_tokens: &mut u32,
) {
    let etype = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match etype {
        "message_start" => {
            if let Some(usage) = val.get("message").and_then(|m| m.get("usage")) {
                if let Some(v) = usage.get("input_tokens").and_then(|x| x.as_u64()) {
                    *input_tokens = v as u32;
                }
                if let Some(v) = usage
                    .get("cache_read_input_tokens")
                    .and_then(|x| x.as_u64())
                {
                    *cached_input_tokens = v as u32;
                }
            }
        }
        "content_block_start" => {
            let index = val.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let block = val.get("content_block");
            let btype = block
                .and_then(|b| b.get("type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if btype == "tool_use" {
                let id = block
                    .and_then(|b| b.get("id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = block
                    .and_then(|b| b.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                tc_accum.insert(index, (id.clone(), name.clone(), String::new()));
                if !id.is_empty() && !name.is_empty() {
                    let _ = window.emit(
                        "llm-tool-call",
                        crate::commands::llm::ToolCallPayload {
                            assistant_id: assistant_id.to_string(),
                            topic_id: topic_id.to_string(),
                            tool_call_id: id,
                            name,
                            arguments: String::new(),
                        },
                    );
                }
            }
        }
        "content_block_delta" => {
            let index = val.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let delta = val.get("delta");
            let dtype = delta
                .and_then(|d| d.get("type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match dtype {
                "text_delta" => {
                    if let Some(s) = delta.and_then(|d| d.get("text")).and_then(|v| v.as_str()) {
                        content_buf.push_str(s);
                        let _ = window.emit(
                            "llm-chunk",
                            crate::core::models::StreamPayload {
                                assistant_id: assistant_id.to_string(),
                                topic_id: topic_id.to_string(),
                                content: s.to_string(),
                                done: false,
                                error: None,
                                input_tokens: None,
                                output_tokens: None,
                                context_tokens: None,
                                cached_input_tokens: None,
                                images: None,
                            },
                        );
                    }
                }
                "thinking_delta" => {
                    if let Some(s) = delta
                        .and_then(|d| d.get("thinking"))
                        .and_then(|v| v.as_str())
                    {
                        reasoning_buf.push_str(s);
                        let _ = window.emit(
                            "llm-reasoning",
                            crate::core::models::StreamPayload {
                                assistant_id: assistant_id.to_string(),
                                topic_id: topic_id.to_string(),
                                content: s.to_string(),
                                done: false,
                                error: None,
                                input_tokens: None,
                                output_tokens: None,
                                context_tokens: None,
                                cached_input_tokens: None,
                                images: None,
                            },
                        );
                    }
                }
                "input_json_delta" => {
                    if let Some(s) = delta
                        .and_then(|d| d.get("partial_json"))
                        .and_then(|v| v.as_str())
                    {
                        if let Some(entry) = tc_accum.get_mut(&index) {
                            entry.2.push_str(s);
                        }
                    }
                }
                _ => {}
            }
        }
        "message_delta" => {
            if let Some(usage) = val.get("usage") {
                if let Some(v) = usage.get("output_tokens").and_then(|x| x.as_u64()) {
                    *output_tokens = v as u32;
                }
                if let Some(v) = usage
                    .get("cache_read_input_tokens")
                    .and_then(|x| x.as_u64())
                {
                    *cached_input_tokens = v as u32;
                }
            }
            if let Some(sr) = val
                .get("delta")
                .and_then(|d| d.get("stop_reason"))
                .and_then(|v| v.as_str())
            {
                *finish_reason = Some(match sr {
                    "max_tokens" => "length".to_string(),
                    "tool_use" => "tool_calls".to_string(),
                    _ => "stop".to_string(),
                });
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_openai_body_to_anthropic() {
        let openai = json!({
            "model": "claude-3-5-sonnet-latest",
            "messages": [
                {"role": "system", "content": "你是助手"},
                {"role": "user", "content": "你好"},
                {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "call_1", "type": "function", "function": {"name": "read_file", "arguments": "{\"path\":\"a.rs\"}"}}
                ]},
                {"role": "tool", "tool_call_id": "call_1", "content": "file content"}
            ],
            "tools": [{"type": "function", "function": {"name": "read_file", "description": "读取", "parameters": {"type": "object", "properties": {}}}}],
            "stream": true,
            "max_tokens": 1024,
            "tool_choice": "auto"
        });
        let body = to_anthropic_body(&openai).unwrap();
        assert_eq!(body["model"], "claude-3-5-sonnet-latest");
        assert_eq!(body["max_tokens"], 1024);
        let sys = body["system"].as_array().unwrap();
        assert_eq!(sys[0]["type"], "text");
        assert_eq!(sys.last().unwrap()["cache_control"]["type"], "ephemeral");
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[0]["content"][0]["type"], "text");
        assert_eq!(msgs[1]["role"], "assistant");
        assert_eq!(msgs[1]["content"][0]["type"], "tool_use");
        assert_eq!(msgs[1]["content"][0]["id"], "call_1");
        assert_eq!(msgs[1]["content"][0]["input"]["path"], "a.rs");
        assert_eq!(msgs[2]["role"], "user");
        assert_eq!(msgs[2]["content"][0]["type"], "tool_result");
        assert_eq!(msgs[2]["content"][0]["tool_use_id"], "call_1");
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools[0]["name"], "read_file");
        assert_eq!(tools[0]["input_schema"]["type"], "object");
        assert_eq!(tools.last().unwrap()["cache_control"]["type"], "ephemeral");
        let last = msgs.last().unwrap();
        let last_block = last["content"].as_array().unwrap().last().unwrap();
        assert_eq!(last_block["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn merges_consecutive_tool_results() {
        let openai = json!({
            "messages": [
                {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "c1", "type": "function", "function": {"name": "a", "arguments": "{}"}},
                    {"id": "c2", "type": "function", "function": {"name": "b", "arguments": "{}"}}
                ]},
                {"role": "tool", "tool_call_id": "c1", "content": "r1"},
                {"role": "tool", "tool_call_id": "c2", "content": "r2"}
            ]
        });
        let body = to_anthropic_body(&openai).unwrap();
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["content"].as_array().unwrap().len(), 2);
        assert_eq!(msgs[1]["content"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn maps_image_data_url() {
        let openai = json!({
            "messages": [
                {"role": "user", "content": [
                    {"type": "text", "text": "看图"},
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA"}}
                ]}
            ]
        });
        let body = to_anthropic_body(&openai).unwrap();
        let blocks = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(blocks[1]["type"], "image");
        assert_eq!(blocks[1]["source"]["media_type"], "image/png");
        assert_eq!(blocks[1]["source"]["data"], "AAAA");
    }

    #[test]
    fn defaults_max_tokens_and_parses_text_usage() {
        let body = to_anthropic_body(
            &json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]}),
        )
        .unwrap();
        assert_eq!(body["max_tokens"], 8192);

        let resp = json!({
            "content": [{"type": "text", "text": "答案"}, {"type": "tool_use", "id": "x"}],
            "usage": {"input_tokens": 10, "output_tokens": 5, "cache_read_input_tokens": 7}
        });
        assert_eq!(anthropic_text_content(&resp), "答案");
        let (i, o, c) = anthropic_usage(&resp);
        assert_eq!((i, o, c), (10, 5, 7));
    }

    #[test]
    fn detects_anthropic_url() {
        assert!(is_anthropic_url("https://api.anthropic.com"));
        assert!(!is_anthropic_url("https://api.openai.com"));
    }
}

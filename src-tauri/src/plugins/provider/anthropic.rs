//! Anthropic provider plugin
//!
//! Endpoint spec:
//! - URL: ${base}/v1/models (base = https://api.anthropic.com, without version suffix)
//! - Auth: x-api-key: <key> + anthropic-version: 2023-06-01 (must not use Authorization: Bearer)
//! - Response: {data: [{id, display_name, created_at}]} - uses the id field, same as OpenAI-compatible
//!
//! Reference: LobeHub packages/model-runtime/src/core/anthropicCompatibleFactory/index.ts:286-310
//!
//! Beyond model listing, this module also provides the native protocol conversion layer for the Anthropic Messages API:
//! - to_anthropic_body: OpenAI-compatible request body -> Anthropic Messages request body (with cache_control breakpoints)
//! - handle_anthropic_stream_event: Anthropic SSE events -> unified delta accumulation (content/thinking/tool calls/usage)
//! - anthropic_text_content / anthropic_usage: non-streaming response parsing

use reqwest::RequestBuilder;
use serde_json::json;
use std::collections::BTreeMap;
use tauri::Emitter;

use super::ProviderPlugin;
use crate::core::events as ev;
use crate::core::models::LiveModel;

pub struct AnthropicProvider;

impl ProviderPlugin for AnthropicProvider {
    fn identifier(&self) -> &'static str {
        "anthropic"
    }

    fn matches(&self, api_url: &str) -> bool {
        is_anthropic_url(api_url)
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
        // The real Anthropic API chat endpoint is POST /v1/messages (not /chat/completions)
        let trimmed = api_url.trim_end_matches('/');
        let trimmed = trimmed
            .strip_suffix("/v1")
            .unwrap_or(trimmed)
            .trim_end_matches('/');
        format!("{}/v1/messages", trimmed)
    }

    fn apply_auth(&self, req: RequestBuilder, api_key: &str) -> RequestBuilder {
        // Anthropic requires both the x-api-key and anthropic-version headers
        if api_key.is_empty() {
            req.header("anthropic-version", "2023-06-01")
        } else {
            req.header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
        }
    }

    /// 阶段4 第5项（聊天路径统一出口）: the chat endpoint /v1/messages uses the
    /// same native auth as the models endpoint.
    fn apply_chat_auth(&self, req: RequestBuilder, api_key: &str) -> RequestBuilder {
        self.apply_auth(req, api_key)
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

// ===== Anthropic Messages API conversion layer =====

/// Returns whether the URL points at the real Anthropic API host (not a compatible proxy).
/// PROV-5: host-suffix match — the historic whole-string `contains` check let
/// `api.anthropic.com.evil.io` claim the native plugin and receive x-api-key.
pub fn is_anthropic_url(api_url: &str) -> bool {
    super::url_host_matches(api_url, "api.anthropic.com")
}

/// Converts an OpenAI-compatible request body into an Anthropic Messages API request body.
///
/// Conversion essentials:
/// - system messages merge into the top-level system (array form, cache_control breakpoint on the last block);
/// - assistant.tool_calls -> tool_use content blocks; role:tool messages -> user tool_result content blocks;
/// - image data URLs -> image content blocks; tools -> Anthropic tool format;
/// - max_tokens is required (defaults to 8192); the last tool and the last message get a cache_control breakpoint.
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

    // Add a cache_control breakpoint to the last content block of the last message (so the next turn's request caches up to that prefix)
    if let Some(last) = messages.last_mut() {
        if let Some(blocks) = last.get_mut("content").and_then(|c| c.as_array_mut()) {
            if let Some(block) = blocks.last_mut() {
                if let Some(obj) = block.as_object_mut() {
                    obj.insert("cache_control".into(), json!({"type": "ephemeral"}));
                }
            }
        }
    }

    // tools -> Anthropic format; the last tool gets a cache_control breakpoint
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
    // Anthropic requires max_tokens
    body.insert(
        "max_tokens".into(),
        json!(openai
            .get("max_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(8192)),
    );
    // Add a cache_control breakpoint to the last system block (caches the full system prompt)
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
    // PROV-14: forward sampling params — the historic conversion silently dropped
    // temperature/top_p, so Anthropic turns always ran at the API default regardless
    // of the configured value.
    if let Some(t) = openai.get("temperature").and_then(|v| v.as_f64()) {
        body.insert("temperature".into(), json!(t));
    }
    if let Some(p) = openai.get("top_p").and_then(|v| v.as_f64()) {
        body.insert("top_p".into(), json!(p));
    }
    // Reasoning effort -> extended thinking budget (low 2048 / medium 8192 / high 16384
    // tokens; the API floor is 1024). Anthropic requires max_tokens > budget_tokens, so
    // the cap is raised when needed, and temperature is dropped (thinking pins it to 1).
    if let Some(effort) = openai.get("reasoning_effort").and_then(|v| v.as_str()) {
        let budget: u64 = match effort {
            "low" => 2048,
            "medium" => 8192,
            "high" => 16384,
            _ => 0,
        };
        if budget > 0 {
            body.insert(
                "thinking".into(),
                json!({"type": "enabled", "budget_tokens": budget}),
            );
            let max_tokens = body
                .get("max_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(8192);
            if max_tokens <= budget {
                body.insert("max_tokens".into(), json!(budget + 8192));
            }
            body.remove("temperature");
        }
    }
    // tool_choice: auto (default) omitted; none -> {type:"none"}; a specific function -> {type:"tool", name}
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
        // "auto" -> not set (Anthropic already defaults to auto)
    }
    Ok(serde_json::Value::Object(body))
}

/// Extracts message text (a string, or the text parts of an array).
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

/// Converts OpenAI content (string or array) into Anthropic content blocks.
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

/// Appends an Anthropic message; adjacent same-role messages are merged (e.g. consecutive tool_results merge into one user message).
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

/// Extracts the first text block from a non-streaming response.
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

/// Extracts usage (input_tokens, output_tokens, cache_read_input_tokens) from a response.
/// Currently used for follow-up usage statistics on the non-streaming path (covered by unit tests).
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

/// Anthropic SSE: extracts the text delta from content_block_delta (returns None for non-text_delta).
/// Shared by the main loop's handle_anthropic_stream_event and the btw side path.
pub fn anthropic_sse_text_delta(val: &serde_json::Value) -> Option<&str> {
    if val.get("type").and_then(|v| v.as_str()) != Some("content_block_delta") {
        return None;
    }
    let delta = val.get("delta")?;
    if delta.get("type").and_then(|v| v.as_str()) != Some("text_delta") {
        return None;
    }
    delta.get("text").and_then(|v| v.as_str())
}

/// Anthropic SSE: whether the event is message_stop (end of stream).
pub fn anthropic_sse_is_message_stop(val: &serde_json::Value) -> bool {
    val.get("type").and_then(|v| v.as_str()) == Some("message_stop")
}

/// Anthropic SSE: extracts (input_tokens, cache_read_input_tokens) from message_start.
/// A None second element means the event carries no cache-hit count (the caller must keep the previous value, not zero it).
pub fn anthropic_sse_message_usage(val: &serde_json::Value) -> Option<(u32, Option<u32>)> {
    let usage = val.get("message")?.get("usage")?;
    let input = usage.get("input_tokens")?.as_u64()? as u32;
    let cached = usage
        .get("cache_read_input_tokens")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);
    Some((input, cached))
}

/// Handles one Anthropic stream event: accumulates content/thinking/tool calls/usage and emits delta events to the frontend.
/// Returns `Some(err)` on a fatal `error` event (PROV-2: previously swallowed by
/// the `_ => {}` arm — the truncated reply was then persisted as a success).
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
    suppress_events: bool,
) -> Option<String> {
    let etype = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match etype {
        "message_start" => {
            if let Some((inp, cached)) = anthropic_sse_message_usage(val) {
                // PROV-15: normalize to OpenAI prompt_tokens semantics, where cached
                // tokens are a SUBSET of the prompt total. Anthropic reports
                // input_tokens WITHOUT cache_read_input_tokens, so the unified
                // input_tokens / context-window display and the cache-hit ratio
                // used provider-inconsistent denominators before.
                *input_tokens = inp.saturating_add(cached.unwrap_or(0));
                // Keep the previous value when the event carries no cache-hit count (do not overwrite with 0)
                if let Some(c) = cached {
                    *cached_input_tokens = c;
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
                if !id.is_empty() && !name.is_empty() && !suppress_events {
                    let _ = window.emit(
                        ev::LLM_TOOL_CALL,
                        crate::core::models::ToolCallPayload {
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
                    if let Some(s) = anthropic_sse_text_delta(val) {
                        content_buf.push_str(s);
                        if !suppress_events {
                            let _ = window.emit(
                                ev::LLM_CHUNK,
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
                                    ttft_ms: None,
                                    images: None,
                                },
                            );
                        }
                    }
                }
                "thinking_delta" => {
                    if let Some(s) = delta
                        .and_then(|d| d.get("thinking"))
                        .and_then(|v| v.as_str())
                    {
                        reasoning_buf.push_str(s);
                        if !suppress_events {
                            let _ = window.emit(
                                ev::LLM_REASONING,
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
                                    ttft_ms: None,
                                    images: None,
                                },
                            );
                        }
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
        // PROV-2: mid-stream error events surface instead of being swallowed
        "error" => {
            return Some(
                anthropic_sse_error_message(val)
                    .unwrap_or_else(|| "Anthropic stream error".to_string()),
            );
        }
        _ => {}
    }
    None
}

/// PROV-2: 提取 Anthropic `error` 流事件的消息（None = 非 error 事件或无消息）。
pub fn anthropic_sse_error_message(val: &serde_json::Value) -> Option<String> {
    if val.get("type").and_then(|v| v.as_str()) != Some("error") {
        return None;
    }
    val.get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PROV-2 回归：error 事件的消息必须被提取出来（此前被 `_ => {}` 吞掉，
    /// 截断回复被当成功持久化）。
    #[test]
    fn anthropic_stream_error_event_surfaces() {
        let val = json!({
            "type": "error",
            "error": {"type": "overloaded_error", "message": "Overloaded"}
        });
        assert_eq!(
            anthropic_sse_error_message(&val),
            Some("Overloaded".to_string())
        );
        let not_error = json!({"type": "message_delta", "usage": {"output_tokens": 3}});
        assert_eq!(anthropic_sse_error_message(&not_error), None);
    }

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
        // PROV-5: host-suffix dispatch — the historic whole-string contains check
        // let these look-alikes claim the native plugin (x-api-key sent to a foreign host).
        assert!(!is_anthropic_url("https://api.anthropic.com.evil.io"));
        assert!(!is_anthropic_url("https://evil.io/proxy/api.anthropic.com"));
        assert!(!is_anthropic_url("https://notanthropic.com"));
        // Real subdomains of the official host still match.
        assert!(is_anthropic_url("https://proxy.api.anthropic.com"));
        assert!(is_anthropic_url("https://api.anthropic.com/v1/messages"));
        // Schemeless input is parsed against https://.
        assert!(is_anthropic_url("api.anthropic.com/v1"));
    }

    /// PROV-14: temperature/top_p must carry over — the historic conversion
    /// silently dropped them, so Anthropic turns always ran at the API default.
    #[test]
    fn sampling_params_are_forwarded() {
        let body = to_anthropic_body(&json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}],
            "temperature": 0.2,
            "top_p": 0.9
        }))
        .unwrap();
        assert_eq!(body["temperature"], 0.2);
        assert_eq!(body["top_p"], 0.9);
        // Absent params stay absent (Anthropic default applies)
        let body2 = to_anthropic_body(&json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .unwrap();
        assert!(body2.get("temperature").is_none());
        assert!(body2.get("top_p").is_none());
    }

    /// Reasoning effort maps to an extended-thinking budget; the max_tokens cap is
    /// raised above the budget (API requirement) and temperature is dropped.
    #[test]
    fn reasoning_effort_maps_to_thinking_budget() {
        // low: budget 2048, default max_tokens 8192 already above it -> untouched
        let low = to_anthropic_body(&json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}],
            "reasoning_effort": "low",
            "temperature": 0.2
        }))
        .unwrap();
        assert_eq!(low["thinking"]["type"], "enabled");
        assert_eq!(low["thinking"]["budget_tokens"], 2048);
        assert_eq!(low["max_tokens"], 8192);
        assert!(low.get("temperature").is_none());

        // high: budget 16384 > default max_tokens 8192 -> cap raised above the budget
        let high = to_anthropic_body(&json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}],
            "reasoning_effort": "high"
        }))
        .unwrap();
        assert_eq!(high["thinking"]["budget_tokens"], 16384);
        assert!(high["max_tokens"].as_u64().unwrap() > 16384);

        // unknown effort value: no thinking block
        let none = to_anthropic_body(&json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}],
            "reasoning_effort": "minimal"
        }))
        .unwrap();
        assert!(none.get("thinking").is_none());
    }

    /// PROV-15: the unified input_tokens must follow OpenAI prompt_tokens
    /// semantics (cached ⊂ total). Anthropic's input_tokens EXCLUDES
    /// cache_read_input_tokens — the message_start handler adds them back so
    /// the context-window display and cache-hit ratio use one denominator.
    #[test]
    fn message_start_input_tokens_include_cached() {
        let ev = json!({
            "type": "message_start",
            "message": {"usage": {"input_tokens": 30, "cache_read_input_tokens": 70}}
        });
        let (inp, cached) = anthropic_sse_message_usage(&ev).expect("usage 应解析");
        assert_eq!((inp, cached), (30, Some(70)));
        // Normalized total = 100 (matches OpenAI prompt_tokens semantics)
        assert_eq!(inp + cached.unwrap_or(0), 100);
        // No cache-hit count: total stays the reported input
        let ev2 = json!({
            "type": "message_start",
            "message": {"usage": {"input_tokens": 42}}
        });
        let (inp2, cached2) = anthropic_sse_message_usage(&ev2).expect("usage 应解析");
        assert_eq!((inp2, cached2), (42, None));
        assert_eq!(inp2 + cached2.unwrap_or(0), 42);
    }
}

pub mod btw;

use crate::commands::attachment::sync_message_attachments;
use crate::core::events as ev;
use crate::core::models::*;
use crate::core::permission::{self, PermissionAction};
use crate::core::state::DbState;
use crate::core::state::StreamManager;
use crate::core::subagent;
use crate::ierr;
use crate::plugins::mcp::PendingApprovals;
use crate::utils::diff::compute_diff_preview;
use crate::utils::file_tools;
use crate::utils::git_tools;
use crate::utils::lsp_agent_tools;
use crate::utils::lsp_tools;
use crate::utils::shell_tools;
use crate::utils::truncate::{truncate_chars_head, truncate_chars_head_tail, truncate_tool_result};
use crate::utils::web_tools;
use base64::engine::Engine as _;
use futures_util::stream::FuturesUnordered;
use futures_util::StreamExt; // for handling streaming data
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, Window}; // Emitter pushes backend->frontend events
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

pub(crate) mod budget;
pub(crate) mod checkpoints;
pub(crate) mod client;
pub(crate) mod compress;
pub(crate) mod handoff;
pub(crate) mod locked_tools;
pub(crate) mod messages;
pub(crate) mod subagents;
pub(crate) mod tasks;
pub(crate) mod titles;
pub(crate) mod tool_dispatch;
pub(crate) mod tool_specs;
pub(crate) mod usage;
pub(crate) mod vision;

pub(crate) use budget::*;
pub(crate) use checkpoints::*;
pub(crate) use client::*;
pub(crate) use compress::*;
pub(crate) use handoff::*;
pub(crate) use locked_tools::*;
pub(crate) use messages::*;
pub(crate) use subagents::*;
pub(crate) use tasks::*;
pub(crate) use titles::*;
pub(crate) use tool_dispatch::*;
pub(crate) use tool_specs::*;
pub(crate) use usage::*;
pub(crate) use vision::*;
/// Round and context budgets for the agent loop (main loop derives from model window;
/// subagents use conservative constants but honor explicit user config for rounds).
///
/// Both main and subagent loops derive budgets here so behavior stays aligned; the
/// subagent default is 100 rounds / 45k tokens, overridable by `AppConfig.max_tool_rounds`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AgentLimits {
    /// Max model-call rounds per run (runaway guard; on hit, finish gracefully with a progress summary)
    pub max_rounds: u32,
    /// Context token budget: exceeding it triggers compression
    pub context_budget: usize,
    /// Token budget for the retained recent region after compression
    pub keep_recent_tokens: usize,
}

impl AgentLimits {
    /// Subagent budget: default 100-round runaway guard (no low round cap; context
    /// management is the backstop); explicit user max_tool_rounds takes precedence.
    /// Context safety valve is a conservative constant (no model window info needed; safe for >=64k windows).
    pub(crate) fn for_subagent(cfg: &AppConfig) -> Self {
        /// Runaway-guard round count (subagent default cap)
        const GUARD_ROUNDS: u32 = 100;
        /// Conservative context budget (tokens)
        const CONTEXT_BUDGET: usize = 45_000;
        Self {
            max_rounds: cfg.max_tool_rounds.unwrap_or(GUARD_ROUNDS),
            context_budget: CONTEXT_BUDGET,
            keep_recent_tokens: (CONTEXT_BUDGET / 2).clamp(4_000, 20_000),
        }
    }
}

/// Accumulated result of one streaming round.
pub(crate) struct RoundResult {
    /// Assistant text content for this round (with rewritten aio-image markers)
    content: String,
    /// Chain-of-thought for this round (already streamed live via llm-reasoning events; kept only for debugging)
    #[allow(dead_code)]
    reasoning: String,
    /// Tool calls issued by the model this round (ascending by index)
    tool_calls: Vec<ToolCallAccum>,
    /// Server-reported prompt tokens (input usage)
    input_tokens: u32,
    /// Server-reported completion tokens (output usage)
    output_tokens: u32,
    /// Cache-hit input tokens (OpenAI cached_tokens / DeepSeek prompt_cache_hit_tokens)
    cached_input_tokens: u32,
    /// Metadata of images generated/saved this round
    images: Vec<GeneratedImage>,
    /// Raw payload shape resent to the API / persisted (unrewritten-token array or string), used for the next round
    api_content: serde_json::Value,
    /// Server-reported finish_reason (e.g. "stop" / "tool_calls" / "length"), used for truncation recovery
    finish_reason: Option<String>,
}

/// A single fully-accumulated tool call.
#[derive(Clone)]
pub(crate) struct ToolCallAccum {
    id: String,
    name: String,
    arguments: String,
}

// ToolCallPayload now lives in core::models (chat domain; plugins/provider needs it);
// re-exported here to keep the crate::commands::llm::ToolCallPayload path stable.
pub use crate::core::models::ToolCallPayload;

/// Defensive check: scans messages for assistant(tool_calls) vs role:tool pairing,
/// logging a warning on missing or mismatched entries to aid debugging.
fn verify_tool_messages(messages: &[serde_json::Value]) {
    // Scan forward, tracking tool_call_ids declared by each assistant message
    let mut pending_ids: Vec<String> = Vec::new();
    let mut assistant_idx: Option<usize> = None;

    for (i, msg) in messages.iter().enumerate() {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if role == "assistant" {
            if let Some(tcs) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                if !tcs.is_empty() {
                    // New assistant(tool_calls) starts while earlier pending_ids are still unmatched -> missing responses
                    if !pending_ids.is_empty() {
                        tracing::warn!(
                            "[verify_tool_messages] assistant[{}] 的 tool_calls {:?} 缺少对应 tool 响应",
                            assistant_idx.unwrap_or(0),
                            pending_ids
                        );
                    }
                    pending_ids = tcs
                        .iter()
                        .filter_map(|tc| tc.get("id").and_then(|v| v.as_str()).map(String::from))
                        .collect();
                    assistant_idx = Some(i);
                }
            }
        } else if role == "tool" {
            if let Some(tool_call_id) = msg.get("tool_call_id").and_then(|v| v.as_str()) {
                if let Some(pos) = pending_ids.iter().position(|id| id == tool_call_id) {
                    pending_ids.remove(pos);
                } else {
                    tracing::warn!(
                        "[verify_tool_messages] tool[{}] 的 tool_call_id `{}` 在前一条 assistant 中未找到对应的 tool_call",
                        i,
                        tool_call_id
                    );
                }
            }
        }
    }

    // Scan finished with tool_call_ids still unmatched
    if !pending_ids.is_empty() {
        tracing::warn!(
            "[verify_tool_messages] 扫描结束: assistant[{}] 的 tool_calls {:?} 缺少对应 tool 响应",
            assistant_idx.unwrap_or(0),
            pending_ids
        );
    }
}

/// One streaming round: build body -> POST -> parse SSE -> accumulate content/reasoning/tool_calls -> emit delta events.
///
/// Differences from the legacy `call_llm_stream`:
/// - accumulators use `BTreeMap` (ordered by index) to avoid multi-tool reordering;
/// - `tokio::select!` watches `token.cancelled()`, returning `Err("cancelled")` immediately
///   so the caller can still run its epilogue;
/// - does **not** emit terminal `llm-chunk(done)`; the caller (`run_agent_turn`) owns the epilogue;
/// - tool_call only emits `llm-tool-call` (frontend "running" bubble); the loop body executes them.
///
/// Returns `RoundResult`; `Err("cancelled")` on cancellation; other errors propagate unchanged.
#[allow(
    clippy::too_many_arguments,
    reason = "stream orchestration keeps request and event context explicit"
)]
pub(crate) async fn stream_one_round(
    window: &Window,
    token: &CancellationToken,
    client: &reqwest::Client,
    api_url: String,
    api_key: &str,
    model: &str,
    messages: &[serde_json::Value],
    tools: Option<&[ToolSpec]>,
    assistant_id: &str,
    topic_id: &str,
    suppress_events: bool,
    app: Option<&AppHandle>,
    process_images: bool,
    max_output_tokens: Option<u32>,
    max_retries: u32,
) -> Result<RoundResult, String> {
    let mut body_map = serde_json::Map::new();
    body_map.insert("model".into(), json!(model));
    body_map.insert("messages".into(), json!(messages));
    body_map.insert("stream".into(), json!(true));
    if let Some(tools) = tools {
        if !tools.is_empty() {
            body_map.insert("tools".into(), json!(tools));
            body_map.insert("tool_choice".into(), json!("auto"));
        }
    }
    // Optional output length cap: prevents unbounded single-round output (reasoning
    // models like o1/o3/gpt-5 need max_completion_tokens; the caller decides per model)
    if let Some(mt) = max_output_tokens {
        body_map.insert("max_tokens".into(), json!(mt));
    }
    let body = serde_json::Value::Object(body_map);

    // Unified request entry with pre-stream retries: adapts to the Anthropic native protocol;
    // transport errors and 429/5xx retry with backoff only before any content is emitted; mid-stream errors are not retried
    let (response, is_anthropic_request) = crate::plugins::llm::post_chat_completion_with_retry(
        client,
        &api_url,
        api_key,
        &body,
        max_retries,
    )
    .await?;

    let mut stream = response.bytes_stream();
    let mut line_buffer = String::new();
    // tool_call accumulator: index -> (id, name, arguments); BTreeMap keeps flush ordered by index
    let mut tc_accum: BTreeMap<usize, (String, String, String)> = BTreeMap::new();

    let mut content_buf = String::new();
    let mut reasoning_buf = String::new();
    // Server-reported finish_reason ("stop" / "tool_calls" / "length"), for caller-side truncation recovery
    let mut finish_reason: Option<String> = None;
    // image_urls parsed from array deltas plus raw API payload parts (resent unrewritten next round to avoid token leakage)
    let mut image_urls: Vec<String> = Vec::new();
    let mut round_api_parts: Vec<serde_json::Value> = Vec::new();
    let mut saw_done = false;
    // Token usage extracted from the SSE stream tail (server-reported)
    let mut input_tokens: u32 = 0;
    let mut output_tokens: u32 = 0;
    let mut cached_input_tokens: u32 = 0;

    loop {
        // Three-way race: cancel signal / stream chunk / 120s inactivity timeout
        let next = tokio::select! {
            _ = token.cancelled() => return Err("cancelled".to_string()),
            item = stream.next() => item,
            _ = tokio::time::sleep(std::time::Duration::from_secs(120)) => {
                return Err(ierr!(
                    "LLM 流超时：120 秒未收到数据，连接可能已断开",
                    "LLM stream timed out: no data for 120s, the connection may have dropped"
                ));
            }
        };

        let chunk = match next {
            Some(Ok(c)) => c,
            Some(Err(e)) => return Err(e.to_string()),
            None => break, // stream ended naturally
        };
        line_buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = line_buffer.find('\n') {
            let line = line_buffer[..pos].trim().to_string();
            line_buffer.drain(..pos + 1);

            if line.is_empty() {
                continue;
            }

            if line == "data: [DONE]" {
                saw_done = true;
                break;
            }

            if let Some(json_str) = line.strip_prefix("data: ") {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                    if is_anthropic_request {
                        // Anthropic Messages stream events: content/thinking/tool calls/usage accumulated uniformly
                        crate::plugins::provider::anthropic::handle_anthropic_stream_event(
                            &val,
                            window,
                            assistant_id,
                            topic_id,
                            &mut content_buf,
                            &mut reasoning_buf,
                            &mut tc_accum,
                            &mut finish_reason,
                            &mut input_tokens,
                            &mut output_tokens,
                            &mut cached_input_tokens,
                        );
                        if val.get("type").and_then(|v| v.as_str()) == Some("message_stop") {
                            saw_done = true;
                            break;
                        }
                    } else if let Some(content) = val["choices"][0]["delta"]["content"].as_str() {
                        content_buf.push_str(content);
                        if !suppress_events {
                            let _ = window.emit(
                                ev::LLM_CHUNK,
                                StreamPayload {
                                    assistant_id: assistant_id.to_string(),
                                    topic_id: topic_id.to_string(),
                                    content: content.to_string(),
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
                    } else if let Some(parts) = val["choices"][0]["delta"]["content"].as_array() {
                        // Image-capable models (vLLM Qwen-Image, OpenAI gpt-image-1 chat mode, etc.)
                        // deliver text and image blocks as an array: text streams per block; image URLs are collected and saved/rewritten after the stream.
                        for part in parts {
                            match part.get("type").and_then(|v| v.as_str()) {
                                Some("text") => {
                                    if let Some(s) = part.get("text").and_then(|v| v.as_str()) {
                                        content_buf.push_str(s);
                                        if !suppress_events {
                                            let _ = window.emit(
                                                ev::LLM_CHUNK,
                                                StreamPayload {
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
                                        round_api_parts.push(json!({ "type": "text", "text": s }));
                                    }
                                }
                                Some("image_url") | Some("output_image") => {
                                    let url = part
                                        .get("image_url")
                                        .and_then(|v| v.get("url"))
                                        .and_then(|v| v.as_str())
                                        .or_else(|| part.get("url").and_then(|v| v.as_str()));
                                    if let Some(u) = url {
                                        image_urls.push(u.to_string());
                                        round_api_parts.push(json!({
                                            "type": "image_url",
                                            "image_url": { "url": u }
                                        }));
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    if let Some(reasoning) = val["choices"][0]["delta"]["reasoning_content"]
                        .as_str()
                        .or_else(|| val["choices"][0]["delta"]["reasoning"].as_str())
                    {
                        if !reasoning.is_empty() {
                            reasoning_buf.push_str(reasoning);
                            if !suppress_events {
                                let _ = window.emit(
                                    ev::LLM_REASONING,
                                    StreamPayload {
                                        assistant_id: assistant_id.to_string(),
                                        topic_id: topic_id.to_string(),
                                        content: reasoning.to_string(),
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
                    }
                    // Accumulate tool_calls
                    if let Some(tcs) = val["choices"][0]["delta"]["tool_calls"].as_array() {
                        for tc in tcs {
                            let index =
                                tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                            let entry = tc_accum
                                .entry(index)
                                .or_insert_with(|| (String::new(), String::new(), String::new()));
                            if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                                entry.0 = id.to_string();
                            }
                            if let Some(name) = tc
                                .get("function")
                                .and_then(|f| f.get("name"))
                                .and_then(|v| v.as_str())
                            {
                                entry.1 = name.to_string();
                            }
                            if let Some(args) = tc
                                .get("function")
                                .and_then(|f| f.get("arguments"))
                                .and_then(|v| v.as_str())
                            {
                                entry.2.push_str(args);
                            }
                        }
                    }
                    // On finish_reason="tool_calls" no immediate flush is needed (the accumulator holds state);
                    // calls are built at the end in index order; this only emits an early flush event to notify the frontend.
                    let finish = val["choices"][0]["finish_reason"].as_str().unwrap_or("");
                    if !finish.is_empty() {
                        finish_reason = Some(finish.to_string());
                    }
                    if finish == "tool_calls" && !suppress_events {
                        for (id, name, args) in tc_accum.values() {
                            if !id.is_empty() && !name.is_empty() {
                                let _ = window.emit(
                                    ev::LLM_TOOL_CALL,
                                    ToolCallPayload {
                                        assistant_id: assistant_id.to_string(),
                                        topic_id: topic_id.to_string(),
                                        tool_call_id: id.clone(),
                                        name: name.clone(),
                                        arguments: args.clone(),
                                    },
                                );
                            }
                        }
                    }
                    // Extract server-reported token usage (OpenAI-compatible APIs attach usage to the final chunk)
                    if let Some(usage) = val.get("usage") {
                        if let Some(pt) = usage.get("prompt_tokens").and_then(|v| v.as_u64()) {
                            input_tokens = pt as u32;
                        }
                        if let Some(ct) = usage.get("completion_tokens").and_then(|v| v.as_u64()) {
                            output_tokens = ct as u32;
                        }
                        // Cache-hit stats: OpenAI uses prompt_tokens_details.cached_tokens, DeepSeek uses prompt_cache_hit_tokens
                        if let Some(cached) = usage
                            .get("prompt_tokens_details")
                            .and_then(|d| d.get("cached_tokens"))
                            .and_then(|v| v.as_u64())
                            .or_else(|| {
                                usage
                                    .get("prompt_cache_hit_tokens")
                                    .and_then(|v| v.as_u64())
                            })
                        {
                            cached_input_tokens = cached as u32;
                        }
                    }
                } // end of else branch (OpenAI-compatible parsing)
            }
        }
        if saw_done {
            break;
        }
    }

    // Build the tool call list in index order (if finish_reason="tool_calls" already emitted notices, do not re-emit)
    let mut tool_calls = Vec::new();
    let need_emit = !saw_done; // if [DONE] never arrived, tool-call notices may not have been emitted yet
    for (id, name, args) in tc_accum.values() {
        if !id.is_empty() && !name.is_empty() {
            tool_calls.push(ToolCallAccum {
                id: id.clone(),
                name: name.clone(),
                arguments: args.clone(),
            });
        }
    }
    // If the finish_reason="tool_calls" path never fired (some providers only send a
    // tool_calls delta at the end), emit llm-tool-call notices here so the frontend shows them.
    if need_emit && !suppress_events {
        for tc in &tool_calls {
            let _ = window.emit(
                ev::LLM_TOOL_CALL,
                ToolCallPayload {
                    assistant_id: assistant_id.to_string(),
                    topic_id: topic_id.to_string(),
                    tool_call_id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                },
            );
        }
    }

    // Payload shape resent to the API: raw array when array parts exist (unrewritten tokens), else plain text string.
    let api_content = if round_api_parts.is_empty() {
        json!(content_buf)
    } else {
        json!(round_api_parts)
    };
    let (final_content, images) = if process_images && !image_urls.is_empty() {
        let mut c = content_buf.clone();
        match crate::utils::generated_images::save_generated_images(
            app.unwrap(),
            &mut c,
            &image_urls,
        )
        .await
        {
            Ok(imgs) => (c, imgs),
            Err(e) => {
                tracing::warn!("保存生成图片失败: {}", e);
                (content_buf.clone(), vec![])
            }
        }
    } else {
        (content_buf.clone(), vec![])
    };

    Ok(RoundResult {
        content: final_content,
        reasoning: reasoning_buf,
        tool_calls,
        input_tokens,
        output_tokens,
        cached_input_tokens,
        images,
        api_content,
        finish_reason,
    })
}

/// Core function: calls the LLM and streams the result back in chunks.
///
/// Note: this command is kept for command stability; the new frontend flow uses [`run_agent_turn`]
/// (the backend single-task autonomous loop). It now only does single-round streaming + a terminal
/// done, with no tool execution/recursion -- tool-call notification events are still emitted (for debugging/compatibility).
#[tauri::command]
#[allow(
    clippy::too_many_arguments,
    reason = "Tauri IPC compatibility requires the existing flat command parameters"
)]
pub async fn call_llm_stream(
    window: Window,                         // Tauri window handle, used to emit events
    state: tauri::State<'_, StreamManager>, // Global state, manages in-flight stream tasks
    db_state: tauri::State<'_, DbState>,
    api_url: String,              // API endpoint URL
    api_key: String,              // API key
    model: String,                // Model name (e.g. gpt-3.5-turbo)
    assistant_id: String,         // Assistant ID (frontend matches messages by it)
    topic_id: String,             // Topic/session ID
    messages: Vec<Message>,       // History context messages
    tools: Option<Vec<ToolSpec>>, // Tool definitions (MCP tools; None or empty array means not sent)
) -> Result<(), String> {
    // Diagnostic logging (debug only): prints each message's role and which key fields are present.
    // Must stay at debug level -- production defaults to warn; conversation-content logs must not reach production logs.
    for (i, m) in messages.iter().enumerate() {
        tracing::debug!(
            "[call_llm_stream] messages[{}] role={} has_toolCallId={} has_toolCalls={} has_name={}",
            i,
            m.role,
            m.tool_call_id.is_some(),
            m.tool_calls.is_some(),
            m.name.is_some()
        );
    }
    // SSRF protection
    crate::utils::url_validation::validate_http_url(
        &api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;
    // 1. Build the task key (structured tuple, avoids "-" concatenation ambiguity)
    let task_key = (assistant_id.clone(), topic_id.clone());

    // 2. If a task is already running for this key, cancel it first (cancel, not abort, so the epilogue still runs)
    if let Some((_, (_, old_token))) = state.0.remove(&task_key) {
        old_token.cancel();
    }

    // 3. Clone variables for the async task (move closure)
    let state_inner = state.0.clone();
    let task_key_inner = task_key.clone();
    let assistant_id_c = assistant_id.clone();
    let topic_id_c = topic_id.clone();
    let messages_for_api = {
        let conn = db_state.0.lock();
        messages
            .iter()
            .map(|message| message_for_api(&conn, message))
            .collect::<Result<Vec<_>, _>>()?
    };
    let tools_slice = tools; // kept for as_slice()

    // Defensive validation (debug builds only): checks tool_calls match tool responses.
    // The per-message JSON scan exists only for logging; release builds skip the cost.
    #[cfg(debug_assertions)]
    verify_tool_messages(&messages_for_api);

    let token = CancellationToken::new();
    let token_inner = token.clone();
    let app_handle = window.app_handle().clone();
    let model_c = model.clone();

    // 4. Spawn the async task that performs the request
    let handle = tokio::spawn(async move {
        let client = streaming_http_client();
        let tools_ref: Option<&[ToolSpec]> = tools_slice.as_deref();
        let result = stream_one_round(
            &window,
            &token_inner,
            &client,
            api_url,
            &api_key,
            &model,
            &messages_for_api,
            tools_ref,
            &assistant_id_c,
            &topic_id_c,
            false,
            Some(&app_handle),
            true,
            None,
            crate::core::models::default_model_call_retries(),
        )
        .await;

        // Epilogue: emit a terminal done on success/cancel/error alike so the frontend always resets isThinking
        match result {
            Ok(round) => {
                // Persist this round's token usage to usage_log
                insert_usage_log(
                    &app_handle,
                    &assistant_id_c,
                    &topic_id_c,
                    &model_c,
                    1,
                    round.input_tokens,
                    round.output_tokens,
                    round.cached_input_tokens,
                );
                let _ = window.emit(
                    ev::LLM_CHUNK,
                    StreamPayload {
                        assistant_id: assistant_id_c.clone(),
                        topic_id: topic_id_c.clone(),
                        content: if round.images.is_empty() {
                            "".into()
                        } else {
                            round.content.clone()
                        },
                        done: true,
                        error: None,
                        input_tokens: Some(round.input_tokens),
                        output_tokens: Some(round.output_tokens),
                        context_tokens: Some(round.input_tokens),
                        cached_input_tokens: Some(round.cached_input_tokens),
                        images: if round.images.is_empty() {
                            None
                        } else {
                            Some(round.images.clone())
                        },
                    },
                );
            }
            Err(e) => {
                tracing::error!("Stream Error: {}", e);
                let is_cancel = e == "cancelled";
                let _ = window.emit(
                    ev::LLM_CHUNK,
                    StreamPayload {
                        assistant_id: assistant_id_c.clone(),
                        topic_id: topic_id_c.clone(),
                        content: if is_cancel {
                            "".into()
                        } else {
                            format!("\n[Error: {}]", e)
                        },
                        done: true,
                        error: if is_cancel { None } else { Some(e) },
                        input_tokens: None,
                        output_tokens: None,
                        context_tokens: None,
                        cached_input_tokens: None,
                        images: None,
                    },
                );
            }
        }

        // Remove the task from global state once it finishes
        state_inner.remove(&task_key_inner);
    });

    // 5. Store the running task handle and cancellation token in global state
    state.0.insert(task_key, (handle, token));
    Ok(())
}

/// Helper: fetches the list of available models from the provider
#[tauri::command]
pub async fn fetch_models(api_url: String, api_key: String) -> Result<Vec<ModelInfo>, String> {
    // SSRF protection
    crate::utils::url_validation::validate_http_url(
        &api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;
    // Build the model-list URL, usually the base URL plus /models
    let mut base_url = api_url.trim_end_matches('/').to_string();
    if base_url.ends_with("/chat/completions") {
        base_url = base_url.replace("/chat/completions", "");
    }
    let final_url = format!("{}/models", base_url);

    let client = non_streaming_http_client();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(45),
        client
            .get(&final_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .send(),
    )
    .await
    .map_err(|_| "获取模型列表超时（45s）".to_string())?
    .map_err(|e| e.to_string())?;

    // Parse the returned model JSON
    let res_data: ModelsResponse = response.json().await.map_err(|e| e.to_string())?;
    Ok(res_data.data)
}

/// Stop function: called when the user clicks "Stop generating".
///
/// Notifies the task to exit gracefully via `CancellationToken::cancel()` (not `abort()`),
/// so after the `select!` branch returns the task still runs its epilogue
/// (emit done + remove itself), guaranteeing the frontend `isThinking` state is reset.
#[tauri::command]
pub async fn stop_llm_stream(
    state: tauri::State<'_, StreamManager>,
    assistant_id: String,
    topic_id: String,
) -> Result<(), String> {
    let task_key = (assistant_id.clone(), topic_id.clone());

    // Take the cancellation token and cancel; the task itself emits done and removes itself from state.
    // Note: cancel only, never abort, so the epilogue is guaranteed.
    if let Some(entry) = state.0.get(&task_key) {
        entry.1.cancel();
    }
    Ok(())
}

/// Executes a built-in tool (direct in-process call, with permission checks and approval).
///
/// Covers file tools (file_tools) and command execution tools (shell_tools), bypassing the MCP channel.
/// The permission model matches `execute_tool_call`.
///
/// `writer_id` labels the calling agent for multi-agent write coordination ("main-agent"
/// or a subagent label): mutating single-path file tools take the per-file exclusive
/// lock (`core::file_locks::FileLockRegistry`) so same-file writes serialize across the
/// main agent and parallel subagents, and cross-writer edits must carry a fresh hashline
/// anchor (anchor_hash / expect_hash) to prevent silent lost updates.
///
/// `workspace_root` overrides the root the tool operates on (worktree-isolated subagents
/// pass their worktree path); permission rules and project memory stay bound to the
/// project root resolved from `project_id`.
#[allow(
    clippy::too_many_arguments,
    reason = "explicit execution context incl. workspace isolation"
)]
pub(crate) async fn execute_builtin_tool(
    app: &AppHandle,
    tool_name: &str,
    arguments: &serde_json::Value,
    project_id: Option<&str>,
    agent_mode: &AgentMode,
    token: &CancellationToken,
    writer_id: &str,
    workspace_root: Option<&str>,
) -> Result<ToolResult, String> {
    let project_root = file_tools::resolve_project_root(app, project_id)?;
    // Tools operate on the workspace root when the caller runs isolated (git worktree);
    // permission rules + memory below stay on the main project root.
    let tools_root = workspace_root
        .map(|s| s.to_string())
        .unwrap_or_else(|| project_root.clone());

    // Fail-closed: when the sandbox is unavailable (no bwrap/sandbox-exec), autonomous modes
    // refuse command execution per config. Normal/Plan modes still go through permission
    // approval (Ask), whose reason carries a sandbox-unavailable warning (see the Ask branch below).
    if tool_name == "execute_command"
        && matches!(agent_mode, AgentMode::Auto | AgentMode::Workflow)
        && !crate::utils::sandbox::sandbox_available()
        && crate::core::config::load_app_config(app)
            .unwrap_or_default()
            .sandbox_required
    {
        return Err(ierr!(
            "沙箱不可用，已拒绝执行命令（fail-closed）。请安装 bubblewrap（Linux）或确认 sandbox-exec 存在（macOS）后重试；或在应用配置中将 sandboxRequired 设为 false 以回退无沙箱执行（不推荐）。",
            "Sandbox unavailable; command execution refused (fail-closed). Install bubblewrap (Linux) or verify sandbox-exec exists (macOS), then retry; or set sandboxRequired to false in app settings to fall back to unsandboxed execution (not recommended)."
        ));
    }

    // Permission check (matches execute_tool_call; server_id keeps crate::core::ids::BUILTIN_FS_SERVER_ID for compatibility with existing rules)
    if *agent_mode != AgentMode::Off {
        let custom_rules = permission::load_permissions(Some(&project_root)).rules;
        let action = permission::check_permission(
            tool_name,
            crate::core::ids::BUILTIN_FS_SERVER_ID,
            arguments,
            agent_mode,
            &custom_rules,
        );

        match action {
            PermissionAction::Deny => {
                return Err(ierr!(
                    "工具 '{}' 已在当前模式下被安全策略禁止执行（Deny）。\n如需执行，请切换到自动模式或在项目权限设置中添加 allow 规则。",
                    "Tool '{}' is blocked by the security policy in the current mode (Deny).\nTo allow it, switch to auto mode or add an allow rule in project permission settings.",
                    tool_name
                ));
            }
            PermissionAction::Ask => {
                let pending = app.state::<PendingApprovals>();
                let mut reason = ierr!(
                    "工具 '{}' 需要您的确认才能执行",
                    "Tool '{}' needs your confirmation to execute",
                    tool_name
                );
                // Command execution: dangerous-command annotation
                if tool_name == "execute_command" {
                    if let Some(cmd) = arguments["command"].as_str() {
                        if let Some(danger) = shell_tools::check_dangerous_command(cmd) {
                            let prefix = match danger.category {
                                shell_tools::DangerCategory::Critical => "🚨 严重危险",
                                shell_tools::DangerCategory::High => "⚠️ 高风险",
                                shell_tools::DangerCategory::Medium => "⚡ 中等风险",
                            };
                            reason = ierr!(
                                "{}\n\n{}\n\n命令: {}\n\n确认执行？",
                                "{}\n\n{}\n\nCommand: {}\n\nConfirm execution?",
                                prefix,
                                danger.risk,
                                cmd
                            );
                        } else {
                            reason = ierr!(
                                "命令: {}\n\n工具 '{}' 需要您的确认才能执行",
                                "Command: {}\n\nTool '{}' needs your confirmation to execute",
                                cmd,
                                tool_name
                            );
                        }
                    }
                }
                // Network declaration and sandbox availability annotations: the approval info presents the full risk surface
                if tool_name == "execute_command" {
                    if arguments["network"].as_bool().unwrap_or(false) {
                        reason.push_str(&ierr!(
                            "\n\n⚠ 此命令声明需要网络访问",
                            "\n\n⚠ This command declares network access"
                        ));
                    }
                    if !crate::utils::sandbox::sandbox_available() {
                        reason.push_str(&ierr!(
                            "\n\n⚠ 沙箱不可用：此命令将在无 OS 级沙箱的环境执行",
                            "\n\n⚠ Sandbox unavailable: this command will run without an OS-level sandbox"
                        ));
                    }
                }
                // File-modifying tools: precompute a diff preview
                let (preview_diff, file_path) =
                    compute_diff_preview(tool_name, arguments, &project_root);
                let approval_fut = crate::commands::mcp::request_tool_approval(
                    app,
                    pending.inner(),
                    crate::core::ids::BUILTIN_FS_SERVER_ID,
                    tool_name,
                    arguments,
                    &reason,
                    preview_diff,
                    file_path,
                );
                tokio::select! {
                    _ = token.cancelled() => return Err("cancelled".into()),
                    res = approval_fut => res?,
                }
            }
            PermissionAction::Allow => {}
        }
    }

    // Dispatch and execute
    if tool_name == "execute_command" {
        let command = arguments["command"].as_str().unwrap_or("").to_string();
        let timeout = arguments["timeout"].as_u64();
        // Network egress control: declared explicitly by the model; default false = no network inside the sandbox
        let allow_network = arguments["network"].as_bool().unwrap_or(false);
        // Windows AppContainer: when enabled in config, pass toolchain R+X dirs to run under the OS-level sandbox
        #[cfg(windows)]
        let ac_dirs: Option<Vec<String>> = {
            let ac_cfg = crate::core::config::load_app_config(app).unwrap_or_default();
            if ac_cfg.sandbox_windows == "appcontainer" {
                Some(ac_cfg.sandbox_windows_toolchain_dirs.clone())
            } else {
                None
            }
        };
        #[cfg(not(windows))]
        let ac_dirs: Option<Vec<String>> = None;
        let root = tools_root.clone();
        Ok(tokio::task::spawn_blocking(move || {
            shell_tools::execute_command(&command, &root, timeout, allow_network, ac_dirs)
        })
        .await
        .unwrap_or_else(|_| shell_tools::tool_err("命令执行线程 panic")))
    } else if tool_name == "web_fetch" {
        let url = arguments["url"].as_str().unwrap_or("");
        let max_bytes = arguments["max_bytes"].as_u64();
        Ok(web_tools::execute_web_fetch(url, max_bytes).await)
    } else if tool_name == "web_search" {
        let query = arguments["query"].as_str().unwrap_or("");
        let count = arguments["count"].as_u64();
        Ok(web_tools::execute_web_search(query, count).await)
    } else if tool_name == "read_lints" {
        lsp_tools::execute(app, &tools_root, arguments).await
    } else if tool_name == "lsp_definition" {
        lsp_agent_tools::execute_lsp_definition(app, &tools_root, arguments).await
    } else if tool_name == "lsp_references" {
        lsp_agent_tools::execute_lsp_references(app, &tools_root, arguments).await
    } else if tool_name == "lsp_hover" {
        lsp_agent_tools::execute_lsp_hover(app, &tools_root, arguments).await
    } else if tool_name == "lsp_symbols" {
        lsp_agent_tools::execute_lsp_symbols(app, &tools_root, arguments).await
    } else if crate::services::memory::tools::MEMORY_TOOL_NAMES.contains(&tool_name) {
        // The memory tool name list is maintained centrally in services/memory/tools.rs
        // (including search_code); never hardcode a literal list here to avoid dispatch/spec drift.
        // Memory stays bound to the main project root even under worktree isolation (the
        // memory DB is per-project, not per-worktree).
        let tool_name_c = tool_name.to_string();
        let arguments_c = arguments.clone();
        let root_c = project_root.clone();
        let app_c2 = app.clone();
        crate::services::memory::tools::execute_memory_tool(
            &app_c2,
            &tool_name_c,
            &arguments_c,
            &root_c,
        )
        .await
    } else if tool_name == "think" {
        Ok(crate::utils::think::execute(arguments))
    } else if tool_name == "project_map" {
        Ok(crate::utils::project_map::execute(&tools_root))
    } else if tool_name.starts_with("git_") {
        let tool_name_c = tool_name.to_string();
        let arguments_c = arguments.clone();
        let root_c = tools_root.clone();
        Ok(tokio::task::spawn_blocking(move || {
            git_tools::execute_git_tool(&tool_name_c, &arguments_c, &root_c)
        })
        .await
        .unwrap_or_else(|_| git_tools::tool_err("Git 工具执行线程 panic")))
    } else if matches!(
        tool_name,
        "read_file"
            | "write_file"
            | "replace_in_file"
            | "search_files"
            | "search_content"
            | "list_directory"
            | "delete_file"
            | "make_directory"
    ) {
        // File tools run under the multi-agent write-coordination policy: mutating
        // single-path tools take the per-file exclusive lock and cross-writer edits must
        // carry a fresh hashline anchor (see commands/llm/locked_tools.rs).
        execute_locked_file_tool(app, tool_name, arguments, &tools_root, token, writer_id).await
    } else {
        // Unknown tools must error out explicitly. Never route unmatched tool names to a
        // file_tools fallback, which would give the model a semantically ambiguous result.
        Ok(file_tools::tool_err(&ierr!(
            "未知工具: {}",
            "Unknown tool: {}",
            tool_name
        )))
    }
}

/// Helper: check if a directory is a git repo
fn is_git_repo(dir: &str) -> bool {
    std::path::Path::new(dir).join(".git").exists()
}

/// Tool execution wrapper with automatic retry (used only for MCP network calls).
pub(crate) async fn execute_tool_with_retry<F, Fut>(
    execute: F,
    config: &crate::core::models::AppConfig,
    token: &tokio_util::sync::CancellationToken,
) -> Result<ToolResult, String>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<ToolResult, String>>,
{
    if !config.auto_retry_enabled {
        return execute().await;
    }
    let max_attempts = config.auto_retry_count as usize + 1;
    let mut last_error = None;
    for attempt in 1..=max_attempts {
        match execute().await {
            Ok(result) => {
                if result.is_error && attempt < max_attempts {
                    if let Some(err_text) = result
                        .content
                        .first()
                        .and_then(|c| c.data.get("text"))
                        .and_then(|v| v.as_str())
                    {
                        if is_retryable_error(err_text) {
                            tokio::select! {
                                _ = token.cancelled() => return Err("cancelled".into()),
                                _ = tokio::time::sleep(std::time::Duration::from_millis(config.auto_retry_delay_ms)) => {}
                            }
                            continue;
                        }
                    }
                }
                return Ok(result);
            }
            Err(e) => {
                last_error = Some(e.clone());
                if attempt < max_attempts && is_retryable_error(&e) {
                    tokio::select! {
                        _ = token.cancelled() => return Err("cancelled".into()),
                        _ = tokio::time::sleep(std::time::Duration::from_millis(config.auto_retry_delay_ms)) => {}
                    }
                    continue;
                }
                return Err(e);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| "auto-retry exhausted".into()))
}

/// Run context for one autonomous turn of the main agent loop (consolidates the former 15 flat parameters).
///
/// IPC compatibility: the Tauri command keeps the flat parameters (old frontend calls unchanged);
/// the command body is a thin wrapper that builds a TurnContext and passes it to the implementation.
pub(crate) struct TurnContext {
    pub api_url: String,
    pub api_key: String,
    pub model: String,
    pub assistant_id: String,
    pub topic_id: String,
    pub messages: Vec<Message>,
    pub agent_mode: AgentMode,
    pub project_id: Option<String>,
    pub web_search_enabled: bool,
    pub profile_model_overrides: Vec<ProfileModelOverride>,
    pub custom_subagent_profiles: Vec<crate::core::models::CustomSubagentProfile>,
    pub locale: Option<String>,
    pub context_window: Option<u32>,
    pub skills: Option<Vec<SkillConfig>>,
    /// MCP server ids opted in by the assistant (opt-in; empty = no MCP tools)
    pub mcp_server_ids: Vec<String>,
    /// Checkpoint of the turn to resume (continue after a crash); None = a normal new turn
    pub resume_turn_id: Option<String>,
}

/// Autonomous agent-loop command (the main entry point of the new frontend flow).
///
/// Runs one tokio task through stream -> detect tools -> permission/approval -> execute MCP -> feed results back -> recurse,
/// until the model stops issuing tool calls or the round limit is hit. The frontend only listens to events and renders:
/// - `llm-round-start`: push an empty assistant placeholder message
/// - `llm-chunk`: append text / `done:true` marks the real end of the whole turn
/// - `llm-reasoning`: append chain-of-thought
/// - `llm-tool-call`: show a "running" bubble (notification only; this command performs the execution)
/// - `llm-tool-result`: update bubble status + append the role:tool message
/// - `tool-approval-requested`: approval request
///
/// # Parameters
/// - `messages`: initial message list (including this turn's user message); the frontend prebuilds system/history/user
/// - `mcp_server_ids`: MCP server ids opted in by the assistant. In Agent mode, tools of connected
///   servers are injected; conflicting tool names are renamed to mcp__<server>__<tool>; tools from
///   untrusted servers still require per-call approval in Auto/Workflow mode (see mcp::execute_tool_call)
/// - `agent_mode`: agent execution mode; affects permission rules and tool injection (Plan: research tools / Normal/Auto/Workflow: orchestration tools / Off: no tools)
/// - `project_id`: project id (used to resolve project-level permission rules)
/// - `locale`: optional UI language; falls back to `zh-CN` when missing on legacy calls
#[tauri::command]
#[allow(
    clippy::too_many_arguments,
    reason = "Tauri IPC compatibility: the flat signature is the existing public command surface"
)]
pub async fn run_agent_turn(
    app: AppHandle,
    window: Window,
    db_state: tauri::State<'_, DbState>,
    stream_mgr: tauri::State<'_, StreamManager>,
    api_url: String,
    api_key: String,
    model: String,
    assistant_id: String,
    topic_id: String,
    messages: Vec<Message>,
    mcp_server_ids: Vec<String>,
    agent_mode: AgentMode,
    project_id: Option<String>,
    web_search_enabled: bool,
    profile_model_overrides: Vec<ProfileModelOverride>,
    custom_subagent_profiles: Vec<crate::core::models::CustomSubagentProfile>,
    locale: Option<String>,
    context_window: Option<u32>,
    skills: Option<Vec<SkillConfig>>,
    resume_turn_id: Option<String>,
) -> Result<(), String> {
    let ctx = TurnContext {
        api_url,
        api_key,
        model,
        assistant_id,
        topic_id,
        messages,
        agent_mode,
        project_id,
        web_search_enabled,
        profile_model_overrides,
        custom_subagent_profiles,
        locale,
        context_window,
        skills,
        mcp_server_ids,
        resume_turn_id,
    };
    run_agent_turn_inner(app, window, db_state, stream_mgr, ctx).await
}

/// Stop a given background subagent (fire-and-forget subagents started by delegate_task with wait=false).
///
/// # Parameters
/// - `subagent_id`: background subagent id (returned in the start confirmation result)
///
/// # Returns
/// true = cancel signal sent; false = not found (already finished or nonexistent; idempotent success).
#[tauri::command]
pub fn stop_subagent(app: AppHandle, subagent_id: String) -> Result<bool, String> {
    let handles = app.state::<crate::core::state::SubagentHandles>();
    // The Ref guard must be released within this statement (a tail-expression match keeps temporaries alive to the block end, triggering E0597)
    let found = match handles.0.get(&subagent_id) {
        Some(entry) => {
            entry.1.cancel();
            tracing::info!("[subagent:{subagent_id}] 用户请求停止后台子智能体");
            true
        }
        None => false,
    };
    Ok(found)
}

/// Implementation of the main agent's one-turn autonomous loop (parameters packed into TurnContext).
pub(crate) async fn run_agent_turn_inner(
    app: AppHandle,
    window: Window,
    db_state: tauri::State<'_, DbState>,
    stream_mgr: tauri::State<'_, StreamManager>,
    ctx: TurnContext,
) -> Result<(), String> {
    let TurnContext {
        api_url,
        api_key,
        model,
        assistant_id,
        topic_id,
        messages,
        agent_mode,
        project_id,
        web_search_enabled,
        profile_model_overrides,
        custom_subagent_profiles,
        locale,
        context_window,
        skills,
        mcp_server_ids,
        resume_turn_id,
    } = ctx;
    // Diagnostic log (debug only): print each message's role and whether key fields are present.
    // Keep this at debug level: production defaults to warn, and conversation-content logs must not land in production logs.
    for (i, m) in messages.iter().enumerate() {
        tracing::debug!(
            "[run_agent_turn] messages[{}] role={} has_toolCallId={} has_toolCalls={} has_name={}",
            i,
            m.role,
            m.tool_call_id.is_some(),
            m.tool_calls.is_some(),
            m.name.is_some()
        );
    }
    // SSRF protection
    crate::utils::url_validation::validate_http_url(
        &api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;
    let task_key = (assistant_id.clone(), topic_id.clone());

    // Cancel the old task for the same topic (cancel, not abort).
    // The old handle must be awaited to finish its epilogue; otherwise the old task's state_inner.remove
    // could run after the new insert and wrongly delete the new task's entry, leaving the new task orphaned.
    if let Some((_, (old_handle, old_token))) = stream_mgr.0.remove(&task_key) {
        old_token.cancel();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(10), old_handle).await;
    }

    // Convert messages to API format up front (incl. attachment image expansion) so synchronous I/O finishes while the lock is held
    let mut messages_for_api: Vec<serde_json::Value> = {
        let conn = db_state.0.lock();
        messages
            .iter()
            .map(|m| message_for_api(&conn, m))
            .collect::<Result<Vec<_>, _>>()?
    };
    // Checkpoint resume: replace with the full context snapshot of the last round before the crash (continue instead of restarting from scratch).
    // turn_id persists across this turn: resume reuses the original turn_id (snapshot continuation); a new turn generates a new id.
    let turn_id = resume_turn_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if let Some(resume_id) = &resume_turn_id {
        let conn = db_state.0.lock();
        match load_checkpoint_messages(&conn, &topic_id, resume_id) {
            Some(checkpoint_msgs) if !checkpoint_msgs.is_empty() => {
                tracing::info!(
                    "[checkpoint] 从检查点恢复回合 {resume_id}（{} 条消息）",
                    checkpoint_msgs.len()
                );
                messages_for_api = checkpoint_msgs;
            }
            _ => {
                tracing::warn!("[checkpoint] 未找到回合 {resume_id} 的活跃检查点，按新回合执行");
            }
        }
    }
    // Defensive validation (debug builds only): release builds skip the logging cost of a full message scan
    #[cfg(debug_assertions)]
    verify_tool_messages(&messages_for_api);
    let locale_c = if locale.as_deref() == Some("en-US") {
        "en-US".to_string()
    } else {
        "zh-CN".to_string()
    };
    messages_for_api.insert(
        0,
        json!({
            "role": "system",
            "content": if locale_c == "en-US" {
                "Use English for AIO-controlled explanations, plans, and summaries. Preserve user content, code, identifiers, tool names, paths, and external output verbatim."
            } else {
                "AIO 控制的说明、计划和总结请使用简体中文。用户内容、代码、标识符、工具名、路径和外部输出保持原文。"
            }
        }),
    );

    // ===== Token efficiency: model-aware context budget =====
    // Compression trigger thresholds are derived from the model context window (passed by the frontend
    // from the model catalog, default 128k), following oh-my-pi's reserve policy: reserve = max(16384, 15% of window).
    let context_window_n = context_window.unwrap_or(128_000).max(16_000) as usize;
    let context_reserve = 16_384usize.max(context_window_n / 100 * 15);
    let context_budget = context_window_n.saturating_sub(context_reserve); // token threshold that triggers compression
    let keep_recent_tokens = (context_budget / 2).clamp(4_000, 20_000); // recent-token budget kept when compressing

    // Reasoning models (OpenAI o1/o3/o4, gpt-5 series) reject the max_tokens field (they need max_completion_tokens),
    // so no output cap is injected for them to avoid API errors; other models get half the window as output budget.
    let model_lower = model.to_lowercase();
    let is_reasoning_model = ["o1", "o3", "o4", "gpt-5"]
        .iter()
        .any(|p| model_lower.contains(p));
    let max_output_tokens = if is_reasoning_model {
        None
    } else {
        Some(((context_window_n / 2) as u32).clamp(1024, 16_384))
    };

    let token = CancellationToken::new();
    let token_inner = token.clone();
    let state_inner = stream_mgr.0.clone();
    let task_key_inner = task_key.clone();
    let assistant_id_c = assistant_id.clone();
    let topic_id_c = topic_id.clone();
    let app_c = app.clone();
    let project_id_c = project_id.clone();
    let profile_overrides_c = profile_model_overrides.clone();
    let custom_profiles_c = custom_subagent_profiles.clone();
    let mcp_server_ids_c = mcp_server_ids.clone();

    let handle = tokio::spawn(async move {
        // Unwrap Skills (the frontend passes them only in Agent mode; empty in chat mode)
        let skills = skills.unwrap_or_default();
        // Load the app config (incl. auto-retry, subagent concurrency cap, memory toggle);
        // reads fall back to defaults on failure
        let app_config = crate::core::config::load_app_config(&app_c).unwrap_or_default();

        // Subagent concurrency cap
        let max_concurrent = app_config.max_concurrent_subagents.unwrap_or(5).max(1) as usize;
        let subagent_semaphore = Arc::new(Semaphore::new(max_concurrent));

        let client = streaming_http_client();

        // Tool selection: inject a different tool set per agent mode
        // - Plan: research tools only (think + project_map + web)
        // - Normal/Auto/Workflow: orchestration tools only (delegate_task + create_workflow + think + web + project_map)
        // - Chat + web search: web tools only
        // - Pure chat: no tools
        // MCP tool pass-through: in Agent mode, inject connected tools from the assistant's selected servers.
        // Only connected servers are pulled (unconnected ones are skipped with the existing warning semantics);
        // tool name conflicts (built-in tools or another server) are renamed at merge time, see build_main_tool_set.
        let mcp_tools: Option<crate::core::models::AssistantTools> =
            if agent_mode != AgentMode::Off && !mcp_server_ids_c.is_empty() {
                let mgr = app_c.state::<crate::plugins::mcp::McpServerManager>();
                let mcp_state = app_c.state::<crate::plugins::mcp::McpServerState>();
                crate::commands::mcp::list_mcp_tools_for_assistant_inner(
                    &app_c,
                    mgr.inner(),
                    mcp_state.inner(),
                    mcp_server_ids_c.clone(),
                    project_id_c.clone(),
                )
                .await
                .ok()
            } else {
                None
            };

        // Tool set assembly (mode matrix + MCP/memory/skill/archive injection), see build_main_tool_set
        let tool_set = build_main_tool_set(
            &app_c,
            &agent_mode,
            web_search_enabled,
            project_id_c.as_deref(),
            &skills,
            app_config.memory_enabled,
            mcp_tools.as_ref(),
            &app_config.main_agent_tool_access,
        );
        let tools = tool_set.tools;
        let tool_server_map = tool_set.tool_server_map;
        let memory_effective = tool_set.memory_effective;
        let tools_slice: Option<&[ToolSpec]> = if tools.is_empty() { None } else { Some(&tools) };

        let mut round: u32 = 0;
        let mut final_error: Option<String> = None;
        let mut was_cancelled = false;
        // Workflow mode: flag whether create_workflow has been called
        let mut workflow_called = false;
        // Token usage accumulated across rounds (for cost stats)
        let mut total_input_tokens: u32 = 0;
        let mut total_output_tokens: u32 = 0;
        let mut total_cached_tokens: u32 = 0;
        // Prefix-stability regression check: flag that compression just happened, to record cache hits on the first post-compression round
        let mut compressed_this_run: bool = false;
        // Unfinished-task tracking (handoff doc): forced stop (round limit / workflow not called) or error
        let mut incomplete_stop: bool = false;
        let mut last_finish_reason: Option<String> = None;
        // Peak context tokens: input_tokens of the last API call (for the context-window display)
        let mut context_input_tokens: u32 = 0;
        // Final text and generated images accumulated across rounds (sent once at done; idempotently replaces frontend chunk accumulation)
        let mut accumulated_content = String::new();
        let mut all_images: Vec<GeneratedImage> = Vec::new();

        // Workflow mode: inject the mandatory system message before the LLM loop (inside the stable prefix to favor prompt cache and compression protection)
        if agent_mode == AgentMode::Workflow {
            messages_for_api.insert(
                2.min(messages_for_api.len()),
                serde_json::json!({
                    "role": "system",
                    "content": concat!(
                        "你正处于「工作流模式」。首要任务：分析用户请求 → 调用 `create_workflow` 创建并执行工作流。\n",
                        "不得直接修改文件、执行命令或调用其他工具。\n",
                        "典型序列：requirements → coder → reviewer；explorer → coder；debugger → coder；requirements → architect → coder → tester。\n",
                        "简单请求可用单步工作流。请立即调用 create_workflow 开始。"
                    )
                }),
            );
        }

        // Project memory auto-injection: retrieve once from the first user message and insert at the stable prefix slot (index=1)
        if memory_effective && app_config.memory_auto_inject {
            if let Some(pid) = &project_id_c {
                if let Ok(project_root) = file_tools::resolve_project_root(&app_c, Some(pid)) {
                    if let Ok(store) = app_c
                        .state::<crate::services::memory::MemoryStoreManager>()
                        .open(&project_root)
                    {
                        let query = messages_for_api
                            .iter()
                            .find(|m| m.get("role").and_then(|v| v.as_str()) == Some("user"))
                            .and_then(|m| m.get("content").and_then(|v| v.as_str()))
                            .unwrap_or("")
                            .to_string();
                        let budget = app_config.memory_injection_budget_tokens as usize;
                        if !query.is_empty() && budget > 0 {
                            // Project-level embedding config (same as the write side) + dimension guard; then hybrid retrieval -> project-level rerank -> budget trim
                            let query_vec = crate::services::memory::retrieve::embed_query(
                                &app_c, &store, &query,
                            )
                            .await;
                            if let Some(block) = crate::services::memory::retrieve::inject_block(
                                &app_c, &store, &query, query_vec, budget,
                            )
                            .await
                            {
                                messages_for_api.insert(
                                    1,
                                    serde_json::json!({
                                        "role": "system",
                                        "content": format!("[项目记忆]\n{}", block)
                                    }),
                                );
                            }
                        }
                    }
                }
            }
        }
        // Previous-turn handoff re-injection: if the last turn ended unfinished (round limit / error / truncation),
        // consume its handoff doc and inject it as a system message so the new turn directly gets the completed work
        // and touched files instead of relying on the user's retelling. Consume-once: deleted after read, never injected twice.
        if let Some(handoff) = consume_handoff_doc(&app_c, &assistant_id_c, &topic_id_c) {
            // The insert point must not go past the last message (the user message must stay at the end)
            let insert_at = 2.min(messages_for_api.len().saturating_sub(1));
            messages_for_api.insert(
                insert_at,
                serde_json::json!({
                    "role": "system",
                    "content": format!("[上回合交接] 上一回合任务未完成，以下是交接文档（含已完成内容与涉及文件），请据此继续：
{}", handoff),
                }),
            );
            tracing::info!(
                "[handoff] 已回注上回合交接文档 ({} 字符)",
                handoff.chars().count()
            );
        }
        // 后台子智能体完成通知（push 通道）：本回合开始前已到达终态的后台任务，
        // consume-once 注入单条合并 system 消息（覆盖"主 Agent 空闲期间后台完成"的场景）。
        if let Some(notice) =
            drain_pending_subagent_notices(&app_c, &assistant_id_c, &topic_id_c, &locale_c)
        {
            let insert_at = 2.min(messages_for_api.len().saturating_sub(1));
            messages_for_api.insert(
                insert_at,
                serde_json::json!({
                    "role": "system",
                    "content": notice,
                }),
            );
            tracing::info!(
                "[subagent-notice] 回合开始注入后台子智能体通知 ({} 字符)",
                notice.chars().count()
            );
        }
        // Context budget tracking: token-level and model-aware (threshold/reserve budgets derived from the window before task start)
        // Configurable max tool-call rounds per turn (default 25, min 1)
        let max_rounds = app_config
            .max_tool_rounds
            .unwrap_or(crate::core::models::DEFAULT_MAX_TOOL_ROUNDS)
            .max(1);
        // Max reminders to call create_workflow in Workflow mode (bounded, avoids infinite loops)
        let max_workflow_reminders: u32 = 2;
        let mut workflow_reminders: u32 = 0;
        // Max continuation retries when output is truncated (finish_reason="length")
        const MAX_LENGTH_RETRIES: u32 = 2;
        let mut length_retries: u32 = 0;
        // Context overflow auto-recovery attempts: compress then retry
        const MAX_OVERFLOW_RETRIES: u32 = 1;
        let mut overflow_retries: u32 = 0;

        'outer: loop {
            round += 1;
            if token_inner.is_cancelled() {
                was_cancelled = true;
                break;
            }

            // Max tool-call rounds reached -> graceful stop (no abort; emit a note and end)
            if round > max_rounds {
                incomplete_stop = true;
                let note = "\n\n⚠️ 已达本轮最大工具调用轮数，任务未完成。如需继续，请再次发送。";
                accumulated_content.push_str(note);
                let _ = window.emit(
                    ev::LLM_CHUNK,
                    StreamPayload {
                        assistant_id: assistant_id_c.clone(),
                        topic_id: topic_id_c.clone(),
                        content: note.into(),
                        done: false,
                        error: None,
                        input_tokens: None,
                        output_tokens: None,
                        context_tokens: None,
                        cached_input_tokens: None,
                        images: None,
                    },
                );
                break 'outer;
            }

            // Notify frontend: a new round starts; it pushes an empty assistant placeholder
            let _ = window.emit(
                ev::LLM_ROUND_START,
                RoundStartPayload {
                    assistant_id: assistant_id_c.clone(),
                    topic_id: topic_id_c.clone(),
                    round,
                },
            );

            // 后台子智能体完成通知（push 通道，轮内）：本回合运行期间到达终态的后台任务，
            // consume-once 注入为尾部 system 消息（本轮 role:tool 已回填、下轮 LLM 调用前，
            // 位置安全；system 消息可出现在消息流任意位置）。
            if let Some(notice) =
                drain_pending_subagent_notices(&app_c, &assistant_id_c, &topic_id_c, &locale_c)
            {
                messages_for_api.push(serde_json::json!({
                    "role": "system",
                    "content": notice,
                }));
                tracing::info!(
                    "[subagent-notice] 轮内注入后台子智能体通知 ({} 字符)",
                    notice.chars().count()
                );
            }

            // No hard round limit below the cap above: complex tasks may need many rounds of tool
            // calls; the loop ends only when the model stops issuing tool calls (task done) or the user stops it.
            let (round_msgs, round_tools) = (messages_for_api.as_slice(), tools_slice);

            let round_result = stream_one_round(
                &window,
                &token_inner,
                &client,
                api_url.clone(),
                &api_key,
                &model,
                round_msgs,
                round_tools,
                &assistant_id_c,
                &topic_id_c,
                false,
                Some(&app_c),
                true,
                max_output_tokens,
                app_config.model_call_retries,
            )
            .await;

            let round_result = match round_result {
                Ok(r) => r,
                Err(e) => {
                    if e == "cancelled" {
                        was_cancelled = true;
                        break 'outer;
                    }
                    // Context overflow auto-recovery: compress earlier messages and retry once so long sessions don't fail outright
                    if overflow_retries < MAX_OVERFLOW_RETRIES
                        && is_context_overflow_error(&e)
                        && messages_for_api.len() > 4
                    {
                        overflow_retries += 1;
                        let mut msgs = std::mem::take(&mut messages_for_api);
                        let res = compress_context(
                            &window,
                            &client,
                            &api_url,
                            &api_key,
                            &model,
                            &mut msgs,
                            &assistant_id_c,
                            &topic_id_c,
                            &token_inner,
                            keep_recent_tokens,
                            "main",
                            &locale_c,
                        )
                        .await;
                        messages_for_api = msgs;
                        if res.is_ok() {
                            tracing::warn!("[overflow] 上下文溢出，已压缩并重试: {}", e);
                            continue;
                        }
                    }
                    incomplete_stop = true;
                    final_error = Some(e);
                    break 'outer;
                }
            };

            // Accumulate token usage
            total_input_tokens += round_result.input_tokens;
            total_output_tokens += round_result.output_tokens;
            total_cached_tokens += round_result.cached_input_tokens;
            last_finish_reason = round_result.finish_reason.clone();

            // Prefix-stability regression check: cache hit rate on the first round after compression (stable prefix blocks should still hit)
            if compressed_this_run {
                compressed_this_run = false;
                let hit_pct = if round_result.input_tokens > 0 {
                    (round_result.cached_input_tokens as f64 / round_result.input_tokens as f64)
                        * 100.0
                } else {
                    0.0
                };
                tracing::info!(
                    "[cache] 压缩后首轮 input_tokens={}, cached_input_tokens={}（命中率 {:.1}%）",
                    round_result.input_tokens,
                    round_result.cached_input_tokens,
                    hit_pct
                );
            }
            // Record peak context (input_tokens of the last round)
            context_input_tokens = round_result.input_tokens;
            // Accumulate final text and generated images (incl. aio-image markers, returned with done)
            accumulated_content.push_str(&round_result.content);
            all_images.extend(round_result.images.clone());

            // Persist this round's token usage to usage_log
            insert_usage_log(
                &app_c,
                &assistant_id_c,
                &topic_id_c,
                &model,
                round,
                round_result.input_tokens,
                round_result.output_tokens,
                round_result.cached_input_tokens,
            );

            // Append this round's assistant message (incl. tool_calls) to the context
            let mut asst_obj = serde_json::Map::new();
            asst_obj.insert("role".into(), json!("assistant"));
            asst_obj.insert(
                "content".into(),
                match &round_result.api_content {
                    serde_json::Value::String(s) if s.is_empty() => json!(null),
                    other => other.clone(),
                },
            );
            if !round_result.tool_calls.is_empty() {
                let tcs: Vec<serde_json::Value> = round_result
                    .tool_calls
                    .iter()
                    .map(|tc| {
                        json!({
                            "id": tc.id,
                            "type": "function",
                            "function": { "name": tc.name, "arguments": tc.arguments },
                        })
                    })
                    .collect();
                asst_obj.insert("tool_calls".into(), json!(tcs));
            }
            messages_for_api.push(serde_json::Value::Object(asst_obj));

            // Output truncated (finish_reason="length") with no tool calls: append a continuation
            // instruction and retry (bounded) instead of ending mid-reply.
            if round_result.finish_reason.as_deref() == Some("length")
                && round_result.tool_calls.is_empty()
                && length_retries < MAX_LENGTH_RETRIES
            {
                length_retries += 1;
                messages_for_api.push(serde_json::json!({
                    "role": "system",
                    "content": "[输出被截断] 请直接继续完成上一条回复中未完成的内容，不要重复已经输出的部分。"
                }));
                continue;
            }

            // No tool calls -> task complete, end the whole turn
            if round_result.tool_calls.is_empty() {
                if agent_mode == AgentMode::Workflow && !workflow_called {
                    // Workflow mode: reminders are bounded (max_workflow_reminders) to avoid an infinite loop
                    if workflow_reminders < max_workflow_reminders {
                        workflow_reminders += 1;
                        messages_for_api.push(serde_json::json!({
                            "role": "system",
                            "content": "【工作流模式强制指令】你尚未调用 create_workflow 工具。请立即分析用户请求并调用 create_workflow 来创建和执行工作流。"
                        }));
                        continue;
                    }
                    // Model still refuses after bounded reminders -> respect the exit signal and stop gracefully
                    incomplete_stop = true;
                    let note = "\n\n⚠️ 模型未按工作流模式调用 create_workflow，已停止。您可以切回普通/自动模式继续。";
                    accumulated_content.push_str(note);
                    let _ = window.emit(
                        ev::LLM_CHUNK,
                        StreamPayload {
                            assistant_id: assistant_id_c.clone(),
                            topic_id: topic_id_c.clone(),
                            content: note.into(),
                            done: false,
                            error: None,
                            input_tokens: None,
                            output_tokens: None,
                            context_tokens: None,
                            cached_input_tokens: None,
                            images: None,
                        },
                    );
                    break 'outer;
                }
                break 'outer;
            }

            // Tool-call execution: three phases
            // Phase 1 - collect concurrent futures (delegate_task(s) delegation + read-only built-in tools);
            //           mutating tools (file writes / commands / git writes / MCP - side effects unknown) run serially in original order
            // Phase 2 - await all futures concurrently via FuturesUnordered, emitting each result on completion
            // Phase 3 - emit serial results in original index order + backfill all role:tool messages
            // (semantics follow Claude Code / Codex CLI parallel tool calling; backfill keeps index order, API semantics unchanged)

            // Records each tool's result (ordered by original index)
            #[derive(Default, Clone)]
            struct ToolExecResult {
                content_text: String,
                result_value: serde_json::Value,
                is_error: bool,
            }
            let mut exec_results: Vec<Option<ToolExecResult>> =
                vec![None; round_result.tool_calls.len()];

            // Collect concurrently executable futures (subagent delegation + read-only built-in tools) with their indices
            let mut parallel_futures: Vec<(
                usize,
                futures_util::future::BoxFuture<'static, Result<ToolResult, String>>,
            )> = Vec::new();

            for (i, tc) in round_result.tool_calls.iter().enumerate() {
                if token_inner.is_cancelled() {
                    was_cancelled = true;
                    break;
                }
                let server_id = tool_server_map.get(&tc.name).cloned();
                let args_val: serde_json::Value =
                    serde_json::from_str(&tc.arguments).unwrap_or(json!({}));

                if server_id.as_deref() == Some(crate::core::ids::BUILTIN_SERVER_ID)
                    && tc.name == "delegate_task"
                {
                    // Check the wait argument: false = run in the background (fire-and-forget)
                    let wait_for_result = args_val
                        .get("wait")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    if !wait_for_result {
                        // Fire-and-forget: the subagent runs in the background, the main agent continues immediately.
                        // Dual channel: outcome cached for get_subagent_result (pull) + completion notice
                        // queued for system-message injection (push).
                        let window_clone = window.clone();
                        let app_clone = app_c.clone();
                        let client_clone = client.clone();
                        let api_url_clone = api_url.clone();
                        let api_key_clone = api_key.clone();
                        let model_clone = model.clone();
                        let assistant_id_clone = assistant_id_c.clone();
                        let topic_id_clone = topic_id_c.clone();
                        let args_val_clone = args_val.clone();
                        let project_id_clone = project_id_c.clone();
                        let agent_mode_clone = agent_mode.clone();
                        let profile_overrides_clone = profile_overrides_c.clone();
                        let custom_profiles_clone = custom_profiles_c.clone();
                        // Child token: parent cancellation cascades; stop_subagent can cancel this background subagent by ID
                        let token_clone = token_inner.child_token();
                        let token_for_registry = token_clone.clone();
                        let locale_clone = locale_c.clone();
                        let mcp_ids_clone = mcp_server_ids_c.clone();
                        let sem_clone = subagent_semaphore.clone();
                        let subagent_id = uuid::Uuid::new_v4().to_string();
                        let sid = subagent_id.clone();
                        let app_c2 = app_c.clone();
                        // 登记 Running 状态（get_subagent_result 可立即查询）
                        let bg_profile = args_val_clone["profile"]
                            .as_str()
                            .unwrap_or("general")
                            .to_string();
                        let bg_task =
                            truncate_chars_head(args_val_clone["task"].as_str().unwrap_or(""), 100);
                        register_running_subagent(&app_c, &subagent_id, bg_profile, bg_task);
                        let jh = tokio::spawn(async move {
                            let result = handle_delegate_task(
                                &window_clone,
                                &app_clone,
                                &client_clone,
                                &api_url_clone,
                                &api_key_clone,
                                &model_clone,
                                &assistant_id_clone,
                                &topic_id_clone,
                                &args_val_clone,
                                project_id_clone.as_deref(),
                                &agent_mode_clone,
                                sem_clone,
                                &token_clone,
                                &profile_overrides_clone,
                                &custom_profiles_clone,
                                &mcp_ids_clone,
                                &locale_clone,
                            )
                            .await;
                            // Remove handle from registry on completion
                            if let Some(handles) =
                                app_c2.try_state::<crate::core::state::SubagentHandles>()
                            {
                                handles.0.remove(&sid);
                            }
                            // 双通道收尾：终态唯一写入点（Done/Failed/Cancelled）+ 通知入队
                            // + 唤醒 get_subagent_result 等待者；错误同时保留 tracing 可见性。
                            let (profile_name, task_summary) = {
                                let completions =
                                    app_c2.state::<crate::core::state::SubagentCompletions>();
                                completions
                                    .outcomes
                                    .get(&sid)
                                    .map(|e| (e.profile_name.clone(), e.task_summary.clone()))
                                    .unwrap_or_default()
                            };
                            record_subagent_outcome(
                                &app_c2,
                                &assistant_id_clone,
                                &topic_id_clone,
                                &sid,
                                &profile_name,
                                &task_summary,
                                &locale_clone,
                                &result,
                            );
                            if let Err(e) = &result {
                                tracing::error!("[subagent:{sid}] 后台子智能体失败: {e}");
                            }
                        });
                        // Register the handle and token so stop_subagent can cancel this subagent by ID.
                        if let Some(handles) =
                            app_c.try_state::<crate::core::state::SubagentHandles>()
                        {
                            handles
                                .0
                                .insert(subagent_id.clone(), (jh, token_for_registry));
                        }
                        // Immediate result: acknowledge the fire-and-forget
                        let content_text = format!(
                            "子智能体已在后台启动 (ID: {})，完成后将自动通知；可随时用 get_subagent_result 查询",
                            subagent_id
                        );
                        exec_results[i] = Some(ToolExecResult {
                            content_text: content_text.clone(),
                            result_value: json!({"status": "started", "subagent_id": subagent_id}),
                            is_error: false,
                        });
                    } else {
                        // Subagent delegation: collect the future, run it in parallel later
                        let window_clone = window.clone();
                        let app_clone = app_c.clone();
                        let client_clone = client.clone();
                        let api_url_clone = api_url.clone();
                        let api_key_clone = api_key.clone();
                        let model_clone = model.clone();
                        let assistant_id_clone = assistant_id_c.clone();
                        let topic_id_clone = topic_id_c.clone();
                        let args_val_clone = args_val.clone();
                        let project_id_clone = project_id_c.clone();
                        let agent_mode_clone = agent_mode.clone();
                        let profile_overrides_clone = profile_overrides_c.clone();
                        let custom_profiles_clone = custom_profiles_c.clone();
                        let token_clone = token_inner.clone();
                        let locale_clone = locale_c.clone();
                        let mcp_ids_clone = mcp_server_ids_c.clone();
                        let sem_clone = subagent_semaphore.clone();

                        let fut = Box::pin(async move {
                            handle_delegate_task(
                                &window_clone,
                                &app_clone,
                                &client_clone,
                                &api_url_clone,
                                &api_key_clone,
                                &model_clone,
                                &assistant_id_clone,
                                &topic_id_clone,
                                &args_val_clone,
                                project_id_clone.as_deref(),
                                &agent_mode_clone,
                                sem_clone,
                                &token_clone,
                                &profile_overrides_clone,
                                &custom_profiles_clone,
                                &mcp_ids_clone,
                                &locale_clone,
                            )
                            .await
                        });
                        parallel_futures.push((i, fut));
                    }
                } else if server_id.as_deref() == Some(crate::core::ids::BUILTIN_SERVER_ID)
                    && tc.name == "delegate_tasks"
                {
                    // Batch parallel: collect one future into parallel_futures
                    let window_clone = window.clone();
                    let app_clone = app_c.clone();
                    let client_clone = client.clone();
                    let api_url_clone = api_url.clone();
                    let api_key_clone = api_key.clone();
                    let model_clone = model.clone();
                    let assistant_id_clone = assistant_id_c.clone();
                    let topic_id_clone = topic_id_c.clone();
                    let args_val_clone = args_val.clone();
                    let project_id_clone = project_id_c.clone();
                    let agent_mode_clone = agent_mode.clone();
                    let profile_overrides_clone = profile_overrides_c.clone();
                    let custom_profiles_clone = custom_profiles_c.clone();
                    let semaphore_clone = subagent_semaphore.clone();
                    let token_clone = token_inner.clone();
                    let locale_clone = locale_c.clone();
                    let mcp_ids_clone = mcp_server_ids_c.clone();

                    let fut = Box::pin(async move {
                        handle_delegate_tasks(
                            &window_clone,
                            &app_clone,
                            &client_clone,
                            &api_url_clone,
                            &api_key_clone,
                            &model_clone,
                            &assistant_id_clone,
                            &topic_id_clone,
                            &args_val_clone,
                            project_id_clone.as_deref(),
                            &agent_mode_clone,
                            &token_clone,
                            &profile_overrides_clone,
                            &custom_profiles_clone,
                            &mcp_ids_clone,
                            semaphore_clone,
                            &locale_clone,
                        )
                        .await
                    });
                    parallel_futures.push((i, fut));
                } else if server_id.as_deref() == Some(crate::core::ids::BUILTIN_SERVER_ID)
                    && tc.name == "get_subagent_result"
                {
                    // Pull 通道：查询/等待后台子智能体结果。与 delegate 系一致进阶段 2
                    // 并行池（同轮可并行等待多个后台任务；自身无副作用）。
                    let app_clone = app_c.clone();
                    let args_val_clone = args_val.clone();
                    let token_clone = token_inner.clone();
                    let fut = Box::pin(async move {
                        execute_get_subagent_result(&app_clone, &args_val_clone, &token_clone).await
                    });
                    parallel_futures.push((i, fut));
                } else if server_id.as_deref() == Some(crate::core::ids::BUILTIN_SERVER_ID)
                    && tc.name == "create_workflow"
                {
                    // Workflow creation: parse arguments and run all steps sequentially
                    workflow_called = true;
                    let title = args_val["title"].as_str().unwrap_or("Workflow").to_string();
                    let steps_raw = args_val["steps"].as_array();
                    let tool_result = if let Some(steps_arr) = steps_raw {
                        if steps_arr.is_empty() {
                            Err(ierr!(
                                "create_workflow: 'steps' 数组不能为空",
                                "create_workflow: 'steps' array must not be empty"
                            ))
                        } else {
                            let mut workflow_steps: Vec<WorkflowStep> = Vec::new();
                            let mut parse_error: Option<String> = None;
                            for (idx, step) in steps_arr.iter().enumerate() {
                                let profile =
                                    step["profile"].as_str().unwrap_or("general").to_string();
                                let name = step["name"].as_str().unwrap_or("").to_string();
                                let task = step["task"].as_str().unwrap_or("").to_string();
                                if task.is_empty() {
                                    parse_error = Some(ierr!(
                                        "create_workflow: 步骤 {} 缺少 'task' 字段",
                                        "create_workflow: step {} is missing the 'task' field",
                                        idx + 1
                                    ));
                                    break;
                                }
                                workflow_steps.push(WorkflowStep {
                                    step_id: format!("step-{}", idx + 1),
                                    profile_id: profile,
                                    name,
                                    task_description: task,
                                    status: WorkflowStepStatus::Pending,
                                    result: None,
                                    started_at: None,
                                    duration: None,
                                });
                            }
                            if let Some(err) = parse_error {
                                Err(err)
                            } else {
                                let workflow = Workflow {
                                    workflow_id: uuid::Uuid::new_v4().to_string(),
                                    title,
                                    steps: workflow_steps,
                                };
                                execute_workflow(
                                    &window,
                                    &app_c,
                                    &client,
                                    &api_url,
                                    &api_key,
                                    &model,
                                    &assistant_id_c,
                                    &topic_id_c,
                                    &workflow,
                                    project_id_c.as_deref(),
                                    subagent_semaphore.clone(),
                                    &custom_profiles_c,
                                    &profile_overrides_c,
                                    &mcp_server_ids_c,
                                    &token_inner,
                                    &locale_c,
                                    &agent_mode,
                                )
                                .await
                            }
                        }
                    } else {
                        Err(ierr!(
                            "create_workflow: 缺少 'steps' 数组",
                            "create_workflow: missing 'steps' array"
                        ))
                    };

                    let (content_text, result_value, is_error) = match tool_result {
                        Ok(tr) => {
                            let text = tr
                                .content
                                .iter()
                                .find(|c| c.kind == "text")
                                .and_then(|c| c.data.get("text"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| {
                                    serde_json::to_string(&tr.content).unwrap_or_default()
                                });
                            let result_value =
                                serde_json::to_value(&tr.content).unwrap_or(json!([]));
                            (text, result_value, tr.is_error)
                        }
                        Err(e) => (format!("[Error] {}", e), json!({ "error": e }), true),
                    };

                    exec_results[i] = Some(ToolExecResult {
                        content_text,
                        result_value,
                        is_error,
                    });
                } else {
                    // Non-delegate tool: execute immediately in order
                    // Extract the file path before tool execution (args_val may be moved into the MCP call)
                    let file_path_for_diff: Option<String> =
                        if ["write_file", "replace_in_file", "delete_file"]
                            .contains(&tc.name.as_str())
                        {
                            args_val["path"].as_str().map(|s| s.to_string())
                        } else {
                            None
                        };
                    let tool_result = if server_id.as_deref()
                        == Some(crate::core::ids::BUILTIN_SERVER_ID)
                        && tc.name == "read_artifact"
                    {
                        execute_artifact_read(
                            &app_c,
                            args_val["artifact_id"].as_str().unwrap_or(""),
                        )
                    } else if server_id.as_deref() == Some(crate::core::ids::BUILTIN_SERVER_ID)
                        && tc.name == "read_skill"
                    {
                        execute_skill_read(&skills, args_val["name"].as_str().unwrap_or(""))
                    } else if server_id.as_deref() == Some(crate::core::ids::BUILTIN_SERVER_ID)
                        && matches!(
                            tc.name.as_str(),
                            "create_tasks" | "update_task" | "list_tasks"
                        )
                    {
                        // Orchestrator task board: in-memory metadata + frontend mirror,
                        // dispatched before the permission path (read_artifact precedent)
                        Ok(execute_task_board_tool(
                            &app_c,
                            &assistant_id_c,
                            &topic_id_c,
                            &tc.name,
                            &args_val,
                        ))
                    } else if server_id.as_deref() == Some(crate::core::ids::BUILTIN_SERVER_ID)
                        && tc.name == "ask_user"
                    {
                        // Clarification questions: pauses the loop until the user answers
                        // (respond_agent_question), the token cancels, or the timeout fires
                        execute_ask_user(
                            &app_c,
                            &assistant_id_c,
                            &topic_id_c,
                            &args_val,
                            &token_inner,
                        )
                        .await
                    } else if server_id.as_deref() == Some(crate::core::ids::BUILTIN_SERVER_ID)
                        && app_config.parallel_tool_execution
                        && is_parallel_safe(&tc.name)
                    {
                        // Read-only built-in tool: execute concurrently (semantics follow Claude Code /
                        // Codex CLI parallel tool calling). Collected as a future that shares the phase-2
                        // FuturesUnordered with subagent delegations, emitting on completion; mutating tools
                        // are unaffected (still serial in original order), role:tool backfill keeps index order.
                        let app_c2 = app_c.clone();
                        let tn = tc.name.clone();
                        let args_val_c = args_val.clone();
                        let pid = project_id_c.clone();
                        let mode = agent_mode.clone();
                        let tok = token_inner.clone();
                        let fut = Box::pin(async move {
                            execute_builtin_tool(
                                &app_c2,
                                &tn,
                                &args_val_c,
                                pid.as_deref(),
                                &mode,
                                &tok,
                                "main-agent",
                                None,
                            )
                            .await
                        });
                        parallel_futures.push((i, fut));
                        continue;
                    } else if server_id.as_deref() == Some(crate::core::ids::BUILTIN_SERVER_ID) {
                        execute_builtin_tool(
                            &app_c,
                            &tc.name,
                            &args_val,
                            project_id_c.as_deref(),
                            &agent_mode,
                            &token_inner,
                            "main-agent",
                            None,
                        )
                        .await
                    } else {
                        // MCP tools go through the auto-retry wrapper (network errors are retried, permission/argument errors are not)
                        let tc_name = tc.name.clone();
                        let sid_opt = server_id.clone();
                        let app_c2 = app_c.clone();
                        let args_val_c = args_val.clone();
                        let pid = project_id_c.clone();
                        let mode = agent_mode.clone();
                        let tok = token_inner.clone();
                        let ac = app_config.clone();
                        execute_tool_with_retry(
                            || {
                                let app = app_c2.clone();
                                let args = args_val_c.clone();
                                let pid_c = pid.clone();
                                let mode_c = mode.clone();
                                let tok_c = tok.clone();
                                let sid = sid_opt.clone();
                                let tn = tc_name.clone();
                                async move {
                                    match sid {
                                        Some(s) => {
                                            crate::commands::mcp::execute_tool_call(
                                                &app, &s, &tn, args, pid_c.as_deref(), &mode_c, &tok_c,
                                            ).await
                                        }
                                        None => Err(ierr!(
                                            "工具 {} 的 MCP server 未连接或意外断开，请检查 MCP 服务器状态",
                                            "The MCP server for tool {} is not connected or dropped unexpectedly; check the MCP server status",
                                            tn
                                        )),
                                    }
                                }
                            },
                            &ac,
                            &tok,
                        ).await
                    };

                    let (content_text, result_value, is_error) = match tool_result {
                        Ok(tr) => {
                            let text = tr
                                .content
                                .iter()
                                .find(|c| c.kind == "text")
                                .and_then(|c| c.data.get("text"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| {
                                    serde_json::to_string(&tr.content).unwrap_or_default()
                                });
                            let result_value =
                                serde_json::to_value(&tr.content).unwrap_or(json!([]));
                            (text, result_value, tr.is_error)
                        }
                        Err(e) => (format!("[Error] {}", e), json!({ "error": e }), true),
                    };
                    // For file-modifying tools, capture the diff for frontend display
                    let file_changes = file_path_for_diff
                        .and_then(|path| {
                            file_tools::resolve_project_root(&app_c, project_id_c.as_deref())
                                .ok()
                                .and_then(|root| git_tools::capture_file_diff(&root, &path))
                        })
                        .map(|fc| vec![fc]);
                    // Emit the result immediately (without waiting for other tools)
                    let _ = window.emit(
                        ev::LLM_TOOL_RESULT,
                        ToolResultPayload {
                            assistant_id: assistant_id_c.clone(),
                            topic_id: topic_id_c.clone(),
                            tool_call_id: tc.id.clone(),
                            name: tc.name.clone(),
                            content: content_text.clone(),
                            result: result_value.clone(),
                            is_error,
                            file_changes,
                            full_content: Some(content_text.clone()),
                        },
                    );

                    // Backfill the role:tool message immediately
                    let mut tool_msg = serde_json::Map::new();
                    tool_msg.insert("role".into(), json!("tool"));
                    tool_msg.insert("content".into(), json!(truncate_tool_result(&content_text)));
                    tool_msg.insert("tool_call_id".into(), json!(tc.id));
                    tool_msg.insert("name".into(), json!(tc.name));
                    messages_for_api.push(serde_json::Value::Object(tool_msg));
                }
            }

            // Phase 2 - run subagent delegations and read-only built-in tools concurrently (FuturesUnordered)
            if !parallel_futures.is_empty() {
                let mut unordered = FuturesUnordered::new();
                for (i, fut) in parallel_futures {
                    unordered.push(async move {
                        let result = fut.await;
                        (i, result)
                    });
                }

                while let Some((i, result)) = unordered.next().await {
                    let (content_text, result_value, is_error) = match result {
                        Ok(tr) => {
                            let text = tr
                                .content
                                .iter()
                                .find(|c| c.kind == "text")
                                .and_then(|c| c.data.get("text"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| {
                                    serde_json::to_string(&tr.content).unwrap_or_default()
                                });
                            let result_value =
                                serde_json::to_value(&tr.content).unwrap_or(json!([]));
                            (text, result_value, tr.is_error)
                        }
                        Err(e) => (format!("[Error] {}", e), json!({ "error": e }), true),
                    };
                    // Emit this delegate result immediately
                    let tc = &round_result.tool_calls[i];
                    let _ = window.emit(
                        ev::LLM_TOOL_RESULT,
                        ToolResultPayload {
                            assistant_id: assistant_id_c.clone(),
                            topic_id: topic_id_c.clone(),
                            tool_call_id: tc.id.clone(),
                            name: tc.name.clone(),
                            content: content_text.clone(),
                            result: result_value.clone(),
                            is_error,
                            file_changes: None,
                            full_content: Some(content_text.clone()),
                        },
                    );

                    // Backfill the role:tool message immediately
                    let mut tool_msg = serde_json::Map::new();
                    tool_msg.insert("role".into(), json!("tool"));
                    tool_msg.insert("content".into(), json!(truncate_tool_result(&content_text)));
                    tool_msg.insert("tool_call_id".into(), json!(tc.id));
                    tool_msg.insert("name".into(), json!(tc.name));
                    messages_for_api.push(serde_json::Value::Object(tool_msg));
                }
            }

            // Emit results in original index order + backfill role:tool messages
            for (i, tc) in round_result.tool_calls.iter().enumerate() {
                if token_inner.is_cancelled() {
                    was_cancelled = true;
                    break;
                }
                if let Some(result) = exec_results[i].take() {
                    let _ = window.emit(
                        ev::LLM_TOOL_RESULT,
                        ToolResultPayload {
                            assistant_id: assistant_id_c.clone(),
                            topic_id: topic_id_c.clone(),
                            tool_call_id: tc.id.clone(),
                            name: tc.name.clone(),
                            content: result.content_text.clone(),
                            result: result.result_value,
                            is_error: result.is_error,
                            file_changes: None,
                            full_content: Some(result.content_text.clone()),
                        },
                    );

                    let mut tool_msg = serde_json::Map::new();
                    tool_msg.insert("role".into(), json!("tool"));
                    tool_msg.insert(
                        "content".into(),
                        json!(truncate_tool_result(&result.content_text)),
                    );
                    tool_msg.insert("tool_call_id".into(), json!(tc.id));
                    tool_msg.insert("name".into(), json!(tc.name));
                    messages_for_api.push(serde_json::Value::Object(tool_msg));
                }
            }

            // Context budget check (token-level, model-aware): estimate the context, compress past the threshold
            let estimated_tokens: usize =
                estimate_context_tokens(&model, &messages_for_api, context_budget);
            if estimated_tokens > context_budget && messages_for_api.len() > 4 {
                let window_c = window.clone();
                let client_c = client.clone();
                let api_url_c = api_url.clone();
                let api_key_c = api_key.clone();
                let model_c = model.clone();
                let a_id = assistant_id_c.clone();
                let t_id = topic_id_c.clone();
                let tok = token_inner.clone();
                let mut msgs = std::mem::take(&mut messages_for_api);
                match compress_context(
                    &window_c,
                    &client_c,
                    &api_url_c,
                    &api_key_c,
                    &model_c,
                    &mut msgs,
                    &a_id,
                    &t_id,
                    &tok,
                    keep_recent_tokens,
                    "main",
                    &locale_c,
                )
                .await
                {
                    Ok(_) => {
                        compressed_this_run = true;
                        let _ = window.emit(
                            ev::LLM_COMPRESSION,
                            json!({
                                "round": round,
                                "result": "ok",
                                "estimated_tokens": estimated_tokens,
                                "keep_recent_tokens": keep_recent_tokens,
                            }),
                        );
                    }
                    Err(e) => {
                        let _ = window.emit(ev::LLM_COMPRESSION_FAILED, json!({"error": e}));
                    }
                }
                messages_for_api = msgs;
            }
        }

        // Checkpoint: the turn's context is settled (after tool backfill + compression decision), persist a snapshot.
        // After a crash, run_agent_turn(resume_turn_id) resumes from the latest active snapshot;
        // the epilogue marks this turn terminal - a leftover active checkpoint is the "recoverable" signal.
        {
            let conn = app_c.state::<crate::core::state::DbState>();
            save_checkpoint(
                &conn.0.lock(),
                &topic_id_c,
                &turn_id,
                round,
                &messages_for_api,
            );
        }

        // ===== Incomplete task -> handoff document =====
        // Triggered by: forced stop (round limit / workflow not called), error, or a truncated final response (length)
        let ended_incomplete = !was_cancelled
            && (incomplete_stop
                || final_error.is_some()
                || last_finish_reason.as_deref() == Some("length"));
        if ended_incomplete {
            match write_handoff_doc(
                &app_c,
                &assistant_id_c,
                &topic_id_c,
                &accumulated_content,
                &messages_for_api,
                &final_error,
            ) {
                Ok(path) => {
                    let reason = if final_error.is_some() {
                        "error"
                    } else if incomplete_stop {
                        "max_rounds"
                    } else {
                        "incomplete"
                    };
                    let _ = window.emit(
                        ev::LLM_HANDOFF,
                        json!({
                            "assistant_id": assistant_id_c,
                            "topic_id": topic_id_c,
                            "path": path,
                            "reason": reason,
                        }),
                    );
                    let note = "\n\n[工作交接] 本次任务未完成，已生成交接文档（含已完成内容与涉及文件，可据此继续）。";
                    accumulated_content.push_str(note);
                    let _ = window.emit(
                        ev::LLM_CHUNK,
                        StreamPayload {
                            assistant_id: assistant_id_c.clone(),
                            topic_id: topic_id_c.clone(),
                            content: note.into(),
                            done: false,
                            error: None,
                            input_tokens: None,
                            output_tokens: None,
                            context_tokens: None,
                            cached_input_tokens: None,
                            images: None,
                        },
                    );
                    tracing::info!("[handoff] 未完成任务已生成交接文档: {}", path);
                }
                Err(err) => tracing::warn!("[handoff] 生成交接文档失败: {}", err),
            }
        }

        // Background automatic fact extraction (sleep-time compute) - does not block the main flow
        if memory_effective && app_config.memory_auto_extract {
            if let Some(pid) = &project_id_c {
                if let Ok(project_root) = file_tools::resolve_project_root(&app_c, Some(pid)) {
                    let transcript = build_extract_transcript(&messages_for_api);
                    let app_x = app_c.clone();
                    let root_x = project_root;
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = crate::services::memory::extract::extract_and_store(
                            &app_x,
                            &root_x,
                            &transcript,
                        )
                        .await
                        {
                            tracing::warn!("[memory] 自动事实提取失败: {e}");
                        }
                    });
                }
            }
        }

        // ===== Epilogue (guaranteed): always emit terminal done on normal stop, cancel, or error =====
        // Reaching the epilogue = non-crash termination -> checkpoints get marked terminal;
        // only crash leftovers (status still active) are the "recoverable" signal.
        {
            let status = if was_cancelled {
                "cancelled"
            } else {
                "completed"
            };
            let conn = app_c.state::<crate::core::state::DbState>();
            finalize_checkpoints(&conn.0.lock(), &topic_id_c, &turn_id, status);
        }
        let error_payload = if was_cancelled {
            None
        } else {
            final_error.clone()
        };
        let _ = window.emit(
            ev::LLM_CHUNK,
            StreamPayload {
                assistant_id: assistant_id_c.clone(),
                topic_id: topic_id_c.clone(),
                content: if let Some(ref e) = final_error {
                    format!("\n[Error: {}]", e)
                } else if !all_images.is_empty() {
                    std::mem::take(&mut accumulated_content)
                } else {
                    "".into()
                },
                done: true,
                error: error_payload,
                input_tokens: if total_input_tokens > 0 {
                    Some(total_input_tokens)
                } else {
                    None
                },
                output_tokens: if total_output_tokens > 0 {
                    Some(total_output_tokens)
                } else {
                    None
                },
                context_tokens: if context_input_tokens > 0 {
                    Some(context_input_tokens)
                } else {
                    None
                },
                cached_input_tokens: if total_cached_tokens > 0 {
                    Some(total_cached_tokens)
                } else {
                    None
                },
                images: if all_images.is_empty() {
                    None
                } else {
                    Some(std::mem::take(&mut all_images))
                },
            },
        );

        // Remove self from global state
        state_inner.remove(&task_key_inner);
    });

    stream_mgr.0.insert(task_key, (handle, token));
    Ok(())
}

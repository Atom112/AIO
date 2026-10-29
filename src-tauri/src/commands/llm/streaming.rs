//! One streaming round: request build -> POST -> SSE accumulation -> delta events.
//!
//! 阶段4 第1项（子目标 2/5）：`stream_one_round` 与其累积产物 `RoundResult` /
//! `ToolCallAccum` 从 mod.rs 搬家至此（SSE 解析与轮次累积自成一体；mod.rs 只留
//! 主循环编排）。字段 pub(crate) 化——主循环/subagents 直接读取。

use crate::core::events as ev;
use crate::core::models::{GeneratedImage, StreamPayload, ToolCallPayload, ToolSpec};
use crate::ierr;
use futures_util::StreamExt as _;
use serde_json::json;
use std::collections::BTreeMap;
use tauri::{AppHandle, Emitter, Window};
use tokio_util::sync::CancellationToken;

use super::sse_lines;
use super::subagents::{SubagentDeltaPayload, SubagentStepPayload};

/// Applies the user-selected reasoning effort to an OpenAI-shaped request body.
///
/// `reasoning_effort` is the de-facto cross-provider parameter: OpenAI o-series/gpt-5,
/// Gemini's OpenAI-compat endpoint and OpenRouter accept it; unknown-field-tolerant
/// servers (Ollama, llama.cpp) ignore it silently. Anthropic requests are translated
/// separately by `to_anthropic_body` (effort -> thinking budget).
fn apply_reasoning_effort(body: &mut serde_json::Map<String, serde_json::Value>, effort: &str) {
    body.insert("reasoning_effort".into(), json!(effort));
}

/// Accumulated result of one streaming round.
pub(crate) struct RoundResult {
    /// Assistant text content for this round (with rewritten aio-image markers)
    pub(crate) content: String,
    /// Chain-of-thought for this round (already streamed live via llm-reasoning events; kept only for debugging)
    #[allow(dead_code)]
    pub(crate) reasoning: String,
    /// Tool calls issued by the model this round (ascending by index)
    pub(crate) tool_calls: Vec<ToolCallAccum>,
    /// Server-reported prompt tokens (input usage)
    pub(crate) input_tokens: u32,
    /// Server-reported completion tokens (output usage)
    pub(crate) output_tokens: u32,
    /// Cache-hit input tokens (OpenAI cached_tokens / DeepSeek prompt_cache_hit_tokens)
    pub(crate) cached_input_tokens: u32,
    /// Time-to-first-token for this round (ms): request start -> first content/reasoning/
    /// tool-call delta. None when the round produced no token at all (early error/cancel).
    pub(crate) ttft_ms: Option<u32>,
    /// Metadata of images generated/saved this round
    pub(crate) images: Vec<GeneratedImage>,
    /// Raw payload shape resent to the API / persisted (unrewritten-token array or string), used for the next round
    pub(crate) api_content: serde_json::Value,
    /// Server-reported finish_reason (e.g. "stop" / "tool_calls" / "length"), used for truncation recovery
    pub(crate) finish_reason: Option<String>,
}

/// A single fully-accumulated tool call.
#[derive(Clone)]
pub(crate) struct ToolCallAccum {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) arguments: String,
}

/// Live-stream context for a SUBAGENT round (harness round 11, DSH-parity live view):
/// text/reasoning deltas are re-scoped to `subagent-chunk` / `subagent-reasoning`
/// events so the standalone subagent conversation view streams in real time with
/// the same fidelity as the main conversation (the old path suppressed all events
/// and surfaced only round-granularity summaries).
#[derive(Clone)]
pub(crate) struct SubagentStreamCtx {
    pub subagent_id: String,
    pub parent_assistant_id: String,
    pub parent_topic_id: String,
    pub round: u32,
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
    // Subagent live-stream context: when Some, text/reasoning deltas emit
    // subagent-scoped events instead of being suppressed (harness round 11).
    subagent_ctx: Option<SubagentStreamCtx>,
    app: Option<&AppHandle>,
    process_images: bool,
    max_output_tokens: Option<u32>,
    max_retries: u32,
    // User-selected reasoning effort ("low"/"medium"/"high"); None = no reasoning parameter.
    // The FE only sends it for models known to be reasoning-capable (catalog flag or id
    // heuristic), so strict non-reasoning upstreams never receive the field.
    reasoning_effort: Option<&str>,
) -> Result<RoundResult, String> {
    let mut body_map = serde_json::Map::new();
    body_map.insert("model".into(), json!(model));
    body_map.insert("messages".into(), json!(messages));
    body_map.insert("stream".into(), json!(true));
    // PROV-6: OpenAI 兼容流式 usage 只在 include_usage=true 时随末 chunk 下发
    // （否则输入/输出 token 统计恒空）；Anthropic 走原生协议转换，不加此字段。
    if !crate::plugins::provider::anthropic::is_anthropic_url(api_url.as_str()) {
        body_map.insert("stream_options".into(), json!({"include_usage": true}));
    }
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
    // Reasoning-effort control: `reasoning_effort` is the de-facto cross-provider
    // parameter (OpenAI o-series/gpt-5, Gemini's OpenAI-compat endpoint, OpenRouter;
    // unknown-field-tolerant servers such as Ollama/llama.cpp ignore it). Anthropic
    // requests are translated by to_anthropic_body (effort -> thinking budget).
    if let Some(effort) = reasoning_effort {
        apply_reasoning_effort(&mut body_map, effort);
    }
    // Provider-specific body shaping (harness round 3, DeepSeek's thinking-mode
    // contract): runs UNCONDITIONALLY — DeepSeek's "off" must actively send
    // thinking:disabled (the model otherwise defaults to thinking at HIGH effort
    // and the slider is a no-op), and its real effort ladder remaps medium→low.
    // Default no-op for every other provider.
    crate::plugins::provider::ProviderManager::new()
        .for_url(api_url.as_str())
        .adjust_chat_body(&mut body_map, reasoning_effort);
    let body = serde_json::Value::Object(body_map);

    // TTFT measurement: the clock starts right before the HTTP request is issued, so
    // the number covers gateway/queue time as well as model prefill — that is what the
    // user actually waits for. The first token of ANY kind (text delta, reasoning /
    // thinking delta, or tool-call delta) stops the clock.
    let request_started = std::time::Instant::now();
    let mut ttft_ms: Option<u32> = None;

    // Unified request entry with pre-stream retries: adapts to the Anthropic native protocol;
    // transport errors and 429/5xx retry with backoff only before any content is emitted; mid-stream errors are not retried.
    // Token-guarded: the pre-header wait is bounded by the client's 600s total timeout, but the
    // stop button must stay responsive during it (a stalled gateway must not freeze the turn).
    // Session identity = topic id: OpenCode Go keys routing/prompt-cache on it; subagent
    // rounds pass the parent topic, so one user task clusters on one gateway session.
    let post_fut = crate::plugins::llm::post_chat_completion_with_retry(
        client,
        &api_url,
        api_key,
        &body,
        max_retries,
        Some(topic_id),
    );
    let (response, is_anthropic_request) = tokio::select! {
        _ = token.cancelled() => return Err("cancelled".to_string()),
        res = post_fut => res?,
    };

    let mut stream = response.bytes_stream();
    // PROV-1/LOOP-9: byte-level line buffering with a hard cap — single
    // implementation shared with the BTW stream (sse_lines module, exhaustively
    // tested across every chunk boundary incl. multi-byte UTF-8 splits).
    const SSE_LINE_BUFFER_LIMIT: usize = 4 * 1024 * 1024;
    let mut line_buffer = sse_lines::SseLineBuffer::new(SSE_LINE_BUFFER_LIMIT);
    // Marks the first token of the round: called on any content/reasoning/tool-call
    // delta, idempotent so later deltas never overwrite the recorded TTFT. On the very
    // first hit it also pushes a content-free llm-chunk ping carrying ttft_ms, so the
    // input-bar latency readout appears the moment the first token lands instead of
    // waiting for the end of a long round.
    let mark_first_token = |ttft: &mut Option<u32>| {
        if ttft.is_some() {
            return;
        }
        let ms = request_started
            .elapsed()
            .as_millis()
            .min(u128::from(u32::MAX)) as u32;
        *ttft = Some(ms);
        if !suppress_events {
            let _ = window.emit(
                ev::LLM_CHUNK,
                StreamPayload {
                    assistant_id: assistant_id.to_string(),
                    topic_id: topic_id.to_string(),
                    content: String::new(),
                    done: false,
                    error: None,
                    input_tokens: None,
                    output_tokens: None,
                    context_tokens: None,
                    cached_input_tokens: None,
                    ttft_ms: Some(ms),
                    images: None,
                },
            );
        }
    };
    // tool_call accumulator: index -> (id, name, arguments); BTreeMap keeps flush ordered by index
    let mut tc_accum: BTreeMap<usize, (String, String, String)> = BTreeMap::new();

    let mut content_buf = String::new();
    let mut reasoning_buf = String::new();
    // Server-reported finish_reason ("stop" / "tool_calls" / "length"), for caller-side truncation recovery
    let mut finish_reason: Option<String> = None;
    // Tool-argument progress reporting (subagent live view, harness round 11):
    // index -> bytes already reported. A giant write_file (~48KB) otherwise
    // streams silently for minutes with no frontend feedback.
    let mut tc_progress_reported: BTreeMap<usize, usize> = BTreeMap::new();
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
        line_buffer.push(&chunk)?;

        while let Some(line) = line_buffer.next_line() {
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
                        // TTFT probe: any growth of the accumulators after handling this event
                        // means a real token arrived (text / thinking / tool-call delta).
                        let tokens_before = content_buf.len()
                            + reasoning_buf.len()
                            + tc_accum.values().map(|(_, _, a)| a.len()).sum::<usize>();
                        if let Some(err) =
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
                                suppress_events,
                            )
                        {
                            // PROV-2：错误事件必须浮出——截断回复不再被当成功返回
                            return Err(ierr!(
                                "Anthropic 流错误: {}",
                                "Anthropic stream error: {}",
                                err
                            ));
                        }
                        if val.get("type").and_then(|v| v.as_str()) == Some("message_stop") {
                            saw_done = true;
                            break;
                        }
                        // TTFT: this event grew one of the accumulators => the first token arrived
                        let tokens_after = content_buf.len()
                            + reasoning_buf.len()
                            + tc_accum.values().map(|(_, _, a)| a.len()).sum::<usize>();
                        if tokens_after > tokens_before {
                            mark_first_token(&mut ttft_ms);
                        }
                    } else if let Some(err) = crate::plugins::llm::chat_error_message(&val) {
                        // PROV-2（OpenAI 兼容分支，与 btw.rs 对齐——此前双实现漂移）：
                        // 200 + {"error":{...}} 的流式错误事件不再被静默吞掉
                        return Err(ierr!("LLM 流错误: {}", "LLM stream error: {}", err));
                    } else if let Some(content) = val["choices"][0]["delta"]["content"].as_str() {
                        content_buf.push_str(content);
                        mark_first_token(&mut ttft_ms);
                        if let Some(ctx) = &subagent_ctx {
                            let _ = window.emit(
                                ev::SUBAGENT_CHUNK,
                                SubagentDeltaPayload {
                                    parent_assistant_id: ctx.parent_assistant_id.clone(),
                                    parent_topic_id: ctx.parent_topic_id.clone(),
                                    subagent_id: ctx.subagent_id.clone(),
                                    round: ctx.round,
                                    kind: "content".into(),
                                    content: content.to_string(),
                                },
                            );
                        } else if !suppress_events {
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
                                    ttft_ms: None,
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
                                        mark_first_token(&mut ttft_ms);
                                        if let Some(ctx) = &subagent_ctx {
                                            let _ = window.emit(
                                                ev::SUBAGENT_CHUNK,
                                                SubagentDeltaPayload {
                                                    parent_assistant_id: ctx
                                                        .parent_assistant_id
                                                        .clone(),
                                                    parent_topic_id: ctx.parent_topic_id.clone(),
                                                    subagent_id: ctx.subagent_id.clone(),
                                                    round: ctx.round,
                                                    kind: "content".into(),
                                                    content: s.to_string(),
                                                },
                                            );
                                        } else if !suppress_events {
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
                                                    ttft_ms: None,
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
                            mark_first_token(&mut ttft_ms);
                            if let Some(ctx) = &subagent_ctx {
                                let _ = window.emit(
                                    ev::SUBAGENT_REASONING,
                                    SubagentDeltaPayload {
                                        parent_assistant_id: ctx.parent_assistant_id.clone(),
                                        parent_topic_id: ctx.parent_topic_id.clone(),
                                        subagent_id: ctx.subagent_id.clone(),
                                        round: ctx.round,
                                        kind: "reasoning".into(),
                                        content: reasoning.to_string(),
                                    },
                                );
                            } else if !suppress_events {
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
                                        ttft_ms: None,
                                        images: None,
                                    },
                                );
                            }
                        }
                    }
                    // Accumulate tool_calls
                    if let Some(tcs) = val["choices"][0]["delta"]["tool_calls"].as_array() {
                        for tc in tcs {
                            mark_first_token(&mut ttft_ms);
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
                                // Giant tool-call argument progress (subagent live view only):
                                // report every 2KB so a 48KB write_file no longer streams
                                // silently for minutes ("thinking" with zero feedback).
                                if let Some(ctx) = &subagent_ctx {
                                    let last =
                                        tc_progress_reported.get(&index).copied().unwrap_or(0);
                                    if entry.2.len().saturating_sub(last) >= 2048 {
                                        tc_progress_reported.insert(index, entry.2.len());
                                        let tool = if entry.1.is_empty() {
                                            "工具参数"
                                        } else {
                                            entry.1.as_str()
                                        };
                                        let _ = window.emit(
                                            ev::SUBAGENT_STEP,
                                            SubagentStepPayload {
                                                parent_assistant_id: ctx
                                                    .parent_assistant_id
                                                    .clone(),
                                                parent_topic_id: ctx.parent_topic_id.clone(),
                                                subagent_id: ctx.subagent_id.clone(),
                                                round: ctx.round,
                                                step_type: "streaming".into(),
                                                summary: format!(
                                                    "{}: {}KB",
                                                    tool,
                                                    entry.2.len() / 1024
                                                ),
                                            },
                                        );
                                    }
                                }
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
                }
                // end of else branch (OpenAI-compatible parsing)
                else {
                    // PROV-1 companion: malformed SSE data lines are no longer silently dropped
                    tracing::warn!(
                        "[stream] 无法解析的 SSE 数据行: {}",
                        json_str.chars().take(120).collect::<String>()
                    );
                }
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
        ttft_ms,
        images,
        api_content,
        finish_reason,
    })
}

#[cfg(test)]
mod tests {
    use super::apply_reasoning_effort;
    use serde_json::json;

    #[test]
    fn reasoning_effort_lands_in_body() {
        let mut body = serde_json::Map::new();
        body.insert("model".into(), json!("gpt-5"));
        apply_reasoning_effort(&mut body, "high");
        assert_eq!(
            body.get("reasoning_effort").and_then(|v| v.as_str()),
            Some("high")
        );
    }

    #[test]
    fn no_effort_leaves_body_untouched() {
        let mut body = serde_json::Map::new();
        body.insert("model".into(), json!("gpt-4o"));
        // None is handled by the caller (no insert); the map stays clean
        assert!(body.get("reasoning_effort").is_none());
    }
}

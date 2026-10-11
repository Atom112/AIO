//! BTW (By The Way) side-question commands.
//!
//! These commands answer quick contextual questions without modifying the main conversation.

use crate::core::events as ev;
use crate::ierr;
use futures_util::StreamExt;
use serde_json::json;
use tauri::{Emitter, Window};

/// `/btw` side-question command: answers a short question from the current conversation context (no tools, streaming, leaves conversation history untouched).
///
/// Takes a message list built by the frontend (conversation history + the btw question),
/// calls the LLM over SSE streaming, and pushes each chunk to the target frontend overlay via the `btw-chunk` event.
#[tauri::command]
pub async fn ask_btw_question(
    window: Window,
    api_url: String,
    api_key: String,
    model: String,
    overlay_id: String,
    messages_json: String,
) -> Result<(), String> {
    let mut messages: Vec<serde_json::Value> = serde_json::from_str(&messages_json)
        .map_err(|e| format!("Invalid messages JSON: {}", e))?;

    // Normalize camelCase keys from frontend → snake_case for OpenAI-compatible API.
    // buildApiMessages() outputs `toolCallId` / `toolCalls`; the LLM expects `tool_call_id` / `tool_calls`.
    for msg in &mut messages {
        if let Some(obj) = msg.as_object_mut() {
            if let Some(v) = obj.remove("toolCallId") {
                obj.insert("tool_call_id".to_string(), v);
            }
            if let Some(v) = obj.remove("toolCalls") {
                obj.insert("tool_calls".to_string(), v);
            }
        }
        // Strip aio-image generated-image tokens so they do not leak to the LLM
        if let Some(serde_json::Value::String(s)) = msg.get_mut("content") {
            *s = crate::utils::generated_images::strip_generated_image_tokens(s);
        }
    }

    // SSRF protection
    crate::utils::url_validation::validate_http_url(
        &api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;

    let client = super::streaming_http_client();
    let body = json!({ "model": model, "messages": messages, "stream": true });

    let (res, is_anthropic) = match tokio::time::timeout(
        std::time::Duration::from_secs(60),
        super::post_chat_completion(&client, &api_url, &api_key, &body),
    )
    .await
    {
        Ok(Ok(resp)) => resp,
        Ok(Err(e)) => {
            let _ = window.emit(
                ev::BTW_CHUNK,
                json!({ "overlay_id": overlay_id, "content": "", "done": true, "error": e.to_string() }),
            );
            return Err(e.to_string());
        }
        Err(_) => {
            let err = ierr!("BTW 请求超时（60s）", "BTW request timed out (60s)");
            let _ = window.emit(
                ev::BTW_CHUNK,
                json!({ "overlay_id": overlay_id, "content": "", "done": true, "error": err.clone() }),
            );
            return Err(err);
        }
    };

    let status = res.status();
    if !status.is_success() {
        let body_text = res.text().await.unwrap_or_default();
        let truncated = if body_text.len() > 512 {
            &body_text[..body_text.floor_char_boundary(512)]
        } else {
            &body_text
        };
        let _ = window.emit(
            ev::BTW_CHUNK,
            json!({ "overlay_id": overlay_id, "content": "", "done": true, "error": format!("LLM API {}: {}", status, truncated) }),
        );
        return Err(format!("LLM API {}", status));
    }

    let mut stream = res.bytes_stream();
    // PROV-1/LOOP-9: byte-level line buffering with a hard cap — shared with the
    // main agent stream (sse_lines module). The historic per-chunk
    // `from_utf8_lossy` corrupted multi-byte UTF-8 split across chunks (Chinese
    // BTW answers garbled) and had no runaway-line cap.
    const BTW_LINE_BUFFER_LIMIT: usize = 4 * 1024 * 1024;
    let mut line_buffer = super::sse_lines::SseLineBuffer::new(BTW_LINE_BUFFER_LIMIT);

    loop {
        let next = tokio::select! {
            item = stream.next() => item,
            _ = tokio::time::sleep(std::time::Duration::from_secs(120)) => {
                let err = ierr!(
                    "BTW 流超时：120 秒未收到数据",
                    "BTW stream timed out: no data for 120s"
                );
                let _ = window.emit(
                    ev::BTW_CHUNK,
                    json!({ "overlay_id": overlay_id, "content": "", "done": true, "error": err.clone() }),
                );
                return Err(err);
            }
        };

        let chunk = match next {
            Some(Ok(c)) => c,
            Some(Err(e)) => {
                let _ = window.emit(
                    ev::BTW_CHUNK,
                    json!({ "overlay_id": overlay_id, "content": "", "done": true, "error": e.to_string() }),
                );
                return Err(e.to_string());
            }
            None => break,
        };

        if let Err(e) = line_buffer.push(&chunk) {
            let _ = window.emit(
                ev::BTW_CHUNK,
                json!({ "overlay_id": overlay_id, "content": "", "done": true, "error": e }),
            );
            return Err(e);
        }

        while let Some(line) = line_buffer.next_line() {
            if line.is_empty() {
                continue;
            }
            if line == "data: [DONE]" {
                let _ = window.emit(
                    ev::BTW_CHUNK,
                    json!({ "overlay_id": overlay_id, "content": "", "done": true }),
                );
                return Ok(());
            }
            if let Some(json_str) = line.strip_prefix("data: ") {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                    if let Some(err) = val.get("error") {
                        let msg = err
                            .get("message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("API Error");
                        let _ = window.emit(
                            ev::BTW_CHUNK,
                            json!({ "overlay_id": overlay_id, "content": "", "done": true, "error": msg }),
                        );
                        return Err(msg.to_string());
                    }
                    if is_anthropic {
                        // Anthropic stream parsing reuses the provider plugin's shared helpers
                        // (single implementation shared with handle_anthropic_stream_event, avoiding drift)
                        use crate::plugins::provider::anthropic as anth;
                        if let Some(content) = anth::anthropic_sse_text_delta(&val) {
                            let _ = window.emit(
                                ev::BTW_CHUNK,
                                json!({ "overlay_id": overlay_id, "content": content, "done": false }),
                            );
                        } else if anth::anthropic_sse_is_message_stop(&val) {
                            let _ = window.emit(
                                ev::BTW_CHUNK,
                                json!({ "overlay_id": overlay_id, "content": "", "done": true }),
                            );
                            return Ok(());
                        }
                    } else if let Some(content) = val["choices"][0]["delta"]["content"].as_str() {
                        let _ = window.emit(
                            ev::BTW_CHUNK,
                            json!({ "overlay_id": overlay_id, "content": content, "done": false }),
                        );
                    }
                }
            }
        }
    }

    // Stream ended without [DONE] — emit terminal event
    let _ = window.emit(
        ev::BTW_CHUNK,
        json!({ "overlay_id": overlay_id, "content": "", "done": true }),
    );
    Ok(())
}

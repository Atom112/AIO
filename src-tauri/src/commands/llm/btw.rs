//! BTW (By The Way) side-question commands.
//!
//! These commands answer quick contextual questions without modifying the main conversation.

use futures_util::StreamExt;
use serde_json::json;
use tauri::{Emitter, Window};

/// `/btw` 侧问命令：基于当前对话上下文回答一个简短问题（无工具、流式、不修改对话历史）。
///
/// 接收前端构建好的消息列表（含对话历史 + btw 问题），
/// 通过 SSE 流式调用 LLM，逐 chunk 通过 `btw-chunk` 事件推送到前端指定 overlay。
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
        // 剥离 aio-image 生成图片标记，防止 token 泄漏到 LLM
        if let Some(serde_json::Value::String(s)) = msg.get_mut("content") {
            *s = crate::utils::generated_images::strip_generated_image_tokens(s);
        }
    }

    // SSRF 防护
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
                "btw-chunk",
                json!({ "overlay_id": overlay_id, "content": "", "done": true, "error": e.to_string() }),
            );
            return Err(e.to_string());
        }
        Err(_) => {
            let _ = window.emit(
                "btw-chunk",
                json!({ "overlay_id": overlay_id, "content": "", "done": true, "error": "BTW 请求超时（60s）" }),
            );
            return Err("BTW 请求超时（60s）".to_string());
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
            "btw-chunk",
            json!({ "overlay_id": overlay_id, "content": "", "done": true, "error": format!("LLM API {}: {}", status, truncated) }),
        );
        return Err(format!("LLM API {}", status));
    }

    let mut stream = res.bytes_stream();
    let mut line_buffer = String::new();

    loop {
        let next = tokio::select! {
            item = stream.next() => item,
            _ = tokio::time::sleep(std::time::Duration::from_secs(120)) => {
                let _ = window.emit(
                    "btw-chunk",
                    json!({ "overlay_id": overlay_id, "content": "", "done": true, "error": "BTW 流超时：120 秒未收到数据" }),
                );
                return Err("BTW 流超时：120 秒未收到数据".to_string());
            }
        };

        let chunk = match next {
            Some(Ok(c)) => c,
            Some(Err(e)) => {
                let _ = window.emit(
                    "btw-chunk",
                    json!({ "overlay_id": overlay_id, "content": "", "done": true, "error": e.to_string() }),
                );
                return Err(e.to_string());
            }
            None => break,
        };

        line_buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = line_buffer.find('\n') {
            let line = line_buffer[..pos].trim().to_string();
            line_buffer.drain(..pos + 1);

            if line.is_empty() {
                continue;
            }
            if line == "data: [DONE]" {
                let _ = window.emit(
                    "btw-chunk",
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
                            "btw-chunk",
                            json!({ "overlay_id": overlay_id, "content": "", "done": true, "error": msg }),
                        );
                        return Err(msg.to_string());
                    }
                    if is_anthropic {
                        // Anthropic 流式：text_delta → content；message_stop → done
                        let etype = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if etype == "content_block_delta" {
                            let dtype = val
                                .get("delta")
                                .and_then(|d| d.get("type"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            if dtype == "text_delta" {
                                if let Some(content) = val
                                    .get("delta")
                                    .and_then(|d| d.get("text"))
                                    .and_then(|v| v.as_str())
                                {
                                    let _ = window.emit(
                                        "btw-chunk",
                                        json!({ "overlay_id": overlay_id, "content": content, "done": false }),
                                    );
                                }
                            }
                        } else if etype == "message_stop" {
                            let _ = window.emit(
                                "btw-chunk",
                                json!({ "overlay_id": overlay_id, "content": "", "done": true }),
                            );
                            return Ok(());
                        }
                    } else if let Some(content) = val["choices"][0]["delta"]["content"].as_str() {
                        let _ = window.emit(
                            "btw-chunk",
                            json!({ "overlay_id": overlay_id, "content": content, "done": false }),
                        );
                    }
                }
            }
        }
    }

    // Stream ended without [DONE] — emit terminal event
    let _ = window.emit(
        "btw-chunk",
        json!({ "overlay_id": overlay_id, "content": "", "done": true }),
    );
    Ok(())
}

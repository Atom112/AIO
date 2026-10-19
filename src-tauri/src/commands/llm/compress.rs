//! Automatic context compression (runtime compression inside the Agent loop, shared by the main loop and sub-agents).

use crate::commands::llm::budget::estimate_message_tokens;
use crate::commands::llm::client::{chat_error_message, chat_text_content, post_chat_completion};
use crate::commands::llm::handoff::archive_early_messages;
use crate::core::events as ev;
use crate::ierr;
use serde_json::json;
use tauri::Manager as _;
use tauri::{Emitter, Window};
use tokio_util::sync::CancellationToken;

/// Automatic context compression: compress early messages into a summary and keep recent messages within a token budget.
/// Used inside the Agent loop to avoid exceeding the model context window.
/// - The leading system block (stable prompt prefix: locale / knowledge / workflow instructions) is never compressed;
/// - The retention region accumulates from the tail within the keep_recent_tokens budget and snaps back to whole-round boundaries (never splits role:tool);
/// - Summary input is bounded (~30k chars max, most recent first) and always includes the first user message (the original task);
/// - The summary prompt and injection marker follow locale (English for en-US, Chinese otherwise).
#[allow(
    clippy::too_many_arguments,
    reason = "context compression reuses the active stream request context"
)]
pub(crate) async fn compress_context(
    window: &Window,
    client: &reqwest::Client,
    api_url: &str,
    api_key: &str,
    model: &str,
    messages: &mut Vec<serde_json::Value>,
    assistant_id: &str,
    topic_id: &str,
    token: &CancellationToken,
    keep_recent_tokens: usize,
    source: &str,
    locale: &str,
) -> Result<(), String> {
    if messages.len() <= 4 {
        return Ok(());
    }

    // Check the cancellation signal
    if token.is_cancelled() {
        return Err("cancelled".into());
    }

    // 1) Protect the leading system block (stable prompt prefix); never compressed
    let mut prefix_end = 0usize;
    while prefix_end < messages.len()
        && messages[prefix_end].get("role").and_then(|v| v.as_str()) == Some("system")
    {
        prefix_end += 1;
    }
    if prefix_end >= messages.len() {
        return Ok(());
    }

    // 2) Accumulate the retention region from the tail within the token budget
    let mut tail_start = messages.len();
    let mut acc: usize = 0;
    for i in (prefix_end..messages.len()).rev() {
        acc += estimate_message_tokens(model, &messages[i]);
        tail_start = i;
        if acc >= keep_recent_tokens {
            break;
        }
    }
    if tail_start == messages.len() {
        return Ok(()); // the whole compressible region fits the budget; nothing to compress
    }
    // The split point must not land on role:tool (separating a tool message from its assistant(tool_calls) triggers an API 400)
    while tail_start < messages.len()
        && messages[tail_start].get("role").and_then(|v| v.as_str()) == Some("tool")
    {
        tail_start += 1;
    }
    if tail_start <= prefix_end {
        return Ok(()); // the retention region already covers the whole compressible region
    }

    let early: Vec<serde_json::Value> = messages.drain(prefix_end..tail_start).collect();

    // 3) Build a bounded summary prompt: first user message (original task) + most recent early messages (~30k chars max, most recent first).
    //    pi-style serialization: assistant tool calls become explicit "[Assistant tool calls]: name(args)"
    //    lines, tool results get a 2000-char budget (they dominate context size).
    let block_for = |msg: &serde_json::Value| -> String {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("?");
        let text = match msg.get("content") {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        };
        let text = &text[..text.floor_char_boundary(text.len().min(2000))];
        if role == "assistant" {
            if let Some(calls) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                let names = calls
                    .iter()
                    .filter_map(|c| {
                        c.get("function").map(|f| {
                            let name = f.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                            let args = f.get("arguments").and_then(|v| v.as_str()).unwrap_or("");
                            let args = &args[..args.floor_char_boundary(args.len().min(200))];
                            format!("{name}({args})")
                        })
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                if !names.is_empty() {
                    return format!("[assistant tool calls]: {names}\n");
                }
            }
        }
        let label = match role {
            "user" => "user".to_string(),
            "assistant" => "assistant".to_string(),
            "tool" => format!(
                "tool result ({})",
                msg.get("name").and_then(|v| v.as_str()).unwrap_or("?")
            ),
            other => other.to_string(),
        };
        format!("[{label}]: {text}\n")
    };
    let mut blocks: Vec<String> = Vec::new();
    let mut budget = 30_000usize;
    for msg in early.iter().rev() {
        let block = block_for(msg);
        if block.len() > budget {
            break;
        }
        budget -= block.len();
        blocks.push(block);
    }
    blocks.reverse(); // restore chronological order
                      // Always include the first user message (the original task description) so the budget never drops it
    if let Some(first_user) = early
        .iter()
        .find(|m| m.get("role").and_then(|v| v.as_str()) == Some("user"))
    {
        if let Some(content) = first_user.get("content") {
            let text = match content {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let text = &text[..text.floor_char_boundary(text.len().min(300))];
            blocks.insert(0, format!("[user]: {text}\n"));
        }
    }

    let is_en = locale == "en-US";
    let mut summary_prompt = String::new();
    for b in &blocks {
        summary_prompt.push_str(b);
    }

    let body = json!({
        "model": model,
        "messages": [
            {"role": "system", "content": crate::commands::llm::summarize_prompt::compaction_instruction(is_en)},
            {"role": "user", "content": summary_prompt}
        ],
        "stream": false,
        "max_tokens": 2000
    });
    // Summary generation; on failure (timeout/network/API/empty) fall back to local archiving (snapcompact-style:
    // zero summary API cost, details recoverable via the read_artifact tool), guaranteeing compression never loses early context.
    let summary = match (async {
        let (res, is_anthropic) = tokio::select! {
            _ = token.cancelled() => return Err("cancelled".to_string()),
            result = tokio::time::timeout(
                std::time::Duration::from_secs(45),
                // Same topic session as the main rounds: the summary request
                // carries the early conversation, so a session-keyed gateway
                // (OpenCode Go prompt-cache routing) can reuse the warm route.
                post_chat_completion(client, api_url, api_key, &body, Some(topic_id)),
            ) => result,
        }
        .map_err(|_| {
            ierr!(
                "压缩摘要请求超时（45s）",
                "Compression summary request timed out (45s)"
            )
        })?
        .map_err(|e| e.to_string())?;

        let val: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;

        if let Some(err) = chat_error_message(&val) {
            return Err(err);
        }

        let s = chat_text_content(&val, is_anthropic);
        if s.is_empty() {
            Err(ierr!("摘要为空", "Summary is empty"))
        } else {
            Ok(s)
        }
    })
    .await
    {
        Ok(s) => s,
        Err(e) if e == "cancelled" => {
            // LOOP-3: early 区已 drain——取消返回前放回原位，调用方拿到的
            // 上下文不缺一整段（即使本轮即将终止）
            messages.splice(prefix_end..prefix_end, early);
            return Err("cancelled".into());
        }
        Err(e) => {
            // LOOP-3: 摘要与归档都失败时，early 消息必须原样放回（历史实现
            // 在 drain 之后用 `?` 传播归档错误，调用方拿到的残缺上下文
            // 继续跑下一轮且无用户可见告警）
            match archive_early_messages(window.app_handle(), &early, model, locale) {
                Ok(placeholder) => {
                    tracing::warn!(
                        "[compression] 摘要生成失败（{}），已本地归档 {} 条消息",
                        e,
                        early.len()
                    );
                    placeholder
                }
                Err(archive_err) => {
                    tracing::error!(
                        "[compression] 摘要与归档双双失败（{} / {}），已原样放回 early 消息",
                        e,
                        archive_err
                    );
                    messages.splice(prefix_end..prefix_end, early);
                    return Err(archive_err.to_string());
                }
            }
        }
    };

    // 4) Rebuild the message list: stable prefix -> summary message -> retained tail
    let tail: Vec<serde_json::Value> = messages.split_off(prefix_end);
    messages.push(json!({
        "role": "system",
        "content": format!(
            "{}\n{}",
            if is_en { "[History Summary — auto-compacted]" } else { "[历史摘要 — 自动压缩]" },
            summary
        )
    }));
    messages.extend(tail);

    let _ = window.emit(ev::LLM_COMPRESSION, json!({
        "assistant_id": assistant_id,
        "topic_id": topic_id,
        "source": source,
        "summary": summary,
        "compressed_chars": early.iter().map(|m| serde_json::to_string(m).map(|s| s.len()).unwrap_or(0)).sum::<usize>(),
    }));

    Ok(())
}

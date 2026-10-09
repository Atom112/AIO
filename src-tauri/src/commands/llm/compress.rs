//! 上下文自动压缩（Agent 循环内运行时压缩，供主循环与子 Agent 共用）。

use crate::commands::llm::*;
use crate::core::events as ev;
use serde_json::json;
use tauri::{Emitter, Window};
use tokio_util::sync::CancellationToken;

/// 上下文自动压缩：将早期消息压缩为摘要，按 token 预算保留最近消息。
/// 用于 Agent 循环中避免超出模型上下文窗口限制。
/// - 前置的 system 消息块（稳定提示词前缀：locale / 知识 / 工作流指令等）永不压缩；
/// - 保留区按 keep_recent_tokens token 预算从尾部累计，并回退到完整轮次边界（不切断 role:tool）；
/// - 摘要输入有界（最多约 30k 字符，最近优先），并固定带上首条 user 消息（原始任务）；
/// - 摘要提示词与注入标记跟随 locale（en-US 英文，其余中文）。
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

    // 检查取消信号
    if token.is_cancelled() {
        return Err("cancelled".into());
    }

    // 1) 保护前置 system 消息块（稳定提示词前缀），永不压缩
    let mut prefix_end = 0usize;
    while prefix_end < messages.len()
        && messages[prefix_end].get("role").and_then(|v| v.as_str()) == Some("system")
    {
        prefix_end += 1;
    }
    if prefix_end >= messages.len() {
        return Ok(());
    }

    // 2) 按 token 预算从尾部累计保留区
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
        return Ok(()); // 整个可压缩区都在预算内，无需压缩
    }
    // 切割点不得落在 role:tool 上（避免 tool 消息与其 assistant(tool_calls) 分离导致 API 400）
    while tail_start < messages.len()
        && messages[tail_start].get("role").and_then(|v| v.as_str()) == Some("tool")
    {
        tail_start += 1;
    }
    if tail_start <= prefix_end {
        return Ok(()); // 保留区已覆盖全部可压缩区
    }

    let early: Vec<serde_json::Value> = messages.drain(prefix_end..tail_start).collect();

    // 3) 构造有界的摘要提示：首条 user 消息（原始任务）+ 最近的早期消息（最多 ~30k 字符，最近优先）
    let mut blocks: Vec<String> = Vec::new();
    let mut budget = 30_000usize;
    for msg in early.iter().rev() {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("?");
        let text = match msg.get("content") {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        };
        let text = &text[..text.floor_char_boundary(text.len().min(500))];
        let block = format!("[{}]: {}\n", role, text);
        if block.len() > budget {
            break;
        }
        budget -= block.len();
        blocks.push(block);
    }
    blocks.reverse(); // 恢复时间顺序
                      // 固定带上首条 user 消息（原始任务描述），防止其被预算丢弃
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
            blocks.insert(0, format!("[user]: {}\n", text));
        }
    }

    let is_en = locale == "en-US";
    let mut summary_prompt = String::from(if is_en {
        "Summarize the following AI assistant conversation history. Preserve key decisions, file paths, code changes, and conclusions. Keep it within roughly 1200 words:\n\n"
    } else {
        "请总结以下 AI 助手对话的历史，保留关键决策、文件路径、代码变更和结论，控制在 2000 字以内：\n\n"
    });
    for b in &blocks {
        summary_prompt.push_str(b);
    }

    let body = json!({
        "model": model,
        "messages": [
            {"role": "system", "content": if is_en {
                "You are a conversation summarization assistant. Summarize the key points concisely in English."
            } else {
                "你是一个对话摘要助手。请用简洁的中文总结对话要点。"
            }},
            {"role": "user", "content": summary_prompt}
        ],
        "stream": false,
        "max_tokens": 1024
    });
    // 摘要生成；失败（超时/网络/API/空）时回退到本地归档（snapcompact 式：零摘要 API 成本，
    // 细节可经 read_artifact 工具恢复），保证压缩绝不丢失早期上下文。
    let summary = match (async {
        let (res, is_anthropic) = tokio::select! {
            _ = token.cancelled() => return Err("cancelled".to_string()),
            result = tokio::time::timeout(
                std::time::Duration::from_secs(45),
                post_chat_completion(client, api_url, api_key, &body),
            ) => result,
        }
        .map_err(|_| "压缩摘要请求超时（45s）".to_string())?
        .map_err(|e| e.to_string())?;

        let val: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;

        if let Some(err) = chat_error_message(&val) {
            return Err(err);
        }

        let s = chat_text_content(&val, is_anthropic);
        if s.is_empty() {
            Err("摘要为空".to_string())
        } else {
            Ok(s)
        }
    })
    .await
    {
        Ok(s) => s,
        Err(e) if e == "cancelled" => return Err("cancelled".into()),
        Err(e) => {
            let placeholder = archive_early_messages(window.app_handle(), &early, model, locale)?;
            tracing::warn!(
                "[compression] 摘要生成失败（{}），已本地归档 {} 条消息",
                e,
                early.len()
            );
            placeholder
        }
    };

    // 4) 重建消息列表：稳定前缀 → 摘要消息 → 保留尾部
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

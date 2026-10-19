//! Topic title and history summary commands (generate_topic_title, summarize_history, archive_history).

use crate::commands::llm::client::{
    chat_error_message, chat_finish_reason, chat_text_content, non_streaming_http_client_pinned,
    post_chat_completion,
};
use crate::commands::llm::handoff::archive_early_messages;
use crate::commands::llm::messages::extract_text_content;
use crate::ierr;
use serde_json::json;

/// Generate a history summary (used by the frontend's manually triggered context compression).
///
/// # Parameters
/// - `api_url` / `api_key` / `model` — target LLM endpoint and model
/// - `messages_json` — JSON array string of messages to summarize (frontend-serialized)
/// - `locale` — UI language ("en-US" gets the English prompt, otherwise Chinese)
///
/// # Behavior
/// Validates the URL, then issues one non-streaming completion request and returns the
/// summary text; on failure a Chinese error string is raised.
#[tauri::command]
pub async fn summarize_history(
    api_url: String,
    api_key: String,
    model: String,
    messages_json: String,
    locale: Option<String>,
) -> Result<String, String> {
    let messages: Vec<serde_json::Value> = serde_json::from_str(&messages_json)
        .map_err(|e| format!("Invalid messages JSON: {}", e))?;
    // diagnostic log (debug only: keeps conversation structure out of production logs)
    for (i, m) in messages.iter().enumerate() {
        tracing::debug!(
            "[summarize_history] messages[{}] role={} has_tool_call_id={} has_tool_calls={} has_name={}",
            i,
            m.get("role").and_then(|v| v.as_str()).unwrap_or("?"),
            m.get("tool_call_id").is_some(),
            m.get("tool_calls").is_some(),
            m.get("name").is_some()
        );
    }
    // SSRF protection
    crate::utils::url_validation::validate_http_url(
        &api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;
    // SEC-7: pinned resolution (key-bearing request; local/LAN allowed, metadata refused)
    let client = non_streaming_http_client_pinned(&api_url).await?;

    let mut messages_for_api: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            let content = match &m["content"] {
                serde_json::Value::String(s) => {
                    json!(crate::utils::generated_images::strip_generated_image_tokens(s))
                }
                other => other.clone(),
            };
            json!({ "role": &m["role"], "content": content })
        })
        .collect();

    let locale = locale.unwrap_or_default();
    let is_en = locale == "en-US";
    // Structured compaction directive (pi sections + DSH verbatim rules); the frontend
    // sends the conversation already serialized as a single user message
    messages_for_api.push(json!({
        "role": "system",
        "content": crate::commands::llm::summarize_prompt::compaction_instruction(is_en),
    }));

    let body = json!({
        "model": model,
        "messages": messages_for_api,
        "stream": false,
        "max_tokens": 2000
    });
    let (res, is_anthropic) = tokio::time::timeout(
        std::time::Duration::from_secs(45),
        post_chat_completion(&client, &api_url, &api_key, &body, None),
    )
    .await
    .map_err(|_| ierr!("摘要请求超时（45s）", "Summary request timed out (45s)"))?
    .map_err(|e| e.to_string())?;

    let val: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;

    if let Some(err) = chat_error_message(&val) {
        return Err(err);
    }

    let summary = chat_text_content(&val, is_anthropic);
    let summary = if summary.is_empty() {
        ierr!("无法生成总结", "Unable to generate a summary")
    } else {
        summary
    };

    Ok(summary)
}

/// Archive early messages locally (fallback when manual /compact summarization fails;
/// snapcompact-style, zero API cost).
///
/// # Parameters
/// - `messages_json` — JSON array string of messages to archive (frontend-serialized)
/// - `model` — model name used for token estimation
/// - `locale` — UI language ("en-US" gets the English placeholder, otherwise Chinese)
///
/// # Behavior
/// Writes {app_data}/archives/{uuid}.txt (directory shared with the read_artifact tool) and
/// returns a locale-dependent `[Archive #id]`-style placeholder; the model can call
/// read_artifact with that placeholder ID to read the original content back.
#[tauri::command]
pub fn archive_history(
    app: tauri::AppHandle,
    messages_json: String,
    model: String,
    locale: Option<String>,
) -> Result<String, String> {
    let messages: Vec<serde_json::Value> = serde_json::from_str(&messages_json)
        .map_err(|e| format!("Invalid messages JSON: {}", e))?;
    if messages.is_empty() {
        return Err(ierr!(
            "归档需要至少一条消息",
            "Archive requires at least one message"
        ));
    }
    archive_early_messages(&app, &messages, &model, &locale.unwrap_or_default())
}

/// Clean a raw model-returned string into a well-formed title:
/// 1. Trim whitespace and paired quotes (half/full width, CJK quotes, backticks)
/// 2. Take the first non-empty line (avoids multi-line output)
/// 3. Recursively strip common Chinese/English prefixes ("Title:" and its Chinese variants)
/// 4. Remove paired Markdown markers (**...** / `...`)
pub(crate) fn clean_topic_title(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // take the first non-empty line
    let first_line = trimmed
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or("");

    let mut s = first_line.to_string();

    // strip common prefixes (up to 3 passes, to handle nested prefixes)
    const PREFIXES: &[&str] = &[
        "好的，标题是：",
        "好的，标题是:",
        "好的，标题：",
        "好的，标题:",
        "好的：",
        "好的:",
        "好的，",
        "好的,",
        "标题是：",
        "标题是:",
        "标题：",
        "标题:",
        "Title:",
        "Title：",
        "title:",
        "title：",
        "以下是",
        "以下为",
        "下面给出",
        "给你一个",
        "Here is the title:",
        "Here is the title：",
        "The title is:",
        "The title is：",
    ];
    for _ in 0..3 {
        let mut matched = false;
        for p in PREFIXES {
            if s.starts_with(p) {
                s = s[p.len()..].trim().to_string();
                matched = true;
                break;
            }
        }
        if !matched {
            break;
        }
    }

    // strip paired leading/trailing quotes (Chinese/English + backticks + CJK quotes)
    s = s
        .trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\''
                    | '`'
                    | '「'
                    | '」'
                    | '『'
                    | '』'
                    | '\u{201C}'
                    | '\u{201D}'
                    | '\u{2018}'
                    | '\u{2019}'
            )
        })
        .to_string();

    // remove paired Markdown markers
    if s.len() > 4 && s.starts_with("**") && s.ends_with("**") {
        s = s[2..s.len() - 2].to_string();
    } else if s.len() > 2 && s.starts_with('`') && s.ends_with('`') {
        s = s[1..s.len() - 1].to_string();
    }

    s.trim().to_string()
}

/// Generate a short title (4-20 characters) for a topic.
/// Called once by the frontend after the first exchange in a new topic; the frontend then
/// sets `topic.renamed` to `true` and does not call again, avoiding duplicate renames.
/// Content generation only, no database writes; persistence happens in the frontend via
/// `save_assistant` after the Store update.
///
/// # Parameters
/// - `api_url` / `api_key` / `model`: LLM credentials used by the caller (same as streaming chat)
/// - `messages`: conversation content for title generation (first 2-4 messages recommended)
///
/// # Returns
/// On success, the cleaned title string (quotes, whitespace, newlines and common prefixes
/// stripped; length capped at 1-20 characters).
///
/// # Failure modes
/// If the LLM returns empty content (finish_reason=stop with empty content), the error carries
/// the model name and raw length for diagnosis; the frontend should use its heuristic fallback.
#[tauri::command]
pub async fn generate_topic_title(
    api_url: String,
    api_key: String,
    model: String,
    messages_json: String,
) -> Result<String, String> {
    let messages: Vec<serde_json::Value> = serde_json::from_str(&messages_json)
        .map_err(|e| format!("Invalid messages JSON: {}", e))?;
    // diagnostic log
    for (i, m) in messages.iter().enumerate() {
        tracing::info!(
            "[generate_topic_title] messages[{}] role={} has_tool_call_id={} has_tool_calls={} has_name={}",
            i,
            m.get("role").and_then(|v| v.as_str()).unwrap_or("?"),
            m.get("tool_call_id").is_some(),
            m.get("tool_calls").is_some(),
            m.get("name").is_some()
        );
    }
    // SSRF protection
    crate::utils::url_validation::validate_http_url(
        &api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;
    if messages.is_empty() {
        return Err(ierr!(
            "生成标题需要至少一条消息",
            "Title generation requires at least one message"
        ));
    }

    // SEC-7: pinned resolution (key-bearing request; local/LAN allowed, metadata refused)
    let client = non_streaming_http_client_pinned(&api_url).await?;

    let mut messages_for_api: Vec<serde_json::Value> = vec![json!({
        "role": "system",
        "content": "你是一个话题标题生成助手，擅长用最少的字数精准概括对话核心内容。"
    })];

    for m in &messages {
        let text = match &m["content"] {
            serde_json::Value::String(s) => {
                crate::utils::generated_images::strip_generated_image_tokens(s)
            }
            other => extract_text_content(other),
        };
        if text.trim().is_empty() {
            continue;
        }
        messages_for_api.push(json!({ "role": &m["role"], "content": text }));
    }

    messages_for_api.push(json!({
        "role": "user",
        "content": "请根据以上对话生成一个 4-20 字的话题标题。\n\
                     严格要求：\n\
                     1. 精准概括核心主题或关键问题\n\
                     2. 不要加引号、冒号、序号、'好的'、'以下是'等多余文字\n\
                     3. 不要使用任何 Markdown 标记\n\
                     4. 你的回复必须且只能包含标题本身"
    }));

    let body = json!({
        "model": model,
        "messages": messages_for_api,
        "stream": false,
        "max_tokens": 200,
        "temperature": 0.0
    });

    let (res, is_anthropic) = tokio::time::timeout(
        std::time::Duration::from_secs(45),
        post_chat_completion(&client, &api_url, &api_key, &body, None),
    )
    .await
    .map_err(|_| {
        ierr!(
            "标题生成请求超时（45s）",
            "Title generation request timed out (45s)"
        )
    })?
    .map_err(|e| e.to_string())?;

    let val: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;

    if let Some(err) = chat_error_message(&val) {
        return Err(err);
    }

    let raw = chat_text_content(&val, is_anthropic);

    let cleaned = clean_topic_title(&raw);

    if cleaned.is_empty() {
        let finish = chat_finish_reason(&val, is_anthropic);
        return Err(ierr!(
            "模型 {} 返回的标题为空 (finish_reason={}, raw_len={})",
            "Model {} returned an empty title (finish_reason={}, raw_len={})",
            model,
            finish,
            raw.len()
        ));
    }

    let truncated: String = if cleaned.chars().count() > 20 {
        cleaned.chars().take(20).collect()
    } else {
        cleaned
    };

    Ok(truncated)
}

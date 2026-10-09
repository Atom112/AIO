//! 话题标题与历史摘要命令（generate_topic_title、summarize_history、archive_history）。

use crate::commands::llm::*;
use crate::ierr;
use serde_json::json;

/// 生成历史摘要（供前端手动触发的上下文压缩使用）。
///
/// # 参数
/// - `api_url` / `api_key` / `model` — 目标 LLM 端点与模型
/// - `messages_json` — 待摘要消息的 JSON 数组字符串（由前端序列化传入）
/// - `locale` — UI 语言（"en-US" 英文提示词，其余中文）
///
/// # 行为
/// 校验 URL 后发起一次非流式补全请求，返回摘要文本；失败时以中文错误串上抛。
#[tauri::command]
pub(crate) async fn summarize_history(
    api_url: String,
    api_key: String,
    model: String,
    messages_json: String,
    locale: Option<String>,
) -> Result<String, String> {
    let messages: Vec<serde_json::Value> = serde_json::from_str(&messages_json)
        .map_err(|e| format!("Invalid messages JSON: {}", e))?;
    // 诊断日志（仅 debug：避免生产日志记录对话结构）
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
    // SSRF 防护
    crate::utils::url_validation::validate_http_url(
        &api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;
    let client = non_streaming_http_client();

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
    messages_for_api.push(json!({
        "role": "system",
        "content": if is_en {
            "Briefly summarize the core content and user needs of the conversation above, as long-term memory for future exchanges (within 300 words)."
        } else {
            "请简要总结以上对话的核心内容和用户需求，作为后续交流的长期记忆（500字以内）。"
        }
    }));

    let body = json!({
        "model": model,
        "messages": messages_for_api,
        "stream": false,
        "max_tokens": 700
    });
    let (res, is_anthropic) = tokio::time::timeout(
        std::time::Duration::from_secs(45),
        post_chat_completion(&client, &api_url, &api_key, &body),
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

/// 归档早期消息到本地（手动 /compact 的摘要失败回退，snapcompact 式零 API 成本）。
///
/// # 参数
/// - `messages_json` — 待归档消息的 JSON 数组字符串（由前端序列化传入）
/// - `model` — 用于 token 估算的模型名
/// - `locale` — UI 语言（"en-US" 英文占位文本，其余中文）
///
/// # 行为
/// 写入 {app_data}/archives/{uuid}.txt（与 read_artifact 工具共用目录），
/// 返回 `[历史归档 #id]` 占位文本；Agent 模式下模型可按占位 ID 调 read_artifact 读回原文。
#[tauri::command]
pub(crate) fn archive_history(
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

/// 清洗模型返回的原始字符串为合规标题。
/// 1. 去除首尾空白与首尾成对引号（半角 / 全角 / 中文书名号 / 反引号）
/// 2. 取第一个非空行（避免多行输出）
/// 3. 递归剥离常见中英文前缀（"标题：" / "Title:" / "好的，" / "以下是" 等）
/// 4. 去除成对 Markdown 标记（**...** / `...`）
pub(crate) fn clean_topic_title(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // 取第一个非空行
    let first_line = trimmed
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or("");

    let mut s = first_line.to_string();

    // 剥离常见前缀（最多尝试 3 轮，防止 "好的，标题是：xxx" 这种嵌套）
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

    // 去除首尾成对引号（中英文 + 反引号 + 书名号）
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

    // 去除成对 Markdown 标记
    if s.len() > 4 && s.starts_with("**") && s.ends_with("**") {
        s = s[2..s.len() - 2].to_string();
    } else if s.len() > 2 && s.starts_with('`') && s.ends_with('`') {
        s = s[1..s.len() - 1].to_string();
    }

    s.trim().to_string()
}

/// 为话题生成一个简短标题（4-20 个字符）。
/// 由前端在新话题的"第一次对话"后调用一次，生成后前端将 `topic.renamed` 置为 `true`，
/// 后续不再调用以避免重复重命名。
/// 仅做内容生成，不写入数据库 —— 持久化由前端在更新 Store 后通过 `save_assistant` 完成。
///
/// # 参数
/// - `api_url` / `api_key` / `model`：调用方所用的 LLM 凭据（与流式对话保持一致）
/// - `messages`：用于生成标题的对话内容（建议取前 2~4 条）
///
/// # 返回
/// 成功时返回清洗后的标题字符串（已去除引号、空白、换行与常见前缀，长度限制在 1-20 字符内）。
///
/// # 失败模式
/// 若 LLM 长时间返回空内容（finish_reason=stop 且 content 为空），错误信息会附带
/// 模型名与原始长度，便于排查。前端应在 catch 中走启发式后备方案。
#[tauri::command]
pub(crate) async fn generate_topic_title(
    api_url: String,
    api_key: String,
    model: String,
    messages_json: String,
) -> Result<String, String> {
    let messages: Vec<serde_json::Value> = serde_json::from_str(&messages_json)
        .map_err(|e| format!("Invalid messages JSON: {}", e))?;
    // 诊断日志
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
    // SSRF 防护
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

    let client = non_streaming_http_client();

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
        post_chat_completion(&client, &api_url, &api_key, &body),
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

//! LLM call wrapper for the memory pipeline: reuses the main agent's non-streaming request path (including the Anthropic adapter).

use crate::ierr;
use serde_json::Value;
use std::time::Duration;
use tauri::AppHandle;

/// LLM call config (apiUrl / apiKey / model come from the app config).
#[derive(Clone, Debug)]
pub(crate) struct LlmConfig {
    pub api_url: String,
    pub api_key: String,
    pub model: String,
}

/// Build the LLM call config from the app config (returns Err when no valid model is configured).
pub(crate) fn llm_config(app: &AppHandle) -> Result<LlmConfig, String> {
    let cfg = crate::core::config::load_app_config(app).unwrap_or_default();
    if cfg.api_url.trim().is_empty() || cfg.default_model.trim().is_empty() {
        return Err(ierr!(
            "未配置模型（apiUrl/model 为空），无法执行记忆提取/仲裁",
            "No model configured (apiUrl/model is empty); cannot run memory extraction/arbitration"
        ));
    }
    Ok(LlmConfig {
        api_url: cfg.api_url,
        api_key: cfg.api_key,
        model: cfg.default_model,
    })
}

/// Issue a non-streaming chat completion asking the model to return JSON (returns Err on failure; if the model reply cannot be parsed as JSON, the caller provides the fallback).
pub(crate) async fn chat_json(
    cfg: &LlmConfig,
    system: &str,
    user: &str,
    max_tokens: u32,
) -> Result<Value, String> {
    crate::utils::url_validation::validate_http_url(
        &cfg.api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;
    // SEC-7: pinned resolution (key-bearing request; local/LAN allowed, metadata refused)
    let client = crate::plugins::llm::non_streaming_http_client_pinned(&cfg.api_url).await?;
    let body = serde_json::json!({
        "model": cfg.model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
        "stream": false,
        "max_tokens": max_tokens,
    });
    let (res, is_anthropic) = tokio::time::timeout(
        Duration::from_secs(45),
        crate::plugins::llm::post_chat_completion(&client, &cfg.api_url, &cfg.api_key, &body),
    )
    .await
    .map_err(|_| ierr!("LLM 调用超时（45s）", "LLM request timed out (45s)"))??;
    let val: Value = res.json().await.map_err(|e| {
        ierr!(
            "解析 LLM 响应失败: {}",
            "Failed to parse LLM response: {}",
            e
        )
    })?;
    if let Some(err) = crate::plugins::llm::chat_error_message(&val) {
        return Err(err);
    }
    let text = crate::plugins::llm::chat_text_content(&val, is_anthropic);
    parse_json_text(&text)
}

/// Parse JSON from model output text (tolerates ```json code fences and surrounding text).
pub(crate) fn parse_json_text(text: &str) -> Result<Value, String> {
    let trimmed = text.trim();
    let mut candidate = trimmed;
    if let Some(stripped) = strip_fence(candidate) {
        candidate = stripped;
    }
    match serde_json::from_str::<Value>(candidate) {
        Ok(v) => Ok(v),
        Err(_) => {
            // Try to extract the substring between the first { and the last }
            if let (Some(start), Some(end)) = (candidate.find('{'), candidate.rfind('}')) {
                if end > start {
                    return serde_json::from_str::<Value>(&candidate[start..=end])
                        .map_err(|e| e.to_string());
                }
            }
            if let (Some(start), Some(end)) = (candidate.find('['), candidate.rfind(']')) {
                if end > start {
                    return serde_json::from_str::<Value>(&candidate[start..=end])
                        .map_err(|e| e.to_string());
                }
            }
            Err(ierr!(
                "无法解析模型输出的 JSON: {}",
                "Failed to parse JSON from model output: {}",
                trimmed.chars().take(120).collect::<String>()
            ))
        }
    }
}

fn strip_fence(text: &str) -> Option<&str> {
    let t = text.trim();
    let first = t.lines().next()?;
    if first.starts_with("```") {
        let rest = t.strip_prefix(first)?;
        let rest = rest.strip_prefix('\n').unwrap_or(rest);
        let end = rest.rfind("```")?;
        Some(rest[..end].trim())
    } else {
        None
    }
}

/// Query rewrite (optional, called before retrieval): asks the model for 1-2 query variants better suited to semantic retrieval.
/// Returns only new variants, never the original query; returns an empty Vec on failure or no output (the caller falls back to the original query).
pub(crate) async fn rewrite_query(cfg: &LlmConfig, query: &str) -> Result<Vec<String>, String> {
    let system = concat!(
        "你是检索查询改写器。给定用户的检索意图，输出 1-2 个更利于语义/关键词检索的查询变体：\n",
        "- 补充同义词、领域术语、中英双语表达；\n",
        "- 保持原意不变，不添油加醋；\n",
        "- 只输出 JSON 字符串数组，如 [\\\"...\\\", \\\"...\\\"]。",
    );
    let user = format!("检索意图：{query}");
    match chat_json(cfg, system, &user, 150).await {
        Ok(v) => {
            let arr = match v {
                Value::Array(a) => a,
                _ => return Ok(Vec::new()),
            };
            let mut out = Vec::new();
            for item in arr {
                if let Some(s) = item.as_str() {
                    let s = s.trim().to_string();
                    if !s.is_empty() && s != query {
                        out.push(s);
                    }
                }
            }
            Ok(out.into_iter().take(2).collect())
        }
        Err(_) => Ok(Vec::new()),
    }
}

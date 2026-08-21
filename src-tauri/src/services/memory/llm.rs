//! 记忆管线用的 LLM 调用封装：复用主 Agent 的非流式请求路径（含 Anthropic 适配）。

use crate::core::models::AppConfig;
use serde_json::Value;
use std::time::Duration;
use tauri::AppHandle;

/// LLM 调用配置（apiUrl / apiKey / model 来自应用配置）。
#[derive(Clone, Debug)]
pub(crate) struct LlmConfig {
    pub api_url: String,
    pub api_key: String,
    pub model: String,
}

/// 从应用配置构建 LLM 调用配置（无有效模型时返回 Err）。
pub(crate) fn llm_config(app: &AppHandle) -> Result<LlmConfig, String> {
    let cfg = crate::commands::config::load_app_config(app.clone()).unwrap_or_default();
    if cfg.api_url.trim().is_empty() || cfg.default_model.trim().is_empty() {
        return Err("未配置模型（apiUrl/model 为空），无法执行记忆提取/仲裁".into());
    }
    Ok(LlmConfig {
        api_url: cfg.api_url,
        api_key: cfg.api_key,
        model: cfg.default_model,
    })
}

/// 发起非流式聊天补全，要求模型返回 JSON（失败返回 Err；模型回复无法解析 JSON 时由调用方兜底）。
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
    let client = crate::commands::llm::non_streaming_http_client();
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
        crate::commands::llm::post_chat_completion(&client, &cfg.api_url, &cfg.api_key, &body),
    )
    .await
    .map_err(|_| "LLM 调用超时（45s）".to_string())??;
    let val: Value = res
        .json()
        .await
        .map_err(|e| format!("解析 LLM 响应失败: {e}"))?;
    if let Some(err) = crate::commands::llm::chat_error_message(&val) {
        return Err(err);
    }
    let text = crate::commands::llm::chat_text_content(&val, is_anthropic);
    parse_json_text(&text)
}

/// 从模型输出文本中解析 JSON（容忍 ```json 代码围栏与前后缀）。
pub(crate) fn parse_json_text(text: &str) -> Result<Value, String> {
    let trimmed = text.trim();
    let mut candidate = trimmed;
    if let Some(stripped) = strip_fence(candidate) {
        candidate = stripped;
    }
    match serde_json::from_str::<Value>(candidate) {
        Ok(v) => Ok(v),
        Err(_) => {
            // 尝试提取第一个 { 到最后一个 } 之间的子串
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
            Err(format!(
                "无法解析模型输出的 JSON: {}",
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

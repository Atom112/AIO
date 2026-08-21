//! 专用图像生成端点 `/images/generations`（dall-e-2/3 等）。
//!
//! 与 chat/completions 内嵌图像不同，这些模型没有 chat 模式，只能走专用非流式端点。
//! 返回的 `b64_json`/`url` 统一经 [`crate::utils::generated_images`] 落盘并重写为
//! `aio-image://` 标记，随后以 `llm-chunk` done 载荷回传，前端渲染逻辑与流式一致。

use crate::core::models::StreamPayload;
use serde_json::json;
use tauri::{AppHandle, Emitter, Window};

/// 把 chat completions 基址推导为 images/generations 地址。
///
/// - `…/chat/completions` → 去掉该后缀后追加 `/images/generations`（保留 `/v1` 等路径前缀）
/// - 其它 → 追加 `/images/generations`
fn images_generations_url(api_url: &str) -> String {
    let trimmed = api_url.trim_end_matches('/');
    if let Some(base) = trimmed.strip_suffix("/chat/completions") {
        format!("{}/images/generations", base)
    } else {
        format!("{}/images/generations", trimmed)
    }
}

/// 调用专用图像生成端点并把结果落盘、重写为本地标记后回传前端。
///
/// # 参数
/// - `api_url`：Provider base URL（chat completions 形态或 v1 形态，自动推导 images 地址）
/// - `api_key`：Provider API key
/// - `model`：图像模型 id（如 dall-e-2 / dall-e-3）
/// - `prompt`：图像生成提示词
/// - `assistant_id` / `topic_id`：用于匹配前端消息与 `llm-chunk` done 载荷
///
/// 成功时 `llm-chunk(done)` 携带重写后的内容与图像元数据；任何错误先 emit
/// `llm-chunk(done, error)` 再返回 `Err`（镜像 run_agent_turn epilogue 契约）。
#[tauri::command]
#[allow(
    clippy::too_many_arguments,
    reason = "Tauri IPC compatibility requires the existing flat command parameters"
)]
pub async fn generate_image(
    window: Window,
    app: AppHandle,
    api_url: String,
    api_key: String,
    model: String,
    prompt: String,
    assistant_id: String,
    topic_id: String,
) -> Result<(), String> {
    // SSRF 防护（与流式路径同款，允许 localhost）
    crate::utils::url_validation::validate_http_url(
        &api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;

    let emit_done = |content: String,
                     error: Option<String>,
                     images: Option<Vec<crate::core::models::GeneratedImage>>| {
        let _ = window.emit(
            "llm-chunk",
            StreamPayload {
                assistant_id: assistant_id.clone(),
                topic_id: topic_id.clone(),
                content,
                done: true,
                error: error.clone(),
                input_tokens: None,
                output_tokens: None,
                context_tokens: None,
                cached_input_tokens: None,
                images,
            },
        );
    };
    let emit_error = |e: String| {
        emit_done(format!("\n[Error: {}]", e), Some(e.clone()), None);
        e
    };

    // 专用非流式 client：图像生成可超过 60s，放宽到 180s
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .map_err(|e| emit_error(e.to_string()))?;

    let response = client
        .post(images_generations_url(&api_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&json!({
            "model": model,
            "prompt": prompt,
            "n": 1,
            "response_format": "b64_json",
        }))
        .send()
        .await
        .map_err(|e| emit_error(e.to_string()))?;

    let status = response.status();
    if !status.is_success() {
        let body_text = response.text().await.unwrap_or_default();
        let truncated = if body_text.len() > 512 {
            &body_text[..body_text.floor_char_boundary(512)]
        } else {
            &body_text
        };
        return Err(emit_error(format!("LLM API {}: {}", status, truncated)));
    }

    let val: serde_json::Value = response
        .json()
        .await
        .map_err(|e| emit_error(e.to_string()))?;

    let item = val
        .get("data")
        .and_then(|d| d.get(0))
        .ok_or_else(|| emit_error("图像生成响应缺少 data[0]".to_string()))?;

    // b64_json（首选）→ data URI；否则回退 url 字段
    let data_or_url = if let Some(b64) = item.get("b64_json").and_then(|v| v.as_str()) {
        format!("data:image/png;base64,{}", b64)
    } else if let Some(u) = item.get("url").and_then(|v| v.as_str()) {
        u.to_string()
    } else {
        return Err(emit_error("图像生成响应缺少 b64_json/url".to_string()));
    };

    let content0 = item
        .get("revised_prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut content = content0;
    let images =
        crate::utils::generated_images::save_generated_images(&app, &mut content, &[data_or_url])
            .await
            .map_err(emit_error)?;

    emit_done(
        content,
        None,
        if images.is_empty() {
            None
        } else {
            Some(images)
        },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_url_from_chat_completions() {
        assert_eq!(
            images_generations_url("https://api.example.com/v1/chat/completions"),
            "https://api.example.com/v1/images/generations"
        );
    }

    #[test]
    fn images_url_from_v1() {
        assert_eq!(
            images_generations_url("https://api.example.com/v1"),
            "https://api.example.com/v1/images/generations"
        );
        assert_eq!(
            images_generations_url("https://api.example.com/v1/"),
            "https://api.example.com/v1/images/generations"
        );
    }

    #[test]
    fn images_url_fallback() {
        assert_eq!(
            images_generations_url("https://api.example.com"),
            "https://api.example.com/images/generations"
        );
    }
}

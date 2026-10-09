//! Dedicated image-generation endpoint `/images/generations` (dall-e-2/3 etc.).
//!
//! Unlike images embedded in chat/completions, these models have no chat mode and only
//! work through the dedicated non-streaming endpoint. Returned `b64_json`/`url` values are
//! uniformly persisted by [`crate::utils::generated_images`] and rewritten into
//! `aio-image://` markers, then sent back via the `llm-chunk` done payload; the frontend renders them the same way as streaming.

use crate::core::events as ev;
use crate::core::models::StreamPayload;
use serde_json::json;
use tauri::{AppHandle, Emitter, Window};

/// Derive the images/generations URL from a chat completions base URL.
///
/// - `…/chat/completions` -> strip that suffix and append `/images/generations` (keeps path prefixes such as `/v1`)
/// - otherwise -> append `/images/generations`
fn images_generations_url(api_url: &str) -> String {
    let trimmed = api_url.trim_end_matches('/');
    if let Some(base) = trimmed.strip_suffix("/chat/completions") {
        format!("{}/images/generations", base)
    } else {
        format!("{}/images/generations", trimmed)
    }
}

/// Call the dedicated image-generation endpoint, persist results, rewrite them into local markers, and return them to the frontend.
///
/// # Parameters
/// - `api_url`: Provider base URL (chat completions form or v1 form; the images URL is derived automatically)
/// - `api_key`: Provider API key
/// - `model`: image model id (e.g. dall-e-2 / dall-e-3)
/// - `prompt`: image generation prompt
/// - `assistant_id` / `topic_id`: used to match the frontend message and the `llm-chunk` done payload
///
/// On success `llm-chunk(done)` carries the rewritten content and image metadata; on any error an
/// `llm-chunk(done, error)` is emitted first, then `Err` is returned (mirroring the run_agent_turn epilogue contract).
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
    // SSRF protection (same as the streaming path; localhost allowed)
    crate::utils::url_validation::validate_http_url(
        &api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;

    let emit_done = |content: String,
                     error: Option<String>,
                     images: Option<Vec<crate::core::models::GeneratedImage>>| {
        let _ = window.emit(
            ev::LLM_CHUNK,
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

    // Dedicated non-streaming client: image generation can exceed 60s, so the timeout is relaxed to 180s
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

    // b64_json (preferred) -> data URI; otherwise fall back to the url field
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

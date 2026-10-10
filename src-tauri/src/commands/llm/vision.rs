//! Auto vision analysis: the vision subagent's "look at the image" call.
//!
//! When the user sends images to a model without vision capability, the
//! frontend invokes this command BEFORE the main-model request: a
//! vision-capable model describes the images, and the description replaces
//! the raw image parts in the conversation (a non-vision model would
//! otherwise ignore the base64 payloads or reject the request outright).

use crate::commands::llm::*;
use crate::core::state::DbState;
use crate::ierr;
use base64::engine::general_purpose;
use serde_json::json;
use tauri::{AppHandle, Manager};

const VISION_SYSTEM_ZH: &str = "你是 AIO 的视觉分析智能体。主对话模型不支持图像输入，由你代为查看用户发送的图片。请输出结构化的描述，不要臆造图中不存在的信息。";

const VISION_SYSTEM_EN: &str = "You are the AIO vision analysis agent. The main conversation model cannot process images, so you look at the user's images on its behalf. Output a structured description; do not invent anything not present in the images.";

/// Analyze user image attachments with a vision-capable model.
///
/// # Parameters
/// - `api_url` / `api_key` / `model` — endpoint of the vision-capable model
/// - `attachment_ids` — attachment ids; image/* entries are loaded from the
///   DB and base64-encoded into the request (non-image ids are skipped)
/// - `user_text` — the user's message text, so the analysis stays relevant
///   to what was actually asked
/// - `locale` — "en-US" analyzes in English, otherwise Chinese
///
/// # Returns
/// The structured image description text (replaces the images for the
/// non-vision main model). Non-streaming, 45s timeout.
#[tauri::command]
pub(crate) async fn vision_analyze_images(
    app: AppHandle,
    api_url: String,
    api_key: String,
    model: String,
    attachment_ids: Vec<String>,
    user_text: String,
    locale: Option<String>,
) -> Result<String, String> {
    // SSRF protection (same policy as the other LLM calls)
    crate::utils::url_validation::validate_http_url(
        &api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;

    // Load image attachments (mirrors message_for_api's attachment path).
    // Scoped so the connection guard is dropped before the first await —
    // a guard alive across an await would make the command future !Send.
    let (image_parts, names): (Vec<serde_json::Value>, Vec<String>) = {
        let state = app.state::<DbState>();
        let conn = state.0.lock();
        let mut parts: Vec<serde_json::Value> = Vec::new();
        let mut file_names: Vec<String> = Vec::new();
        for id in &attachment_ids {
            let row = conn
                .query_row(
                    "SELECT file_name, mime_type, storage_path FROM attachments WHERE id = ?1",
                    [id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .map_err(|e| {
                    ierr!(
                        "读取附件 {} 失败: {}",
                        "Failed to read attachment {}: {}",
                        id,
                        e
                    )
                })?;
            if !row.1.starts_with("image/") {
                continue;
            }
            let bytes = std::fs::read(&row.2).map_err(|e| {
                ierr!(
                    "读取图片附件 {} 失败: {}",
                    "Failed to read image attachment {}: {}",
                    row.0,
                    e
                )
            })?;
            parts.push(json!({
                "type": "image_url",
                "image_url": { "url": format!("data:{};base64,{}", row.1, general_purpose::STANDARD.encode(bytes)) }
            }));
            file_names.push(row.0);
        }
        (parts, file_names)
    };
    if image_parts.is_empty() {
        return Err(ierr!(
            "附件中没有图片",
            "No images found in the attachments"
        ));
    }

    let is_en = locale.as_deref() == Some("en-US");
    let user_label = if is_en {
        "User message"
    } else {
        "用户消息"
    };
    let instruction = if is_en {
        format!(
            "The images below were sent by the user ({}). The main model cannot see them, so describe everything relevant: the scene and main subjects, any visible text (transcribe verbatim), charts/tables with their data, UI elements, and details related to the user's question. Output a structured description in English.",
            names.join(", ")
        )
    } else {
        format!(
            "以下是用户发送的图片（{}）。主模型无法查看这些图片，请描述其中所有关键内容：场景与主体、可见文字（逐字转录）、图表与其中的数据、界面元素、以及与用户问题直接相关的细节。输出结构化的中文描述。",
            names.join("、")
        )
    };
    let text_part = if user_text.trim().is_empty() {
        instruction
    } else {
        format!("{}\n\n{}：{}", instruction, user_label, user_text)
    };
    let mut parts = vec![json!({ "type": "text", "text": text_part })];
    parts.extend(image_parts);

    let body = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": if is_en { VISION_SYSTEM_EN } else { VISION_SYSTEM_ZH } },
            { "role": "user", "content": serde_json::Value::Array(parts) }
        ],
        "stream": false,
        "max_tokens": 1024
    });

    let client = non_streaming_http_client();
    let (res, is_anthropic) = tokio::time::timeout(
        std::time::Duration::from_secs(45),
        post_chat_completion(&client, &api_url, &api_key, &body),
    )
    .await
    .map_err(|_| {
        ierr!(
            "视觉分析请求超时（45s）",
            "Vision analysis request timed out (45s)"
        )
    })?
    .map_err(|e| e.to_string())?;

    let val: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    if let Some(err) = chat_error_message(&val) {
        return Err(err);
    }
    let text = chat_text_content(&val, is_anthropic);
    if text.is_empty() {
        return Err(ierr!(
            "视觉分析结果为空",
            "Vision analysis returned an empty result"
        ));
    }
    Ok(text)
}

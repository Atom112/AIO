//! Message domain: Message-to-API-JSON conversion plus topic-message append/delete commands.

// 阶段4 第1项（消 glob）：显式 use 取代原父模块 glob。
use crate::commands::attachment::sync_message_attachments;
use crate::core::models::Message;
use crate::core::state::DbState;
use crate::ierr;
use base64::engine::general_purpose;
use base64::Engine as _;
use rusqlite::params;
use serde_json::json;

pub(crate) fn message_for_api(
    conn: &rusqlite::Connection,
    message: &Message,
) -> Result<serde_json::Value, String> {
    let mut content = message.content.clone();
    if let Some(files) = &message.display_files {
        if files.iter().any(|file| file.id.is_some()) {
            let base_text = match &message.content {
                serde_json::Value::String(text) => text.clone(),
                other => extract_text_content(other),
            };
            let mut document_sections = Vec::new();
            let mut image_data_urls = Vec::new();

            for file in files {
                let Some(attachment_id) = file.id.as_deref() else {
                    continue;
                };
                let attachment = conn
                    .query_row(
                        "SELECT file_name, mime_type, storage_path, extracted_text
                         FROM attachments WHERE id = ?1",
                        [attachment_id],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, Option<String>>(3)?,
                            ))
                        },
                    )
                    .map_err(|e| {
                        ierr!(
                            "读取附件 {} 失败: {}",
                            "Failed to read attachment {}: {}",
                            file.name,
                            e
                        )
                    })?;

                if attachment.1.starts_with("image/") {
                    let bytes = std::fs::read(&attachment.2).map_err(|e| {
                        ierr!(
                            "读取图片附件 {} 失败: {}",
                            "Failed to read image attachment {}: {}",
                            attachment.0,
                            e
                        )
                    })?;
                    image_data_urls.push(format!(
                        "data:{};base64,{}",
                        attachment.1,
                        general_purpose::STANDARD.encode(bytes)
                    ));
                } else if let Some(text) = attachment.3 {
                    document_sections.push(format!("[{}]\n{}", file.name, text));
                }
            }

            let expanded_text = if document_sections.is_empty() {
                base_text
            } else {
                format!(
                    "参考文件内容：\n{}\n---\n{}",
                    document_sections.join("\n"),
                    base_text
                )
            };
            content = if image_data_urls.is_empty() {
                json!(expanded_text)
            } else {
                let mut parts = vec![json!({ "type": "text", "text": expanded_text })];
                parts.extend(
                    image_data_urls
                        .into_iter()
                        .map(|url| json!({ "type": "image_url", "image_url": { "url": url } })),
                );
                serde_json::Value::Array(parts)
            };
        }
    }

    // Model-generated images: strip aio-image tokens and expand images metadata back into image_url blocks (history replay)
    if let Some(images) = &message.images {
        if !images.is_empty() {
            let base_text = match &content {
                serde_json::Value::String(text) => {
                    crate::utils::generated_images::strip_generated_image_tokens(text)
                }
                other => extract_text_content(other),
            };
            let mut parts = vec![json!({ "type": "text", "text": base_text })];
            for img in images {
                if let Ok(bytes) = std::fs::read(&img.storage_path) {
                    parts.push(json!({
                        "type": "image_url",
                        "image_url": {
                            "url": format!(
                                "data:{};base64,{}",
                                img.mime_type,
                                general_purpose::STANDARD.encode(bytes)
                            )
                        }
                    }));
                } else {
                    tracing::warn!("读取生成图片失败: {}", img.storage_path);
                }
            }
            content = serde_json::Value::Array(parts);
        }
    }

    let mut object = serde_json::Map::new();
    object.insert("role".into(), json!(message.role));
    object.insert("content".into(), content);
    if let Some(tool_call_id) = &message.tool_call_id {
        object.insert("tool_call_id".into(), json!(tool_call_id));
    }
    if let Some(name) = &message.name {
        object.insert("name".into(), json!(name));
    }
    if let Some(tool_calls) = &message.tool_calls {
        object.insert("tool_calls".into(), json!(tool_calls));
    }
    Ok(serde_json::Value::Object(object))
}

/// Append one message to a topic (the authoritative write path into storage).
///
/// # Parameters
/// - `topic_id` — target topic ID
/// - `message` — message body; `id` defaults to a generated UUID when absent; `display_files` / `tool_calls`
///   are stored as JSON text columns
///
/// # Behavior
/// Single INSERT (never overwrites existing messages); returns an error string on write failure.
#[tauri::command]
pub async fn append_message(
    state: tauri::State<'_, DbState>,
    topic_id: String,
    message: Message,
) -> Result<(), String> {
    let conn = (*state).0.lock();
    let message_id = message
        .id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let files_json = serde_json::to_string(&message.display_files).ok();
    let content_json = serde_json::to_string(&message.content).unwrap_or_default();
    let tool_calls_json = serde_json::to_string(&message.tool_calls).ok();

    conn.execute(
        "INSERT INTO messages
         (id, topic_id, role, content, model_id, display_files, display_text, reasoning,
          tool_call_id, name, tool_calls_json, input_tokens, output_tokens,
          agent_steps_json, interim_content, agent_start_time, parent_message_id, branch_index,
          images_json, cached_input_tokens)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
        params![
            message_id,
            topic_id,
            message.role,
            content_json,
            message.model_id,
            files_json,
            message.display_text,
            message.reasoning,
            message.tool_call_id,
            message.name,
            tool_calls_json,
            message.input_tokens,
            message.output_tokens,
            serde_json::to_string(&message.agent_steps).ok(),
            message.interim_content,
            message.agent_start_time,
            message.parent_message_id,
            message.branch_index,
            serde_json::to_string(&message.images).ok(),
            message.cached_input_tokens,
        ],
    )
    .map_err(|e| e.to_string())?;
    sync_message_attachments(&conn, &message_id, message.display_files.as_ref())?;
    Ok(())
}

/// Delete one message within a topic.
///
/// # Parameters
/// - `topic_id` / `message_id` — both must match (the WHERE constrains both), preventing cross-topic deletion
///
/// # Behavior
/// Plain DELETE; silently succeeds when the target is missing (idempotent).
#[tauri::command]
pub async fn delete_topic_message(
    state: tauri::State<'_, DbState>,
    topic_id: String,
    message_id: String,
) -> Result<(), String> {
    let conn = (*state).0.lock();
    conn.execute(
        "DELETE FROM messages WHERE id = ?1 AND topic_id = ?2",
        params![message_id, topic_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Extract plain text from message content; multimodal arrays (OpenAI vision format) keep only the text parts.
pub(crate) fn extract_text_content(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| {
                if v.get("type")?.as_str()? == "text" {
                    v.get("text")?.as_str().map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

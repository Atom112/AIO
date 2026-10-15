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

/// LOOP-10: attachment metadata snapshot taken under the DB lock — exactly one
/// SQLite row read per referenced attachment, NO disk I/O, NO base64. The file
/// reads and encoding happen in [`finish_message_for_api`] after the lock is
/// released (the historic single-pass version held the GLOBAL db lock across
/// every image read of the whole history, every turn).
pub(crate) struct AttachmentMeta {
    /// Display name from the message's file entry (used in document headers).
    display_name: String,
    mime: String,
    storage_path: String,
    extracted_text: Option<String>,
}

/// LOOP-10: one message's conversion split into its locked phase (this job)
/// and its lock-free phase ([`finish_message_for_api`]).
pub(crate) struct MessageApiJob<'a> {
    message: &'a Message,
    attachments: Vec<AttachmentMeta>,
}

/// LOOP-10 phase 1 (lock-held): collect the metadata a message's expansion
/// needs. Only SQLite work — disk reads are deferred to phase 2.
pub(crate) fn message_metadata_for_api<'a>(
    conn: &rusqlite::Connection,
    message: &'a Message,
) -> Result<MessageApiJob<'a>, String> {
    let mut attachments = Vec::new();
    if let Some(files) = &message.display_files {
        for file in files {
            let Some(attachment_id) = file.id.as_deref() else {
                continue;
            };
            let row = conn
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
            attachments.push(AttachmentMeta {
                display_name: file.name.clone(),
                mime: row.1,
                storage_path: row.2,
                extracted_text: row.3,
            });
        }
    }
    Ok(MessageApiJob {
        message,
        attachments,
    })
}

/// LOOP-10 phase 2 (lock-free): disk reads + base64 + JSON assembly.
pub(crate) fn finish_message_for_api(job: MessageApiJob<'_>) -> Result<serde_json::Value, String> {
    let message = job.message;
    let mut content = message.content.clone();
    if !job.attachments.is_empty() {
        let base_text = match &message.content {
            serde_json::Value::String(text) => text.clone(),
            other => extract_text_content(other),
        };
        let mut document_sections = Vec::new();
        let mut image_data_urls = Vec::new();

        for attachment in &job.attachments {
            if attachment.mime.starts_with("image/") {
                let bytes = std::fs::read(&attachment.storage_path).map_err(|e| {
                    ierr!(
                        "读取图片附件 {} 失败: {}",
                        "Failed to read image attachment {}: {}",
                        attachment.display_name,
                        e
                    )
                })?;
                image_data_urls.push(format!(
                    "data:{};base64,{}",
                    attachment.mime,
                    general_purpose::STANDARD.encode(bytes)
                ));
            } else if let Some(text) = &attachment.extracted_text {
                document_sections.push(format!("[{}]\n{}", attachment.display_name, text));
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

/// Convenience single-call form (metadata + finish back to back). Test-only:
/// the agent-loop call sites use the two phases separately so the global DB
/// lock never spans a disk read (LOOP-10).
#[cfg(test)]
pub(crate) fn message_for_api(
    conn: &rusqlite::Connection,
    message: &Message,
) -> Result<serde_json::Value, String> {
    finish_message_for_api(message_metadata_for_api(conn, message)?)
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
          images_json, cached_input_tokens, context_tokens)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
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
            message.context_tokens,
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

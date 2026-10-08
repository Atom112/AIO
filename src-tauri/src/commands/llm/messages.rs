//! 消息域：Message 到 API JSON 的转换与话题消息的追加/删除命令。

use crate::commands::llm::*;
use crate::core::state::DbState;
use base64::engine::general_purpose;
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
                    .map_err(|e| format!("读取附件 {} 失败: {}", file.name, e))?;

                if attachment.1.starts_with("image/") {
                    let bytes = std::fs::read(&attachment.2)
                        .map_err(|e| format!("读取图片附件 {} 失败: {}", attachment.0, e))?;
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

    // 模型生成图像：剥离 aio-image 标记并把 images 元数据展开回 image_url 块（历史回放）
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

/// 向话题追加一条消息（落库的权威写入路径）。
///
/// # 参数
/// - `topic_id` — 目标话题 ID
/// - `message` — 消息体；`id` 缺省时自动生成 UUID，`display_files` / `tool_calls`
///   以 JSON 文本列存储
///
/// # 行为
/// 单条 INSERT（不会覆盖既有消息）；写入失败返回错误串。
#[tauri::command]
pub(crate) async fn append_message(
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

/// 删除话题内的一条消息。
///
/// # 参数
/// - `topic_id` / `message_id` — 两者必须匹配（WHERE 同时限定），避免跨话题误删
///
/// # 行为
/// 直接 DELETE；目标不存在时静默成功（幂等）。
#[tauri::command]
pub(crate) async fn delete_topic_message(
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

/// 从消息内容中提取纯文本，多模态数组（OpenAI vision 格式）只保留 text 部分。
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

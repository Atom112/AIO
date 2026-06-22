use crate::core::models::{FileMeta, StoredAttachment};
use crate::core::state::DbState;
use crate::utils::file_parser::{
    attachment_mime_type, extract_file_content, validate_attachment_path,
};
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

fn attachment_storage_path(
    app: &AppHandle,
    sha256: &str,
    extension: &str,
) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("attachments")
        .join(&sha256[..2]);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(format!("{}.{}", sha256, extension)))
}

/// Copies a selected chat attachment into app data, deduplicates it by SHA-256,
/// extracts supported document text, and returns metadata for the pending message.
#[tauri::command]
pub async fn store_chat_attachment(
    app: AppHandle,
    state: tauri::State<'_, DbState>,
    path: String,
) -> Result<StoredAttachment, String> {
    let source = validate_attachment_path(&path)?;
    let bytes = std::fs::read(&source).map_err(|e| e.to_string())?;
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("bin")
        .to_lowercase();
    let mime_type = attachment_mime_type(&extension).to_string();
    let file_name = source
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("attachment")
        .to_string();

    {
        let conn = state.0.lock().map_err(|e| e.to_string())?;
        if let Ok(mut existing) = conn.query_row(
            "SELECT id, file_name, mime_type, size, storage_path
             FROM attachments WHERE sha256 = ?1",
            [&sha256],
            |row| {
                Ok(StoredAttachment {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    mime_type: row.get(2)?,
                    size: row.get::<_, i64>(3)? as u64,
                    storage_path: row.get(4)?,
                })
            },
        ) {
            existing.name = file_name;
            return Ok(existing);
        }
    }

    let destination = attachment_storage_path(&app, &sha256, &extension)?;
    if !destination.exists() {
        std::fs::write(&destination, &bytes).map_err(|e| e.to_string())?;
    }

    let extracted_text = match extract_file_content(&source, &extension) {
        Ok(text) => text,
        Err(error) => {
            let _ = std::fs::remove_file(&destination);
            return Err(error);
        }
    };
    let id = uuid::Uuid::new_v4().to_string();
    let size = bytes.len() as u64;
    let storage_path = destination.to_string_lossy().to_string();
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO attachments
         (id, sha256, file_name, mime_type, size, storage_path, extracted_text)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id,
            sha256,
            file_name,
            mime_type,
            size as i64,
            storage_path,
            extracted_text
        ],
    )
    .map_err(|e| e.to_string())?;

    Ok(StoredAttachment {
        id,
        name: file_name,
        mime_type,
        size,
        storage_path,
    })
}

/// Deletes an unattached pending upload. Files referenced by any message are retained.
#[tauri::command]
pub fn discard_chat_attachment(
    state: tauri::State<'_, DbState>,
    attachment_id: String,
) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let referenced: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM message_attachments WHERE attachment_id = ?1",
            [&attachment_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if referenced > 0 {
        return Ok(());
    }

    let path: Option<String> = conn
        .query_row(
            "SELECT storage_path FROM attachments WHERE id = ?1",
            [&attachment_id],
            |row| row.get(0),
        )
        .ok();
    conn.execute("DELETE FROM attachments WHERE id = ?1", [&attachment_id])
        .map_err(|e| e.to_string())?;
    if let Some(path) = path {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

pub fn sync_message_attachments(
    conn: &Connection,
    message_id: &str,
    files: Option<&Vec<FileMeta>>,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM message_attachments WHERE message_id = ?1",
        [message_id],
    )
    .map_err(|e| e.to_string())?;

    if let Some(files) = files {
        for (index, file) in files.iter().enumerate() {
            if let Some(attachment_id) = &file.id {
                conn.execute(
                    "INSERT OR IGNORE INTO message_attachments
                     (message_id, attachment_id, sort_order) VALUES (?1, ?2, ?3)",
                    params![message_id, attachment_id, index as i64],
                )
                .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

pub fn cleanup_attachment_ids(conn: &Connection, attachment_ids: &[String]) -> Result<(), String> {
    for attachment_id in attachment_ids {
        let referenced: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM message_attachments WHERE attachment_id = ?1",
                [attachment_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if referenced > 0 {
            continue;
        }
        let path: Option<String> = conn
            .query_row(
                "SELECT storage_path FROM attachments WHERE id = ?1",
                [attachment_id],
                |row| row.get(0),
            )
            .ok();
        conn.execute("DELETE FROM attachments WHERE id = ?1", [attachment_id])
            .map_err(|e| e.to_string())?;
        if let Some(path) = path {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(())
}

pub fn load_message_attachments(
    conn: &Connection,
    message_id: &str,
) -> Result<Vec<FileMeta>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.file_name, a.mime_type, a.size
             FROM message_attachments ma
             JOIN attachments a ON a.id = ma.attachment_id
             WHERE ma.message_id = ?1 ORDER BY ma.sort_order",
        )
        .map_err(|e| e.to_string())?;
    let files = stmt
        .query_map([message_id], |row| {
            Ok(FileMeta {
                id: Some(row.get(0)?),
                name: row.get(1)?,
                mime_type: Some(row.get(2)?),
                size: Some(row.get::<_, i64>(3)? as u64),
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(files)
}

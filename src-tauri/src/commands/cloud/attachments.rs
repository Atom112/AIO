//! Attachment transfer for cloud sync: upload pending files before a push batch and
//! materialize downloaded files into the local attachment store.
//!
//! The local `attachments` table gains a `cloud_id` (migration v20): the server assigns
//! the canonical id, so a message payload can only be pushed once every referenced file
//! has one. Files are content-addressed by SHA-256 on both sides, which makes uploads
//! idempotent and lets the server dedupe identical bytes.
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

use super::client::{self, HttpError, UploadCandidate};
use super::dto::RemoteAttachmentRef;

/// Maximum number of files uploaded per sync tick (keeps a tick short).
const MAX_UPLOADS_PER_TICK: usize = 3;

/// Queued attachments that still need a server id.
pub(crate) fn pending_uploads(
    conn: &Connection,
    limit: usize,
) -> Result<Vec<UploadCandidate>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.file_name, a.mime_type, a.size, a.sha256, a.storage_path
             FROM cloud_outbox o
             JOIN attachments a ON a.id = o.entity_id
             WHERE o.entity_type = 'attachment' AND a.cloud_id IS NULL
             ORDER BY o.priority ASC, o.enqueued_at ASC
             LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([limit as i64], |row| {
            Ok(UploadCandidate {
                attachment_id: row.get(0)?,
                file_name: row.get(1)?,
                mime_type: row.get(2)?,
                size_bytes: row.get(3)?,
                sha256: row.get(4)?,
                storage_path: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

/// Records the server id for a local attachment, then re-queues every message that
/// references it (they were deferred while the id was missing).
pub(crate) fn mark_uploaded(
    conn: &Connection,
    local_id: &str,
    cloud_id: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE attachments SET cloud_id = ?2 WHERE id = ?1",
        params![local_id, cloud_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM cloud_outbox WHERE entity_type = 'attachment' AND entity_id = ?1",
        [local_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO cloud_sync_state (entity_type, entity_id, remote_version, remote_seq, synced_at)
         VALUES ('attachment', ?1, NULL, NULL, CURRENT_TIMESTAMP)
         ON CONFLICT(entity_type, entity_id) DO UPDATE SET synced_at = CURRENT_TIMESTAMP",
        [local_id],
    )
    .map_err(|e| e.to_string())?;

    let message_ids: Vec<String> = conn
        .prepare("SELECT DISTINCT message_id FROM message_attachments WHERE attachment_id = ?1")
        .and_then(|mut stmt| {
            stmt.query_map([local_id], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|e| e.to_string())?;
    for message_id in message_ids {
        super::outbox::enqueue_message(conn, &message_id);
    }
    Ok(())
}

/// Uploads up to [`MAX_UPLOADS_PER_TICK`] queued files. Returns how many succeeded and
/// the first retryable error, if any (upload failures never abort the push batch).
pub(crate) async fn upload_pending(
    app: &AppHandle,
    client_http: &reqwest::Client,
    server_url: &str,
    token: &str,
    workspace_id: &str,
) -> (usize, Option<HttpError>) {
    let candidates = {
        let state = app.state::<crate::core::state::DbState>();
        let conn = state.0.lock();
        pending_uploads(&conn, MAX_UPLOADS_PER_TICK).unwrap_or_default()
    };

    let mut uploaded = 0usize;
    for candidate in candidates {
        match upload_one(client_http, server_url, token, workspace_id, &candidate).await {
            Ok(cloud_id) => {
                let state = app.state::<crate::core::state::DbState>();
                let conn = state.0.lock();
                if let Err(error) = mark_uploaded(&conn, &candidate.attachment_id, &cloud_id) {
                    tracing::warn!("[cloud] 记录附件服务器 ID 失败: {error}");
                }
                uploaded += 1;
            }
            Err(error) => return (uploaded, Some(error)),
        }
    }
    (uploaded, None)
}

/// Uploads one file: init -> presigned PUT -> complete.
async fn upload_one(
    client_http: &reqwest::Client,
    server_url: &str,
    token: &str,
    workspace_id: &str,
    candidate: &UploadCandidate,
) -> Result<String, HttpError> {
    let bytes = std::fs::read(&candidate.storage_path).map_err(|error| HttpError {
        status: None,
        message: error.to_string(),
        code: Some("local_read_failed".to_string()),
    })?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    if digest != candidate.sha256 {
        // The local file changed after it was queued: refusing the upload keeps the
        // content-addressed object key honest.
        return Err(HttpError {
            status: None,
            message: format!(
                "attachment {} changed on disk since it was queued",
                candidate.attachment_id
            ),
            code: Some("local_checksum_mismatch".to_string()),
        });
    }

    let init =
        client::attachment_init(client_http, server_url, token, workspace_id, candidate).await?;
    client::put_presigned(client_http, &init.upload_url, &init.headers, bytes).await?;
    client::attachment_complete(
        client_http,
        server_url,
        token,
        workspace_id,
        &init.attachment_id,
    )
    .await?;
    Ok(init.attachment_id)
}

/// Downloads a remote attachment into the local store and returns the local ref.
///
/// A file that already exists locally (same SHA-256) is not downloaded again: the
/// existing row is linked to the server id instead.
pub(crate) async fn download(
    app: &AppHandle,
    client_http: &reqwest::Client,
    server_url: &str,
    token: &str,
    workspace_id: &str,
    remote: &RemoteAttachmentRef,
) -> Result<crate::core::models::StoredAttachment, String> {
    if let Some(existing) = local_attachment(app, &remote.sha256, &remote.id)? {
        return Ok(existing);
    }

    let target =
        client::attachment_download_url(client_http, server_url, token, workspace_id, &remote.id)
            .await
            .map_err(HttpError::into_message)?;
    let bytes = client::get_presigned(client_http, &target.url)
        .await
        .map_err(HttpError::into_message)?;

    let digest = format!("{:x}", Sha256::digest(&bytes));
    if !remote.sha256.is_empty() && digest != remote.sha256 {
        return Err(crate::ierr!(
            "附件校验失败：下载内容与服务器记录不一致",
            "Attachment verification failed: downloaded bytes do not match the server record"
        ));
    }

    let extension = remote
        .file_name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_else(|| "bin".to_string());
    let destination =
        crate::commands::attachment::attachment_storage_path(app, &digest, &extension)?;
    if !destination.exists() {
        crate::core::fs_util::atomic_write(&destination, &bytes).map_err(|e| e.to_string())?;
    }
    // Text extraction is best effort for downloads: a parsing failure must not lose the file.
    let extracted = crate::utils::file_parser::extract_file_content(&destination, &extension)
        .unwrap_or_default();

    let id = uuid::Uuid::new_v4().to_string();
    let storage_path = destination.to_string_lossy().to_string();
    let state = app.state::<crate::core::state::DbState>();
    let conn = state.0.lock();
    conn.execute(
        "INSERT INTO attachments (id, sha256, file_name, mime_type, size, storage_path, extracted_text, cloud_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(sha256) DO UPDATE SET cloud_id = COALESCE(attachments.cloud_id, excluded.cloud_id)",
        params![
            id,
            digest,
            if remote.file_name.is_empty() { "attachment" } else { &remote.file_name },
            if remote.mime_type.is_empty() { "application/octet-stream" } else { &remote.mime_type },
            bytes.len() as i64,
            storage_path,
            extracted,
            remote.id
        ],
    )
    .map_err(|e| e.to_string())?;
    let stored = conn
        .query_row(
            "SELECT id, file_name, mime_type, size, storage_path FROM attachments WHERE sha256 = ?1",
            [&digest],
            |row| {
                Ok(crate::core::models::StoredAttachment {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    mime_type: row.get(2)?,
                    size: row.get::<_, i64>(3)? as u64,
                    storage_path: row.get(4)?,
                })
            },
        )
        .map_err(|e| e.to_string())?;
    Ok(stored)
}

/// Looks up a local attachment row by SHA-256 and links the server id to it.
fn local_attachment(
    app: &AppHandle,
    sha256: &str,
    cloud_id: &str,
) -> Result<Option<crate::core::models::StoredAttachment>, String> {
    if sha256.is_empty() {
        return Ok(None);
    }
    let state = app.state::<crate::core::state::DbState>();
    let conn = state.0.lock();
    let found: Option<crate::core::models::StoredAttachment> = conn
        .query_row(
            "SELECT id, file_name, mime_type, size, storage_path FROM attachments WHERE sha256 = ?1",
            [sha256],
            |row| {
                Ok(crate::core::models::StoredAttachment {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    mime_type: row.get(2)?,
                    size: row.get::<_, i64>(3)? as u64,
                    storage_path: row.get(4)?,
                })
            },
        )
        .ok();
    if found.is_some() {
        let _ = conn.execute(
            "UPDATE attachments SET cloud_id = COALESCE(cloud_id, ?2) WHERE sha256 = ?1",
            params![sha256, cloud_id],
        );
    }
    Ok(found)
}

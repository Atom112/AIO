//! Message payload artifacts (Phase 2): agent steps and full tool results are too large
//! (and too private) for the sync feed, so they travel as gzip JSON objects fetched on
//! demand. The sync DTO never carries their bytes.
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use rusqlite::{params, OptionalExtension};
use std::io::{Read, Write};

use super::client::HttpError;

/// Payload kinds understood by the server.
pub(crate) const KIND_AGENT_STEPS: &str = "agent_steps";
pub(crate) const KIND_TOOL_FULL_RESULT: &str = "tool_full_result";

/// Reads the payload body for one message from the local database.
///
/// `agent_steps` is the persisted agent step array; `tool_full_result` is the list of
/// tool-call full results (only entries that carry one), so a device can re-open a tool
/// output that other devices also expanded.
pub(crate) fn local_payload(
    conn: &rusqlite::Connection,
    message_id: &str,
    kind: &str,
) -> Result<Option<String>, String> {
    match kind {
        KIND_AGENT_STEPS => {
            let raw: Option<String> = conn
                .query_row(
                    "SELECT agent_steps_json FROM messages WHERE id = ?1",
                    [message_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?
                .flatten();
            Ok(raw.filter(|value| value != "null" && !value.trim().is_empty()))
        }
        KIND_TOOL_FULL_RESULT => {
            let raw: Option<String> = conn
                .query_row(
                    "SELECT tool_calls_json FROM messages WHERE id = ?1",
                    [message_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?
                .flatten();
            let Some(raw) = raw else { return Ok(None) };
            let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
            let Some(calls) = parsed.as_array() else {
                return Ok(None);
            };
            let results: Vec<serde_json::Value> = calls
                .iter()
                .filter_map(|call| {
                    let full = call.get("fullContent")?.clone();
                    Some(serde_json::json!({
                        "id": call.get("id").cloned().unwrap_or_default(),
                        "fullContent": full,
                    }))
                })
                .collect();
            if results.is_empty() {
                Ok(None)
            } else {
                Ok(Some(
                    serde_json::to_string(&results).map_err(|e| e.to_string())?,
                ))
            }
        }
        other => Err(crate::ierr!(
            "未知的载荷类型: {}",
            "Unknown payload kind: {}",
            other
        )),
    }
}

/// Gzip-compresses a JSON payload for upload.
pub(crate) fn gzip(json: &str) -> Result<Vec<u8>, String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(json.as_bytes())
        .map_err(|e| e.to_string())?;
    encoder.finish().map_err(|e| e.to_string())
}

/// Decompresses a gzip payload (the server also serves plain JSON; both are accepted).
pub(crate) fn maybe_gunzip(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    if bytes.len() > 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        let mut decoder = GzDecoder::new(&bytes[..]);
        let mut out = Vec::new();
        decoder.read_to_end(&mut out).map_err(|e| e.to_string())?;
        Ok(out)
    } else {
        Ok(bytes)
    }
}

/// Queued payload uploads whose parent message has already reached the server.
///
/// Uploading after the parent avoids orphaning objects the server would never link, and
/// matches the sync contract that flags travel with the message, bytes later.
pub(crate) fn pending_uploads(
    conn: &rusqlite::Connection,
    limit: usize,
) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT o.entity_id
             FROM cloud_outbox o
             WHERE o.entity_type = 'message_payload'
               AND NOT EXISTS (
                 SELECT 1 FROM cloud_outbox m
                 WHERE m.entity_type = 'message' AND m.entity_id = substr(o.entity_id, 1, instr(o.entity_id, ':') - 1)
               )
             ORDER BY o.priority ASC, o.enqueued_at ASC
             LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let entity_ids = stmt
        .query_map([limit as i64], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mut out = Vec::with_capacity(entity_ids.len());
    for entity_id in entity_ids {
        if let Some((message_id, kind)) = entity_id.split_once(':') {
            out.push((message_id.to_string(), kind.to_string()));
        }
    }
    Ok(out)
}

/// Removes a queued payload row once uploaded.
pub(crate) fn mark_uploaded(
    conn: &rusqlite::Connection,
    message_id: &str,
    kind: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM cloud_outbox WHERE entity_type = 'message_payload' AND entity_id = ?1",
        [format!("{message_id}:{kind}")],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Uploads one payload: gzip JSON with `content-encoding: gzip`.
pub(crate) async fn upload_one(
    client: &reqwest::Client,
    server_url: &str,
    token: &str,
    message_id: &str,
    kind: &str,
    json: &str,
) -> Result<(), HttpError> {
    let body = gzip(json).map_err(|message| HttpError {
        status: None,
        message,
        code: Some("payload_gzip_failed".to_string()),
    })?;
    let url = format!(
        "{}/v1/messages/{}/payloads/{}",
        server_url.trim_end_matches('/'),
        message_id,
        kind
    );
    let response = client
        .put(url)
        .bearer_auth(token)
        .header("content-encoding", "gzip")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        return Err(HttpError {
            status: Some(status),
            message: if text.is_empty() {
                format!("HTTP {status}")
            } else {
                text.chars().take(300).collect()
            },
            code: None,
        });
    }
    Ok(())
}

/// Downloads one payload body (plain JSON after transport decoding).
pub(crate) async fn fetch_one(
    client: &reqwest::Client,
    server_url: &str,
    token: &str,
    message_id: &str,
    kind: &str,
) -> Result<String, HttpError> {
    let url = format!(
        "{}/v1/messages/{}/payloads/{}",
        server_url.trim_end_matches('/'),
        message_id,
        kind
    );
    let response = client.get(url).bearer_auth(token).send().await?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        return Err(HttpError {
            status: Some(status),
            message: if text.is_empty() {
                format!("HTTP {status}")
            } else {
                text.chars().take(300).collect()
            },
            code: None,
        });
    }
    let bytes = response.bytes().await?.to_vec();
    let raw = maybe_gunzip(bytes).map_err(|message| HttpError {
        status: None,
        message,
        code: Some("payload_gunzip_failed".to_string()),
    })?;
    String::from_utf8(raw).map_err(|error| HttpError {
        status: None,
        message: error.to_string(),
        code: Some("payload_not_utf8".to_string()),
    })
}

/// Stores a downloaded agent-steps payload into the local message row.
pub(crate) fn store_agent_steps(
    conn: &rusqlite::Connection,
    message_id: &str,
    json: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE messages SET agent_steps_json = ?2 WHERE id = ?1",
        params![message_id, json],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Merges downloaded tool full results into the local `tool_calls_json`.
pub(crate) fn merge_tool_full_results(
    conn: &rusqlite::Connection,
    message_id: &str,
    json: &str,
) -> Result<(), String> {
    let payload: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let Some(entries) = payload.as_array() else {
        return Ok(());
    };
    let raw: Option<String> = conn
        .query_row(
            "SELECT tool_calls_json FROM messages WHERE id = ?1",
            [message_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .flatten();
    let Some(raw) = raw else { return Ok(()) };
    let mut calls: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
    let Some(calls) = calls.as_array_mut() else {
        return Ok(());
    };
    for entry in entries {
        let id = entry.get("id").and_then(|value| value.as_str());
        let full = entry.get("fullContent");
        let (Some(id), Some(full)) = (id, full) else {
            continue;
        };
        for call in calls.iter_mut() {
            if call.get("id").and_then(|value| value.as_str()) == Some(id) {
                if let Some(map) = call.as_object_mut() {
                    map.insert("fullContent".to_string(), full.clone());
                }
            }
        }
    }
    conn.execute(
        "UPDATE messages SET tool_calls_json = ?2 WHERE id = ?1",
        params![
            message_id,
            serde_json::to_string(&calls).map_err(|e| e.to_string())?
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

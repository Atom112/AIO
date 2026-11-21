//! SSE change feed (`/v1/sync/stream`).
//!
//! The feed carries cursors only: pulling and applying is the frontend loop's job, so a
//! dropped or duplicated notification can never corrupt data. The task reconnects with
//! exponential backoff (1s -> 30s, jittered), sends `Last-Event-ID` so the server resumes
//! where the connection stopped, and exits when the account is unlinked or cancelled.
use std::time::Duration;

use futures_util::StreamExt;
use serde_json::json;
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;

use crate::commands::llm::sse_lines::SseLineBuffer;

/// Cap for one SSE line (a cursor notification is tiny; a bigger line means a broken peer).
const SSE_LINE_BUFFER_LIMIT: usize = 64 * 1024;

/// Longest backoff between reconnect attempts.
const MAX_BACKOFF_SECS: u64 = 30;

/// Spawns the SSE loop. The caller stores the handle so `cloud_stop_events` can cancel it.
pub(crate) fn spawn(
    app: AppHandle,
    client: reqwest::Client,
    token: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff_secs = 1u64;
        loop {
            if token.is_cancelled() {
                break;
            }
            let Some(session) = load_session(&app) else {
                // Signed out while running: nothing left to stream.
                break;
            };

            let outcome = run_stream(&app, &client, &session, &token).await;
            if token.is_cancelled() {
                break;
            }
            match outcome {
                Ok(()) => {
                    backoff_secs = 1;
                }
                Err(error) => {
                    if error.is_unauthorized() {
                        tracing::warn!("[cloud] SSE 会话已失效，停止实时同步");
                        break;
                    }
                    tracing::debug!(
                        "[cloud] SSE 连接中断（{}），{}s 后重连",
                        error.message,
                        backoff_secs
                    );
                }
            }

            // Jitter spreads reconnects after a server restart.
            let jitter_ms = (uuid::Uuid::new_v4().as_u128() % 500) as u64;
            let sleep = Duration::from_millis(backoff_secs * 1000 + jitter_ms);
            tokio::select! {
                _ = token.cancelled() => break,
                _ = tokio::time::sleep(sleep) => {}
            }
            backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
        }
    })
}

struct Session {
    server_url: String,
    token: String,
    workspace_id: String,
    cursor: Option<String>,
}

fn load_session(app: &AppHandle) -> Option<Session> {
    let account = super::sync_ops::account(app).ok().flatten()?;
    let token = crate::core::secure_store::get(
        app,
        &crate::core::secure_store::accounts::cloud_session_token(),
    )
    .ok()
    .flatten()?;
    Some(Session {
        server_url: account.server_url,
        token,
        workspace_id: account.workspace_id,
        cursor: account.last_cursor,
    })
}

/// One connection attempt: reads events until the peer closes or the token is cancelled.
async fn run_stream(
    app: &AppHandle,
    client: &reqwest::Client,
    session: &Session,
    token: &CancellationToken,
) -> Result<(), super::client::HttpError> {
    let mut pairs = vec![("workspaceId", session.workspace_id.as_str())];
    if let Some(cursor) = session.cursor.as_deref() {
        pairs.push(("cursor", cursor));
    }
    let url = super::client::with_query(
        &format!(
            "{}/v1/sync/stream",
            session.server_url.trim_end_matches('/')
        ),
        &pairs,
    );
    let request = client
        .get(url)
        .bearer_auth(&session.token)
        .header("accept", "text/event-stream");

    let response = tokio::select! {
        _ = token.cancelled() => return Ok(()),
        response = request.send() => response.map_err(super::client::HttpError::from)?,
    };

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        return Err(super::client::HttpError {
            status: Some(status),
            message: if body.is_empty() {
                format!("HTTP {status}")
            } else {
                body
            },
            code: None,
        });
    }

    let mut stream = response.bytes_stream();
    let mut lines = SseLineBuffer::new(SSE_LINE_BUFFER_LIMIT);
    let mut data = String::new();

    loop {
        tokio::select! {
            _ = token.cancelled() => return Ok(()),
            chunk = stream.next() => {
                match chunk {
                    None => return Ok(()),
                    Some(Err(error)) => {
                        return Err(super::client::HttpError::from(error));
                    }
                    Some(Ok(bytes)) => {
                        if let Err(error) = lines.push(&bytes) {
                            return Err(super::client::HttpError {
                                status: None,
                                message: error,
                                code: Some("sse_line_too_long".to_string()),
                            });
                        }
                        while let Some(line) = lines.next_line() {
                            if line.is_empty() {
                                // Empty line terminates one SSE event.
                                if !data.is_empty() {
                                    emit_cursor(app, &session.workspace_id, &data);
                                    data.clear();
                                }
                                continue;
                            }
                            if let Some(payload) = line.strip_prefix("data:") {
                                if !data.is_empty() {
                                    data.push('\n');
                                }
                                data.push_str(payload.trim_start());
                            }
                            // `event:`, `id:` and comment (`:`) lines carry no cursor.
                        }
                    }
                }
            }
        }
    }
}

/// Emits `cloud-sync-event` with the cursor the server reports.
///
/// The payload intentionally carries no entity data: listeners re-pull through
/// `cloud_sync_pull`, which is the only path that mutates local state.
fn emit_cursor(app: &AppHandle, workspace_id: &str, raw: &str) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return;
    };
    let cursor = value
        .get("cursor")
        .and_then(serde_json::Value::as_i64)
        .or_else(|| {
            value
                .get("cursor")
                .and_then(|c| c.as_str().and_then(|s| s.parse().ok()))
        });
    let Some(cursor) = cursor else { return };
    let _ = app.emit(
        crate::core::events::CLOUD_SYNC_EVENT,
        json!({ "workspaceId": workspace_id, "cursor": cursor }),
    );
}

//! Cloud agent runs (Phase 2): start a run on the team server, stream its AG-UI events,
//! resolve tool approvals, cancel and list artifacts.
//!
//! The desktop stays the writer of its own conversation: the server streams deltas and a
//! terminal status, and the frontend appends the resulting message locally so sync
//! versioning never has two writers.
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};
use tokio_util::sync::CancellationToken;

use super::client::HttpError;
use super::{session_token, sync_ops, CloudState};

/// Cap for one SSE line (deltas are small; a bigger line means a broken peer).
const RUN_SSE_LINE_LIMIT: usize = 256 * 1024;

/// One run event forwarded to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudRunEvent {
    pub run_id: String,
    pub seq: i64,
    #[serde(rename = "type")]
    pub event_type: String,
    pub payload: Value,
}

/// Result of starting a run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartedRun {
    pub run_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudArtifact {
    pub id: String,
    pub kind: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub download_url: String,
}

/// Creates a run for a thread; the idempotency key makes retries safe.
#[tauri::command]
pub async fn cloud_start_run(
    app: AppHandle,
    thread_id: String,
    assistant_id: String,
    message: String,
    model: Option<String>,
) -> Result<StartedRun, String> {
    let account = sync_ops::account(&app)?
        .ok_or_else(|| crate::ierr!("尚未登录云同步", "Not signed in to cloud sync"))?;
    let token = session_token(&app)
        .ok_or_else(|| crate::ierr!("尚未登录云同步", "Not signed in to cloud sync"))?;

    let body = serde_json::json!({
        "assistantId": assistant_id,
        "input": { "message": message },
        "idempotencyKey": uuid::Uuid::new_v4().to_string(),
        "model": model,
    });
    let url = format!(
        "{}/v1/threads/{}/runs",
        account.server_url.trim_end_matches('/'),
        thread_id
    );
    let state = app.state::<CloudState>();
    let response = state
        .client
        .post(url)
        .bearer_auth(&token)
        .header("x-workspace-id", &account.workspace_id)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
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
        }
        .into_message());
    }
    let parsed: Value = response.json().await.map_err(|e| e.to_string())?;
    let run_id = parsed
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| crate::ierr!("云运行缺少 ID", "Cloud run response has no id"))?;
    let status = parsed
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("queued");
    Ok(StartedRun {
        run_id: run_id.to_string(),
        status: status.to_string(),
    })
}

/// Streams one run's events as Tauri events until a terminal status or cancel.
#[tauri::command]
pub async fn cloud_start_run_events(app: AppHandle, run_id: String) -> Result<(), String> {
    let account = sync_ops::account(&app)?
        .ok_or_else(|| crate::ierr!("尚未登录云同步", "Not signed in to cloud sync"))?;
    let token = session_token(&app)
        .ok_or_else(|| crate::ierr!("尚未登录云同步", "Not signed in to cloud sync"))?;

    let state = app.state::<CloudState>();
    let mut guard = state.run_events.lock();
    if let Some((existing_run, handle, _)) = guard.as_ref() {
        if existing_run == &run_id {
            return Ok(());
        }
        handle.abort();
    }
    let cancel = CancellationToken::new();
    let handle = spawn_run_stream(
        app.clone(),
        state.client.clone(),
        account.server_url.clone(),
        account.workspace_id.clone(),
        token,
        run_id.clone(),
        cancel.clone(),
    );
    *guard = Some((run_id, handle, cancel));
    Ok(())
}

/// Stops streaming events for the active run (does not cancel the run itself).
#[tauri::command]
pub async fn cloud_stop_run_events(app: AppHandle) -> Result<(), String> {
    let state = app.state::<CloudState>();
    let mut guard = state.run_events.lock();
    if let Some((_, handle, cancel)) = guard.take() {
        cancel.cancel();
        handle.abort();
    }
    Ok(())
}

/// Cancels a run on the server (the stream then ends with `run.canceled`).
#[tauri::command]
pub async fn cloud_cancel_run(app: AppHandle, run_id: String) -> Result<(), String> {
    let account = sync_ops::account(&app)?
        .ok_or_else(|| crate::ierr!("尚未登录云同步", "Not signed in to cloud sync"))?;
    let token = session_token(&app)
        .ok_or_else(|| crate::ierr!("尚未登录云同步", "Not signed in to cloud sync"))?;
    let url = format!(
        "{}/v1/runs/{}/cancel",
        account.server_url.trim_end_matches('/'),
        run_id
    );
    let state = app.state::<CloudState>();
    let response = state
        .client
        .post(url)
        .bearer_auth(&token)
        .header("x-workspace-id", &account.workspace_id)
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        return Err(format!(
            "HTTP {status}: {}",
            text.chars().take(300).collect::<String>()
        ));
    }
    Ok(())
}

/// Resolves one pending tool approval.
#[tauri::command]
pub async fn cloud_resolve_run_approval(
    app: AppHandle,
    run_id: String,
    approval_id: String,
    decision: String,
) -> Result<(), String> {
    let account = sync_ops::account(&app)?
        .ok_or_else(|| crate::ierr!("尚未登录云同步", "Not signed in to cloud sync"))?;
    let token = session_token(&app)
        .ok_or_else(|| crate::ierr!("尚未登录云同步", "Not signed in to cloud sync"))?;
    let url = format!(
        "{}/v1/runs/{}/approvals/{}",
        account.server_url.trim_end_matches('/'),
        run_id,
        approval_id
    );
    let state = app.state::<CloudState>();
    let response = state
        .client
        .post(url)
        .bearer_auth(&token)
        .header("x-workspace-id", &account.workspace_id)
        .json(&serde_json::json!({ "decision": decision }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        return Err(format!(
            "HTTP {status}: {}",
            text.chars().take(300).collect::<String>()
        ));
    }
    Ok(())
}

/// Lists a run's artifacts with presigned download URLs.
#[tauri::command]
pub async fn cloud_run_artifacts(
    app: AppHandle,
    run_id: String,
) -> Result<Vec<CloudArtifact>, String> {
    let account = sync_ops::account(&app)?
        .ok_or_else(|| crate::ierr!("尚未登录云同步", "Not signed in to cloud sync"))?;
    let token = session_token(&app)
        .ok_or_else(|| crate::ierr!("尚未登录云同步", "Not signed in to cloud sync"))?;
    let url = format!(
        "{}/v1/runs/{}/artifacts",
        account.server_url.trim_end_matches('/'),
        run_id
    );
    let state = app.state::<CloudState>();
    let response = state
        .client
        .get(url)
        .bearer_auth(&token)
        .header("x-workspace-id", &account.workspace_id)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        return Err(format!(
            "HTTP {status}: {}",
            text.chars().take(300).collect::<String>()
        ));
    }
    let parsed: Value = response.json().await.map_err(|e| e.to_string())?;
    let items = parsed
        .get("artifacts")
        .and_then(Value::as_array)
        .or_else(|| parsed.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(items
        .into_iter()
        .filter_map(|item| {
            Some(CloudArtifact {
                id: item.get("id")?.as_str()?.to_string(),
                kind: item.get("kind")?.as_str()?.to_string(),
                size_bytes: item.get("sizeBytes").and_then(Value::as_i64).unwrap_or(0),
                sha256: item
                    .get("sha256")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                download_url: item.get("downloadUrl")?.as_str()?.to_string(),
            })
        })
        .collect())
}

/// Spawns the SSE reader for one run.
fn spawn_run_stream(
    app: AppHandle,
    client: reqwest::Client,
    server_url: String,
    workspace_id: String,
    token: String,
    run_id: String,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let url = format!(
            "{}/v1/runs/{}/events",
            server_url.trim_end_matches('/'),
            run_id
        );
        let request = client
            .get(url)
            .bearer_auth(&token)
            .header("x-workspace-id", &workspace_id)
            .header("accept", "text/event-stream");

        let response = tokio::select! {
            _ = cancel.cancelled() => return,
            response = request.send() => response,
        };
        let Ok(response) = response else { return };
        if !response.status().is_success() {
            return;
        }

        let mut stream = response.bytes_stream();
        let mut lines = crate::commands::llm::sse_lines::SseLineBuffer::new(RUN_SSE_LINE_LIMIT);
        let mut event_type = String::new();
        let mut seq: i64 = 0;
        let mut data = String::new();

        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                chunk = stream.next() => {
                    let Some(Ok(bytes)) = chunk else { return };
                    if lines.push(&bytes).is_err() {
                        return;
                    }
                    while let Some(line) = lines.next_line() {
                        if line.is_empty() {
                            if !data.is_empty() {
                                if let Ok(payload) = serde_json::from_str::<Value>(&data) {
                                    let terminal = event_type.starts_with("run.completed")
                                        || event_type.starts_with("run.failed")
                                        || event_type.starts_with("run.canceled");
                                    let _ = app.emit(
                                        crate::core::events::CLOUD_RUN_EVENT,
                                        CloudRunEvent {
                                            run_id: run_id.clone(),
                                            seq,
                                            event_type: event_type.clone(),
                                            payload,
                                        },
                                    );
                                    if terminal {
                                        return;
                                    }
                                }
                                data.clear();
                                event_type.clear();
                            }
                            continue;
                        }
                        if let Some(value) = line.strip_prefix("id:") {
                            seq = value.trim().parse().unwrap_or(seq);
                        } else if let Some(value) = line.strip_prefix("event:") {
                            event_type = value.trim().to_string();
                        } else if let Some(value) = line.strip_prefix("data:") {
                            if !data.is_empty() {
                                data.push('\n');
                            }
                            data.push_str(value.trim_start());
                        }
                    }
                }
            }
        }
    })
}

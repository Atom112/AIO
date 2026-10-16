//! Harness workbench commands (thin IPC wrappers over plugins/harness).

use crate::core::models::chat::Message;
use crate::core::state::DbState;
use crate::ierr;
use crate::plugins::harness::manager::{list_running_harness_sessions, HarnessManager};
use crate::plugins::harness::registry::{self, HarnessToolStatus};
use crate::plugins::harness::store::{self, HarnessSessionRow, HarnessWorkspaceRow};
use tauri::{AppHandle, State};

/// Detect every built-in harness tool's availability (ready / installable / missing).
#[tauri::command]
pub async fn harness_detect_tools() -> Result<Vec<HarnessToolStatus>, String> {
    Ok(registry::detect_tools())
}

/// List every registered harness workspace.
#[tauri::command]
pub async fn harness_list_workspaces(
    state: State<'_, DbState>,
) -> Result<Vec<HarnessWorkspaceRow>, String> {
    let conn = state.0.lock();
    store::list_workspaces(&conn)
}

/// Register a workspace for a harness tool. `path` must be an existing directory;
/// duplicate (tool, path) registrations return the existing row.
#[tauri::command]
pub async fn harness_add_workspace(
    state: State<'_, DbState>,
    tool_id: String,
    path: String,
    name: Option<String>,
) -> Result<HarnessWorkspaceRow, String> {
    let conn = state.0.lock();
    let name = name.as_deref().map(str::to_string);
    store::add_workspace(&conn, &tool_id, &path, name.as_deref())
}

/// Remove a workspace together with every topic recorded under it.
#[tauri::command]
pub async fn harness_remove_workspace(
    state: State<'_, DbState>,
    workspace_id: String,
) -> Result<(), String> {
    let conn = state.0.lock();
    store::remove_workspace(&conn, &workspace_id)
}

/// List the topics of one (tool, workspace).
#[tauri::command]
pub async fn harness_list_sessions(
    state: State<'_, DbState>,
    tool_id: String,
    workspace_path: String,
) -> Result<Vec<HarnessSessionRow>, String> {
    let conn = state.0.lock();
    store::list_sessions(&conn, &tool_id, &workspace_path)
}

/// Create a new topic under (tool, workspace). `title` is a frontend-supplied
/// localized default (auto-rename on first message happens in the frontend).
#[tauri::command]
pub async fn harness_new_session(
    state: State<'_, DbState>,
    tool_id: String,
    workspace_path: String,
    title: String,
) -> Result<HarnessSessionRow, String> {
    let conn = state.0.lock();
    store::create_session(&conn, &tool_id, &workspace_path, &title)
}

/// Rename a topic.
#[tauri::command]
pub async fn harness_rename_session(
    state: State<'_, DbState>,
    session_row_id: String,
    title: String,
) -> Result<(), String> {
    let conn = state.0.lock();
    store::rename_session(&conn, &session_row_id, &title)
}

/// Delete a topic (its messages cascade).
#[tauri::command]
pub async fn harness_delete_session(
    state: State<'_, DbState>,
    session_row_id: String,
) -> Result<(), String> {
    let conn = state.0.lock();
    store::delete_session(&conn, &session_row_id)
}

/// Load a topic's message history (AIO's own DB is the display source of truth).
#[tauri::command]
pub async fn harness_load_messages(
    state: State<'_, DbState>,
    session_row_id: String,
) -> Result<Vec<Message>, String> {
    let conn = state.0.lock();
    store::load_messages(&conn, &session_row_id)
}

/// Persist one message built by the frontend (assistant turn finalization —
/// the frontend assembles the Message model from the harness event stream).
#[tauri::command]
pub async fn harness_append_message(
    state: State<'_, DbState>,
    session_row_id: String,
    message: Message,
) -> Result<(), String> {
    let conn = state.0.lock();
    let stored = store::append_message(&conn, &session_row_id, &message)?;
    let _ = stored; // id backfill happens on the frontend copy
    Ok(())
}

/// Send a user prompt to a harness topic: persists the user message, ensures the
/// agent runtime (spawning + native handshake, resume via the stored harness
/// session id when the runtime is not live), and submits the prompt. Returns the
/// persisted user message for the frontend to render.
#[tauri::command]
pub async fn harness_send_message(
    app: AppHandle,
    state: State<'_, DbState>,
    manager: State<'_, HarnessManager>,
    session_row_id: String,
    text: String,
) -> Result<Message, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err(ierr!("消息为空", "Message is empty"));
    }
    let (stored, row) = {
        let conn = state.0.lock();
        let row = store::get_session(&conn, &session_row_id)?;
        let message = Message {
            id: None,
            role: "user".to_string(),
            content: serde_json::json!(text),
            ..Default::default()
        };
        let stored = store::append_message(&conn, &session_row_id, &message)?;
        store::touch_session(&conn, &session_row_id)?;
        (stored, row)
    };
    manager
        .ensure_session(
            &app,
            &row.tool_id,
            &session_row_id,
            &row.workspace_path,
            row.harness_session_id.clone(),
        )
        .await?
        .cmd_tx
        .send(crate::plugins::harness::adapter::HarnessCommand::Prompt { text: text.clone() })
        .map_err(|_| {
            ierr!(
                "harness 会话已停止（发送失败）",
                "harness session has stopped (send failed)"
            )
        })?;
    Ok(stored)
}

/// Stop the running turn of a harness topic (protocol-level cancel, then a
/// hard kill after the grace window). Safe to call when nothing is running.
#[tauri::command]
pub async fn harness_stop_session(
    manager: State<'_, HarnessManager>,
    session_row_id: String,
) -> Result<(), String> {
    manager.stop_session(&session_row_id).await
}

/// Answer a pending harness approval (resolves the bubble's oneshot; false
/// when the request already expired).
#[tauri::command]
pub async fn harness_respond_approval(
    manager: State<'_, HarnessManager>,
    request_id: String,
    option_id: String,
) -> Result<(), String> {
    if !manager.respond_approval(&request_id, &option_id) {
        return Err(ierr!(
            "该审批请求已失效",
            "The approval request has expired"
        ));
    }
    Ok(())
}

/// Which harness topics are still running (stop-button reconciliation after a
/// page remount — runtimes survive navigation).
#[tauri::command]
pub async fn harness_list_running(app: AppHandle) -> Result<Vec<String>, String> {
    Ok(list_running_harness_sessions(&app))
}

//! Harness workbench commands (thin IPC wrappers over plugins/harness).

use crate::core::models::chat::Message;
use crate::core::state::DbState;
use crate::ierr;
use crate::plugins::harness::manager::{list_running_harness_sessions, HarnessManager};
use crate::plugins::harness::registry::{self, HarnessToolStatus};
use crate::plugins::harness::store::{
    self, HarnessCustomAgent, HarnessSessionRow, HarnessWorkspaceRow,
};
use tauri::{AppHandle, Manager, State};

/// Detect every harness tool's availability (ready / installable / missing),
/// covering built-ins and user-registered custom ACP agents.
#[tauri::command]
pub async fn harness_detect_tools(app: AppHandle) -> Result<Vec<HarnessToolStatus>, String> {
    Ok(registry::detect_tools(&app))
}

/// List user-registered custom ACP agents.
#[tauri::command]
pub async fn harness_list_custom_agents(app: AppHandle) -> Result<Vec<HarnessCustomAgent>, String> {
    store::list_custom_agents(&app)
}

/// Insert or update a custom ACP agent (id generated when empty; command must
/// resolve to an ACP-speaking CLI — the user authors the command line themselves).
#[tauri::command]
pub async fn harness_save_custom_agent(
    app: AppHandle,
    agent: HarnessCustomAgent,
) -> Result<HarnessCustomAgent, String> {
    store::save_custom_agent(&app, agent)
}

/// Delete a custom ACP agent by id.
#[tauri::command]
pub async fn harness_delete_custom_agent(app: AppHandle, id: String) -> Result<(), String> {
    store::delete_custom_agent(&app, &id)
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
///
/// `prefs` carries the frontend's stored native-adapter preferences (permission
/// mode / model / thinking level) applied to a freshly spawned runtime; ACP
/// agents own their state and ignore them.
#[tauri::command]
pub async fn harness_send_message(
    app: AppHandle,
    state: State<'_, DbState>,
    manager: State<'_, HarnessManager>,
    session_row_id: String,
    text: String,
    prefs: Option<crate::plugins::harness::adapter::DriverSpawnPrefs>,
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
            prefs.unwrap_or_default(),
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

/// Eagerly connect a harness topic: spawn the agent runtime (handshake +
/// session/new-or-resume) WITHOUT a prompt, so the live state and config
/// controls appear immediately on selection instead of on first send.
/// Idempotent — an existing runtime is reused. `prefs` are the frontend's
/// native-adapter preferences (same semantics as harness_send_message).
#[tauri::command]
pub async fn harness_connect_topic(
    app: AppHandle,
    state: State<'_, DbState>,
    manager: State<'_, HarnessManager>,
    session_row_id: String,
    prefs: Option<crate::plugins::harness::adapter::DriverSpawnPrefs>,
) -> Result<(), String> {
    let row = {
        let conn = state.0.lock();
        store::get_session(&conn, &session_row_id)?
    };
    manager
        .ensure_session(
            &app,
            &row.tool_id,
            &session_row_id,
            &row.workspace_path,
            row.harness_session_id.clone(),
            prefs.unwrap_or_default(),
        )
        .await?;
    Ok(())
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

/// Switch the permission mode of a running harness session (Claude Code native:
/// set_permission_mode control frame; other adapters ignore it). No-op when
/// the session is not live — the caller applies the choice at next spawn.
#[tauri::command]
pub async fn harness_set_permission_mode(
    manager: State<'_, HarnessManager>,
    session_row_id: String,
    mode: String,
) -> Result<(), String> {
    manager.set_permission_mode(&session_row_id, &mode);
    Ok(())
}

/// Set a session config option (model selector / thought level / boolean) on a
/// running harness session. ACP → session/set_config_option (live); claude
/// native → set_model control frame (model, live) / effort stored for respawn;
/// codex native → stored for the next per-turn spawn. No-op when the session
/// is not live — the caller persists the choice for the next spawn.
#[tauri::command]
pub async fn harness_set_config_option(
    manager: State<'_, HarnessManager>,
    session_row_id: String,
    config_id: String,
    value: serde_json::Value,
) -> Result<(), String> {
    manager.set_config_option(&session_row_id, &config_id, value);
    Ok(())
}

/// Synthesized config options for a NATIVE adapter without a live session
/// (first-use dropdown data before any turn has run). ACP agents own their
/// option lists (reported in session/new), so they return an empty list here.
#[tauri::command]
pub async fn harness_tool_config_options(
    app: AppHandle,
    tool_id: String,
) -> Result<Vec<crate::plugins::harness::model::HarnessConfigOption>, String> {
    use crate::plugins::harness::registry::{lookup_any, HarnessToolKind};
    let Some(spec) = lookup_any(&app, &tool_id) else {
        return Ok(Vec::new());
    };
    let options = match spec.kind {
        HarnessToolKind::ClaudeNative { .. } => {
            if crate::plugins::harness::claude::detect_native_binary().is_some() {
                crate::plugins::harness::claude::claude_config_options(None, None)
            } else {
                Vec::new()
            }
        }
        HarnessToolKind::CodexExec { .. } => {
            // Native presets when the CLI exists; registry-declared spawn
            // options (sandbox) render either way.
            let mut options = if crate::utils::exec_resolve::resolve_executable("codex").is_some() {
                crate::plugins::harness::codex::codex_config_options(None, None)
            } else {
                Vec::new()
            };
            options.extend(
                spec.spawn_options
                    .iter()
                    .map(|s| crate::plugins::harness::acp::synth_spawn_option(s, None)),
            );
            options
        }
        // ACP agents own their option lists (reported in session/new), but
        // registry-declared spawn options (e.g. omp --approval-mode) are
        // synthesized here so they render even before the first spawn.
        HarnessToolKind::Acp { .. } => spec
            .spawn_options
            .iter()
            .map(|s| crate::plugins::harness::acp::synth_spawn_option(s, None))
            .collect(),
    };
    Ok(options)
}

// ===== Tool setup: auth status, AIO-stored API key, instruction file =====

/// Setup surface of one harness tool for the settings modal.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessToolSetupInfo {
    /// A probe path exists → the tool's own auth/config setup is present.
    pub configured: bool,
    /// Env var receiving an AIO-stored API key at spawn (None = the tool only
    /// supports its own auth flow).
    pub api_key_env: Option<String>,
    /// A key is stored in AIO's secure store for this tool.
    pub has_aio_key: bool,
    pub auth_docs_url: String,
    /// Workspace instruction file convention (CLAUDE.md / AGENTS.md / ...).
    pub instruction_file: String,
}

/// Resolve a registry probe path: `~/…` → the home directory, `%APPDATA%/…` →
/// the roaming app-data dir.
fn resolve_probe_path(raw: &str) -> Option<std::path::PathBuf> {
    if let Some(rest) = raw.strip_prefix("~/") {
        let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        return Some(std::path::Path::new(&std::env::var_os(home_var)?).join(rest));
    }
    if let Some(rest) = raw.strip_prefix("%APPDATA%/") {
        return Some(std::path::Path::new(&std::env::var_os("APPDATA")?).join(rest));
    }
    Some(std::path::PathBuf::from(raw))
}

/// Setup/auth status of one tool for the settings modal.
#[tauri::command]
pub async fn harness_tool_setup_info(
    app: AppHandle,
    tool_id: String,
) -> Result<HarnessToolSetupInfo, String> {
    let spec = registry::lookup_any(&app, &tool_id).ok_or_else(|| {
        ierr!(
            "未知的 harness 工具: {tool_id}",
            "Unknown harness tool: {tool_id}"
        )
    })?;
    let configured = spec
        .setup
        .auth_probe_paths
        .iter()
        .filter_map(|p| resolve_probe_path(p))
        .any(|p| p.exists());
    let has_aio_key = crate::core::secure_store::get(
        &app,
        &crate::core::secure_store::accounts::harness_tool_key(&tool_id),
    )
    .ok()
    .flatten()
    .map(|k| !k.trim().is_empty())
    .unwrap_or(false);
    Ok(HarnessToolSetupInfo {
        configured,
        api_key_env: spec.setup.api_key_env.clone(),
        has_aio_key,
        auth_docs_url: spec.setup.auth_docs_url.clone(),
        instruction_file: spec.setup.instruction_file.clone(),
    })
}

/// Store (or clear with an empty string) a tool's API key in the OS keychain;
/// injected as the tool's provider env var at the next spawn.
#[tauri::command]
pub async fn harness_set_tool_api_key(
    app: AppHandle,
    tool_id: String,
    api_key: String,
) -> Result<(), String> {
    let spec = registry::lookup_any(&app, &tool_id).ok_or_else(|| {
        ierr!(
            "未知的 harness 工具: {tool_id}",
            "Unknown harness tool: {tool_id}"
        )
    })?;
    if spec.setup.api_key_env.is_none() {
        return Err(ierr!(
            "该工具不支持 API Key 注入",
            "This tool does not support API key injection"
        ));
    }
    let account = crate::core::secure_store::accounts::harness_tool_key(&tool_id);
    if api_key.trim().is_empty() {
        crate::core::secure_store::delete(&app, &account).map_err(|e| e.to_string())?;
    } else {
        crate::core::secure_store::set(&app, &account, api_key.trim())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// One workspace instruction file payload.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessInstruction {
    /// Absolute path of the instruction file.
    pub path: String,
    pub exists: bool,
    pub content: String,
}

/// The instruction file path for a (tool, workspace) — the workspace must be a
/// registered harness workspace for that tool (the registry file name is ours,
/// the path comes from the frontend: the registration check is the boundary).
fn instruction_path(
    app: &AppHandle,
    tool_id: &str,
    workspace_path: &str,
) -> Result<std::path::PathBuf, String> {
    let spec = registry::lookup_any(app, tool_id).ok_or_else(|| {
        ierr!(
            "未知的 harness 工具: {tool_id}",
            "Unknown harness tool: {tool_id}"
        )
    })?;
    let db = app.state::<DbState>();
    let conn = db.0.lock();
    let registered = store::list_workspaces(&conn)
        .map_err(|e| e.to_string())?
        .iter()
        .any(|w| w.tool_id == tool_id && w.path == workspace_path);
    if !registered {
        return Err(ierr!(
            "工作区未注册到该工具",
            "Workspace is not registered for this tool"
        ));
    }
    Ok(std::path::Path::new(workspace_path).join(&spec.setup.instruction_file))
}

/// Read the workspace's instruction file for the tool (empty when absent).
#[tauri::command]
pub async fn harness_instruction_read(
    app: AppHandle,
    tool_id: String,
    workspace_path: String,
) -> Result<HarnessInstruction, String> {
    let path = instruction_path(&app, &tool_id, &workspace_path)?;
    let exists = path.exists();
    let content = if exists {
        std::fs::read_to_string(&path).map_err(|e| e.to_string())?
    } else {
        String::new()
    };
    Ok(HarnessInstruction {
        path: path.to_string_lossy().into_owned(),
        exists,
        content,
    })
}

/// Write (create or overwrite) the workspace's instruction file for the tool —
/// atomic write, no backup (the file is user-authored prose, not state).
#[tauri::command]
pub async fn harness_instruction_write(
    app: AppHandle,
    tool_id: String,
    workspace_path: String,
    content: String,
) -> Result<(), String> {
    let path = instruction_path(&app, &tool_id, &workspace_path)?;
    crate::core::fs_util::atomic_write(&path, content.as_bytes()).map_err(|e| e.to_string())
}

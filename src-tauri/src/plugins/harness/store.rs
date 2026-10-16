//! Harness store: SQLite persistence for the harness workbench (workspaces,
//! topics, messages). Message rows reuse the chat `Message` model serialized as
//! JSON so the frontend renders harness history with the same components.

use crate::core::models::chat::Message;
use crate::ierr;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

/// One workspace registered under a tool (unique per tool+path).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HarnessWorkspaceRow {
    pub id: String,
    pub tool_id: String,
    pub path: String,
    pub name: String,
    pub created_at: String,
}

/// One harness topic ("session row" — the harness-side session id is attached
/// once the native handshake reports it, enabling resume after restarts).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HarnessSessionRow {
    pub id: String,
    pub tool_id: String,
    pub workspace_path: String,
    pub harness_session_id: Option<String>,
    pub title: String,
    pub model_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Stored message row: full chat-model JSON plus role/created_at columns.
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct StoredHarnessMessage {
    #[serde(flatten)]
    message: Message,
    created_at: String,
}

fn row_workspace(r: &rusqlite::Row<'_>) -> rusqlite::Result<HarnessWorkspaceRow> {
    Ok(HarnessWorkspaceRow {
        id: r.get(0)?,
        tool_id: r.get(1)?,
        path: r.get(2)?,
        name: r.get(3)?,
        created_at: r.get(4)?,
    })
}

fn row_session(r: &rusqlite::Row<'_>) -> rusqlite::Result<HarnessSessionRow> {
    Ok(HarnessSessionRow {
        id: r.get(0)?,
        tool_id: r.get(1)?,
        workspace_path: r.get(2)?,
        harness_session_id: r.get(3)?,
        title: r.get(4)?,
        model_id: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

// ===== Workspaces =====

/// All registered workspaces (all tools, ordered by creation).
pub fn list_workspaces(conn: &Connection) -> Result<Vec<HarnessWorkspaceRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, tool_id, path, name, created_at FROM harness_workspaces
             ORDER BY created_at ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], row_workspace)
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

/// Canonicalize a directory path for cross-CLI consumption: resolves the real
/// path but STRIPS the Windows verbatim prefix (`\\?\C:\...`) that
/// `Path::canonicalize` produces — third-party CLIs mis-handle verbatim paths
/// (observed live: DSH's realpath chokes with EISDIR on the bare drive).
pub fn canonical_workspace_path(p: &std::path::Path) -> Result<String, String> {
    let resolved = p.canonicalize().map_err(|e| e.to_string())?;
    let s = resolved.to_string_lossy().into_owned();
    #[cfg(windows)]
    {
        const VERBATIM: &str = r"\\?\";
        Ok(s.trim_start_matches(VERBATIM).to_string())
    }
    #[cfg(not(windows))]
    {
        Ok(s)
    }
}

/// Register a workspace for a tool. Validates the path is an existing directory;
/// the name defaults to the directory basename.
pub fn add_workspace(
    conn: &Connection,
    tool_id: &str,
    path: &str,
    name: Option<&str>,
) -> Result<HarnessWorkspaceRow, String> {
    let p = std::path::Path::new(path);
    if !p.is_dir() {
        return Err(ierr!(
            "路径不是有效目录: {}",
            "Path is not a valid directory: {}",
            path
        ));
    }
    let canonical = canonical_workspace_path(p)?;
    let name = name
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .or_else(|| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| canonical.clone());
    let exists: Option<String> = conn
        .query_row(
            "SELECT id FROM harness_workspaces WHERE tool_id = ?1 AND path = ?2",
            [tool_id, canonical.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(id) = exists {
        return conn
            .query_row(
                "SELECT id, tool_id, path, name, created_at FROM harness_workspaces WHERE id = ?1",
                [id],
                row_workspace,
            )
            .map_err(|e| e.to_string());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::utils::time::now_iso();
    conn.execute(
        "INSERT INTO harness_workspaces (id, tool_id, path, name, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![id, tool_id, canonical, name, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(HarnessWorkspaceRow {
        id,
        tool_id: tool_id.to_string(),
        path: canonical,
        name,
        created_at: now,
    })
}

/// Remove a workspace and every topic under it (messages cascade).
pub fn remove_workspace(conn: &Connection, workspace_id: &str) -> Result<(), String> {
    let sessions: Vec<String> = {
        let paths: Vec<(String, String)> = conn
            .prepare("SELECT tool_id, path FROM harness_workspaces WHERE id = ?1")
            .map_err(|e| e.to_string())?
            .query_map([workspace_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let mut ids = Vec::new();
        for (tool_id, path) in paths {
            let mut stmt = conn
                .prepare(
                    "SELECT id FROM harness_sessions WHERE tool_id = ?1 AND workspace_path = ?2",
                )
                .map_err(|e| e.to_string())?;
            let found = stmt
                .query_map(rusqlite::params![tool_id, path], |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            ids.extend(found);
        }
        ids
    };
    for sid in sessions {
        conn.execute("DELETE FROM harness_sessions WHERE id = ?1", [sid])
            .map_err(|e| e.to_string())?;
    }
    conn.execute(
        "DELETE FROM harness_workspaces WHERE id = ?1",
        [workspace_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ===== Sessions (topics) =====

/// List topics for one (tool, workspace).
pub fn list_sessions(
    conn: &Connection,
    tool_id: &str,
    workspace_path: &str,
) -> Result<Vec<HarnessSessionRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, tool_id, workspace_path, harness_session_id, title, model_id, created_at, updated_at
             FROM harness_sessions WHERE tool_id = ?1 AND workspace_path = ?2
             ORDER BY updated_at DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![tool_id, workspace_path], row_session)
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

/// Create a new topic under (tool, workspace).
pub fn create_session(
    conn: &Connection,
    tool_id: &str,
    workspace_path: &str,
    title: &str,
) -> Result<HarnessSessionRow, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = crate::utils::time::now_iso();
    conn.execute(
        "INSERT INTO harness_sessions (id, tool_id, workspace_path, title, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        rusqlite::params![id, tool_id, workspace_path, title, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(HarnessSessionRow {
        id,
        tool_id: tool_id.to_string(),
        workspace_path: workspace_path.to_string(),
        harness_session_id: None,
        title: title.to_string(),
        model_id: None,
        created_at: now.clone(),
        updated_at: now,
    })
}

/// Load one topic row.
pub fn get_session(conn: &Connection, session_row_id: &str) -> Result<HarnessSessionRow, String> {
    conn.query_row(
        "SELECT id, tool_id, workspace_path, harness_session_id, title, model_id, created_at, updated_at
         FROM harness_sessions WHERE id = ?1",
        [session_row_id],
        row_session,
    )
    .map_err(|e| e.to_string())
}

/// Attach the harness-side session id (idempotent upsert of the column).
pub fn set_session_harness_id(
    conn: &Connection,
    session_row_id: &str,
    harness_session_id: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE harness_sessions SET harness_session_id = ?2, updated_at = ?3 WHERE id = ?1",
        rusqlite::params![
            session_row_id,
            harness_session_id,
            crate::utils::time::now_iso()
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Rename a topic.
pub fn rename_session(conn: &Connection, session_row_id: &str, title: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE harness_sessions SET title = ?2, updated_at = ?3 WHERE id = ?1",
        rusqlite::params![session_row_id, title, crate::utils::time::now_iso()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Bump the updated_at stamp (keeps "recent topic" ordering).
pub fn touch_session(conn: &Connection, session_row_id: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE harness_sessions SET updated_at = ?2 WHERE id = ?1",
        rusqlite::params![session_row_id, crate::utils::time::now_iso()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Delete one topic (messages cascade via FK).
pub fn delete_session(conn: &Connection, session_row_id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM harness_sessions WHERE id = ?1",
        [session_row_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ===== Messages =====

/// Append one message to a topic. Fills a generated id when absent; returns the
/// stored message (id included) so the caller can render it.
pub fn append_message(
    conn: &Connection,
    session_row_id: &str,
    message: &Message,
) -> Result<Message, String> {
    let mut stored = message.clone();
    if stored.id.as_deref().map(str::is_empty).unwrap_or(true) {
        stored.id = Some(uuid::Uuid::new_v4().to_string());
    }
    let data = serde_json::to_string(&stored).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO harness_messages (id, session_row_id, role, created_at, data)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            stored.id,
            session_row_id,
            stored.role,
            crate::utils::time::now_iso(),
            data
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(stored)
}

/// Load a topic's messages in insertion order.
pub fn load_messages(conn: &Connection, session_row_id: &str) -> Result<Vec<Message>, String> {
    let mut stmt = conn
        .prepare("SELECT data FROM harness_messages WHERE session_row_id = ?1 ORDER BY rowid ASC")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([session_row_id], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    rows.into_iter()
        .map(|data| serde_json::from_str::<StoredHarnessMessage>(&data).map(|s| s.message))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

// ===== Custom ACP agents (app-data JSON file; AIO is not the trust boundary —
// the user authors the command line themselves, same as MCP stdio servers) =====

/// A user-registered ACP-speaking agent (command + argv).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HarnessCustomAgent {
    pub id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
}

/// File name of the custom-agent registry inside the app data dir.
const CUSTOM_AGENTS_FILE: &str = "harness-agents.json";

fn custom_agents_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    use tauri::Manager;
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(dir.join(CUSTOM_AGENTS_FILE))
}

/// Load every custom agent (empty list when the file is absent/corrupted — a
/// corrupted registry must not brick the harness page).
pub fn list_custom_agents(app: &tauri::AppHandle) -> Result<Vec<HarnessCustomAgent>, String> {
    let path = custom_agents_path(app)?;
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Ok(Vec::new());
    };
    serde_json::from_str(&raw).map_err(|e| e.to_string())
}

/// Insert or update one custom agent (id upsert; the id is generated when empty).
pub fn save_custom_agent(
    app: &tauri::AppHandle,
    mut agent: HarnessCustomAgent,
) -> Result<HarnessCustomAgent, String> {
    if agent.name.trim().is_empty() {
        return Err(ierr!("名称不能为空", "Name must not be empty"));
    }
    if agent.command.trim().is_empty() {
        return Err(ierr!("命令不能为空", "Command must not be empty"));
    }
    if agent.id.trim().is_empty() {
        agent.id = uuid::Uuid::new_v4().to_string();
    }
    let mut agents = list_custom_agents(app)?;
    if let Some(existing) = agents.iter_mut().find(|a| a.id == agent.id) {
        *existing = agent.clone();
    } else {
        agents.push(agent.clone());
    }
    write_custom_agents(app, &agents)?;
    Ok(agent)
}

/// Delete one custom agent by id (missing id is a no-op).
pub fn delete_custom_agent(app: &tauri::AppHandle, id: &str) -> Result<(), String> {
    let mut agents = list_custom_agents(app)?;
    let before = agents.len();
    agents.retain(|a| a.id != id);
    if agents.len() != before {
        write_custom_agents(app, &agents)?;
    }
    Ok(())
}

fn write_custom_agents(
    app: &tauri::AppHandle,
    agents: &[HarnessCustomAgent],
) -> Result<(), String> {
    let path = custom_agents_path(app)?;
    let raw = serde_json::to_string_pretty(agents).map_err(|e| e.to_string())?;
    crate::core::fs_util::atomic_write(&path, raw.as_bytes()).map_err(|e| e.to_string())
}

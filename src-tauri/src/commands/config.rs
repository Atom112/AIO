//! App configuration and assistant management commands (assistant/config/profile overrides, etc.).
//! Disk persistence and migration for config live in core/config.rs (AppConfigDisk); this file keeps only thin IPC wrappers.

use crate::commands::attachment::{
    cleanup_attachment_ids, load_message_attachments_batch, sync_message_attachments,
};
use crate::core::models::*;
use crate::core::secure_store;
use crate::core::state::DbState;
use crate::ierr;
use rusqlite::params;
use std::fs;
use tauri::{AppHandle, Manager};

/// Save the general app configuration (business logic lives in core::config).
/// HOR-4: keyring write + atomic disk write — async + blocking pool.
#[tauri::command]
pub async fn save_app_config(app: AppHandle, config: AppConfig) -> Result<(), String> {
    tokio::task::spawn_blocking(move || crate::core::config::save_app_config(&app, &config))
        .await
        .map_err(|e| e.to_string())?
}

/// Load the general app configuration (business logic lives in core::config).
/// HOR-4: keyring read + fs read (plus legacy plaintext migration writes) — async + blocking pool.
#[tauri::command]
pub async fn load_app_config(app: AppHandle) -> Result<AppConfig, String> {
    tokio::task::spawn_blocking(move || crate::core::config::load_app_config(&app))
        .await
        .map_err(|e| e.to_string())?
}

/// Partial app-config patch accepted by `patch_app_config` (FE-07).
/// Only the fields the settings page edits per keystroke are patchable;
/// `None` fields keep their stored value.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PatchAppConfig {
    max_tool_rounds: Option<u32>,
    max_concurrent_subagents: Option<u32>,
    subagent_isolation_mode: Option<String>,
}

/// Atomically merge a partial app-config update: load → merge `Some` fields → save,
/// all inside one backend call (FE-07). The settings page previously ran a full
/// load→modify→save round-trip from the UI on every keystroke (2 IPC calls plus a
/// read-modify-write race); it now sends only the changed field, debounced.
/// Runs on the blocking pool: it touches the OS keychain and the config file.
#[tauri::command]
pub async fn patch_app_config(app: AppHandle, patch: PatchAppConfig) -> Result<AppConfig, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut cfg = crate::core::config::load_app_config(&app)?;
        if let Some(v) = patch.max_tool_rounds {
            cfg.max_tool_rounds = Some(v);
        }
        if let Some(v) = patch.max_concurrent_subagents {
            cfg.max_concurrent_subagents = Some(v);
        }
        if let Some(v) = patch.subagent_isolation_mode {
            cfg.subagent_isolation_mode = v;
        }
        crate::core::config::save_app_config(&app, &cfg)?;
        Ok(cfg)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Sync the frontend locale to the backend (language of user-visible text such as error messages).
/// Called once at frontend startup and once per language switch; missing/unknown values fall back to Chinese.
#[tauri::command]
pub fn set_locale(locale: String) {
    crate::core::i18n::set_locale(locale);
}

/// Load all assistants (with their topics and message trees).
#[tauri::command]
pub async fn load_assistants(state: tauri::State<'_, DbState>) -> Result<Vec<Assistant>, String> {
    let conn = state.0.lock();

    // 1. Load assistants
    let mut stmt = conn
        .prepare("SELECT id, name, prompt, model_id, mcp_server_ids, skill_ids, project_id, agent_mode, assistant_type FROM assistants ORDER BY id")
        .map_err(|e| e.to_string())?;
    let assistant_iter = stmt
        .query_map([], |row| {
            let mcp_ids_json: Option<String> = row.get(4)?;
            let mcp_server_ids: Vec<String> = mcp_ids_json
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let skill_ids_json: Option<String> = row.get(5)?;
            let skill_ids: Vec<String> = skill_ids_json
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let project_id: Option<String> = row.get(6)?;
            let agent_mode_str: Option<String> = row.get(7)?;
            // SEC-18 (parity with load_assistant_agent_mode): an unparsable persisted
            // mode falls back to Off fail-closed, with a visible log — the list path
            // used to fall back silently.
            let agent_mode = agent_mode_str
                .clone()
                .and_then(|s| serde_json::from_str(&format!("\"{}\"", s)).ok())
                .unwrap_or_else(|| {
                    if agent_mode_str.is_some() {
                        tracing::warn!(
                            "[sec-18] persisted agent_mode failed to parse; list entry falls back to Off (fail-closed)"
                        );
                    }
                    crate::core::models::AgentMode::Off
                });
            Ok(Assistant {
                id: row.get(0)?,
                name: row.get(1)?,
                prompt: row.get(2)?,
                model_id: row.get(3)?,
                mcp_server_ids,
                skill_ids,
                project_id,
                agent_mode,
                assistant_type: row.get(8)?,
                topics: vec![],
            })
        })
        .map_err(|e| e.to_string())?;

    let mut assistants = Vec::new();
    for asst in assistant_iter {
        let mut asst = asst.map_err(|e| e.to_string())?;

        // 2. Load topics for each assistant
        let mut t_stmt = conn
            .prepare("SELECT id, name, summary, renamed, branched_from_message_id, parent_topic_id FROM topics WHERE assistant_id = ?")
            .map_err(|e| e.to_string())?;
        let topic_iter = t_stmt
            .query_map([&asst.id], |row| {
                Ok(Topic {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    summary: row.get(2)?,
                    // SQLite INTEGER (0/1) → bool
                    renamed: row.get::<_, i64>(3)? != 0,
                    branched_from_message_id: row.get(4)?,
                    parent_topic_id: row.get(5)?,
                    history: vec![],
                })
            })
            .map_err(|e| e.to_string())?;

        for topic in topic_iter {
            let mut topic = topic.map_err(|e| e.to_string())?;

            // 3. Load history messages (including tool_call_id / name / tool_calls_json / input_tokens / output_tokens / cached_input_tokens, so tool-call sessions resume across restarts and token stats are kept)
            let mut m_stmt = conn.prepare("SELECT id, role, content, model_id, display_files, display_text, reasoning, tool_call_id, name, tool_calls_json, input_tokens, output_tokens, agent_steps_json, interim_content, agent_start_time, parent_message_id, branch_index, images_json, cached_input_tokens FROM messages WHERE topic_id = ? ORDER BY timestamp ASC")
    .map_err(|e| e.to_string())?;

            let msg_iter = m_stmt
                .query_map([&topic.id], |row| {
                    // Extract display_files (at index 4)
                    let display_files_json: Option<String> = row.get(4)?;
                    let display_files =
                        display_files_json.and_then(|s| serde_json::from_str(&s).ok());

                    // Extract content (at index 2)
                    let content_json: String = row.get(2)?;
                    let content_value = serde_json::from_str(&content_json)
                        .unwrap_or(serde_json::Value::String(content_json));

                    // Extract tool_calls_json (at index 9)
                    let tool_calls_json: Option<String> = row.get(9)?;
                    let tool_calls = tool_calls_json
                        .and_then(|s| serde_json::from_str::<Vec<ToolCall>>(&s).ok());

                    Ok(Message {
                        id: row.get(0)?,                   // index 0: id
                        role: row.get(1)?,                 // index 1: role
                        content: content_value,            // index 2: content (JSON)
                        model_id: row.get(3)?,             // index 3: model_id
                        display_files,                     // already-parsed files
                        display_text: row.get(5)?,         // index 5: display_text
                        tool_call_id: row.get(7)?,         // index 7: tool_call_id
                        name: row.get(8)?,                 // index 8: name
                        tool_calls,                        // index 9: tool_calls_json (parsed)
                        reasoning: row.get(6)?,            // index 6: reasoning
                        input_tokens: row.get(10)?,        // index 10: input_tokens
                        output_tokens: row.get(11)?,       // index 11: output_tokens
                        cached_input_tokens: row.get(18)?, // index 18: cached_input_tokens
                        agent_steps: row
                            .get::<_, Option<String>>(12)?
                            .and_then(|s| serde_json::from_str(&s).ok()), // index 12: agent_steps_json
                        interim_content: row.get(13)?, // index 13: interim_content
                        agent_start_time: row.get(14)?, // index 14: agent_start_time
                        parent_message_id: row.get(15)?, // index 15: parent_message_id
                        branch_index: row.get(16)?,    // index 16: branch_index
                        images: row
                            .get::<_, Option<String>>(17)?
                            .and_then(|s| serde_json::from_str(&s).ok()), // index 17: images_json
                        full_tool_result: None, // session-only in-memory field, not persisted
                    })
                })
                .map_err(|e| e.to_string())?;

            // Batch-load attachments: collect all message IDs first, then query once (avoids N+1 queries)
            let mut messages: Vec<Message> = Vec::new();
            let mut msg_ids: Vec<String> = Vec::new();
            for msg in msg_iter {
                let message = msg.map_err(|e| e.to_string())?;
                if let Some(mid) = &message.id {
                    msg_ids.push(mid.clone());
                }
                messages.push(message);
            }

            let attachments_map = load_message_attachments_batch(&conn, &msg_ids)?;

            for mut message in messages {
                if let Some(message_id) = &message.id {
                    if let Some(mut stored_files) = attachments_map.get(message_id).cloned() {
                        if !stored_files.is_empty() {
                            if let Some(ref display_files) = message.display_files {
                                for (stored, display) in stored_files.iter_mut().zip(display_files)
                                {
                                    stored.name = display.name.clone();
                                }
                            }
                            message.display_files = Some(stored_files);
                        }
                    }
                }
                topic.history.push(message);
            }
            asst.topics.push(topic);
        }
        assistants.push(asst);
    }

    Ok(assistants)
}

/// Synchronously persists an assistant (called inside spawn_blocking to avoid blocking the async runtime).
///
/// Wraps the writes in one SQLite transaction (batched writes): avoids per-row autocommit fsync, which gets expensive as history grows.
fn persist_assistant(conn: &rusqlite::Connection, assistant: &Assistant) -> Result<(), String> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| e.to_string())?;
    let result = persist_assistant_inner(conn, assistant);
    if result.is_ok() {
        conn.execute_batch("COMMIT").map_err(|e| e.to_string())?;
    } else {
        let _ = conn.execute_batch("ROLLBACK");
    }
    result
}

/// The transactional body of persist_assistant.
fn persist_assistant_inner(
    conn: &rusqlite::Connection,
    assistant: &Assistant,
) -> Result<(), String> {
    // 1. Insert/update the assistant's base info
    let mcp_ids_json =
        serde_json::to_string(&assistant.mcp_server_ids).unwrap_or_else(|_| "[]".to_string());
    let skill_ids_json =
        serde_json::to_string(&assistant.skill_ids).unwrap_or_else(|_| "[]".to_string());
    let agent_mode_str = serde_json::to_string(&assistant.agent_mode)
        .unwrap_or_else(|_| "\"off\"".to_string())
        .trim_matches('"')
        .to_string();
    conn.execute(
        "INSERT INTO assistants (id, name, prompt, model_id, mcp_server_ids, skill_ids, project_id, agent_mode, assistant_type) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(id) DO UPDATE SET name=?2, prompt=?3, model_id=?4, mcp_server_ids=?5, skill_ids=?6, project_id=?7, agent_mode=?8, assistant_type=?9",
        params![assistant.id, assistant.name, assistant.prompt, assistant.model_id, mcp_ids_json, skill_ids_json, assistant.project_id, agent_mode_str, assistant.assistant_type],
    )
    .map_err(|e| e.to_string())?;

    // 2. Delete topics that were removed on the frontend (otherwise they would reappear on reload)
    let current_topic_ids: Vec<String> = assistant.topics.iter().map(|t| t.id.clone()).collect();
    let mut stmt = conn
        .prepare("SELECT id FROM topics WHERE assistant_id = ?")
        .map_err(|e| e.to_string())?;
    let db_topic_ids: Vec<String> = stmt
        .query_map([&assistant.id], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|e| e.to_string())?;

    for db_id in db_topic_ids {
        if !current_topic_ids.contains(&db_id) {
            let attachment_ids = attachment_ids_for_topic(conn, &db_id)?;
            // Delete checkpoints together with the topic (that table has no foreign key, so this must be explicit)
            crate::commands::llm::checkpoints::delete_topic_checkpoints(conn, &db_id);
            conn.execute("DELETE FROM topics WHERE id = ?", params![db_id])
                .map_err(|e| e.to_string())?;
            cleanup_attachment_ids(conn, &attachment_ids)?;
        }
    }

    // 3. Incrementally sync each topic
    for topic in &assistant.topics {
        conn.execute(
            "INSERT INTO topics (id, assistant_id, name, summary, renamed, branched_from_message_id, parent_topic_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET name=?3, summary=?4, renamed=?5, branched_from_message_id=?6, parent_topic_id=?7",
            params![topic.id, assistant.id, topic.name, topic.summary, topic.renamed as i64, topic.branched_from_message_id, topic.parent_topic_id],
        )
        .map_err(|e| e.to_string())?;

        // 4. Incremental message sync
        // Messages sync by ID via the ON CONFLICT clause (insert new, update existing) instead of DELETE ALL + reinsert
        let current_message_ids: Vec<String> = topic
            .history
            .iter()
            .filter_map(|message| message.id.clone())
            .collect();
        let mut message_stmt = conn
            .prepare("SELECT id FROM messages WHERE topic_id = ?1")
            .map_err(|e| e.to_string())?;
        let db_message_ids = message_stmt
            .query_map([&topic.id], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(message_stmt);
        for db_message_id in db_message_ids {
            if !current_message_ids.contains(&db_message_id) {
                let attachment_ids = attachment_ids_for_message(conn, &db_message_id)?;
                conn.execute("DELETE FROM messages WHERE id = ?1", [&db_message_id])
                    .map_err(|e| e.to_string())?;
                cleanup_attachment_ids(conn, &attachment_ids)?;
            }
        }

        for msg in &topic.history {
            let msg_id = msg
                .id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let files_json = serde_json::to_string(&msg.display_files).ok();
            let content_json = serde_json::to_string(&msg.content).unwrap_or_default();
            let tool_calls_json = serde_json::to_string(&msg.tool_calls).ok();

            // Persist tool_call_id / name / tool_calls_json / input_tokens / output_tokens / cached_input_tokens so tool-call sessions resume across restarts and token stats are kept.
            // ON CONFLICT(id) DO UPDATE overwrites rows on re-save; DO NOTHING would leave stale content.
            conn.execute(
                "INSERT INTO messages (id, topic_id, role, content, model_id, display_files, display_text, reasoning, tool_call_id, name, tool_calls_json, input_tokens, output_tokens, agent_steps_json, interim_content, agent_start_time, parent_message_id, branch_index, images_json, cached_input_tokens)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
                 ON CONFLICT(id) DO UPDATE SET
                   content = excluded.content,
                   reasoning = excluded.reasoning,
                   tool_call_id = excluded.tool_call_id,
                   name = excluded.name,
                   tool_calls_json = excluded.tool_calls_json,
                   display_files = excluded.display_files,
                   display_text = excluded.display_text,
                   input_tokens = excluded.input_tokens,
                   output_tokens = excluded.output_tokens,
                   cached_input_tokens = excluded.cached_input_tokens,
                   agent_steps_json = excluded.agent_steps_json,
                   interim_content = excluded.interim_content,
                   agent_start_time = excluded.agent_start_time,
                   parent_message_id = excluded.parent_message_id,
                   branch_index = excluded.branch_index,
                   images_json = excluded.images_json",
                params![msg_id, topic.id, msg.role, content_json, msg.model_id, files_json, msg.display_text, msg.reasoning, msg.tool_call_id, msg.name, tool_calls_json, msg.input_tokens, msg.output_tokens, serde_json::to_string(&msg.agent_steps).ok(), msg.interim_content, msg.agent_start_time, msg.parent_message_id, msg.branch_index, serde_json::to_string(&msg.images).ok(), msg.cached_input_tokens],
            ).map_err(|e| e.to_string())?;
            sync_message_attachments(conn, &msg_id, msg.display_files.as_ref())?;
        }
    }

    Ok(())
}

/// 读取单个 assistant 的 agent_mode（MCP-13：`run_agent_turn` 的服务端模式裁决——
/// IPC 参数可伪造，权限模式必须以数据库里的持久化记录为准；找不到记录返回 None，
/// 调用方回退到最保守的 Off）。
pub(crate) fn load_assistant_agent_mode(
    conn: &rusqlite::Connection,
    assistant_id: &str,
) -> Option<crate::core::models::AgentMode> {
    let mode_str: Option<String> = conn
        .query_row(
            "SELECT agent_mode FROM assistants WHERE id = ?1",
            [assistant_id],
            |row| row.get(0),
        )
        .ok()
        .flatten();
    mode_str.and_then(|s| match serde_json::from_str(&format!("\"{s}\"")) {
        Ok(mode) => Some(mode),
        // SEC-18: an unparseable persisted mode (hand-edited/corrupted DB row, or a
        // value from a future version) falls back to the caller's fail-closed Off.
        // That fallback is correct post-SEC-19 (Off = no tools), but the drift must
        // be visible instead of silently turning an Auto assistant into chat mode.
        Err(e) => {
            tracing::warn!(
                "[sec-18] assistant {} 的 agent_mode 值无法解析（{:?}），回退 Off（fail-closed）: {e}",
                assistant_id,
                s
            );
            None
        }
    })
}

/// Persist a single assistant (with all its topics and messages).
///
/// # Arguments
/// - `assistant` — the full assistant object (create and update share this command; upsert by id)
///
/// # Behavior
/// Runs the transactional write on a blocking thread while holding the DB lock (batch commit avoids per-row fsync).
#[tauri::command]
pub async fn save_assistant(app: tauri::AppHandle, assistant: Assistant) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let db_state = app.state::<DbState>();
        let conn = db_state.0.lock();
        persist_assistant(&conn, &assistant)
    })
    .await
    .map_err(|e| {
        ierr!(
            "保存助手线程失败: {}",
            "Failed to run assistant save task: {}",
            e
        )
    })?
}

/// Delete an assistant and its associated data.
///
/// # Arguments
/// - `id` — assistant ID
///
/// # Behavior
/// Relies on foreign-key ON DELETE CASCADE to delete its topics and messages; also cleans up
/// the assistant's attachment files. Succeeds silently when the assistant does not exist (idempotent).
#[tauri::command]
pub async fn delete_assistant(state: tauri::State<'_, DbState>, id: String) -> Result<(), String> {
    let conn = state.0.lock();
    let attachment_ids = attachment_ids_for_assistant(&conn, &id)?;
    // ON DELETE CASCADE automatically removes the associated topics and messages
    conn.execute("DELETE FROM assistants WHERE id = ?", params![id])
        .map_err(|e| e.to_string())?;
    cleanup_attachment_ids(&conn, &attachment_ids)?;
    Ok(())
}

fn attachment_ids_for_message(
    conn: &rusqlite::Connection,
    message_id: &str,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT attachment_id FROM message_attachments WHERE message_id = ?1")
        .map_err(|e| e.to_string())?;
    let ids = stmt
        .query_map([message_id], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(ids)
}

fn attachment_ids_for_topic(
    conn: &rusqlite::Connection,
    topic_id: &str,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT ma.attachment_id
             FROM message_attachments ma
             JOIN messages m ON m.id = ma.message_id
             WHERE m.topic_id = ?1",
        )
        .map_err(|e| e.to_string())?;
    let ids = stmt
        .query_map([topic_id], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(ids)
}

fn attachment_ids_for_assistant(
    conn: &rusqlite::Connection,
    assistant_id: &str,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT ma.attachment_id
             FROM message_attachments ma
             JOIN messages m ON m.id = ma.message_id
             JOIN topics t ON t.id = m.topic_id
             WHERE t.assistant_id = ?1",
        )
        .map_err(|e| e.to_string())?;
    let ids = stmt
        .query_map([assistant_id], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(ids)
}

/// Save the activated-models list (api_key is stripped into the system keyring; no plaintext keys on disk)
#[tauri::command]
pub async fn save_activated_models(
    app: AppHandle,
    models: Vec<ActivatedModel>,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || save_activated_models_inner(app, models))
        .await
        .map_err(|e| e.to_string())?
}

/// HOR-4: keyring writes per non-empty key + atomic disk write (blocking-pool body).
fn save_activated_models_inner(app: AppHandle, models: Vec<ActivatedModel>) -> Result<(), String> {
    let path = crate::core::paths::app_config_file("activated_models.json")?;

    // Strip api_key into secure_store so persisted models contain no plaintext keys.
    // Strip only when the secure-store write succeeds; on failure keep the plaintext and report the error (never silently lose a key).
    let mut sanitized: Vec<serde_json::Value> = Vec::new();
    for m in &models {
        let key_stored = if !m.api_key.is_empty() && m.api_key != "local-no-key" {
            let account = secure_store::accounts::activated_model_key(&m.api_url, &m.model_id);
            secure_store::set_reported(&app, &account, &m.api_key)
        } else {
            true // no key to store (or a placeholder) - safe to strip
        };
        // Serialize to a JSON Value; drop the api_key field depending on the store result
        let mut v = serde_json::to_value(m).map_err(|e| e.to_string())?;
        if key_stored {
            if let Some(obj) = v.as_object_mut() {
                obj.remove("api_key");
            }
        }
        sanitized.push(v);
    }

    let json = serde_json::to_string_pretty(&sanitized).map_err(|e| e.to_string())?;
    crate::core::fs_util::atomic_write(&path, json.as_bytes()).map_err(|e| e.to_string())?;
    Ok(())
}

/// Load the activated-models list (api_key restored from the keyring)
#[tauri::command]
pub async fn load_activated_models(app: AppHandle) -> Result<Vec<ActivatedModel>, String> {
    tokio::task::spawn_blocking(move || load_activated_models_inner(app))
        .await
        .map_err(|e| e.to_string())?
}

/// HOR-4: fs read + per-model keyring reads and legacy migration writes (blocking-pool body).
fn load_activated_models_inner(app: AppHandle) -> Result<Vec<ActivatedModel>, String> {
    let path = crate::core::paths::app_config_dir()?.join("activated_models.json");

    if !path.exists() {
        return Ok(vec![]);
    }
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut models: Vec<ActivatedModel> =
        serde_json::from_str(&content).map_err(|e| e.to_string())?;

    // Restore api_key from secure_store and auto-migrate legacy data (old files containing plaintext api_key)
    let mut needs_migration = false;
    let mut migrated_ok: std::collections::HashSet<String> = std::collections::HashSet::new();
    for m in &mut models {
        // Legacy format check (JSON still contains api_key directly)
        if !m.api_key.is_empty() && m.api_key != "local-no-key" {
            let account = secure_store::accounts::activated_model_key(&m.api_url, &m.model_id);
            // Migrate into secure storage; only successfully migrated entries may have their plaintext stripped on write-back (never silently lose a key)
            if secure_store::set_reported(&app, &account, &m.api_key) {
                migrated_ok.insert(account);
            }
            needs_migration = true;
        }

        // Restore from secure_store (overriding the legacy JSON value, or filling in a key missing in the new format)
        let account = secure_store::accounts::activated_model_key(&m.api_url, &m.model_id);
        if let Ok(Some(key)) = secure_store::get(&app, &account) {
            m.api_key = key;
        }
    }

    // Migration: write back the sanitized JSON immediately (strip plaintext api_key only for successfully migrated entries)
    if needs_migration {
        let mut sanitized: Vec<serde_json::Value> = Vec::new();
        for m in &models {
            let mut v = serde_json::to_value(m).map_err(|e| e.to_string())?;
            let account = secure_store::accounts::activated_model_key(&m.api_url, &m.model_id);
            let may_strip = migrated_ok.contains(&account)
                || m.api_key.is_empty()
                || m.api_key == "local-no-key";
            if may_strip {
                if let Some(obj) = v.as_object_mut() {
                    obj.remove("api_key");
                }
            }
            sanitized.push(v);
        }
        if let Ok(json) = serde_json::to_string_pretty(&sanitized) {
            if let Err(e) = crate::core::fs_util::atomic_write(&path, json.as_bytes()) {
                tracing::error!("迁移写回 activated_models.json 失败: {e}");
            }
        }
    }

    Ok(models)
}

/// Save the raw model info list fetched from the cloud or an API
#[tauri::command]
pub fn save_fetched_models(models: Vec<ModelInfo>) -> Result<(), String> {
    let path = crate::core::paths::app_config_file("fetched_models.json")?;
    let json = serde_json::to_string_pretty(&models).map_err(|e| e.to_string())?;
    crate::core::fs_util::atomic_write(&path, json.as_bytes()).map_err(|e| e.to_string())?;
    Ok(())
}

/// Load the previously fetched model info list
#[tauri::command]
pub fn load_fetched_models() -> Result<Vec<ModelInfo>, String> {
    let path = crate::core::paths::app_config_dir()?.join("fetched_models.json");

    if !path.exists() {
        return Ok(vec![]);
    }
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let models: Vec<ModelInfo> = serde_json::from_str(&content).map_err(|e| e.to_string())?;
    Ok(models)
}

// ====== Per-Profile Model Override Persistence ======

///
/// Load profile-model-overrides.json from disk and return the override list.
/// A missing or corrupt file returns an empty vec (not an error; all profiles fall back to the parent model).
#[tauri::command]
pub fn load_profile_model_overrides() -> Result<Vec<ProfileModelOverride>, String> {
    let path = crate::core::paths::app_config_dir()?.join("profile-model-overrides.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path).map_err(|e| {
        ierr!(
            "读取 profile-model-overrides.json 失败: {}",
            "Failed to read profile-model-overrides.json: {}",
            e
        )
    })?;
    let file: ProfileModelOverridesFile = serde_json::from_str(&raw).map_err(|e| {
        ierr!(
            "解析 profile-model-overrides.json 失败: {}",
            "Failed to parse profile-model-overrides.json: {}",
            e
        )
    })?;
    Ok(file.overrides)
}

///
/// Persist the subagent profile model overrides to disk.
/// FE-12: the api_key field is stripped before writing — the file only needs
/// profile → model identity on load (the live key is resolved fresh from the
/// provider config each turn), so plaintext keys must not land in this file.
#[tauri::command]
pub fn save_profile_model_overrides(
    mut overrides: Vec<ProfileModelOverride>,
) -> Result<(), String> {
    for o in overrides.iter_mut() {
        o.api_key.clear();
    }
    let path = crate::core::paths::app_config_file("profile-model-overrides.json")?;
    let file = ProfileModelOverridesFile {
        version: 1,
        updated_at: crate::utils::time::now_iso_z(),
        overrides,
    };
    let json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    crate::core::fs_util::atomic_write(&path, json.as_bytes()).map_err(|e| e.to_string())?;
    Ok(())
}

// ====== Custom Subagent Profiles (CRUD) ======
use crate::core::models::CustomSubagentProfile;

use crate::core::models::CustomSubagentProfilesFile;

fn custom_profiles_path() -> Result<std::path::PathBuf, String> {
    Ok(crate::core::paths::app_config_dir()?.join("custom-subagent-profiles.json"))
}

///
/// Load all custom subagent profiles from disk.
/// A missing or corrupt file returns an empty vec (not an error; it just means no custom profiles).
#[tauri::command]
pub fn list_custom_subagent_profiles() -> Result<Vec<CustomSubagentProfile>, String> {
    let path = custom_profiles_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path).map_err(|e| {
        ierr!(
            "读取 custom-subagent-profiles.json 失败: {}",
            "Failed to read custom-subagent-profiles.json: {}",
            e
        )
    })?;
    let file: CustomSubagentProfilesFile = serde_json::from_str(&raw).map_err(|e| {
        ierr!(
            "解析 custom-subagent-profiles.json 失败: {}",
            "Failed to parse custom-subagent-profiles.json: {}",
            e
        )
    })?;
    Ok(file.profiles)
}

///
/// Save (insert or update) a custom subagent profile.
/// Deduplicated by id: an existing id is overwritten, otherwise appended.
#[tauri::command]
pub fn save_custom_subagent_profile(profile: CustomSubagentProfile) -> Result<(), String> {
    let path = custom_profiles_path()?;
    let mut profiles: Vec<CustomSubagentProfile> = if path.exists() {
        let raw = fs::read_to_string(&path).map_err(|e| {
            ierr!(
                "读取 custom-subagent-profiles.json 失败: {}",
                "Failed to read custom-subagent-profiles.json: {}",
                e
            )
        })?;
        let file: CustomSubagentProfilesFile = serde_json::from_str(&raw).map_err(|e| {
            ierr!(
                "解析 custom-subagent-profiles.json 失败: {}",
                "Failed to parse custom-subagent-profiles.json: {}",
                e
            )
        })?;
        file.profiles
    } else {
        Vec::new()
    };
    // Upsert by id
    if let Some(existing) = profiles.iter_mut().find(|p| p.id == profile.id) {
        *existing = profile;
    } else {
        profiles.push(profile);
    }
    // Write back
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    let file = CustomSubagentProfilesFile {
        version: 1,
        updated_at: crate::utils::time::now_iso_z(),
        profiles,
    };
    let json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    crate::core::fs_util::atomic_write(&path, json.as_bytes()).map_err(|e| e.to_string())?;
    Ok(())
}

///
/// Delete a custom subagent profile by id.
#[tauri::command]
pub fn delete_custom_subagent_profile(profile_id: String) -> Result<(), String> {
    let path = custom_profiles_path()?;
    if !path.exists() {
        return Ok(());
    }
    let raw = fs::read_to_string(&path).map_err(|e| {
        ierr!(
            "读取 custom-subagent-profiles.json 失败: {}",
            "Failed to read custom-subagent-profiles.json: {}",
            e
        )
    })?;
    let file: CustomSubagentProfilesFile = serde_json::from_str(&raw).map_err(|e| {
        ierr!(
            "解析 custom-subagent-profiles.json 失败: {}",
            "Failed to parse custom-subagent-profiles.json: {}",
            e
        )
    })?;
    let profiles: Vec<CustomSubagentProfile> = file
        .profiles
        .into_iter()
        .filter(|p| p.id != profile_id)
        .collect();
    // Write back
    let updated = CustomSubagentProfilesFile {
        version: 1,
        updated_at: crate::utils::time::now_iso_z(),
        profiles,
    };
    let json = serde_json::to_string_pretty(&updated).map_err(|e| e.to_string())?;
    crate::core::fs_util::atomic_write(&path, json.as_bytes()).map_err(|e| e.to_string())?;
    Ok(())
}

// ====== System auto-start ======

/// Enable or disable system auto-start.
/// - Windows: registry HKCU\Software\Microsoft\Windows\CurrentVersion\Run
/// - macOS: ~/Library/LaunchAgents/com.loch.aio.plist
/// - Linux: ~/.config/autostart/aio.desktop
#[tauri::command]
pub async fn set_auto_start(app: AppHandle, enabled: bool) -> Result<(), String> {
    tokio::task::spawn_blocking(move || set_auto_start_inner(app, enabled))
        .await
        .map_err(|e| e.to_string())?
}

/// HOR-4: spawns `reg` (Windows) / writes launch-agent plist or .desktop file
/// (macOS/Linux) — subprocess + disk work (blocking-pool body).
fn set_auto_start_inner(app: AppHandle, enabled: bool) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let app_name = "AIO";
    // Resolve the current executable path
    let exe_path = std::env::current_exe().map_err(|e| {
        ierr!(
            "无法获取可执行文件路径: {}",
            "Failed to get executable path: {}",
            e
        )
    })?;
    let exe_str = exe_path.to_string_lossy().to_string();

    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        // SEC-06: PATH-only resolution — a bare "reg" spawn would consult the process
        // cwd before PATH; reg.exe lives in System32 and always resolves.
        let reg = crate::utils::exec_resolve::resolve_executable("reg")
            .ok_or_else(|| ierr!("未找到 reg.exe（System32）", "reg.exe not found (System32)"))?;
        if enabled {
            let output = Command::new(reg)
                .args([
                    "add",
                    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                    "/v",
                    app_name,
                    "/t",
                    "REG_SZ",
                    "/d",
                    &exe_str,
                    "/f",
                ])
                .output()
                .map_err(|e| {
                    ierr!(
                        "执行注册表写入失败: {}",
                        "Failed to run registry write: {}",
                        e
                    )
                })?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(ierr!(
                    "注册表写入失败: {}",
                    "Registry write failed: {}",
                    stderr
                ));
            }
        } else {
            let output = Command::new(reg)
                .args([
                    "delete",
                    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                    "/v",
                    app_name,
                    "/f",
                ])
                .output()
                .map_err(|e| {
                    ierr!(
                        "执行注册表删除失败: {}",
                        "Failed to run registry delete: {}",
                        e
                    )
                })?;
            // Deleting a nonexistent key is not an error (reg delete /f still returns success for missing keys)
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                // reg delete: the localized not-found error (matched via the stderr check below) is treated as success
                if !stderr.contains("找不到") {
                    return Err(ierr!(
                        "注册表删除失败: {}",
                        "Registry delete failed: {}",
                        stderr
                    ));
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let plist_dir = dirs::home_dir()
            .ok_or_else(|| ierr!("无法获取用户主目录", "Failed to get user home directory"))?
            .join("Library/LaunchAgents");
        fs::create_dir_all(&plist_dir).map_err(|e| e.to_string())?;
        let plist_path = plist_dir.join(format!("{}.plist", crate::core::paths::BUNDLE_ID));
        if enabled {
            let plist = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>"#,
                crate::core::paths::BUNDLE_ID,
                exe_str
            );
            crate::core::fs_util::atomic_write(&plist_path, plist.as_bytes()).map_err(|e| {
                ierr!(
                    "写入 LaunchAgent 失败: {}",
                    "Failed to write LaunchAgent: {}",
                    e
                )
            })?;
        } else {
            if plist_path.exists() {
                fs::remove_file(&plist_path).map_err(|e| {
                    ierr!(
                        "删除 LaunchAgent 失败: {}",
                        "Failed to delete LaunchAgent: {}",
                        e
                    )
                })?;
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let autostart_dir = dirs::home_dir()
            .ok_or_else(|| ierr!("无法获取用户主目录", "Failed to get user home directory"))?
            .join(".config/autostart");
        fs::create_dir_all(&autostart_dir).map_err(|e| e.to_string())?;
        let desktop_path = autostart_dir.join("aio.desktop");
        if enabled {
            let desktop = format!(
                r#"[Desktop Entry]
Type=Application
Name=AIO
Exec={}
Terminal=false
X-GNOME-Autostart-enabled=true"#,
                exe_str
            );
            crate::core::fs_util::atomic_write(&desktop_path, desktop.as_bytes()).map_err(|e| {
                ierr!(
                    "写入 autostart 失败: {}",
                    "Failed to write autostart file: {}",
                    e
                )
            })?;
        } else {
            if desktop_path.exists() {
                fs::remove_file(&desktop_path).map_err(|e| {
                    ierr!(
                        "删除 autostart 失败: {}",
                        "Failed to delete autostart file: {}",
                        e
                    )
                })?;
            }
        }
    }

    // Persist the state
    let mut config = crate::core::config::load_app_config(&app)?;
    config.auto_start_enabled = enabled;
    crate::core::config::save_app_config(&app, &config)
}

/// Return whether system auto-start is enabled.
/// HOR-4: keyring read + fs read — async + blocking pool.
#[tauri::command]
pub async fn is_auto_start_enabled(app: AppHandle) -> Result<bool, String> {
    tokio::task::spawn_blocking(move || {
        let config = crate::core::config::load_app_config(&app)?;
        Ok(config.auto_start_enabled)
    })
    .await
    .map_err(|e| e.to_string())?
}

// ====== Topic branching ======

/// Branch a new topic off the given message.
/// - Copies the source topic's messages up to and including that message into the new topic
/// - The new topic's `branched_from_message_id` is set to the source message ID
/// - The new topic's `parent_topic_id` is set to the source topic ID
/// Returns the new topic object.
#[tauri::command]
pub async fn branch_topic(
    state: tauri::State<'_, DbState>,
    source_topic_id: String,
    source_message_id: String,
) -> Result<Topic, String> {
    let conn = state.0.lock();

    // 1. Look up the source topic's assistant_id
    let assistant_id: String = conn
        .query_row(
            "SELECT assistant_id FROM topics WHERE id = ?1",
            params![source_topic_id],
            |row| row.get(0),
        )
        .map_err(|e| ierr!("查询源话题失败: {}", "Failed to query source topic: {}", e))?;

    // 2. Read all messages of the source topic, up to the branch point (inclusive)
    let mut m_stmt = conn
        .prepare(
            "SELECT id, role, content, model_id, display_files, display_text, reasoning,
             tool_call_id, name, tool_calls_json, input_tokens, output_tokens,
             agent_steps_json, interim_content, agent_start_time, parent_message_id, branch_index,
             images_json, cached_input_tokens
             FROM messages WHERE topic_id = ?1 ORDER BY timestamp ASC",
        )
        .map_err(|e| e.to_string())?;

    let all_msgs: Vec<Message> = m_stmt
        .query_map(params![source_topic_id], |row| {
            let display_files_json: Option<String> = row.get(4)?;
            let content_json: String = row.get(2)?;
            let tool_calls_json: Option<String> = row.get(9)?;
            Ok(Message {
                id: row.get(0)?,
                role: row.get(1)?,
                content: serde_json::from_str(&content_json)
                    .unwrap_or(serde_json::Value::String(content_json)),
                model_id: row.get(3)?,
                display_files: display_files_json.and_then(|s| serde_json::from_str(&s).ok()),
                display_text: row.get(5)?,
                reasoning: row.get(6)?,
                tool_call_id: row.get(7)?,
                name: row.get(8)?,
                tool_calls: tool_calls_json.and_then(|s| serde_json::from_str(&s).ok()),
                input_tokens: row.get(10)?,
                output_tokens: row.get(11)?,
                cached_input_tokens: row.get(18)?,
                agent_steps: row
                    .get::<_, Option<String>>(12)?
                    .and_then(|s| serde_json::from_str(&s).ok()),
                interim_content: row.get(13)?,
                agent_start_time: row.get(14)?,
                parent_message_id: row.get(15)?,
                branch_index: row.get(16)?,
                images: row
                    .get::<_, Option<String>>(17)?
                    .and_then(|s| serde_json::from_str(&s).ok()),
                full_tool_result: None,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    drop(m_stmt);

    // Truncate at the branch point (inclusive)
    let branch_pos = all_msgs
        .iter()
        .position(|m| m.id.as_deref() == Some(&source_message_id));
    let history: Vec<Message> = match branch_pos {
        Some(pos) => all_msgs.into_iter().take(pos + 1).collect(),
        None => {
            return Err(ierr!(
                "消息 {} 不在话题 {} 中",
                "Message {} is not in topic {}",
                source_message_id,
                source_topic_id
            ))
        }
    };

    // 3. Create the new topic
    let new_topic_id = uuid::Uuid::new_v4().to_string();
    let source_topic_name: String = conn
        .query_row(
            "SELECT name FROM topics WHERE id = ?1",
            params![source_topic_id],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "分支".into());

    // Count existing sibling topics (same parent_topic_id) for automatic numbering
    let sibling_count: i32 = conn
        .query_row(
            "SELECT COUNT(*) FROM topics WHERE parent_topic_id = ?1",
            params![source_topic_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let branch_name = format!("{} - 分支 {}", source_topic_name, sibling_count + 1);

    let new_topic = Topic {
        id: new_topic_id.clone(),
        name: branch_name,
        history: history.clone(),
        summary: None,
        renamed: false,
        branched_from_message_id: Some(source_message_id.clone()),
        parent_topic_id: Some(source_topic_id.clone()),
    };

    // 4. Insert the new topic
    conn.execute(
        "INSERT INTO topics (id, assistant_id, name, summary, renamed, branched_from_message_id, parent_topic_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            new_topic_id,
            assistant_id,
            new_topic.name,
            new_topic.summary,
            new_topic.renamed as i64,
            new_topic.branched_from_message_id,
            new_topic.parent_topic_id,
        ],
    )
    .map_err(|e| ierr!("创建分支话题失败: {}", "Failed to create branch topic: {}", e))?;

    // 5. Insert the copied messages
    for msg in &history {
        let msg_id = uuid::Uuid::new_v4().to_string();
        let files_json = serde_json::to_string(&msg.display_files).ok();
        let content_json = serde_json::to_string(&msg.content).unwrap_or_default();
        let tool_calls_json = serde_json::to_string(&msg.tool_calls).ok();
        conn.execute(
            "INSERT INTO messages (id, topic_id, role, content, model_id, display_files, display_text, reasoning,
             tool_call_id, name, tool_calls_json, input_tokens, output_tokens,
             agent_steps_json, interim_content, agent_start_time, parent_message_id, branch_index, cached_input_tokens)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                msg_id,
                new_topic_id,
                msg.role,
                content_json,
                msg.model_id,
                files_json,
                msg.display_text,
                msg.reasoning,
                msg.tool_call_id,
                msg.name,
                tool_calls_json,
                msg.input_tokens,
                msg.output_tokens,
                serde_json::to_string(&msg.agent_steps).ok(),
                msg.interim_content,
                msg.agent_start_time,
                msg.parent_message_id,
                msg.branch_index,
                msg.cached_input_tokens,
            ],
        )
        .map_err(|e| ierr!("插入分支消息失败: {}", "Failed to insert branch messages: {}", e))?;
    }

    Ok(new_topic)
}

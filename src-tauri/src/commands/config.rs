use crate::commands::attachment::{
    cleanup_attachment_ids, load_message_attachments_batch, sync_message_attachments,
};
use crate::core::models::*;
use crate::core::secure_store;
use crate::core::state::DbState;
use base64::{engine::general_purpose, Engine as _};
use rusqlite::params;
use std::fs; // 导入标准库文件系统模块
use tauri::{AppHandle, Manager};

/// 应用配置文件持久化结构：api_key 不入库，统一存到系统钥匙串
#[derive(serde::Serialize, serde::Deserialize)]
struct AppConfigDisk {
    api_url: String,
    default_model: String,
    local_model_path: String,
    #[serde(default, rename = "knowledgeEnabled")]
    knowledge_enabled: bool,
    #[serde(default, rename = "autoStartEnabled")]
    auto_start_enabled: bool,
    #[serde(default = "default_max_tool_rounds", rename = "maxToolRounds")]
    max_tool_rounds: u32,
    #[serde(
        default = "default_max_concurrent_subagents",
        rename = "maxConcurrentSubagents"
    )]
    max_concurrent_subagents: u32,
}

fn default_max_tool_rounds() -> u32 {
    crate::core::models::DEFAULT_MAX_TOOL_ROUNDS
}

fn default_max_concurrent_subagents() -> u32 {
    5
}

/// 保存应用程序通用配置
/// #[tauri::command] 标记允许此函数从前端通过 invoke 调用
#[tauri::command]
pub fn save_app_config(app: AppHandle, config: AppConfig) -> Result<(), String> {
    // api_key 走系统钥匙串（keyring），落盘仅写其他字段
    if !config.api_key.is_empty() {
        secure_store::set(&app, secure_store::accounts::APP_API_KEY, &config.api_key)
            .map_err(|e| e.to_string())?;
    } else {
        let _ = secure_store::delete(&app, secure_store::accounts::APP_API_KEY);
    }

    // 1. 获取操作系统的用户配置目录 (如 Windows 的 AppData/Roaming 或 Linux 的 ~/.config)
    let mut path = dirs::config_dir().ok_or_else(|| "无法获取系统配置目录".to_string())?;

    // 2. 在配置目录下创建 "AIO" 文件夹
    path.push("com.loch.aio");
    if !path.exists() {
        fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    }

    // 3. 指定配置文件名为 config.json
    path.push("config.json");

    let disk = AppConfigDisk {
        api_url: config.api_url,
        default_model: config.default_model,
        local_model_path: config.local_model_path,
        knowledge_enabled: config.knowledge_enabled,
        auto_start_enabled: config.auto_start_enabled,
        max_tool_rounds: config
            .max_tool_rounds
            .unwrap_or(crate::core::models::DEFAULT_MAX_TOOL_ROUNDS),
        max_concurrent_subagents: config.max_concurrent_subagents.unwrap_or(5),
    };
    let json = serde_json::to_string_pretty(&disk).map_err(|e| e.to_string())?;
    // 原子写入
    let tmp_path = path.with_extension("tmp");
    use std::io::Write;
    let mut f = std::fs::File::create(&tmp_path).map_err(|e| e.to_string())?;
    f.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    std::fs::rename(&tmp_path, &path).map_err(|e| e.to_string())?;
    Ok(())
}

/// 读取应用程序通用配置
#[tauri::command]
pub fn load_app_config(app: AppHandle) -> Result<AppConfig, String> {
    let mut path = dirs::config_dir().ok_or("无法获取配置目录")?;
    path.push("com.loch.aio/config.json");

    // 优先尝试 v2 schema（不含 api_key 字段）
    if path.exists() {
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(disk) = serde_json::from_str::<AppConfigDisk>(&content) {
                let api_key = secure_store::get(&app, secure_store::accounts::APP_API_KEY)
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default();
                return Ok(AppConfig {
                    api_url: disk.api_url,
                    api_key,
                    default_model: disk.default_model,
                    local_model_path: disk.local_model_path,
                    auto_retry_enabled: true,
                    auto_retry_count: 2,
                    auto_retry_delay_ms: 500,
                    knowledge_enabled: disk.knowledge_enabled,
                    auto_start_enabled: disk.auto_start_enabled,
                    max_concurrent_subagents: Some(disk.max_concurrent_subagents),
                    max_tool_rounds: Some(disk.max_tool_rounds),
                });
            }
            // 兼容旧 schema（含明文 api_key）：读出后迁出到 keyring
            if let Ok(legacy) = serde_json::from_str::<AppConfig>(&content) {
                if !legacy.api_key.is_empty() {
                    let _ = secure_store::set(
                        &app,
                        secure_store::accounts::APP_API_KEY,
                        &legacy.api_key,
                    );
                }
                let mut disk = AppConfigDisk {
                    api_url: legacy.api_url.clone(),
                    default_model: legacy.default_model.clone(),
                    local_model_path: legacy.local_model_path.clone(),
                    knowledge_enabled: false,
                    auto_start_enabled: false,
                    max_tool_rounds: crate::core::models::DEFAULT_MAX_TOOL_ROUNDS,
                    max_concurrent_subagents: 5,
                };
                disk.api_url = legacy.api_url;
                disk.default_model = legacy.default_model;
                disk.local_model_path = legacy.local_model_path;
                let _ = fs::write(
                    &path,
                    serde_json::to_string_pretty(&disk).unwrap_or_default(),
                );
                return Ok(AppConfig {
                    api_url: disk.api_url,
                    api_key: legacy.api_key,
                    default_model: disk.default_model,
                    local_model_path: disk.local_model_path,
                    auto_retry_enabled: true,
                    auto_retry_count: 2,
                    auto_retry_delay_ms: 500,
                    knowledge_enabled: false,
                    auto_start_enabled: false,
                    max_concurrent_subagents: Some(disk.max_concurrent_subagents),
                    max_tool_rounds: Some(disk.max_tool_rounds),
                });
            }
        }
    }

    Ok(AppConfig {
        api_url: "".into(),
        api_key: "".into(),
        default_model: "".into(),
        local_model_path: "".into(),
        auto_retry_enabled: true,
        auto_retry_count: 2,
        auto_retry_delay_ms: 500,
        knowledge_enabled: false,
        auto_start_enabled: false,
        max_concurrent_subagents: None,
        max_tool_rounds: None,
    })
}

/// 异步加载所有已保存的 AI 助手配置
#[tauri::command]
pub async fn load_assistants(state: tauri::State<'_, DbState>) -> Result<Vec<Assistant>, String> {
    let conn = state.0.lock();

    // 1. 加载助手
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
            let agent_mode = agent_mode_str
                .and_then(|s| serde_json::from_str(&format!("\"{}\"", s)).ok())
                .unwrap_or(crate::core::models::AgentMode::Off);
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

        // 2. 为每个助手加载话题
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

            // 3. 加载历史消息（含 tool_call_id / name / tool_calls_json / input_tokens / output_tokens，支持跨重启续接工具调用会话及 token 统计）
            let mut m_stmt = conn.prepare("SELECT id, role, content, model_id, display_files, display_text, reasoning, tool_call_id, name, tool_calls_json, input_tokens, output_tokens, agent_steps_json, interim_content, agent_start_time, parent_message_id, branch_index, images_json FROM messages WHERE topic_id = ? ORDER BY timestamp ASC")
    .map_err(|e| e.to_string())?;

            let msg_iter = m_stmt
                .query_map([&topic.id], |row| {
                    // 提取 display_files (在 index 4)
                    let display_files_json: Option<String> = row.get(4)?;
                    let display_files =
                        display_files_json.and_then(|s| serde_json::from_str(&s).ok());

                    // 提取 content (在 index 2)
                    let content_json: String = row.get(2)?;
                    let content_value = serde_json::from_str(&content_json)
                        .unwrap_or(serde_json::Value::String(content_json));

                    // 提取 tool_calls_json (在 index 9)
                    let tool_calls_json: Option<String> = row.get(9)?;
                    let tool_calls = tool_calls_json
                        .and_then(|s| serde_json::from_str::<Vec<ToolCall>>(&s).ok());

                    Ok(Message {
                        id: row.get(0)?,             // index 0: id
                        role: row.get(1)?,           // index 1: role
                        content: content_value,      // index 2: content (JSON)
                        model_id: row.get(3)?,       // index 3: model_id
                        display_files,               // 已经解析好的 files
                        display_text: row.get(5)?,   // index 5: display_text
                        tool_call_id: row.get(7)?,   // index 7: tool_call_id
                        name: row.get(8)?,           // index 8: name
                        tool_calls,                  // index 9: tool_calls_json（已解析）
                        reasoning: row.get(6)?,      // index 6: reasoning
                        input_tokens: row.get(10)?,  // index 10: input_tokens
                        output_tokens: row.get(11)?, // index 11: output_tokens
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
                        full_tool_result: None,        // 会话级内存字段，不持久化
                    })
                })
                .map_err(|e| e.to_string())?;

            // 批量加载附件：先收集所有消息 ID，再一次性查询（修复 N+1 查询）
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

/// 同步执行助手持久化（在 spawn_blocking 中调用，避免阻塞异步运行时 —— PERF-05）。
///
/// 外层包一个 SQLite 事务（PERF-03）：避免逐条自动提交的大量 fsync，随历史增长显著降本。
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

/// persist_assistant 的事务内主体。
fn persist_assistant_inner(
    conn: &rusqlite::Connection,
    assistant: &Assistant,
) -> Result<(), String> {
    // 1. 保存/更新助手基本信息
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

    // 2. 【核心修复】清理已被前端删除的话题 (解决死而复生问题)
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
            conn.execute("DELETE FROM topics WHERE id = ?", params![db_id])
                .map_err(|e| e.to_string())?;
            cleanup_attachment_ids(conn, &attachment_ids)?;
        }
    }

    // 3. 遍历话题执行增量同步
    for topic in &assistant.topics {
        conn.execute(
            "INSERT INTO topics (id, assistant_id, name, summary, renamed, branched_from_message_id, parent_topic_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET name=?3, summary=?4, renamed=?5, branched_from_message_id=?6, parent_topic_id=?7",
            params![topic.id, assistant.id, topic.name, topic.summary, topic.renamed as i64, topic.branched_from_message_id, topic.parent_topic_id],
        )
        .map_err(|e| e.to_string())?;

        // 4. 【性能优化重点】增量同步消息
        // 不再 DELETE ALL，而是使用 ON CONFLICT DO NOTHING (如果 ID 存在则跳过，不存在则插入)
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
            // 假设 Message 结构体现在也有了 id 字段
            let msg_id = msg
                .id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let files_json = serde_json::to_string(&msg.display_files).ok();
            let content_json = serde_json::to_string(&msg.content).unwrap_or_default();
            let tool_calls_json = serde_json::to_string(&msg.tool_calls).ok();

            // 写入 tool_call_id / name / tool_calls_json / input_tokens / output_tokens，支持跨重启续接工具调用会话及 token 统计。
            // 用 ON CONFLICT(id) DO UPDATE 覆盖更新（旧实现 DO NOTHING 会导致再次保存不更新内容）。
            conn.execute(
                "INSERT INTO messages (id, topic_id, role, content, model_id, display_files, display_text, reasoning, tool_call_id, name, tool_calls_json, input_tokens, output_tokens, agent_steps_json, interim_content, agent_start_time, parent_message_id, branch_index, images_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
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
                   agent_steps_json = excluded.agent_steps_json,
                   interim_content = excluded.interim_content,
                   agent_start_time = excluded.agent_start_time,
                   parent_message_id = excluded.parent_message_id,
                   branch_index = excluded.branch_index,
                   images_json = excluded.images_json",
                params![msg_id, topic.id, msg.role, content_json, msg.model_id, files_json, msg.display_text, msg.reasoning, msg.tool_call_id, msg.name, tool_calls_json, msg.input_tokens, msg.output_tokens, serde_json::to_string(&msg.agent_steps).ok(), msg.interim_content, msg.agent_start_time, msg.parent_message_id, msg.branch_index, serde_json::to_string(&msg.images).ok()],
            ).map_err(|e| e.to_string())?;
            sync_message_attachments(conn, &msg_id, msg.display_files.as_ref())?;
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn save_assistant(app: tauri::AppHandle, assistant: Assistant) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let db_state = app.state::<DbState>();
        let conn = db_state.0.lock();
        persist_assistant(&conn, &assistant)
    })
    .await
    .map_err(|e| format!("保存助手线程失败: {e}"))?
}

#[tauri::command]
pub async fn delete_assistant(state: tauri::State<'_, DbState>, id: String) -> Result<(), String> {
    let conn = state.0.lock();
    let attachment_ids = attachment_ids_for_assistant(&conn, &id)?;
    // 由于设置了 ON DELETE CASCADE，会自动删除关联的话题和消息
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

/// 保存"已激活模型"列表（api_key 剥离到系统 keyring，落盘不含明文密钥）
#[tauri::command]
pub fn save_activated_models(app: AppHandle, models: Vec<ActivatedModel>) -> Result<(), String> {
    let mut path = dirs::config_dir().unwrap();
    path.push("com.loch.aio");
    if !path.exists() {
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    }
    path.push("activated_models.json");

    // 剥离 api_key 到 secure_store，落盘模型不含明文密钥
    let mut sanitized: Vec<serde_json::Value> = Vec::new();
    for m in &models {
        if !m.api_key.is_empty() && m.api_key != "local-no-key" {
            let account = secure_store::accounts::activated_model_key(&m.api_url, &m.model_id);
            let _ = secure_store::set(&app, &account, &m.api_key);
        }
        // 序列化为 JSON Value，移除 api_key 字段
        let mut v = serde_json::to_value(m).map_err(|e| e.to_string())?;
        if let Some(obj) = v.as_object_mut() {
            obj.remove("api_key");
        }
        sanitized.push(v);
    }

    let json = serde_json::to_string_pretty(&sanitized).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}

/// 加载"已激活模型"列表（从 keyring 恢复 api_key）
#[tauri::command]
pub fn load_activated_models(app: AppHandle) -> Result<Vec<ActivatedModel>, String> {
    let mut path = dirs::config_dir().unwrap();
    path.push("com.loch.aio");
    path.push("activated_models.json");

    if !path.exists() {
        return Ok(vec![]);
    }
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut models: Vec<ActivatedModel> =
        serde_json::from_str(&content).map_err(|e| e.to_string())?;

    // 从 secure_store 恢复 api_key，并自动迁移旧数据（含明文 api_key 的旧文件）
    let mut needs_migration = false;
    for m in &mut models {
        // 检查是否为旧格式（JSON 中直接含有 api_key）
        if !m.api_key.is_empty() && m.api_key != "local-no-key" {
            let account = secure_store::accounts::activated_model_key(&m.api_url, &m.model_id);
            let _ = secure_store::set(&app, &account, &m.api_key);
            needs_migration = true;
        }

        // 从 secure_store 恢复（覆盖旧 JSON 中的值，或补充新格式缺失的 key）
        let account = secure_store::accounts::activated_model_key(&m.api_url, &m.model_id);
        if let Ok(Some(key)) = secure_store::get(&app, &account) {
            m.api_key = key;
        }
    }

    // 迁移：立即写回清理后的 JSON（剔除旧格式的明文 api_key）
    if needs_migration {
        let mut sanitized: Vec<serde_json::Value> = Vec::new();
        for m in &models {
            let mut v = serde_json::to_value(m).map_err(|e| e.to_string())?;
            if let Some(obj) = v.as_object_mut() {
                obj.remove("api_key");
            }
            sanitized.push(v);
        }
        if let Ok(json) = serde_json::to_string_pretty(&sanitized) {
            let _ = std::fs::write(&path, json);
        }
    }

    Ok(models)
}

/// 保存从云端或 API 获取的模型原始信息列表
#[tauri::command]
pub fn save_fetched_models(models: Vec<ModelInfo>) -> Result<(), String> {
    let mut path = dirs::config_dir().unwrap();
    path.push("com.loch.aio");
    if !path.exists() {
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    }
    path.push("fetched_models.json");
    let json = serde_json::to_string_pretty(&models).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}

/// 加载之前获取过的模型信息列表
#[tauri::command]
pub fn load_fetched_models() -> Result<Vec<ModelInfo>, String> {
    let mut path = dirs::config_dir().unwrap();
    path.push("com.loch.aio");
    path.push("fetched_models.json");

    if !path.exists() {
        return Ok(vec![]);
    }
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let models: Vec<ModelInfo> = serde_json::from_str(&content).map_err(|e| e.to_string())?;
    Ok(models)
}

#[tauri::command]
pub async fn upload_avatar(app: tauri::AppHandle, data_url: String) -> Result<String, String> {
    // M4 防护：data URL 字符串本身有上限 (Base64 编码后体积膨胀 ~33%)
    // 256x256 JPEG 0.8 质量通常 < 50KB，10MB 字符串已远超实际需要
    const MAX_DATA_URL_LEN: usize = 10 * 1024 * 1024;
    if data_url.len() > MAX_DATA_URL_LEN {
        return Err(format!(
            "头像数据过大 ({} 字节，上限 {} 字节)",
            data_url.len(),
            MAX_DATA_URL_LEN
        ));
    }

    let app_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let avatars_dir = app_dir.join("avatars");

    // 1. 确保目录存在
    if !avatars_dir.exists() {
        std::fs::create_dir_all(&avatars_dir).map_err(|e| e.to_string())?;
    } else {
        // --- 核心修复：删除所有旧的 user_avatar 缓存 ---
        // 我们只删除以此前缀开头的文件，避免误删目录下可能存在的其它资源
        if let Ok(entries) = std::fs::read_dir(&avatars_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                        if file_name.starts_with("user_avatar_") {
                            let _ = std::fs::remove_file(path);
                        }
                    }
                }
            }
        }
    }

    // 2. 解析 Base64 数据
    let base64_str = data_url.split(',').nth(1).ok_or("无效的图像数据")?;
    let bytes = general_purpose::STANDARD
        .decode(base64_str)
        .map_err(|e| e.to_string())?;

    // 校验解码后大小（5MB 图像上限）
    if bytes.len() > 5 * 1024 * 1024 {
        return Err(format!("解码后图像过大 ({} 字节)", bytes.len()));
    }

    // 3. 生成新文件名 (保留 UUID 依然是必要的，可以让前端识别到路径变化从而刷新图片)
    let file_name = format!("user_avatar_{}.png", uuid::Uuid::new_v4());
    let dest_path = avatars_dir.join(&file_name);

    std::fs::write(&dest_path, bytes).map_err(|e| e.to_string())?;

    // 返回新路径供前端更新 localStorage
    Ok(dest_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn clear_local_avatar_cache(app: tauri::AppHandle) -> Result<(), String> {
    let app_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let avatars_dir = app_dir.join("avatars");

    if avatars_dir.exists() {
        // 直接删除整个文件夹并重建，或者遍历删除
        let _ = std::fs::remove_dir_all(&avatars_dir);
        std::fs::create_dir_all(&avatars_dir).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 读取用户通过文件选择器选中的头像原始字节（10MB 上限，绕过 fs:allow-read-file ** 需求）
#[tauri::command]
pub async fn read_avatar_source(path: String) -> Result<String, String> {
    use std::io::Read;
    const MAX_BYTES: u64 = 10 * 1024 * 1024;
    let p = std::path::Path::new(&path);
    if !p.exists() {
        return Err("文件不存在".into());
    }
    let meta = std::fs::metadata(p).map_err(|e| e.to_string())?;
    if meta.len() > MAX_BYTES {
        return Err(format!("文件过大 (上限 {}MB)", MAX_BYTES / 1024 / 1024));
    }
    // 校验扩展名
    let ext = p
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !["png", "jpg", "jpeg", "webp", "bmp", "gif"].contains(&ext.as_str()) {
        return Err("仅支持 png/jpg/jpeg/webp/bmp/gif 图像".into());
    }
    let mut file = std::fs::File::open(p).map_err(|e| e.to_string())?;
    let mut buf = Vec::with_capacity(meta.len() as usize);
    file.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let mime = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "gif" => "image/gif",
        _ => "application/octet-stream",
    };
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
    Ok(format!("data:{};base64,{}", mime, b64))
}

// ====== Per-Profile Model Override Persistence ======

///
/// 从磁盘加载 profile-model-overrides.json 并返回覆盖配置列表。
/// 文件不存在或损坏时返回空 vec（不视为错误，所有 profile 回退到父模型）。
#[tauri::command]
pub fn load_profile_model_overrides() -> Result<Vec<ProfileModelOverride>, String> {
    let mut path = dirs::config_dir().ok_or("无法获取系统配置目录")?;
    path.push("com.loch.aio");
    path.push("profile-model-overrides.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path)
        .map_err(|e| format!("读取 profile-model-overrides.json 失败: {}", e))?;
    let file: ProfileModelOverridesFile = serde_json::from_str(&raw)
        .map_err(|e| format!("解析 profile-model-overrides.json 失败: {}", e))?;
    Ok(file.overrides)
}

///
/// 将子智能体 profile 的模型覆盖配置持久化到磁盘。
#[tauri::command]
pub fn save_profile_model_overrides(overrides: Vec<ProfileModelOverride>) -> Result<(), String> {
    let mut path = dirs::config_dir().ok_or("无法获取系统配置目录")?;
    path.push("com.loch.aio");
    if !path.exists() {
        fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    }
    path.push("profile-model-overrides.json");
    let file = ProfileModelOverridesFile {
        version: 1,
        updated_at: {
            use std::time::SystemTime;
            let dur = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default();
            let secs = dur.as_secs();
            let tm = secs_to_date_parts(secs);
            format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
                tm.0, tm.1, tm.2, tm.3, tm.4, tm.5
            )
        },
        overrides,
    };
    let json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}

/// Basic epoch-to-(year,month,day,hour,min,sec) conversion.
/// Approximate — good enough for file metadata.
fn secs_to_date_parts(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    const SECS_PER_DAY: u64 = 86400;
    let days = secs / SECS_PER_DAY;
    let time_secs = secs % SECS_PER_DAY;
    let hours = time_secs / 3600;
    let mins = (time_secs % 3600) / 60;
    let s = time_secs % 60;
    // Simple Gregorian: days since 1970-01-01
    let mut y = 1970u64;
    let mut remaining = days;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }
    let month_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 1u64;
    for &md in &month_days {
        if remaining < md {
            break;
        }
        remaining -= md;
        m += 1;
    }
    (y, m, remaining + 1, hours, mins, s)
}

fn is_leap(y: u64) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

// ====== Custom Subagent Profiles (CRUD) ======
use crate::core::models::CustomSubagentProfile;

use crate::core::models::CustomSubagentProfilesFile;

fn custom_profiles_path() -> Result<std::path::PathBuf, String> {
    let mut path = dirs::config_dir().ok_or("无法获取系统配置目录")?;
    path.push("com.loch.aio");
    path.push("custom-subagent-profiles.json");
    Ok(path)
}

///
/// 从磁盘加载所有自定义子智能体配置文件。
/// 文件不存在或损坏时返回空 vec（不视为错误，仅无自定义配置）。
#[tauri::command]
pub fn list_custom_subagent_profiles() -> Result<Vec<CustomSubagentProfile>, String> {
    let path = custom_profiles_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path)
        .map_err(|e| format!("读取 custom-subagent-profiles.json 失败: {}", e))?;
    let file: CustomSubagentProfilesFile = serde_json::from_str(&raw)
        .map_err(|e| format!("解析 custom-subagent-profiles.json 失败: {}", e))?;
    Ok(file.profiles)
}

///
/// 保存（插入或更新）一个自定义子智能体配置文件。
/// 按 id 去重：同名 id 覆盖，否则追加。
#[tauri::command]
pub fn save_custom_subagent_profile(profile: CustomSubagentProfile) -> Result<(), String> {
    let path = custom_profiles_path()?;
    let mut profiles: Vec<CustomSubagentProfile> = if path.exists() {
        let raw = fs::read_to_string(&path)
            .map_err(|e| format!("读取 custom-subagent-profiles.json 失败: {}", e))?;
        let file: CustomSubagentProfilesFile = serde_json::from_str(&raw)
            .map_err(|e| format!("解析 custom-subagent-profiles.json 失败: {}", e))?;
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
        updated_at: {
            use std::time::SystemTime;
            let dur = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default();
            let secs = dur.as_secs();
            let tm = secs_to_date_parts(secs);
            format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
                tm.0, tm.1, tm.2, tm.3, tm.4, tm.5
            )
        },
        profiles,
    };
    let json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}

///
/// 按 id 删除一个自定义子智能体配置文件。
#[tauri::command]
pub fn delete_custom_subagent_profile(profile_id: String) -> Result<(), String> {
    let path = custom_profiles_path()?;
    if !path.exists() {
        return Ok(());
    }
    let raw = fs::read_to_string(&path)
        .map_err(|e| format!("读取 custom-subagent-profiles.json 失败: {}", e))?;
    let file: CustomSubagentProfilesFile = serde_json::from_str(&raw)
        .map_err(|e| format!("解析 custom-subagent-profiles.json 失败: {}", e))?;
    let profiles: Vec<CustomSubagentProfile> = file
        .profiles
        .into_iter()
        .filter(|p| p.id != profile_id)
        .collect();
    // Write back
    let updated = CustomSubagentProfilesFile {
        version: 1,
        updated_at: {
            use std::time::SystemTime;
            let dur = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default();
            let secs = dur.as_secs();
            let tm = secs_to_date_parts(secs);
            format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
                tm.0, tm.1, tm.2, tm.3, tm.4, tm.5
            )
        },
        profiles,
    };
    let json = serde_json::to_string_pretty(&updated).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}

// ====== 系统自启 ======

/// 启用或禁用系统自启。
/// - Windows: 注册表 HKCU\Software\Microsoft\Windows\CurrentVersion\Run
/// - macOS: ~/Library/LaunchAgents/com.loch.aio.plist
/// - Linux: ~/.config/autostart/aio.desktop
#[tauri::command]
pub fn set_auto_start(app: AppHandle, enabled: bool) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let app_name = "AIO";
    // 获取当前可执行文件路径
    let exe_path = std::env::current_exe().map_err(|e| format!("无法获取可执行文件路径: {}", e))?;
    let exe_str = exe_path.to_string_lossy().to_string();

    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        if enabled {
            let output = Command::new("reg")
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
                .map_err(|e| format!("执行注册表写入失败: {}", e))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!("注册表写入失败: {}", stderr));
            }
        } else {
            let output = Command::new("reg")
                .args([
                    "delete",
                    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                    "/v",
                    app_name,
                    "/f",
                ])
                .output()
                .map_err(|e| format!("执行注册表删除失败: {}", e))?;
            // 删除不存在的键不算错误（reg delete /f 对不存在的键仍返回成功）
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                // reg delete: "错误: 系统找不到指定的注册表项或值" 视为成功
                if !stderr.contains("找不到") {
                    return Err(format!("注册表删除失败: {}", stderr));
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let plist_dir = dirs::home_dir()
            .ok_or_else(|| "无法获取用户主目录".to_string())?
            .join("Library/LaunchAgents");
        fs::create_dir_all(&plist_dir).map_err(|e| e.to_string())?;
        let plist_path = plist_dir.join("com.loch.aio.plist");
        if enabled {
            let plist = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.loch.aio</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>"#,
                exe_str
            );
            fs::write(&plist_path, plist).map_err(|e| format!("写入 LaunchAgent 失败: {}", e))?;
        } else {
            if plist_path.exists() {
                fs::remove_file(&plist_path)
                    .map_err(|e| format!("删除 LaunchAgent 失败: {}", e))?;
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let autostart_dir = dirs::home_dir()
            .ok_or_else(|| "无法获取用户主目录".to_string())?
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
            fs::write(&desktop_path, desktop).map_err(|e| format!("写入 autostart 失败: {}", e))?;
        } else {
            if desktop_path.exists() {
                fs::remove_file(&desktop_path)
                    .map_err(|e| format!("删除 autostart 失败: {}", e))?;
            }
        }
    }

    // 持久化状态
    let mut config = load_app_config(app.clone())?;
    config.auto_start_enabled = enabled;
    save_app_config(app, config)
}

/// 查询系统自启是否已开启。
#[tauri::command]
pub fn is_auto_start_enabled(app: AppHandle) -> Result<bool, String> {
    let config = load_app_config(app)?;
    Ok(config.auto_start_enabled)
}

// ====== 会话分支 ======

/// 从指定消息处分叉出新话题。
/// - 将源话题中该消息及之前的所有消息复制到新话题
/// - 新话题的 `branched_from_message_id` 设为源消息 ID
/// - 新话题的 `parent_topic_id` 设为源话题 ID
/// 返回新话题对象。
#[tauri::command]
pub async fn branch_topic(
    state: tauri::State<'_, DbState>,
    source_topic_id: String,
    source_message_id: String,
) -> Result<Topic, String> {
    let conn = state.0.lock();

    // 1. 查询源话题的 assistant_id
    let assistant_id: String = conn
        .query_row(
            "SELECT assistant_id FROM topics WHERE id = ?1",
            params![source_topic_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("查询源话题失败: {}", e))?;

    // 2. 读取源话题的所有消息，截取到分支点（含）
    let mut m_stmt = conn
        .prepare(
            "SELECT id, role, content, model_id, display_files, display_text, reasoning,
             tool_call_id, name, tool_calls_json, input_tokens, output_tokens,
             agent_steps_json, interim_content, agent_start_time, parent_message_id, branch_index,
             images_json
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

    // 截取到分支点（含）
    let branch_pos = all_msgs
        .iter()
        .position(|m| m.id.as_deref() == Some(&source_message_id));
    let history: Vec<Message> = match branch_pos {
        Some(pos) => all_msgs.into_iter().take(pos + 1).collect(),
        None => {
            return Err(format!(
                "消息 {} 不在话题 {} 中",
                source_message_id, source_topic_id
            ))
        }
    };

    // 3. 创建新话题
    let new_topic_id = uuid::Uuid::new_v4().to_string();
    let source_topic_name: String = conn
        .query_row(
            "SELECT name FROM topics WHERE id = ?1",
            params![source_topic_id],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "分支".into());

    // 统计已有子话题数量（同一 parent_topic_id），自动编号
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

    // 4. 插入新话题
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
    .map_err(|e| format!("创建分支话题失败: {}", e))?;

    // 5. 插入复制的消息
    for msg in &history {
        let msg_id = uuid::Uuid::new_v4().to_string();
        let files_json = serde_json::to_string(&msg.display_files).ok();
        let content_json = serde_json::to_string(&msg.content).unwrap_or_default();
        let tool_calls_json = serde_json::to_string(&msg.tool_calls).ok();
        conn.execute(
            "INSERT INTO messages (id, topic_id, role, content, model_id, display_files, display_text, reasoning,
             tool_call_id, name, tool_calls_json, input_tokens, output_tokens,
             agent_steps_json, interim_content, agent_start_time, parent_message_id, branch_index)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
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
            ],
        )
        .map_err(|e| format!("插入分支消息失败: {}", e))?;
    }

    Ok(new_topic)
}

use crate::models::*; // 导入模型定义，如 AppConfig, Assistant, ActivatedModel 等
use base64::{engine::general_purpose, Engine as _};
use std::fs; // 导入标准库文件系统模块
use tauri::Manager;
use crate::DbState;
use rusqlite::params;

/// 保存应用程序通用配置
/// #[tauri::command] 标记允许此函数从前端通过 invoke 调用
#[tauri::command]
pub fn save_app_config(config: AppConfig) -> Result<(), String> {
    // 1. 获取操作系统的用户配置目录 (如 Windows 的 AppData/Roaming 或 Linux 的 ~/.config)
    let mut path = dirs::config_dir().expect("无法获取系统配置目录");

    // 2. 在配置目录下创建 "AIO" 文件夹
    path.push("com.loch.aio");
    if !path.exists() {
        fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    }

    // 3. 指定配置文件名为 config.json
    path.push("config.json");

    // 4. 将配置对象序列化为格式化后的 JSON 字符串
    let json = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;

    // 5. 写入文件
    fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}

/// 读取应用程序通用配置
#[tauri::command]
pub fn load_app_config() -> Result<AppConfig, String> {
    let mut path = dirs::config_dir().ok_or("无法获取配置目录")?;
    path.push("com.loch.aio/config.json");

    // 如果配置文件不存在，返回一个默认的空白配置
    if !path.exists() {
        return Ok(AppConfig {
            api_url: "".into(),
            api_key: "".into(),
            default_model: "".into(),
            local_model_path: "".into(),
        });
    }

    // 读取文件内容并反序列化为 AppConfig 结构体
    let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&content).map_err(|e| e.to_string())
}

/// 异步加载所有已保存的 AI 助手配置
/// 注意：此处增加了对 updated_at 和 is_deleted 字段的处理，以配合增量同步
#[tauri::command]
pub async fn load_assistants(state: tauri::State<'_, DbState>) -> Result<Vec<Assistant>, String> {
    let conn = state.0.lock().unwrap();
    
    // 1. 加载助手：只加载未删除的助手
    let mut stmt = conn.prepare("SELECT id, name, prompt FROM assistants WHERE is_deleted = 0 ORDER BY id").map_err(|e| e.to_string())?;
    let assistant_iter = stmt.query_map([], |row| {
        Ok(Assistant {
            id: row.get(0)?,
            name: row.get(1)?,
            prompt: row.get(2)?,
            topics: vec![], // 后续填充
        })
    }).map_err(|e| e.to_string())?;

    let mut assistants = Vec::new();
    for asst in assistant_iter {
        let mut asst = asst.map_err(|e| e.to_string())?;
        
        // 2. 为每个助手加载话题：增加 updated_at 和 is_deleted 的查询
        let mut t_stmt = conn.prepare("SELECT id, name, summary, updated_at, is_deleted FROM topics WHERE assistant_id = ? AND is_deleted = 0").map_err(|e| e.to_string())?;
        let topic_iter = t_stmt.query_map([&asst.id], |row| {
            Ok(Topic {
                id: row.get(0)?,
                name: row.get(1)?,
                summary: row.get(2)?,
                updated_at: row.get(3)?,
                is_deleted: row.get::<_, i32>(4)? == 1,
                history: vec![], 
            })
        }).map_err(|e| e.to_string())?;

        for topic in topic_iter {
            let mut topic = topic.map_err(|e| e.to_string())?;
            
            // 3. 加载历史消息：增加 id 和 updated_at 的查询
            let mut m_stmt = conn.prepare("SELECT id, role, content, model_id, display_files, display_text, updated_at FROM messages WHERE topic_id = ? AND is_deleted = 0 ORDER BY timestamp ASC")
                .map_err(|e| e.to_string())?;
            let msg_iter = m_stmt.query_map([&topic.id], |row| {
                let display_files_json: Option<String> = row.get(4)?;
                let display_files = display_files_json.and_then(|s| serde_json::from_str(&s).ok());
                
                Ok(Message {
                    id: row.get(0)?,
                    role: row.get(1)?,
                    content: serde_json::from_str(&row.get::<_, String>(2)?).unwrap_or(serde_json::Value::String("".into())),
                    model_id: row.get(3)?,
                    display_files,
                    display_text: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            }).map_err(|e| e.to_string())?;

            for msg in msg_iter {
                topic.history.push(msg.map_err(|e| e.to_string())?);
            }
            asst.topics.push(topic);
        }
        assistants.push(asst);
    }
    
    Ok(assistants)
}

#[tauri::command]
pub async fn save_assistant(state: tauri::State<'_, DbState>, assistant: Assistant) -> Result<(), String> {
    let conn = state.0.lock().unwrap();

    // 注意：SQLite 触发器会自动维护 updated_at，此处手动插入字段以防冲突
    conn.execute(
        "INSERT INTO assistants (id, name, prompt, is_deleted) VALUES (?1, ?2, ?3, 0)
         ON CONFLICT(id) DO UPDATE SET name=?2, prompt=?3, is_deleted=0, updated_at=CURRENT_TIMESTAMP",
        params![assistant.id, assistant.name, assistant.prompt],
    ).map_err(|e| e.to_string())?;

    for topic in assistant.topics {
        conn.execute(
            "INSERT INTO topics (id, assistant_id, name, summary, is_deleted) VALUES (?1, ?2, ?3, ?4, 0)
             ON CONFLICT(id) DO UPDATE SET name=?3, summary=?4, is_deleted=0, updated_at=CURRENT_TIMESTAMP",
            params![topic.id, assistant.id, topic.name, topic.summary],
        ).map_err(|e| e.to_string())?;

        // 消息保存：全量更新逻辑。
        // 为了支持增量同步，不建议 DELETE 物理删除，这里改为将该话题下所有不在当前列表中的消息设为逻辑删除
        let message_ids: Vec<String> = topic.history.iter().map(|m| m.id.clone()).collect();
        let id_list = message_ids.iter().map(|id| format!("'{}'", id)).collect::<Vec<_>>().join(",");
        
        let delete_sql = if id_list.is_empty() {
            format!("UPDATE messages SET is_deleted = 1, updated_at = CURRENT_TIMESTAMP WHERE topic_id = '{}'", topic.id)
        } else {
            format!("UPDATE messages SET is_deleted = 1, updated_at = CURRENT_TIMESTAMP WHERE topic_id = '{}' AND id NOT IN ({})", topic.id, id_list)
        };
        conn.execute(&delete_sql, []).map_err(|e| e.to_string())?;

        for msg in topic.history {
            let files_json = serde_json::to_string(&msg.display_files).ok();
            let content_json = serde_json::to_string(&msg.content).unwrap_or_default();
            conn.execute(
                "INSERT INTO messages (id, topic_id, role, content, model_id, display_files, display_text, is_deleted) 
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)
                 ON CONFLICT(id) DO UPDATE SET content=?4, model_id=?5, display_files=?6, display_text=?7, is_deleted=0, updated_at=CURRENT_TIMESTAMP",
                params![msg.id, topic.id, msg.role, content_json, msg.model_id, files_json, msg.display_text],
            ).map_err(|e| e.to_string())?;
        }
    }
    
    Ok(())
}

#[tauri::command]
pub async fn delete_assistant(state: tauri::State<'_, DbState>, id: String) -> Result<(), String> {
    let conn = state.0.lock().unwrap();
    // 逻辑删除主助手
    conn.execute(
        "UPDATE assistants SET is_deleted = 1, updated_at = CURRENT_TIMESTAMP WHERE id = ?", 
        params![id]
    ).map_err(|e| e.to_string())?;
    
    // 逻辑删除关联的话题
    conn.execute(
        "UPDATE topics SET is_deleted = 1, updated_at = CURRENT_TIMESTAMP WHERE assistant_id = ?", 
        params![id]
    ).map_err(|e| e.to_string())?;

    // 逻辑删除关联的消息
    conn.execute(
        "UPDATE messages SET is_deleted = 1, updated_at = CURRENT_TIMESTAMP WHERE topic_id IN (SELECT id FROM topics WHERE assistant_id = ?)", 
        params![id]
    ).map_err(|e| e.to_string())?;

    Ok(())
}

/// 保存“已激活模型”列表（用户在界面上勾选开启的模型）
#[tauri::command]
pub fn save_activated_models(models: Vec<ActivatedModel>) -> Result<(), String> {
    let mut path = dirs::config_dir().unwrap();
    path.push("com.loch.aio");
    if !path.exists() {
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    }
    path.push("activated_models.json");
    let json = serde_json::to_string_pretty(&models).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}

/// 加载“已激活模型”列表
#[tauri::command]
pub fn load_activated_models() -> Result<Vec<ActivatedModel>, String> {
    let mut path = dirs::config_dir().unwrap();
    path.push("com.loch.aio");
    path.push("activated_models.json");

    if !path.exists() {
        return Ok(vec![]); // 不存在则返回空列表
    }
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let models: Vec<ActivatedModel> = serde_json::from_str(&content).map_err(|e| e.to_string())?;
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
    let app_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let avatars_dir = app_dir.join("avatars");

    // 1. 确保目录存在
    if !avatars_dir.exists() {
        std::fs::create_dir_all(&avatars_dir).map_err(|e| e.to_string())?;
    } else {
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

    // 3. 生成新文件名
    let file_name = format!("user_avatar_{}.png", uuid::Uuid::new_v4());
    let dest_path = avatars_dir.join(&file_name);

    std::fs::write(&dest_path, bytes).map_err(|e| e.to_string())?;

    Ok(dest_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn clear_local_avatar_cache(app: tauri::AppHandle) -> Result<(), String> {
    let app_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let avatars_dir = app_dir.join("avatars");

    if avatars_dir.exists() {
        let _ = std::fs::remove_dir_all(&avatars_dir);
        std::fs::create_dir_all(&avatars_dir).map_err(|e| e.to_string())?;
    }
    Ok(())
}
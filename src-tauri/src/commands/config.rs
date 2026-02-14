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
#[tauri::command]
pub async fn load_assistants(state: tauri::State<'_, DbState>) -> Result<Vec<Assistant>, String> {
    let conn = state.0.lock().unwrap();
    
    // 1. 加载助手
    let mut stmt = conn.prepare("SELECT id, name, prompt FROM assistants ORDER BY id").map_err(|e| e.to_string())?;
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
        
        // 2. 为每个助手加载话题
        let mut t_stmt = conn.prepare("SELECT id, name, summary FROM topics WHERE assistant_id = ?").map_err(|e| e.to_string())?;
        let topic_iter = t_stmt.query_map([&asst.id], |row| {
            Ok(Topic {
                id: row.get(0)?,
                name: row.get(1)?,
                summary: row.get(2)?,
                history: vec![], // 大数据量下建议按需加载，此处暂时全量加载以兼容原有前端
            })
        }).map_err(|e| e.to_string())?;

        for topic in topic_iter {
            let mut topic = topic.map_err(|e| e.to_string())?;
            
            // 3. 加载历史消息
            let mut m_stmt = conn.prepare("SELECT role, content, model_id, display_files, display_text FROM messages WHERE topic_id = ? ORDER BY id ASC")
                .map_err(|e| e.to_string())?;
            let msg_iter = m_stmt.query_map([&topic.id], |row| {
                let display_files_json: Option<String> = row.get(3)?;
                let display_files = display_files_json.and_then(|s| serde_json::from_str(&s).ok());
                
                Ok(Message {
                    role: row.get(0)?,
                    content: serde_json::from_str(&row.get::<_, String>(1)?).unwrap_or(serde_json::Value::String("".into())),
                    model_id: row.get(2)?,
                    display_files,
                    display_text: row.get(4)?,
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

    // 使用事务保证原子性
    // 注意：这里的逻辑改为：如果已存在则更新，如果不存在则插入
    conn.execute(
        "INSERT INTO assistants (id, name, prompt) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET name=?2, prompt=?3",
        params![assistant.id, assistant.name, assistant.prompt],
    ).map_err(|e| e.to_string())?;

    for topic in assistant.topics {
        conn.execute(
            "INSERT INTO topics (id, assistant_id, name, summary) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET name=?3, summary=?4",
            params![topic.id, assistant.id, topic.name, topic.summary],
        ).map_err(|e| e.to_string())?;

        // 消息保存建议在消息产生的瞬间单独进行(见下文)，此处为兼容现有全量保存逻辑：
        // 先删除旧消息再插入新消息（效率较低，但兼容旧前端逻辑）
        conn.execute("DELETE FROM messages WHERE topic_id = ?", params![topic.id]).map_err(|e| e.to_string())?;
        for msg in topic.history {
            let files_json = serde_json::to_string(&msg.display_files).ok();
            let content_json = serde_json::to_string(&msg.content).unwrap_or_default();
            conn.execute(
                "INSERT INTO messages (topic_id, role, content, model_id, display_files, display_text) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![topic.id, msg.role, content_json, msg.model_id, files_json, msg.display_text],
            ).map_err(|e| e.to_string())?;
        }
    }
    
    Ok(())
}

#[tauri::command]
pub async fn delete_assistant(state: tauri::State<'_, DbState>, id: String) -> Result<(), String> {
    let conn = state.0.lock().unwrap();
    // 由于设置了 ON DELETE CASCADE，会自动删除关联的话题和消息
    conn.execute("DELETE FROM assistants WHERE id = ?", params![id]).map_err(|e| e.to_string())?;
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
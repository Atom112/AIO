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
    };
    let json = serde_json::to_string_pretty(&disk).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())?;
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
                });
            }
            // 兼容旧 schema（含明文 api_key）：读出后迁出到 keyring
            if let Ok(legacy) = serde_json::from_str::<AppConfig>(&content) {
                if !legacy.api_key.is_empty() {
                    let _ = secure_store::set(&app, secure_store::accounts::APP_API_KEY, &legacy.api_key);
                }
                let mut disk = AppConfigDisk {
                    api_url: legacy.api_url.clone(),
                    default_model: legacy.default_model.clone(),
                    local_model_path: legacy.local_model_path.clone(),
                };
                disk.api_url = legacy.api_url;
                disk.default_model = legacy.default_model;
                disk.local_model_path = legacy.local_model_path;
                let _ = fs::write(&path, serde_json::to_string_pretty(&disk).unwrap_or_default());
                return Ok(AppConfig {
                    api_url: disk.api_url,
                    api_key: legacy.api_key,
                    default_model: disk.default_model,
                    local_model_path: disk.local_model_path,
                });
            }
        }
    }

    Ok(AppConfig {
        api_url: "".into(),
        api_key: "".into(),
        default_model: "".into(),
        local_model_path: "".into(),
    })
}

/// 异步加载所有已保存的 AI 助手配置
#[tauri::command]
pub async fn load_assistants(state: tauri::State<'_, DbState>) -> Result<Vec<Assistant>, String> {
    let conn = state.0.lock().unwrap();

    // 1. 加载助手
    let mut stmt = conn
        .prepare("SELECT id, name, prompt FROM assistants ORDER BY id")
        .map_err(|e| e.to_string())?;
    let assistant_iter = stmt
        .query_map([], |row| {
            Ok(Assistant {
                id: row.get(0)?,
                name: row.get(1)?,
                prompt: row.get(2)?,
                topics: vec![], // 后续填充
            })
        })
        .map_err(|e| e.to_string())?;

    let mut assistants = Vec::new();
    for asst in assistant_iter {
        let mut asst = asst.map_err(|e| e.to_string())?;

        // 2. 为每个助手加载话题
        let mut t_stmt = conn
            .prepare("SELECT id, name, summary, renamed FROM topics WHERE assistant_id = ?")
            .map_err(|e| e.to_string())?;
        let topic_iter = t_stmt
            .query_map([&asst.id], |row| {
                Ok(Topic {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    summary: row.get(2)?,
                    // SQLite INTEGER (0/1) → bool
                    renamed: row.get::<_, i64>(3)? != 0,
                    history: vec![], // 大数据量下建议按需加载，此处暂时全量加载以兼容原有前端
                })
            })
            .map_err(|e| e.to_string())?;

        for topic in topic_iter {
            let mut topic = topic.map_err(|e| e.to_string())?;

            // 3. 加载历史消息
            let mut m_stmt = conn.prepare("SELECT id, role, content, model_id, display_files, display_text FROM messages WHERE topic_id = ? ORDER BY timestamp ASC")
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

                    Ok(Message {
                        id: row.get(0)?,           // index 0: id
                        role: row.get(1)?,         // index 1: role
                        content: content_value,    // index 2: content (JSON)
                        model_id: row.get(3)?,     // index 3: model_id
                        display_files,             // 已经解析好的 files
                        display_text: row.get(5)?, // index 5: display_text
                    })
                })
                .map_err(|e| e.to_string())?;

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
pub async fn save_assistant(
    state: tauri::State<'_, DbState>,
    assistant: Assistant,
) -> Result<(), String> {
    let conn = state.0.lock().unwrap();

    // 1. 保存/更新助手基本信息
    conn.execute(
        "INSERT INTO assistants (id, name, prompt) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET name=?2, prompt=?3",
        params![assistant.id, assistant.name, assistant.prompt],
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
            conn.execute("DELETE FROM topics WHERE id = ?", params![db_id])
                .map_err(|e| e.to_string())?;
        }
    }

    // 3. 遍历话题执行增量同步
    for topic in assistant.topics {
        conn.execute(
            "INSERT INTO topics (id, assistant_id, name, summary, renamed) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET name=?3, summary=?4, renamed=?5",
            params![topic.id, assistant.id, topic.name, topic.summary, topic.renamed as i64],
        )
        .map_err(|e| e.to_string())?;

        // 4. 【性能优化重点】增量同步消息
        // 不再 DELETE ALL，而是使用 ON CONFLICT DO NOTHING (如果 ID 存在则跳过，不存在则插入)
        for msg in topic.history {
            // 假设 Message 结构体现在也有了 id 字段
            let msg_id = msg
                .id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let files_json = serde_json::to_string(&msg.display_files).ok();
            let content_json = serde_json::to_string(&msg.content).unwrap_or_default();

            conn.execute(
                "INSERT INTO messages (id, topic_id, role, content, model_id, display_files, display_text) 
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO NOTHING", // 关键：已存在的 ID 不再重复写入
                params![msg_id, topic.id, msg.role, content_json, msg.model_id, files_json, msg.display_text],
            ).map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn delete_assistant(state: tauri::State<'_, DbState>, id: String) -> Result<(), String> {
    let conn = state.0.lock().unwrap();
    // 由于设置了 ON DELETE CASCADE，会自动删除关联的话题和消息
    conn.execute("DELETE FROM assistants WHERE id = ?", params![id])
        .map_err(|e| e.to_string())?;
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
    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
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

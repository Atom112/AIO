use crate::models::*; // 导入模型定义，如 AppConfig, Assistant, ActivatedModel 等
use base64::{engine::general_purpose, Engine as _};
use std::fs; // 导入标准库文件系统模块
use tauri::Manager;

/// 保存应用程序通用配置
/// #[tauri::command] 标记允许此函数从前端通过 invoke 调用
#[tauri::command]
pub fn save_app_config(config: AppConfig) -> Result<(), String> {
    // 1. 获取操作系统的用户配置目录 (如 Windows 的 AppData/Roaming 或 Linux 的 ~/.config)
    let mut path = dirs::config_dir().expect("无法获取系统配置目录");

    // 2. 在配置目录下创建 "AIO" 文件夹
    path.push("AIO");
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
    path.push("AIO/config.json");

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
pub async fn load_assistants() -> Result<Vec<Assistant>, String> {
    let mut path = dirs::config_dir().ok_or("无法获取配置目录")?;
    path.push("AIO");
    path.push("assistants"); // 助手信息存储在 AIO/assistants/ 目录下

    // 如果目录不存在则创建
    if !path.exists() {
        fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    }

    let mut assistants = Vec::new();

    // 遍历 assistants 目录下的所有文件
    for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path();

        // 只处理后缀名为 .json 的文件
        if p.extension().and_then(|s| s.to_str()) == Some("json") {
            let contents = fs::read_to_string(&p).map_err(|e| e.to_string())?;
            // 如果解析成功，则加入列表
            if let Ok(asst) = serde_json::from_str::<Assistant>(&contents) {
                assistants.push(asst);
            }
        }
    }

    // 根据 ID 对助手列表进行排序，确保前端显示顺序一致
    assistants.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(assistants)
}

/// 保存单个 AI 助手信息
#[tauri::command]
pub async fn save_assistant(assistant: Assistant) -> Result<(), String> {
    let mut path = dirs::config_dir().ok_or("无法获取目录")?;
    path.push("AIO");
    path.push("assistants");

    // 自动创建文件夹
    if !path.exists() {
        fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    }

    // 文件名使用助手的 ID (例如: assistant_id_1.json)
    path.push(format!("{}.json", assistant.id));

    let json = serde_json::to_string_pretty(&assistant).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}

/// 删除指定的 AI 助手文件
#[tauri::command]
pub async fn delete_assistant(id: String) -> Result<(), String> {
    let mut path = dirs::config_dir().ok_or("无法获取目录")?;
    path.push("AIO");
    path.push("assistants");
    path.push(format!("{}.json", id));

    // 如果文件存在，则执行删除操作
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 保存“已激活模型”列表（用户在界面上勾选开启的模型）
#[tauri::command]
pub fn save_activated_models(models: Vec<ActivatedModel>) -> Result<(), String> {
    let mut path = dirs::config_dir().unwrap();
    path.push("AIO");
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
    path.push("AIO");
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
    path.push("AIO");
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
    path.push("AIO");
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
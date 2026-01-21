// src-tauri/src/main.rs
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
struct Message {
    role: String,
    content: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct Topic {
    id: String,
    name: String,
    #[serde(default)]
    history: Vec<Message>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Assistant {
    id: String,
    name: String,
    prompt: String,
    #[serde(default)]
    topics: Vec<Topic>, // 修改：将 history 替换为 topics
}

// 获取存储助手的文件夹路径
fn get_assistants_dir() -> Result<PathBuf, String> {
    let mut path = dirs::config_dir().ok_or("无法获取配置目录")?;
    path.push("YourAppName");
    path.push("assistants"); // 存储在 assistants 子文件夹下
    if !path.exists() {
        fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    }
    Ok(path)
}

// --- 命令 ---

// 加载所有助手：遍历文件夹下的所有 .json 文件
#[tauri::command]
async fn load_assistants() -> Result<Vec<Assistant>, String> {
    let dir = get_assistants_dir()?;
    let mut assistants = Vec::new();

    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        
        // 只读取 .json 文件
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            let contents = fs::read_to_string(&path).map_err(|e| e.to_string())?;
            if let Ok(asst) = serde_json::from_str::<Assistant>(&contents) {
                assistants.push(asst);
            }
        }
    }
    // 按创建时间或 ID 排序（可选）
    assistants.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(assistants)
}

// 保存单个助手：文件名使用助手的 ID
#[tauri::command]
async fn save_assistant(assistant: Assistant) -> Result<(), String> {
    let mut path = get_assistants_dir()?;
    path.push(format!("{}.json", assistant.id)); // 文件名如: 1710500123.json

    let json = serde_json::to_string_pretty(&assistant).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}

// 删除单个助手的对应文件
#[tauri::command]
async fn delete_assistant(id: String) -> Result<(), String> {
    let mut path = get_assistants_dir()?;
    path.push(format!("{}.json", id));
    
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            load_assistants,
            save_assistant,
            delete_assistant
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
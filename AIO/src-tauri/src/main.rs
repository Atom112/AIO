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

#[tauri::command]
async fn call_llm(
    api_key: String,
    model: String,
    messages: Vec<Message>,
) -> Result<String, String> {
    let client = reqwest::Client::new();

    // --- 修改为 Aihubmix 的标准 API 地址 ---
    let url = "https://aihubmix.com/v1/chat/completions";

    let body = serde_json::json!({
        "model": model,          // 例如 "gpt-4o" 或 "deepseek-chat"
        "messages": messages,
        "stream": false
    });

    let response = client
        .post(url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("网络请求失败: {}", e))?;

    if response.status().is_success() {
        let res_json: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
        let content = res_json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("无返回内容")
            .to_string();
        Ok(content)
    } else {
        // --- 核心修复：在这里先存一下状态 ---
        let status = response.status();

        // .text() 会消耗掉 response
        let err_text = response
            .text()
            .await
            .unwrap_or_else(|_| "无法读取错误详情".to_string());

        // 这里使用之前存好的 status 变量
        Err(format!("Aihubmix 错误 ({}): {}", status, err_text))
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            load_assistants,
            save_assistant,
            delete_assistant,
            call_llm
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

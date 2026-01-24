// src-tauri/src/main.rs
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use tauri::{Emitter, Window}; // 导入 Emitter 用于发送事件 // 用于处理流

// 定义传输给前端的流式增量包
#[derive(Serialize, Clone)]
struct StreamPayload {
    assistant_id: String,
    topic_id: String,
    content: String,
    done: bool,
}
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

#[derive(Serialize, Deserialize, Clone)]
struct ModelInfo {
    id: String,
    owned_by: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
struct ModelsResponse {
    data: Vec<ModelInfo>,
}

#[derive(Serialize, Deserialize, Debug)]
struct AppConfig {
    #[serde(rename = "apiUrl")]
    api_url: String,
    #[serde(rename = "apiKey")]
    api_key: String,
    #[serde(rename = "defaultModel")]
    default_model: String,
}

// 获取配置文件路径 (和 assistants 文件夹同级)
fn get_config_path() -> PathBuf {
    let mut path = dirs::config_dir().unwrap();
    path.push("YourAppName"); // 换成你的应用名
    if !path.exists() {
        std::fs::create_dir_all(&path).unwrap();
    }
    path.push("config.json");
    path
}

#[tauri::command]
fn save_app_config(config: AppConfig) -> Result<(), String> {
    let path = get_config_path();
    let json = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn load_app_config() -> Result<AppConfig, String> {
    let path = get_config_path();
    if !path.exists() {
        // 如果文件不存在，返回默认配置
        return Ok(AppConfig {
            api_url: "".into(),
            api_key: "".into(),
            default_model: "".into(),
        });
    }
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let config: AppConfig = serde_json::from_str(&content).map_err(|e| e.to_string())?;
    Ok(config)
}

// 获取存储助手的文件夹路径
fn get_assistants_dir() -> Result<PathBuf, String> {
    let mut path = dirs::config_dir().ok_or("无法获取配置目录")?;
    path.push("AIO");
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
async fn call_llm_stream(
    window: Window,
    mut api_url: String,  // 新增参数
    api_key: String,
    model: String,
    assistant_id: String,
    topic_id: String,
    messages: Vec<Message>,
) -> Result<(), String> {

    api_url = api_url.trim_end_matches('/').to_string();
    let final_url = if !api_url.ends_with("/chat/completions") {
        format!("{}/chat/completions", api_url)
    } else {
        api_url
    };

    let client = reqwest::Client::new();
    

    let body = json!({ "model": model, "messages": messages, "stream": true });

    let response = client
        .post(&final_url) // 使用动态 URL
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let mut stream = response.bytes_stream();
    let mut line_buffer = String::new();

    while let Some(item) = stream.next().await {
        let chunk = item.map_err(|e| e.to_string())?;
        line_buffer.push_str(&String::from_utf8_lossy(&chunk));

        // 循环处理 buffer 中所有完整的行
        while let Some(pos) = line_buffer.find('\n') {
            // 提取第一行并从 buffer 中移除
            let line = line_buffer[..pos].trim().to_string();
            line_buffer.drain(..pos + 1);

            if line.is_empty() {
                continue;
            }
            if line == "data: [DONE]" {
                window
                    .emit(
                        "llm-chunk",
                        StreamPayload {
                            assistant_id: assistant_id.clone(),
                            topic_id: topic_id.clone(),
                            content: "".into(),
                            done: true,
                        },
                    )
                    .unwrap();
                return Ok(());
            }

            if line.starts_with("data: ") {
                let json_str = &line[6..];
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                    // 安全访问：判断 choices 下标是否存在 (对应 Python 的 list index 错误)
                    if let Some(content) = val["choices"][0]["delta"]["content"].as_str() {
                        // ❌ 绝对不要在这里发送全部累积的内容
                        // ✅ 只发送本次收到的内容片段
                        window
                            .emit(
                                "llm-chunk",
                                StreamPayload {
                                    assistant_id: assistant_id.clone(),
                                    topic_id: topic_id.clone(),
                                    content: content.to_string(), // 仅发送当前片段
                                    done: false,
                                },
                            )
                            .unwrap();
                    }
                }
            }
        }
    }
    Ok(())
}

#[tauri::command]
async fn fetch_models(api_url: String, api_key: String) -> Result<Vec<ModelInfo>, String> {
    // 自动处理 URL：去掉末尾斜杠，并确保指向 /models
    let mut base_url = api_url.trim_end_matches('/').to_string();
    if base_url.ends_with("/chat/completions") {
        base_url = base_url.replace("/chat/completions", "");
    }
    let final_url = format!("{}/models", base_url);

    let client = reqwest::Client::new();
    let response = client
        .get(&final_url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("服务器返回错误: {}", response.status()));
    }

    let res_data: ModelsResponse = response.json().await.map_err(|e| format!("解析失败 (请确认接口支持/v1/models): {}", e))?;
    Ok(res_data.data)
}


fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            load_assistants,
            save_assistant,
            delete_assistant,
            call_llm_stream,
            fetch_models,
            save_app_config,
            load_app_config
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

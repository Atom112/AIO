// src-tauri/src/main.rs
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
async fn call_llm_stream(
    window: Window,
    api_key: String,
    model: String,
    assistant_id: String,
    topic_id: String,
    messages: Vec<Message>,
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let url = "https://aihubmix.com/v1/chat/completions";

    let body = json!({ "model": model, "messages": messages, "stream": true });

    let response = client
        .post(url)
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

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            load_assistants,
            save_assistant,
            delete_assistant,
            call_llm_stream
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

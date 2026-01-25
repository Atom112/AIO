// src-tauri/src/lib.rs
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs::{self, File};
use std::io::{Read}; // 必须导入 Seek 才能处理 Zip
use std::path::{Path};
use tauri::{Emitter, Window};
use zip::ZipArchive;

// --- 基础数据结构 ---
#[derive(Serialize, Clone)]
struct StreamPayload {
    assistant_id: String,
    topic_id: String,
    content: String,
    done: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Message {
    role: String,
    content: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Topic {
    id: String,
    name: String,
    #[serde(default)]
    history: Vec<Message>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Assistant {
    id: String,
    name: String,
    prompt: String,
    #[serde(default)]
    topics: Vec<Topic>,
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
pub struct AppConfig {
    #[serde(rename = "apiUrl")]
    api_url: String,
    #[serde(rename = "apiKey")]
    api_key: String,
    #[serde(rename = "defaultModel")]
    default_model: String,
}

// --- 文件解析辅助函数 ---

fn extract_text_from_xml(xml: &str) -> String {
    let reader = xml::EventReader::new(xml.as_bytes());
    let mut out = String::new();
    let mut in_text_tag = false;

    for e in reader {
        match e {
            Ok(xml::reader::XmlEvent::StartElement { name, .. }) => {
                // docx 文字在 w:t, pptx 文字在 a:t
                if name.local_name == "t" {
                    in_text_tag = true;
                }
            }
            Ok(xml::reader::XmlEvent::Characters(content)) => {
                if in_text_tag {
                    out.push_str(&content);
                }
            }
            Ok(xml::reader::XmlEvent::EndElement { name, .. }) => {
                if name.local_name == "t" {
                    in_text_tag = false;
                }
            }
            _ => {}
        }
    }
    out
}

fn read_office_file(path: &str, file_type: &str) -> Result<String, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut full_text = String::new();

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = file.name().to_string();

        let is_target = if file_type == "docx" {
            name == "word/document.xml"
        } else {
            // 注意：Rust 官方方法是 ends_with (下划线)
            name.starts_with("ppt/slides/slide") && name.ends_with(".xml")
        };

        if is_target {
            let mut content = String::new();
            file.read_to_string(&mut content).map_err(|e| e.to_string())?;
            full_text.push_str(&extract_text_from_xml(&content));
            full_text.push('\n');
        }
    }
    Ok(full_text)
}

// --- Tauri Commands (核心功能) ---

#[tauri::command]
async fn process_file_content(path: String) -> Result<String, String> {
    let path_obj = Path::new(&path);
    let extension = path_obj
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();

    match extension.as_str() {
        "pdf" => {
            // pdf_extract 会返回自己的 Error 类型，需要 map_err 转为 String
            pdf_extract::extract_text(&path).map_err(|e| format!("PDF解析失败: {}", e))
        }
        "docx" | "pptx" => read_office_file(&path, &extension),
        _ => {
            // 默认按文本/代码读取
            let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
            let (res, _, _) = encoding_rs::UTF_8.decode(&bytes);
            Ok(res.into_owned())
        }
    }
}

// ... 这里保留你之前的 save_app_config, load_app_config, load_assistants 等所有命令 ...
// (为了篇幅，这里缩略，请务必保留你原来的业务命令函数)

#[tauri::command]
fn save_app_config(config: AppConfig) -> Result<(), String> {
    let mut path = dirs::config_dir().unwrap();
    path.push("AIO");
    if !path.exists() { std::fs::create_dir_all(&path).unwrap(); }
    path.push("config.json");
    let json = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn load_app_config() -> Result<AppConfig, String> {
    let mut path = dirs::config_dir().unwrap();
    path.push("AIO");
    path.push("config.json");
    if !path.exists() {
        return Ok(AppConfig { api_url: "".into(), api_key: "".into(), default_model: "".into() });
    }
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let config: AppConfig = serde_json::from_str(&content).map_err(|e| e.to_string())?;
    Ok(config)
}

#[tauri::command]
async fn load_assistants() -> Result<Vec<Assistant>, String> {
    let mut path = dirs::config_dir().ok_or("无法获取配置目录")?;
    path.push("AIO"); path.push("assistants");
    if !path.exists() { fs::create_dir_all(&path).map_err(|e| e.to_string())?; }
    let mut assistants = Vec::new();
    for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) == Some("json") {
            let contents = fs::read_to_string(&p).map_err(|e| e.to_string())?;
            if let Ok(asst) = serde_json::from_str::<Assistant>(&contents) { assistants.push(asst); }
        }
    }
    assistants.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(assistants)
}

#[tauri::command]
async fn save_assistant(assistant: Assistant) -> Result<(), String> {
    let mut path = dirs::config_dir().ok_or("无法获取目录")?;
    path.push("AIO"); path.push("assistants");
    path.push(format!("{}.json", assistant.id));
    let json = serde_json::to_string_pretty(&assistant).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn delete_assistant(id: String) -> Result<(), String> {
    let mut path = dirs::config_dir().ok_or("无法获取目录")?;
    path.push("AIO"); path.push("assistants");
    path.push(format!("{}.json", id));
    if path.exists() { fs::remove_file(path).map_err(|e| e.to_string())?; }
    Ok(())
}

#[tauri::command]
async fn fetch_models(api_url: String, api_key: String) -> Result<Vec<ModelInfo>, String> {
    let mut base_url = api_url.trim_end_matches('/').to_string();
    if base_url.ends_with("/chat/completions") {
        base_url = base_url.replace("/chat/completions", "");
    }
    let final_url = format!("{}/models", base_url);
    let client = reqwest::Client::new();
    let response = client.get(&final_url).header("Authorization", format!("Bearer {}", api_key)).send().await.map_err(|e| e.to_string())?;
    let res_data: ModelsResponse = response.json().await.map_err(|e| e.to_string())?;
    Ok(res_data.data)
}

#[tauri::command]
async fn call_llm_stream(
    window: Window,
    mut api_url: String,
    api_key: String,
    model: String,
    assistant_id: String,
    topic_id: String,
    messages: Vec<Message>,
) -> Result<(), String> {
    api_url = api_url.trim_end_matches('/').to_string();
    let final_url = if !api_url.ends_with("/chat/completions") { format!("{}/chat/completions", api_url) } else { api_url };
    let client = reqwest::Client::new();
    let body = json!({ "model": model, "messages": messages, "stream": true });
    let response = client.post(&final_url).header("Authorization", format!("Bearer {}", api_key)).json(&body).send().await.map_err(|e| e.to_string())?;
    let mut stream = response.bytes_stream();
    let mut line_buffer = String::new();
    while let Some(item) = stream.next().await {
        let chunk = item.map_err(|e| e.to_string())?;
        line_buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = line_buffer.find('\n') {
            let line = line_buffer[..pos].trim().to_string();
            line_buffer.drain(..pos + 1);
            if line.is_empty() { continue; }
            if line == "data: [DONE]" {
                window.emit("llm-chunk", StreamPayload { assistant_id: assistant_id.clone(), topic_id: topic_id.clone(), content: "".into(), done: true }).unwrap();
                return Ok(());
            }
            if line.starts_with("data: ") {
                let json_str = &line[6..];
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                    if let Some(content) = val["choices"][0]["delta"]["content"].as_str() {
                        window.emit("llm-chunk", StreamPayload { assistant_id: assistant_id.clone(), topic_id: topic_id.clone(), content: content.to_string(), done: false }).unwrap();
                    }
                }
            }
        }
    }
    Ok(())
}

// --- 应用程序入口 ---

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            load_assistants,
            save_assistant,
            delete_assistant,
            call_llm_stream,
            fetch_models,
            save_app_config,
            load_app_config,
            process_file_content // <--- 关键：确保新命令被注册
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
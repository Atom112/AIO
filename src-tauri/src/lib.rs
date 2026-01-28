// src-tauri/src/lib.rs
use dashmap::DashMap;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs::{self, File};
use std::io::Read; // 必须导入 Seek 才能处理 Zip
use std::path::Path;
use std::sync::Arc;
use tauri::{Emitter, Window};
use tokio::task::JoinHandle;
use zip::ZipArchive;

use std::os::windows::process::CommandExt; // 仅 Windows 需要，用于隐藏窗口
//use std::process::Command;
use std::sync::Mutex;
use tauri::{path::BaseDirectory, Manager};
use std::io::{BufRead, BufReader};
use tokio::time::{sleep, Duration};
use tokio::task;

// --- 基础数据结构 ---
pub struct StreamManager(pub Arc<DashMap<String, JoinHandle<()>>>);
pub struct LocalLlamaState {
    pub child_process: Mutex<Option<std::process::Child>>, // 修改类型为 std child
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ActivatedModel {
    pub api_url: String,
    pub api_key: String,
    pub model_id: String,
    pub owned_by: String,
    #[serde(skip_serializing_if = "Option::is_none")] // 只是为了 JSON 好看，可选
    pub local_path: Option<String>, 
}

#[derive(Serialize, Clone)]
struct StreamPayload {
    assistant_id: String,
    topic_id: String,
    content: String,
    done: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FileMeta {
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Message {
    role: String,
    content: String,
    #[serde(rename = "displayFiles", skip_serializing_if = "Option::is_none")]
    pub display_files: Option<Vec<FileMeta>>,
    #[serde(rename = "displayText", skip_serializing_if = "Option::is_none")]
    pub display_text: Option<String>,
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
    #[serde(rename = "localModelPath", default)] // 新增字段
    local_model_path: String,
}

#[tauri::command]
fn is_local_server_running(state: tauri::State<'_, LocalLlamaState>) -> bool {
    let mut lock = state.child_process.lock().unwrap();
    // 检查子进程是否存在且没有退出
    if let Some(child) = lock.as_mut() {
        // 3. try_wait 会返回 Result<Option<ExitStatus>>
        // Ok(None) 表示进程还在运行
        // Ok(Some(_)) 表示进程已经退出
        // Err(_) 表示查询出错（通常认为进程已失效）
        match child.try_wait() {
            Ok(None) => return true,
            _ => return false,
        }
    }
    false
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
            file.read_to_string(&mut content)
                .map_err(|e| e.to_string())?;
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
fn save_fetched_models(models: Vec<ModelInfo>) -> Result<(), String> {
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

#[tauri::command]
fn load_fetched_models() -> Result<Vec<ModelInfo>, String> {
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
fn save_app_config(config: AppConfig) -> Result<(), String> {
    let mut path = dirs::config_dir().unwrap();
    path.push("AIO");
    if !path.exists() {
        std::fs::create_dir_all(&path).unwrap();
    }
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
        return Ok(AppConfig {
            api_url: "".into(),
            api_key: "".into(),
            default_model: "".into(),
            local_model_path: "".into(),
        });
    }
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let config: AppConfig = serde_json::from_str(&content).map_err(|e| e.to_string())?;
    Ok(config)
}

#[tauri::command]
async fn load_assistants() -> Result<Vec<Assistant>, String> {
    let mut path = dirs::config_dir().ok_or("无法获取配置目录")?;
    path.push("AIO");
    path.push("assistants");
    if !path.exists() {
        fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    }
    let mut assistants = Vec::new();
    for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) == Some("json") {
            let contents = fs::read_to_string(&p).map_err(|e| e.to_string())?;
            if let Ok(asst) = serde_json::from_str::<Assistant>(&contents) {
                assistants.push(asst);
            }
        }
    }
    assistants.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(assistants)
}

#[tauri::command]
async fn save_assistant(assistant: Assistant) -> Result<(), String> {
    let mut path = dirs::config_dir().ok_or("无法获取目录")?;
    path.push("AIO");
    path.push("assistants");
    path.push(format!("{}.json", assistant.id));
    let json = serde_json::to_string_pretty(&assistant).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn delete_assistant(id: String) -> Result<(), String> {
    let mut path = dirs::config_dir().ok_or("无法获取目录")?;
    path.push("AIO");
    path.push("assistants");
    path.push(format!("{}.json", id));
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
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
    let response = client
        .get(&final_url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let res_data: ModelsResponse = response.json().await.map_err(|e| e.to_string())?;
    Ok(res_data.data)
}

#[tauri::command]
async fn call_llm_stream(
    window: Window,
    state: tauri::State<'_, StreamManager>, // 注入管理器状态
    mut api_url: String,
    api_key: String,
    model: String,
    assistant_id: String,
    topic_id: String,
    messages: Vec<Message>,
) -> Result<(), String> {
    // 1. 生成唯一任务键
    let task_key = format!("{}-{}", assistant_id, topic_id);

    // 2. 如果存在正在运行的相同任务，先终止它
    if let Some((_, old_handle)) = state.0.remove(&task_key) {
        old_handle.abort();
    }

    // 准备克隆变量用于异步块
    let state_inner = state.0.clone();
    let task_key_inner = task_key.clone();
    let assistant_id_c = assistant_id.clone();
    let topic_id_c = topic_id.clone();

    // 3. 开启后台异步任务
    let handle = tokio::spawn(async move {
        let result: Result<(), String> = async {
            api_url = api_url.trim_end_matches('/').to_string();
            let final_url = if !api_url.ends_with("/chat/completions") {
                format!("{}/chat/completions", api_url)
            } else {
                api_url
            };

            let client = reqwest::Client::new();
            let messages_for_api: Vec<serde_json::Value> = messages
                .iter()
                .map(|m| {
                    json!({
                        "role": m.role,
                        "content": m.content
                    })
                })
                .collect();
            let body = json!({
                "model": model,
                "messages": messages_for_api,
                "stream": true
            });

            let response = client
                .post(&final_url)
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

                while let Some(pos) = line_buffer.find('\n') {
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
                                    assistant_id: assistant_id_c.clone(),
                                    topic_id: topic_id_c.clone(),
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
                            if let Some(content) = val["choices"][0]["delta"]["content"].as_str() {
                                window
                                    .emit(
                                        "llm-chunk",
                                        StreamPayload {
                                            assistant_id: assistant_id_c.clone(),
                                            topic_id: topic_id_c.clone(),
                                            content: content.to_string(),
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
        .await;

        // 如果出错，通知前端结束（或者是为了让前端重置 loading 状态）
        if let Err(e) = result {
            println!("Stream Error: {}", e);
            window
                .emit(
                    "llm-chunk",
                    StreamPayload {
                        assistant_id: assistant_id_c,
                        topic_id: topic_id_c,
                        content: format!("\n[Error: {}]", e),
                        done: true,
                    },
                )
                .unwrap();
        }

        // 核心：任务执行完毕（正常结束或报错），移除 Handle
        state_inner.remove(&task_key_inner);
    });

    // 4. 将新任务句柄存入内存
    state.0.insert(task_key, handle);

    Ok(())
}

#[tauri::command]
async fn stop_llm_stream(
    state: tauri::State<'_, StreamManager>,
    assistant_id: String,
    topic_id: String,
) -> Result<(), String> {
    let task_key = format!("{}-{}", assistant_id, topic_id);
    if let Some((_, handle)) = state.0.remove(&task_key) {
        handle.abort(); // 强制停止异步任务
    }
    Ok(())
}

#[tauri::command]
fn save_activated_models(models: Vec<ActivatedModel>) -> Result<(), String> {
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

#[tauri::command]
fn load_activated_models() -> Result<Vec<ActivatedModel>, String> {
    let mut path = dirs::config_dir().unwrap();
    path.push("AIO");
    path.push("activated_models.json");
    if !path.exists() {
        return Ok(vec![]);
    }
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let models: Vec<ActivatedModel> = serde_json::from_str(&content).map_err(|e| e.to_string())?;
    Ok(models)
}

#[tauri::command]
async fn start_local_server(
    app: tauri::AppHandle,
    state: tauri::State<'_, LocalLlamaState>,
    model_path: String,
    port: u16,
    gpu_layers: i32,
) -> Result<String, String> {
    // 🔍 关键验证：确保前端传来的参数正确
    println!("[DEBUG] 启动参数 - 模型: {}, 端口: {}, GPU层数: {}", model_path, port, gpu_layers);
    
    if gpu_layers <= 0 {
        return Err("GPU 层数必须大于 0，建议设置为 99 或 999".to_string());
    }

    // 1. 停止旧服务
    stop_local_server(state.clone()).await?;
    
    // 短暂延迟确保端口释放
    sleep(Duration::from_millis(500)).await;

    // 2. 获取资源目录
    let resource_dir = app
        .path()
        .resolve("resources/llama-backend", BaseDirectory::Resource)
        .map_err(|e| format!("无法解析资源路径: {}", e))?;

    let exe_path = resource_dir.join("llama-server.exe");

    if !exe_path.exists() {
        return Err(format!("找不到执行文件: {:?}", exe_path));
    }

    // 📁 检查模型路径是否存在（llama-server 报错不明显，提前检查）
    if !std::path::Path::new(&model_path).exists() {
        return Err(format!("模型文件不存在: {}", model_path));
    }

    // 3. 构造命令
    let mut cmd = std::process::Command::new(&exe_path);
    cmd.current_dir(&resource_dir) // 关键：确保 DLL 能被找到
        .args([
            "-m", &model_path,
            "--port", &port.to_string(),
            "-ngl", &gpu_layers.to_string(),
            "-c", "4096",
            "--host", "127.0.0.1",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000);

    // 4. 启动进程
    let mut child = cmd.spawn().map_err(|e| format!("启动失败: {}", e))?;

    // 5. 🎯 关键：启动日志监控线程（用于查看 GPU 卸载状态）
    let stderr = child.stderr.take().expect("无法获取 stderr");
    task::spawn_blocking(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            if let Ok(line) = line {
                println!("[llama-server] {}", line);
                
                // 关键日志检测
                if line.contains("offloaded") {
                    println!("🎯 GPU 卸载状态: {}", line);
                }
                if line.contains("CUDA") {
                    println!("🎯 CUDA 信息: {}", line);
                }
                if line.contains("error") || line.contains("Error") || line.contains("failed") {
                    println!("❌ LLAMA 错误: {}", line);
                }
            }
        }
    });

    // 6. 等待服务初始化（使用 tokio sleep 而非 thread sleep）
    sleep(Duration::from_millis(2000)).await;
    
    // 检查进程是否还在运行
    match child.try_wait() {
        Ok(None) => println!("✅ 进程正常运行中"),
        Ok(Some(status)) => {
            return Err(format!("进程启动后立即退出，退出码: {}", status));
        }
        Err(e) => return Err(format!("无法检查进程状态: {}", e)),
    }

    // 7. 健康检查：尝试访问 /health 或 /v1/models
    let client = reqwest::Client::new();
    let health_url = format!("http://127.0.0.1:{}/health", port);
    
    match client.get(&health_url).timeout(Duration::from_secs(5)).send().await {
        Ok(_) => println!("✅ 健康检查通过"),
        Err(_) => {
            let _ = child.kill();
            return Err("服务未响应健康检查，可能启动失败".to_string());
        }
    }

    // 8. 保存句柄
    {
        let mut lock = state.child_process.lock().unwrap();
        *lock = Some(child);
    }

    Ok(format!("http://127.0.0.1:{}/v1", port))
}

#[tauri::command]
async fn stop_local_server(state: tauri::State<'_, LocalLlamaState>) -> Result<(), String> {
    let mut lock = state.child_process.lock().unwrap();
    if let Some(mut child) = lock.take() {
        // 尝试优雅关闭，如果不行就强制杀死
        let _ = child.kill();
    }
    Ok(())
}

// --- 应用程序入口 ---

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(StreamManager(Arc::new(DashMap::new())))
        .manage(LocalLlamaState {
            child_process: Mutex::new(None),
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            load_assistants,
            save_assistant,
            delete_assistant,
            call_llm_stream,
            fetch_models,
            save_app_config,
            load_app_config,
            process_file_content,
            stop_llm_stream,
            save_activated_models,
            load_activated_models,
            save_fetched_models,
            load_fetched_models,
            start_local_server,
            stop_local_server,
            is_local_server_running
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                let state = window.state::<LocalLlamaState>();
                let mut lock = state.child_process.lock().unwrap();
                if let Some(mut child) = lock.take() {
                    let _ = child.kill(); // 彻底杀死进程
                    println!("Llama server terminated due to window close.");
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

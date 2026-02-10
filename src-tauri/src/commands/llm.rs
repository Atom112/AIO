use crate::models::*;
use crate::StreamManager;
use futures_util::StreamExt; // 用于处理流式数据
use serde_json::json;
use tauri::{Emitter, Window}; // Emitter 用于从后端向前端推送事件

/// 核心函数：调用 LLM 并分块回传结果（流式输出）
/// #[tauri::command] 允许前端通过 invoke 调用
#[tauri::command]
pub async fn call_llm_stream(
    window: Window,                         // Tauri 窗口句柄，用于发送事件
    state: tauri::State<'_, StreamManager>, // 全局状态，用于管理正在进行的流任务
    mut api_url: String,                    // API 地址
    api_key: String,                        // API 密钥
    model: String,                          // 模型名称（如 gpt-3.5-turbo）
    assistant_id: String,                   // 助手 ID（用于前端匹配消息）
    topic_id: String,                       // 话题/会话 ID
    messages: Vec<Message>,                 // 历史上下文消息列表
) -> Result<(), String> {
    // 1. 生成唯一的任务 Key，格式为 "助手ID-话题ID"
    let task_key = format!("{}-{}", assistant_id, topic_id);

    // 2. 如果当前 Key 已有任务在运行，先终止旧任务（防止一个对话框出现两个回复）
    if let Some((_, old_handle)) = state.0.remove(&task_key) {
        old_handle.abort();
    }

    // 3. 克隆变量以便进入异步线程（move 闭包）
    let state_inner = state.0.clone();
    let task_key_inner = task_key.clone();
    let assistant_id_c = assistant_id.clone();
    let topic_id_c = topic_id.clone();

    // 4. 创建异步任务执行请求
    let handle = tokio::spawn(async move {
        let result: Result<(), String> = async {
            // 安全处理 URL，确保以 /chat/completions 结尾
            api_url = api_url.trim_end_matches('/').to_string();
            let final_url = if !api_url.ends_with("/chat/completions") {
                format!("{}/chat/completions", api_url)
            } else {
                api_url
            };

            let client = reqwest::Client::new();

            // 构造符合 OpenAI API 标准的消息格式
            let messages_for_api: Vec<serde_json::Value> = messages
                .iter()
                .map(|m| {
                    json!({
                        "role": m.role,
                        "content": m.content // 这里现在可以直接是字符串或数组对象
                    })
                })
                .collect();

            // 构造请求体，开启 stream 模式
            let body = json!({
                "model": model,
                "messages": messages_for_api,
                "stream": true
            });

            // 发送 POST 请求
            let response = client
                .post(&final_url)
                .header("Authorization", format!("Bearer {}", api_key))
                .json(&body)
                .send()
                .await
                .map_err(|e| e.to_string())?;

            // 获取响应字节流
            let mut stream = response.bytes_stream();
            let mut line_buffer = String::new(); // 用于累积不完整的字节分块

            // 5. 循环处理流式返回的数据块
            while let Some(item) = stream.next().await {
                let chunk = item.map_err(|e| e.to_string())?;
                line_buffer.push_str(&String::from_utf8_lossy(&chunk));

                // LLM API 通常按行返回 (SSE 格式)
                while let Some(pos) = line_buffer.find('\n') {
                    let line = line_buffer[..pos].trim().to_string();
                    line_buffer.drain(..pos + 1); // 从缓冲区移除已处理的行

                    if line.is_empty() {
                        continue;
                    }

                    // 检查是否流传输结束
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

                    // 解析每行数据: data: {"choices":[{"delta":{"content":"..."}}]}
                    if line.starts_with("data: ") {
                        let json_str = &line[6..];
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                            if let Some(content) = val["choices"][0]["delta"]["content"].as_str() {
                                // 将解析出的片段实时推送到前端
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

        // 6. 错误处理：如果请求失败，发送错误信息给前端
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

        // 任务完成后，从全局状态中移除 handle
        state_inner.remove(&task_key_inner);
    });

    // 7. 将当前正在执行的任务句柄存入全局状态，以便后续可以“手动停止”
    state.0.insert(task_key, handle);
    Ok(())
}

/// 辅助函数：从服务商获取可用的模型列表
#[tauri::command]
pub async fn fetch_models(api_url: String, api_key: String) -> Result<Vec<ModelInfo>, String> {
    // 构造模型获取地址，通常是基础 URL 后接 /models
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

    // 解析返回的模型 JSON 数据
    let res_data: ModelsResponse = response.json().await.map_err(|e| e.to_string())?;
    Ok(res_data.data)
}

/// 停止函数：用户点击“停止生成”时调用
#[tauri::command]
pub async fn stop_llm_stream(
    state: tauri::State<'_, StreamManager>,
    assistant_id: String,
    topic_id: String,
) -> Result<(), String> {
    let task_key = format!("{}-{}", assistant_id, topic_id);

    // 从状态中取出对应的任务句柄并执行 abort() 强制停止任务
    if let Some((_, handle)) = state.0.remove(&task_key) {
        handle.abort();
    }
    Ok(())
}

#[tauri::command]
pub async fn summarize_history(
    api_url: String,
    api_key: String,
    model: String,
    messages: Vec<Message>,
) -> Result<String, String> {
    let client = reqwest::Client::new();

    let mut messages_for_api: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| json!({ "role": m.role, "content": m.content }))
        .collect();

    messages_for_api.push(json!({
        "role": "system",
        "content": "请简要总结以上对话的核心内容和用户需求，作为后续交流的长期记忆（500字以内）。"
    }));

    let body = json!({
        "model": model,
        "messages": messages_for_api,
        "stream": false
    });

    // --- 修复后的 URL 拼接逻辑 ---
    let base_url = api_url
        .trim_end_matches('/')
        .replace("/chat/completions", "");
    let endpoint = format!("{}/chat/completions", base_url);

    let res = client
        .post(endpoint)
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let val: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;

    // 增加一个简单的错误检查
    if let Some(err) = val.get("error") {
        return Err(err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("API Error")
            .to_string());
    }

    let summary = val["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("无法生成总结")
        .to_string();

    Ok(summary)
}

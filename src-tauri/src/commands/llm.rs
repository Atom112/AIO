use crate::core::state::DbState;
use rusqlite::params;
use crate::core::models::*;
use crate::core::state::StreamManager;
use futures_util::StreamExt; // 用于处理流式数据
use serde::Serialize;
use serde_json::json;
use std::time::Duration;
use tauri::{Emitter, Window}; // Emitter 用于从后端向前端推送事件

/// 构造带超时的 reqwest 客户端（防止 DoS）
fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// 流式 tool_call 累积载荷（发往前端用）
#[derive(Serialize, Clone)]
pub struct ToolCallPayload {
    pub assistant_id: String,
    pub topic_id: String,
    pub tool_call_id: String,
    pub name: String,
    pub arguments: String,
}

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
    tools: Option<Vec<ToolSpec>>,           // 工具定义（MCP 工具，None 或空数组则不发送）
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

            let client = http_client();

            // 构造符合 OpenAI API 标准的消息格式
            // 支持 role="tool"（带 tool_call_id）和 assistant 携带 tool_calls
            let messages_for_api: Vec<serde_json::Value> = messages
                .iter()
                .map(|m| {
                    let mut obj = serde_json::Map::new();
                    obj.insert("role".into(), json!(m.role));
                    obj.insert("content".into(), m.content.clone());
                    if let Some(tcid) = &m.tool_call_id {
                        obj.insert("tool_call_id".into(), json!(tcid));
                    }
                    if let Some(name) = &m.name {
                        obj.insert("name".into(), json!(name));
                    }
                    if let Some(tcs) = &m.tool_calls {
                        obj.insert("tool_calls".into(), json!(tcs));
                    }
                    serde_json::Value::Object(obj)
                })
                .collect();

            // 构造请求体，开启 stream 模式
            // 若传入 tools 且非空，则附加到 body
            let mut body_map = serde_json::Map::new();
            body_map.insert("model".into(), json!(model));
            body_map.insert("messages".into(), json!(messages_for_api));
            body_map.insert("stream".into(), json!(true));
            if let Some(tools) = &tools {
                if !tools.is_empty() {
                    body_map.insert("tools".into(), json!(tools));
                    body_map.insert("tool_choice".into(), json!("auto"));
                }
            }
            let body = serde_json::Value::Object(body_map);

            // 发送 POST 请求
            let response = client
                .post(&final_url)
                .header("Authorization", format!("Bearer {}", api_key))
                .json(&body)
                .send()
                .await
                .map_err(|e| e.to_string())?;

            // 检查 HTTP 状态码：非 2xx 时提前报错，避免对错误 JSON 走 SSE 解析
            let status = response.status();
            if !status.is_success() {
                let body_text = response.text().await.unwrap_or_default();
                let truncated = if body_text.len() > 512 { &body_text[..512] } else { &body_text };
                return Err(format!("LLM API {}: {}", status, truncated));
            }

            // 获取响应字节流
            let mut stream = response.bytes_stream();
            let mut line_buffer = String::new(); // 用于累积不完整的字节分块

            // tool_call 累积状态：按 index 维护 id/name/arguments
            // index → (id, name, arguments)
            let mut tc_accum: std::collections::HashMap<usize, (String, String, String)> =
                std::collections::HashMap::new();

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
                        // 在结束前 flush 累积中的 tool_calls
                        for (_idx, (id, name, args)) in tc_accum.drain() {
                            if !id.is_empty() && !name.is_empty() {
                                let _ = window.emit(
                                    "llm-tool-call",
                                    ToolCallPayload {
                                        assistant_id: assistant_id_c.clone(),
                                        topic_id: topic_id_c.clone(),
                                        tool_call_id: id,
                                        name,
                                        arguments: args,
                                    },
                                );
                            }
                        }
                        let _ = window.emit(
                            "llm-chunk",
                            StreamPayload {
                                assistant_id: assistant_id_c.clone(),
                                topic_id: topic_id_c.clone(),
                                content: "".into(),
                                done: true,
                            },
                        );
                        return Ok(());
                    }

                    // 解析每行数据: data: {"choices":[{"delta":{"content":"..."}}]}
                    if line.starts_with("data: ") {
                        let json_str = &line[6..];
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                            // 文本片段
                            if let Some(content) = val["choices"][0]["delta"]["content"].as_str() {
                                let _ = window.emit(
                                    "llm-chunk",
                                    StreamPayload {
                                        assistant_id: assistant_id_c.clone(),
                                        topic_id: topic_id_c.clone(),
                                        content: content.to_string(),
                                        done: false,
                                    },
                                );
                            }
                            // tool_calls 累积
                            if let Some(tcs) = val["choices"][0]["delta"]["tool_calls"].as_array() {
                                for tc in tcs {
                                    let index = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                                    let entry = tc_accum.entry(index).or_insert_with(|| {
                                        (String::new(), String::new(), String::new())
                                    });
                                    if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                                        entry.0 = id.to_string();
                                    }
                                    if let Some(name) = tc
                                        .get("function")
                                        .and_then(|f| f.get("name"))
                                        .and_then(|v| v.as_str())
                                    {
                                        entry.1 = name.to_string();
                                    }
                                    if let Some(args) = tc
                                        .get("function")
                                        .and_then(|f| f.get("arguments"))
                                        .and_then(|v| v.as_str())
                                    {
                                        entry.2.push_str(args);
                                    }
                                }
                            }
                            // finish_reason="tool_calls" 触发 flush
                            let finish = val["choices"][0]["finish_reason"]
                                .as_str()
                                .unwrap_or("");
                            if finish == "tool_calls" {
                                for (_idx, (id, name, args)) in tc_accum.drain() {
                                    if !id.is_empty() && !name.is_empty() {
                                        let _ = window.emit(
                                            "llm-tool-call",
                                            ToolCallPayload {
                                                assistant_id: assistant_id_c.clone(),
                                                topic_id: topic_id_c.clone(),
                                                tool_call_id: id,
                                                name,
                                                arguments: args,
                                            },
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // 流自然结束（未到 [DONE]）：flush 残余 tool_calls，然后 emit done
            for (_idx, (id, name, args)) in tc_accum.drain() {
                if !id.is_empty() && !name.is_empty() {
                    let _ = window.emit(
                        "llm-tool-call",
                        ToolCallPayload {
                            assistant_id: assistant_id_c.clone(),
                            topic_id: topic_id_c.clone(),
                            tool_call_id: id,
                            name,
                            arguments: args,
                        },
                    );
                }
            }
            let _ = window.emit(
                "llm-chunk",
                StreamPayload {
                    assistant_id: assistant_id_c.clone(),
                    topic_id: topic_id_c.clone(),
                    content: "".into(),
                    done: true,
                },
            );
            Ok(())
        }
        .await;

        // 6. 错误处理：如果请求失败，发送错误信息给前端
        if let Err(e) = result {
            tracing::error!("Stream Error: {}", e);
            let _ = window.emit(
                "llm-chunk",
                StreamPayload {
                    assistant_id: assistant_id_c,
                    topic_id: topic_id_c,
                    content: format!("\n[Error: {}]", e),
                    done: true,
                },
            );
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

    let client = http_client();
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
    let client = http_client();

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

#[tauri::command]
pub async fn append_message(
    state: tauri::State<'_, DbState>,
    topic_id: String,
    message: Message
) -> Result<(), String> {
    let conn = (*state).0.lock().unwrap();
    let files_json = serde_json::to_string(&message.display_files).ok();
    let content_json = serde_json::to_string(&message.content).unwrap_or_default();

    conn.execute(
        "INSERT INTO messages (topic_id, role, content, model_id, display_files, display_text) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![topic_id, message.role, content_json, message.model_id, files_json, message.display_text],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

/// 从消息内容中提取纯文本，多模态数组（OpenAI vision 格式）只保留 text 部分。
fn extract_text_content(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| {
                if v.get("type")?.as_str()? == "text" {
                    v.get("text")?.as_str().map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

/// 清洗模型返回的原始字符串为合规标题。
/// 1. 去除首尾空白与首尾成对引号（半角 / 全角 / 中文书名号 / 反引号）
/// 2. 取第一个非空行（避免多行输出）
/// 3. 递归剥离常见中英文前缀（"标题：" / "Title:" / "好的，" / "以下是" 等）
/// 4. 去除成对 Markdown 标记（**...** / `...`）
fn clean_topic_title(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // 取第一个非空行
    let first_line = trimmed
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or("");

    let mut s = first_line.to_string();

    // 剥离常见前缀（最多尝试 3 轮，防止 "好的，标题是：xxx" 这种嵌套）
    const PREFIXES: &[&str] = &[
        "好的，标题是：", "好的，标题是:", "好的，标题：", "好的，标题:",
        "好的：", "好的:", "好的，", "好的,",
        "标题是：", "标题是:", "标题：", "标题:",
        "Title:", "Title：", "title:", "title：",
        "以下是", "以下为", "下面给出", "给你一个",
        "Here is the title:", "Here is the title：",
        "The title is:", "The title is：",
    ];
    for _ in 0..3 {
        let mut matched = false;
        for p in PREFIXES {
            if s.starts_with(p) {
                s = s[p.len()..].trim().to_string();
                matched = true;
                break;
            }
        }
        if !matched {
            break;
        }
    }

    // 去除首尾成对引号（中英文 + 反引号 + 书名号）
    s = s
        .trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\''
                    | '`'
                    | '「'
                    | '」'
                    | '『'
                    | '』'
                    | '\u{201C}'
                    | '\u{201D}'
                    | '\u{2018}'
                    | '\u{2019}'
            )
        })
        .to_string();

    // 去除成对 Markdown 标记
    if s.len() > 4 && s.starts_with("**") && s.ends_with("**") {
        s = s[2..s.len() - 2].to_string();
    } else if s.len() > 2 && s.starts_with('`') && s.ends_with('`') {
        s = s[1..s.len() - 1].to_string();
    }

    s.trim().to_string()
}

/// 为话题生成一个简短标题（4-20 个字符）。
/// 由前端在新话题的"第一次对话"后调用一次，生成后前端将 `topic.renamed` 置为 `true`，
/// 后续不再调用以避免重复重命名。
/// 仅做内容生成，不写入数据库 —— 持久化由前端在更新 Store 后通过 `save_assistant` 完成。
///
/// # 参数
/// - `api_url` / `api_key` / `model`：调用方所用的 LLM 凭据（与流式对话保持一致）
/// - `messages`：用于生成标题的对话内容（建议取前 2~4 条）
///
/// # 返回
/// 成功时返回清洗后的标题字符串（已去除引号、空白、换行与常见前缀，长度限制在 1-20 字符内）。
///
/// # 失败模式
/// 若 LLM 长时间返回空内容（finish_reason=stop 且 content 为空），错误信息会附带
/// 模型名与原始长度，便于排查。前端应在 catch 中走启发式后备方案。
#[tauri::command]
pub async fn generate_topic_title(
    api_url: String,
    api_key: String,
    model: String,
    messages: Vec<Message>,
) -> Result<String, String> {
    if messages.is_empty() {
        return Err("生成标题需要至少一条消息".to_string());
    }

    let client = http_client();

    // 消息顺序遵循 LLM 约定：system 指令 → 对话上下文 → user 明确任务请求
    // 将 system 放最前、user 任务请求放最后，能显著提升小模型 / 本地模型的格式遵循度
    let mut messages_for_api: Vec<serde_json::Value> = vec![json!({
        "role": "system",
        "content": "你是一个话题标题生成助手，擅长用最少的字数精准概括对话核心内容。"
    })];

    // 注入对话历史：多模态 content 只取 text 部分，避免图片 base64 干扰生成
    for m in &messages {
        let text = extract_text_content(&m.content);
        if text.trim().is_empty() {
            continue;
        }
        messages_for_api.push(json!({ "role": m.role, "content": text }));
    }

    // 末尾追加明确的 user 任务请求，作为模型"应输出什么"的最终信号
    messages_for_api.push(json!({
        "role": "user",
        "content": "请根据以上对话生成一个 4-20 字的话题标题。\n\
                     严格要求：\n\
                     1. 精准概括核心主题或关键问题\n\
                     2. 不要加引号、冒号、序号、'好的'、'以下是'等多余文字\n\
                     3. 不要使用任何 Markdown 标记\n\
                     4. 你的回复必须且只能包含标题本身"
    }));

    let body = json!({
        "model": model,
        "messages": messages_for_api,
        "stream": false,
        // 200 token 足够覆盖"标题：xxx + 解释"等冗余输出；
        // 我们会在 Rust 侧再截断到 20 字符
        "max_tokens": 200,
        "temperature": 0.0
    });

    // URL 处理：去掉末尾斜杠与可能的 /chat/completions 后缀
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

    if let Some(err) = val.get("error") {
        return Err(err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("API Error")
            .to_string());
    }

    let raw = val["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();

    let cleaned = clean_topic_title(&raw);

    if cleaned.is_empty() {
        // 附带诊断信息：模型 / finish_reason / 原始长度
        let finish = val["choices"][0]["finish_reason"]
            .as_str()
            .unwrap_or("unknown");
        return Err(format!(
            "模型 {} 返回的标题为空 (finish_reason={}, raw_len={})",
            model,
            finish,
            raw.len()
        ));
    }

    // 长度限制：超过 20 字符截断（按字符而非字节，避免中文乱码）
    let truncated: String = if cleaned.chars().count() > 20 {
        cleaned.chars().take(20).collect()
    } else {
        cleaned
    };

    Ok(truncated)
}

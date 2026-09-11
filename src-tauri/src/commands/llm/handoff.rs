//! 压缩归档与交接文档（snapcompact 式归档、handoff 文档、子智能体进度摘要）。

use crate::commands::llm::*;
use tauri::{AppHandle, Window};

/// 从子智能体上下文提取已完成工作的进度摘要（命中轮数上限或未产出最终文本时使用），
/// 保证即使任务被截断也能向主 Agent 返回有效信息。
pub(crate) fn build_subagent_progress_summary(
    sub_msgs: &[serde_json::Value],
    max_rounds: usize,
) -> String {
    let mut last_assistant_text: Option<String> = None;
    let mut summary_text: Option<String> = None;
    let mut files: Vec<String> = Vec::new();
    let mut tool_count: usize = 0;

    for msg in sub_msgs {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if role == "assistant" {
            if let Some(s) = msg.get("content").and_then(|v| v.as_str()) {
                if !s.trim().is_empty() {
                    last_assistant_text = Some(s.to_string());
                }
            }
            if let Some(tcs) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                tool_count += tcs.len();
                for tc in tcs {
                    if let Some(args) = tc
                        .get("function")
                        .and_then(|f| f.get("arguments"))
                        .and_then(|v| v.as_str())
                    {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(args) {
                            if let Some(p) = v.get("path").and_then(|x| x.as_str()) {
                                if !files.contains(&p.to_string()) {
                                    files.push(p.to_string());
                                }
                            }
                        }
                    }
                }
            }
        } else if role == "system" {
            if let Some(s) = msg.get("content").and_then(|v| v.as_str()) {
                if s.contains("历史摘要") || s.contains("历史归档") {
                    summary_text = Some(s.to_string());
                }
            }
        }
    }

    let last_text = last_assistant_text
        .map(|s| {
            let t = &s[..s.floor_char_boundary(s.len().min(600))];
            t.to_string()
        })
        .unwrap_or_else(|| "（无中间文本输出）".to_string());
    let summary_part = match summary_text {
        Some(s) => format!(
            "\n\n压缩摘要（早期发现）：{}\n",
            truncate_chars_head_tail(&s, 800)
        ),
        None => String::new(),
    };
    let file_list = if files.is_empty() {
        "（未记录）".to_string()
    } else {
        files
            .iter()
            .map(|f| format!("- {}", f))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "[已达子智能体最大轮数限制（{} 轮），任务未完成。已完成工作如下（共 {} 次工具调用）：\n\n最后输出：{}\n{}涉及文件：\n{}\n\n请基于以上进度继续或让用户决定下一步。]",
        max_rounds, tool_count, last_text, summary_part, file_list
    )
}

/// 本地归档早期消息（snapcompact 式零成本压缩回退）：
/// 写入 {app_data}/archives/{uuid}.txt，返回 [历史归档 #id] 占位文本（含条数/token 估算/涉及工具）。
pub(crate) fn archive_early_messages(
    window: &Window,
    early: &[serde_json::Value],
    model: &str,
) -> Result<String, String> {
    let app = window.app_handle();
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取数据目录失败: {e}"))?
        .join("archives");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let artifact_id = uuid::Uuid::new_v4().simple().to_string();

    let mut out = String::new();
    let mut total_tokens: usize = 0;
    let mut tool_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for msg in early {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("?");
        let text = match msg.get("content") {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        };
        let text = &text[..text.floor_char_boundary(text.len().min(2000))];
        total_tokens += estimate_message_tokens(model, msg);
        if let Some(name) = msg.get("name").and_then(|v| v.as_str()) {
            tool_names.insert(name.to_string());
        }
        out.push_str(&format!("[{}]: {}\n", role, text));
    }
    let path = dir.join(format!("{}.txt", artifact_id));
    std::fs::write(&path, &out).map_err(|e| e.to_string())?;

    let names = if tool_names.is_empty() {
        String::new()
    } else {
        format!(
            "涉及工具: {}。",
            tool_names.into_iter().collect::<Vec<_>>().join(", ")
        )
    };
    Ok(format!(
        "[历史归档 #{}] 早期对话已本地归档（{} 条消息，约 {} tokens）。{}需要细节时用 read_artifact 工具读取。",
        artifact_id,
        early.len(),
        total_tokens,
        names
    ))
}

/// 生成工作交接文档（handoff）：写入 {app_data}/handoffs/{assistant}-{topic}-{ts}.md。
/// 内容：时间/状态、已完成内容（截断）、涉及文件（从工具调用参数提取）、未完成原因。
pub(crate) fn write_handoff_doc(
    app: &AppHandle,
    assistant_id: &str,
    topic_id: &str,
    accumulated: &str,
    messages: &[serde_json::Value],
    final_error: &Option<String>,
) -> Result<String, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取数据目录失败: {e}"))?
        .join("handoffs");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    // 收集涉及文件（从 assistant 消息的 tool_calls 参数中提取 path）
    let mut files: Vec<String> = Vec::new();
    for msg in messages {
        if let Some(tcs) = msg.get("tool_calls").and_then(|v| v.as_array()) {
            for tc in tcs {
                let name = tc
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if [
                    "read_file",
                    "write_file",
                    "replace_in_file",
                    "delete_file",
                    "make_directory",
                ]
                .contains(&name)
                {
                    if let Some(args) = tc
                        .get("function")
                        .and_then(|f| f.get("arguments"))
                        .and_then(|v| v.as_str())
                    {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(args) {
                            if let Some(p) = v.get("path").and_then(|x| x.as_str()) {
                                if !files.contains(&p.to_string()) {
                                    files.push(p.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let safe_a: String = assistant_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(12)
        .collect();
    let safe_t: String = topic_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(12)
        .collect();
    let path = dir.join(format!("{}-{}-{}.md", safe_a, safe_t, ts));

    let files_section = if files.is_empty() {
        "（无）".to_string()
    } else {
        files
            .iter()
            .map(|f| format!("- {}", f))
            .collect::<Vec<_>>()
            .join(
                "
",
            )
    };
    let body = format!(
        "# 工作交接文档（未完成任务）

- 时间: {}
- 助手: {}
- 话题: {}
- 错误: {}

## 已完成内容

{}

## 涉及文件

{}

## 下一步

请基于以上状态继续未完成的工作。
",
        ts,
        assistant_id,
        topic_id,
        final_error.as_deref().unwrap_or("无"),
        truncate_chars_head_tail(accumulated, 4000),
        files_section
    );
    std::fs::write(&path, body).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}
// compute_diff_preview / simple_diff 已下沉到 utils::diff（通用工具函数）

/// 为自动事实提取构造对话转写：取最近 12 条非 system 消息，截断到 ~10k 字符。
pub(crate) fn build_extract_transcript(messages: &[serde_json::Value]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let recent: Vec<&serde_json::Value> = messages.iter().rev().take(12).collect();
    // 序号从 0 递增（[role#idx]），供提取器以 evidence 引用依据消息
    let mut idx = 0usize;
    for m in recent.iter().rev() {
        let role = m.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if role == "system" {
            continue;
        }
        let content = m.get("content").and_then(|v| v.as_str()).unwrap_or("");
        if content.trim().is_empty() {
            continue;
        }
        parts.push(format!("[{role}#{idx}] {content}"));
        idx += 1;
    }
    let joined = parts.join("\n");
    joined.chars().take(10_000).collect()
}

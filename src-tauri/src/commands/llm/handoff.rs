//! 压缩归档与交接文档（snapcompact 式归档、handoff 文档、子智能体进度摘要）。

use crate::commands::llm::*;
use tauri::AppHandle;

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
/// 归档与 read_artifact 工具共用同一目录，模型可按占位 ID 读回完整内容。
/// 占位文本跟随 locale（en-US 英文，其余中文）。
pub(crate) fn archive_early_messages(
    app: &AppHandle,
    early: &[serde_json::Value],
    model: &str,
    locale: &str,
) -> Result<String, String> {
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

    let is_en = locale == "en-US";
    let names = if tool_names.is_empty() {
        String::new()
    } else {
        let joined = tool_names.into_iter().collect::<Vec<_>>().join(", ");
        if is_en {
            format!("Tools involved: {joined}.")
        } else {
            format!("涉及工具: {joined}。")
        }
    };
    Ok(if is_en {
        format!(
            "[Archive #{artifact_id}] Early conversation archived locally ({} messages, ~{} tokens). {names}Use the read_artifact tool when details are needed.",
            early.len(),
            total_tokens
        )
    } else {
        format!(
            "[历史归档 #{artifact_id}] 早期对话已本地归档（{} 条消息，约 {} tokens）。{names}需要细节时用 read_artifact 工具读取。",
            early.len(),
            total_tokens
        )
    })
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
    let path = dir.join(format!(
        "{}-{}-{}.md",
        sanitize_id(assistant_id),
        sanitize_id(topic_id),
        ts
    ));

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

/// ID 安全化（文件名前缀用）：仅保留字母数字与连字符，截断 12 字符。
fn sanitize_id(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(12)
        .collect()
}

/// 从指定目录消费交接文档（核心逻辑，可单测）：
/// - 查找 {assistant}-{topic}-*.md，按文件名字典序（unix 秒后缀 → 时间序）取最新；
/// - 读取内容（头尾截断 4000 字符，与写入侧一致）；
/// - 删除该前缀全部文件（consume-once：每次未完成回合只回注一次；
///   更早的同话题文档已被最新者取代，一并清理）。
pub(crate) fn consume_handoff_doc_from_dir(
    dir: &std::path::Path,
    assistant_id: &str,
    topic_id: &str,
) -> Option<String> {
    let prefix = format!("{}-{}-", sanitize_id(assistant_id), sanitize_id(topic_id));
    let mut candidates: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with(&prefix) && n.ends_with(".md"))
                .unwrap_or(false)
        })
        .collect();
    candidates.sort();
    let newest = candidates.pop()?;
    // 清理被取代的旧文档（同前缀）
    for old in &candidates {
        let _ = std::fs::remove_file(old);
    }
    let content = std::fs::read_to_string(&newest).ok()?;
    let _ = std::fs::remove_file(&newest);
    if content.trim().is_empty() {
        return None;
    }
    Some(truncate_chars_head_tail(&content, 4000))
}

/// 消费上回合交接文档（app_data/handoffs），供新回合回注。
/// 无文档/目录不存在时返回 None。
pub(crate) fn consume_handoff_doc(
    app: &AppHandle,
    assistant_id: &str,
    topic_id: &str,
) -> Option<String> {
    let dir = app.path().app_data_dir().ok()?.join("handoffs");
    consume_handoff_doc_from_dir(&dir, assistant_id, topic_id)
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

#[cfg(test)]
mod handoff_tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("aio-handoff-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 消费语义：取最新、consume-once、清理同前缀旧文件、不影响其他话题。
    #[test]
    fn consume_returns_newest_and_cleans_up() {
        let dir = temp_dir("consume");
        std::fs::write(
            dir.join("a1-t1-100.md"),
            "# 旧交接
内容A",
        )
        .unwrap();
        std::fs::write(
            dir.join("a1-t1-200.md"),
            "# 新交接
内容B",
        )
        .unwrap();
        std::fs::write(dir.join("a1-t2-300.md"), "# 其他话题").unwrap();

        let got = consume_handoff_doc_from_dir(&dir, "a1", "t1").expect("应找到最新交接");
        assert!(got.contains("新交接"), "应取最新: {got}");
        assert!(
            consume_handoff_doc_from_dir(&dir, "a1", "t1").is_none(),
            "consume-once：第二次应为空"
        );
        assert!(
            consume_handoff_doc_from_dir(&dir, "a1", "t2").is_some(),
            "其他话题不受影响"
        );
        // t1 的两个文档在第一次消费时清理；t2 的文档在上一行断言时被消费
        let remaining: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name())
            .collect();
        assert!(
            remaining.is_empty(),
            "所有文档应已被消费清理: {remaining:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 长文档头尾截断到 4000 字符（与写入侧一致）。
    #[test]
    fn consume_truncates_long_content() {
        let dir = temp_dir("truncate");
        let long = "x".repeat(10_000);
        std::fs::write(
            dir.join(format!("{}-{}-1.md", "a".repeat(12), "b".repeat(12))),
            &long,
        )
        .unwrap();
        // 长 ID 会被 sanitize 到 12 字符：与写入侧命名一致
        let got =
            consume_handoff_doc_from_dir(&dir, &"a".repeat(20), &"b".repeat(20)).expect("应找到");
        assert!(got.len() < 5_000, "应头尾截断: {}", got.len());
        assert!(got.contains("已省略"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 空文档视为无交接。
    #[test]
    fn consume_empty_doc_returns_none() {
        let dir = temp_dir("empty");
        std::fs::write(
            dir.join("a-t-1.md"),
            "   
",
        )
        .unwrap();
        assert!(consume_handoff_doc_from_dir(&dir, "a", "t").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

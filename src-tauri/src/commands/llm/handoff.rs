//! Compression archives and handoff documents (snapcompact-style archives, handoff docs, subagent progress summaries).

use crate::commands::llm::*;
use crate::ierr;
use tauri::AppHandle;

/// Extract a progress summary of completed work from the subagent context (used when the
/// round cap is hit or no final text was produced), so a truncated task still returns useful info.
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

/// Archive early messages locally (snapcompact-style zero-cost compression fallback):
/// writes {app_data}/archives/{uuid}.txt and returns a locale-dependent `[Archive #id]`-style
/// placeholder (message count / token estimate / tools involved). The archive directory is
/// shared with read_artifact, so the model can read the full content back by placeholder ID.
/// Placeholder text follows the locale (en-US English, otherwise Chinese).
pub(crate) fn archive_early_messages(
    app: &AppHandle,
    early: &[serde_json::Value],
    model: &str,
    locale: &str,
) -> Result<String, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| {
            ierr!(
                "获取数据目录失败: {}",
                "Failed to resolve the app data directory: {}",
                e
            )
        })?
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
    crate::core::fs_util::atomic_write(&path, out.as_bytes()).map_err(|e| e.to_string())?;

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

/// Write a work handoff document: {app_data}/handoffs/{assistant}-{topic}-{ts}.md.
/// Contents: time/status, completed work (truncated), files touched (from tool-call args), failure reason.
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
        .map_err(|e| {
            ierr!(
                "获取数据目录失败: {}",
                "Failed to resolve the app data directory: {}",
                e
            )
        })?
        .join("handoffs");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    // collect files involved (path extracted from tool_calls args in assistant messages)
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
    crate::core::fs_util::atomic_write(&path, body.as_bytes()).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

/// Sanitize an ID for a filename prefix: keep only alphanumerics and hyphens, truncated to 12 chars.
fn sanitize_id(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(12)
        .collect()
}

/// Consume the handoff document from a directory (core logic, unit-testable):
/// - find {assistant}-{topic}-*.md and take the newest by lexicographic filename order
///   (unix-second suffix -> chronological order);
/// - read the content (head/tail truncated to 4000 chars, matching the writer);
/// - delete all files with that prefix (consume-once: re-injected once per unfinished
///   turn; superseded earlier docs for the same topic are cleaned up as well).
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
    // clean up superseded older docs (same prefix)
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

/// Consume the previous turn's handoff document (app_data/handoffs) for re-injection into
/// the new turn. Returns None when no document or directory exists.
pub(crate) fn consume_handoff_doc(
    app: &AppHandle,
    assistant_id: &str,
    topic_id: &str,
) -> Option<String> {
    let dir = app.path().app_data_dir().ok()?.join("handoffs");
    consume_handoff_doc_from_dir(&dir, assistant_id, topic_id)
}
// compute_diff_preview / simple_diff live in utils::diff (generic utilities)

/// Build a conversation transcript for automated fact extraction: last 12 non-system messages, truncated to ~10k chars.
pub(crate) fn build_extract_transcript(messages: &[serde_json::Value]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let recent: Vec<&serde_json::Value> = messages.iter().rev().take(12).collect();
    // index increments from 0 ([role#idx]), letting the extractor cite messages as evidence
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

    /// Consume semantics: newest wins, consume-once, same-prefix cleanup, other topics untouched.
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
        // t1's two docs are cleaned up by the first consume; t2's doc was consumed by the assertion above
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

    /// Long documents are head/tail truncated to 4000 chars (matching the writer).
    #[test]
    fn consume_truncates_long_content() {
        let dir = temp_dir("truncate");
        let long = "x".repeat(10_000);
        std::fs::write(
            dir.join(format!("{}-{}-1.md", "a".repeat(12), "b".repeat(12))),
            &long,
        )
        .unwrap();
        // long IDs are sanitized to 12 chars: matches the writer-side naming
        let got =
            consume_handoff_doc_from_dir(&dir, &"a".repeat(20), &"b".repeat(20)).expect("应找到");
        assert!(got.len() < 5_000, "应头尾截断: {}", got.len());
        assert!(got.contains("已省略"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An empty document counts as no handoff.
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

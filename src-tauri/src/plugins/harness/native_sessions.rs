//! Native-adapter session import: Claude Code and Codex keep their own
//! transcripts on disk. This module lists them (read-only) and converts the
//! latest history into AIO display messages when an imported topic is created.
//!
//! - Codex: `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-*.jsonl`, `session_meta`
//!   carries the thread id / cwd; `response_item` messages carry user/assistant
//!   text.
//! - Claude Code: `$CLAUDE_CONFIG_DIR|~/.claude/projects/<encoded>/*.jsonl`;
//!   records carry `sessionId`, `cwd`, `type: user|assistant` and the SDK
//!   message content blocks.
//!
//! Parsing is intentionally display-only: the native CLI still owns the actual
//! session and is resumed with its own `--resume` / `exec resume` flag.

use crate::core::models::chat::Message;
use crate::plugins::harness::model::HarnessAgentSessionInfo;
use serde_json::Value;
use std::io::BufRead;
use std::path::{Path, PathBuf};

/// Cap the replayed display history (the native CLI keeps the full context).
const MAX_IMPORT_MESSAGES: usize = 200;

/// List native-adapter sessions for a tool and workspace.
pub fn list_sessions(tool_id: &str, cwd: &str) -> Result<Vec<HarnessAgentSessionInfo>, String> {
    match tool_id {
        "codex" => Ok(list_codex(cwd)),
        "claude" => Ok(list_claude(cwd)),
        _ => Ok(Vec::new()),
    }
}

/// Parse the latest display history of one native session.
pub fn import_history(tool_id: &str, session_id: &str, cwd: &str) -> Result<Vec<Message>, String> {
    match tool_id {
        "codex" => Ok(codex_history(session_id, cwd)),
        "claude" => Ok(claude_history(session_id, cwd)),
        _ => Ok(Vec::new()),
    }
}

struct ParsedSession {
    id: String,
    cwd: String,
    title: Option<String>,
    updated_at: Option<String>,
    messages: Vec<Message>,
}

impl ParsedSession {
    fn info(&self) -> HarnessAgentSessionInfo {
        HarnessAgentSessionInfo {
            session_id: self.id.clone(),
            title: self.title.clone(),
            updated_at: self.updated_at.clone(),
            message_count: Some(self.messages.len() as u64),
            imported: false,
        }
    }
}

// ===== Codex =====

fn codex_home() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("CODEX_HOME") {
        return Some(PathBuf::from(root));
    }
    home_dir().map(|h| h.join(".codex"))
}

fn list_codex(cwd: &str) -> Vec<HarnessAgentSessionInfo> {
    let Some(root) = codex_home().map(|h| h.join("sessions")) else {
        return Vec::new();
    };
    list_codex_at(&root, cwd)
}

fn list_codex_at(root: &Path, cwd: &str) -> Vec<HarnessAgentSessionInfo> {
    let target = normalize_path(cwd);
    let mut out = coded_files(root, |name| {
        name.starts_with("rollout-") && name.ends_with(".jsonl")
    })
    .into_iter()
    .filter_map(|path| parse_codex(&path))
    .filter(|s| normalize_path(&s.cwd) == target)
    .map(|s| s.info())
    .collect::<Vec<_>>();
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    out
}

fn codex_history(session_id: &str, cwd: &str) -> Vec<Message> {
    let Some(root) = codex_home().map(|h| h.join("sessions")) else {
        return Vec::new();
    };
    codex_history_at(&root, session_id, cwd)
}

fn codex_history_at(root: &Path, session_id: &str, cwd: &str) -> Vec<Message> {
    let target = normalize_path(cwd);
    for path in coded_files(root, |name| {
        name.starts_with("rollout-") && name.ends_with(".jsonl")
    }) {
        if let Some(parsed) = parse_codex(&path) {
            if parsed.id == session_id && normalize_path(&parsed.cwd) == target {
                return parsed.messages;
            }
        }
    }
    Vec::new()
}

fn parse_codex(path: &Path) -> Option<ParsedSession> {
    let file = std::fs::File::open(path).ok()?;
    let mut id: Option<String> = None;
    let mut cwd = String::new();
    let mut updated_at: Option<String> = None;
    let mut messages: Vec<Message> = Vec::new();
    let mut title: Option<String> = None;

    for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(ts) = value.get("timestamp").and_then(Value::as_str) {
            updated_at = Some(ts.to_string());
        }
        match value.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                let payload = value.get("payload").unwrap_or(&Value::Null);
                id = id
                    .or_else(|| {
                        payload
                            .get("session_id")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .or_else(|| {
                        payload
                            .get("id")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    });
                if let Some(value) = payload.get("cwd").and_then(Value::as_str) {
                    cwd = value.to_string();
                }
            }
            Some("response_item") => {
                let payload = value.get("payload").unwrap_or(&Value::Null);
                if payload.get("type").and_then(Value::as_str) != Some("message") {
                    continue;
                }
                let role = payload.get("role").and_then(Value::as_str).unwrap_or("");
                if role != "user" && role != "assistant" {
                    continue;
                }
                let text = content_text(payload.get("content"));
                if text.trim().is_empty() || (role == "user" && is_injected(&text)) {
                    continue;
                }
                if role == "user" && title.is_none() {
                    title = Some(first_line(&text));
                }
                push_display_message(&mut messages, role, &text);
            }
            _ => {}
        }
    }
    if id.is_none() {
        id = codex_id_from_filename(path);
    }
    Some(ParsedSession {
        id: id?,
        cwd,
        title,
        updated_at,
        messages,
    })
}

fn codex_id_from_filename(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy();
    // rollout-<timestamp>-<uuid>; the UUID is the last 36 characters.
    let idx = stem.len().checked_sub(36)?;
    let candidate = &stem[idx..];
    candidate
        .chars()
        .all(|c| c.is_ascii_hexdigit() || c == '-')
        .then(|| candidate.to_string())
}

// ===== Claude Code =====

fn claude_home() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return Some(PathBuf::from(root).join("projects"));
    }
    home_dir().map(|h| h.join(".claude").join("projects"))
}

fn list_claude(cwd: &str) -> Vec<HarnessAgentSessionInfo> {
    let Some(root) = claude_home() else {
        return Vec::new();
    };
    list_claude_at(&root, cwd)
}

fn list_claude_at(root: &Path, cwd: &str) -> Vec<HarnessAgentSessionInfo> {
    let target = normalize_path(cwd);
    let mut out = coded_files(root, |name| name.ends_with(".jsonl"))
        .into_iter()
        .filter_map(|path| parse_claude(&path))
        .filter(|s| normalize_path(&s.cwd) == target)
        .map(|s| s.info())
        .collect::<Vec<_>>();
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    out
}

fn claude_history(session_id: &str, cwd: &str) -> Vec<Message> {
    let Some(root) = claude_home() else {
        return Vec::new();
    };
    claude_history_at(&root, session_id, cwd)
}

fn claude_history_at(root: &Path, session_id: &str, cwd: &str) -> Vec<Message> {
    let target = normalize_path(cwd);
    for path in coded_files(root, |name| name.ends_with(".jsonl")) {
        if let Some(parsed) = parse_claude(&path) {
            if parsed.id == session_id && normalize_path(&parsed.cwd) == target {
                return parsed.messages;
            }
        }
    }
    Vec::new()
}

fn parse_claude(path: &Path) -> Option<ParsedSession> {
    let file = std::fs::File::open(path).ok()?;
    let mut id: Option<String> = None;
    let mut cwd = String::new();
    let mut updated_at: Option<String> = None;
    let mut messages: Vec<Message> = Vec::new();
    let mut title: Option<String> = None;

    for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(ts) = value.get("timestamp").and_then(Value::as_str) {
            updated_at = Some(ts.to_string());
        }
        if value.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        id = id.or_else(|| {
            value
                .get("sessionId")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
        if let Some(value) = value.get("cwd").and_then(Value::as_str) {
            cwd = value.to_string();
        }
        match value.get("type").and_then(Value::as_str) {
            Some("summary") => {
                if title.is_none() {
                    title = value.get("summary").and_then(Value::as_str).map(first_line);
                }
            }
            Some("user") | Some("assistant") => {
                let role = value.get("type").and_then(Value::as_str).unwrap_or("");
                let text = value
                    .pointer("/message/content")
                    .map(claude_content_text)
                    .unwrap_or_default();
                if text.trim().is_empty() || (role == "user" && is_injected(&text)) {
                    continue;
                }
                if role == "user" && title.is_none() {
                    title = Some(first_line(&text));
                }
                push_display_message(&mut messages, role, &text);
            }
            _ => {}
        }
    }
    if id.is_none() {
        id = path.file_stem().map(|s| s.to_string_lossy().into_owned());
    }
    Some(ParsedSession {
        id: id?,
        cwd,
        title,
        updated_at,
        messages,
    })
}

/// Claude content: a plain string or text blocks; tool_result/thinking ignored.
fn claude_content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

// ===== Shared helpers =====

fn home_dir() -> Option<PathBuf> {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(key).map(PathBuf::from)
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/").trim_end_matches('/').to_lowercase()
}

fn content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .or_else(|| item.get("input_text"))
                    .or_else(|| item.get("output_text"))
                    .and_then(Value::as_str)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// System-injected user messages (environment, AGENTS.md, abort markers) are not
/// conversation turns and must not become display bubbles.
fn is_injected(text: &str) -> bool {
    let head = text.trim_start();
    head.starts_with("<environment_context>")
        || head.starts_with("<user_instructions>")
        || head.starts_with("<turn_aborted>")
        || head.starts_with("# AGENTS.md instructions")
        || head.starts_with("<INSTRUCTIONS>")
}

fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let mut out: String = line.chars().take(80).collect();
    if line.chars().count() > 80 {
        out.push('…');
    }
    out
}

fn push_display_message(messages: &mut Vec<Message>, role: &str, text: &str) {
    messages.push(Message {
        id: Some(uuid::Uuid::new_v4().to_string()),
        role: role.to_string(),
        content: serde_json::json!(text),
        ..Default::default()
    });
    if messages.len() > MAX_IMPORT_MESSAGES {
        messages.drain(..messages.len() - MAX_IMPORT_MESSAGES);
    }
}

/// Files below `root` matching `accept` (depth-bounded, symlink-free walk).
fn coded_files(root: &Path, accept: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, accept: &dyn Fn(&str) -> bool, out: &mut Vec<PathBuf>) {
        if depth > 6 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                walk(&path, depth + 1, accept, out);
            } else if meta.is_file()
                && path
                    .file_name()
                    .map(|n| accept(&n.to_string_lossy()))
                    .unwrap_or(false)
            {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, 0, &accept, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_root(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("aio-native-sessions-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_jsonl(path: &Path, lines: &[Value]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = std::fs::File::create(path).unwrap();
        for line in lines {
            writeln!(file, "{line}").unwrap();
        }
    }

    /// Codex rollout: session_meta supplies id/cwd, response_item carries the
    /// real user/assistant text; injected environment turns are filtered.
    #[test]
    fn lists_and_parses_codex_rollout() {
        let root = temp_root("codex");
        let file = root.join(
            "2026/10/31/rollout-2026-10-31T10-00-00-01a1910c-f16b-75a2-ba0a-bad47ddf4bde.jsonl",
        );
        write_jsonl(
            &file,
            &[
                serde_json::json!({"timestamp":"2026-10-31T10:00:00Z","type":"session_meta","payload":{"session_id":"01a1910c-f16b-75a2-ba0a-bad47ddf4bde","cwd":"D:\\proj","timestamp":"2026-10-31T10:00:00Z"}}),
                serde_json::json!({"timestamp":"2026-10-31T10:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>ignored</environment_context>"}]}}),
                serde_json::json!({"timestamp":"2026-10-31T10:00:02Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Fix the bug"}]}}),
                serde_json::json!({"timestamp":"2026-10-31T10:00:03Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Done."}]}}),
            ],
        );
        let listed = list_codex_at(&root, "d:\\proj\\");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, "01a1910c-f16b-75a2-ba0a-bad47ddf4bde");
        assert_eq!(listed[0].title.as_deref(), Some("Fix the bug"));
        assert_eq!(listed[0].message_count, Some(2));
        let history = codex_history_at(&root, &listed[0].session_id, "D:\\proj");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].content, serde_json::json!("Fix the bug"));
        assert_eq!(history[1].role, "assistant");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Claude project transcript: summary is the title, user/assistant text
    /// blocks are kept, tool_result-only user rows are dropped.
    #[test]
    fn lists_and_parses_claude_transcript() {
        let root = temp_root("claude");
        let file = root.join("project").join("sess_1.jsonl");
        write_jsonl(
            &file,
            &[
                serde_json::json!({"type":"summary","summary":"Fix login flow","sessionId":"sess_1","cwd":"D:\\proj","timestamp":"2026-10-31T10:00:00Z"}),
                serde_json::json!({"type":"user","sessionId":"sess_1","cwd":"D:\\proj","timestamp":"2026-10-31T10:00:01Z","message":{"role":"user","content":[{"type":"text","text":"<environment_context>ignored</environment_context>"}]}}),
                serde_json::json!({"type":"user","sessionId":"sess_1","cwd":"D:\\proj","timestamp":"2026-10-31T10:00:02Z","message":{"role":"user","content":[{"type":"text","text":"Login is broken"}]}}),
                serde_json::json!({"type":"assistant","sessionId":"sess_1","cwd":"D:\\proj","timestamp":"2026-10-31T10:00:03Z","message":{"role":"assistant","content":[{"type":"text","text":"I will inspect it."}]}}),
                serde_json::json!({"type":"user","sessionId":"sess_1","cwd":"D:\\proj","timestamp":"2026-10-31T10:00:04Z","message":{"role":"user","content":[{"type":"tool_result","content":"tool output"}]}}),
            ],
        );
        let listed = list_claude_at(&root, "D:\\proj");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, "sess_1");
        assert_eq!(listed[0].title.as_deref(), Some("Fix login flow"));
        assert_eq!(listed[0].message_count, Some(2));
        let history = claude_history_at(&root, "sess_1", "D:\\proj");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].content, serde_json::json!("Login is broken"));
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// Live read of the local Codex session store (no model call).
    #[test]
    #[ignore = "live: reads the local ~/.codex session store"]
    fn codex_live_list_and_history() {
        let cwd = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repo root");
        let cwd = cwd.to_string_lossy();
        let sessions = list_sessions("codex", &cwd).expect("codex list");
        assert!(!sessions.is_empty(), "应列出至少一个 Codex 会话");
        let target = sessions
            .iter()
            .find(|s| s.message_count.unwrap_or(0) > 0)
            .expect("至少一个非空会话");
        let history = import_history("codex", &target.session_id, &cwd).expect("codex history");
        assert!(!history.is_empty(), "Codex 会话应能解析出显示消息");
    }
}

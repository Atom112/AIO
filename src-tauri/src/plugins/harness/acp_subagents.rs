//! ACP subagent-session aggregation (the subagents RFD + legacy `_meta` tags).
//!
//! The stable ACP schema does not type these notifications yet, so the driver
//! parses them from raw wire JSON. This module owns the display aggregation:
//! association patches (`subagent_update`), child-addressed text/tool events,
//! directed session messages and the normalized card snapshot.

use crate::plugins::harness::model::{HarnessEvent, HarnessSubagentInfo};
use serde_json::Value;

/// Aggregated display state of one subagent child session.
#[derive(Clone, Default)]
pub(super) struct SubagentRuntime {
    pub(super) title: Option<String>,
    pub(super) description: Option<String>,
    pub(super) agent: Option<String>,
    /// running | requires_action | unknown | idle
    pub(super) state: String,
    pub(super) stop_reason: Option<String>,
    pub(super) error: Option<String>,
    pub(super) recent_tools: Vec<String>,
    pub(super) recent_output: Vec<String>,
    pub(super) result: Option<String>,
}

/// Apply one `subagent_update` association patch to a runtime snapshot.
pub(super) fn apply_subagent_update(runtime: &mut SubagentRuntime, update: &Value) {
    apply_patch_str(&mut runtime.title, update.get("title"));
    apply_patch_str(&mut runtime.description, update.get("description"));
    if let Some(state) = update.get("state").filter(|v| v.is_object()) {
        if let Some(value) = state.get("state").and_then(Value::as_str) {
            runtime.state = value.to_string();
        }
        apply_patch_str(&mut runtime.stop_reason, state.get("stopReason"));
        if let Some(error) = state.get("error") {
            runtime.error = error
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| error.as_str())
                .map(str::to_string);
        }
    }
}

/// Fold child-session events into a runtime snapshot. Returns true when the
/// visible card changed (state changes go through `apply_subagent_update`).
pub(super) fn ingest_subagent_events(
    runtime: &mut SubagentRuntime,
    events: Vec<HarnessEvent>,
) -> bool {
    if runtime.state.is_empty() {
        runtime.state = "running".to_string();
    }
    let mut changed = false;
    for event in events {
        match event {
            HarnessEvent::TextDelta { text } => {
                push_recent(&mut runtime.recent_output, text, 2000);
                changed = true;
            }
            HarnessEvent::ThinkingDelta { .. } => {}
            HarnessEvent::ToolCallStart { name, title, .. } => {
                if let Some(label) = title.or(name) {
                    push_recent(&mut runtime.recent_tools, label, 200);
                    changed = true;
                }
            }
            HarnessEvent::ToolCallUpdate {
                content: Some(text),
                ..
            } => {
                push_recent(&mut runtime.recent_output, text, 2000);
                changed = true;
            }
            HarnessEvent::ToolCallEnd {
                content: Some(text),
                ..
            } => {
                runtime.result = Some(truncate_display(&text, 10000));
                changed = true;
            }
            _ => {}
        }
    }
    changed
}

/// Normalized card snapshot of one RFD child session.
pub(super) fn subagent_info(id: &str, runtime: &SubagentRuntime) -> HarnessSubagentInfo {
    let status = match runtime.state.as_str() {
        "" | "running" | "requires_action" | "unknown" => "running".to_string(),
        "idle" => match runtime.stop_reason.as_deref() {
            Some("error") | Some("failed") => "failed".to_string(),
            Some("cancelled") | Some("disconnected") => "cancelled".to_string(),
            _ => "completed".to_string(),
        },
        other => other.to_string(),
    };
    let result = match status.as_str() {
        "failed" => runtime.error.clone().or_else(|| runtime.result.clone()),
        "completed" => runtime
            .result
            .clone()
            .or_else(|| runtime.recent_output.last().cloned()),
        _ => runtime.result.clone(),
    };
    HarnessSubagentInfo {
        id: id.to_string(),
        tool_call_id: String::new(),
        agent: runtime.agent.clone(),
        title: runtime.title.clone(),
        task: runtime.description.clone(),
        status,
        model: None,
        thinking_level: None,
        duration_ms: None,
        tokens: None,
        cost_usd: None,
        recent_tools: runtime.recent_tools.clone(),
        recent_output: runtime.recent_output.clone(),
        result,
    }
}

/// Text from a `session_message` content array or a `session_message_chunk`
/// content block (normal text blocks; other content kinds are skipped).
pub(super) fn content_text_of(value: Option<&Value>) -> Option<String> {
    let value = value?;
    let mut out = String::new();
    match value {
        Value::String(text) => out.push_str(text),
        Value::Array(items) => items
            .iter()
            .for_each(|item| append_block_text(&mut out, item)),
        Value::Object(_) => append_block_text(&mut out, value),
        _ => {}
    }
    (!out.is_empty()).then_some(out)
}

/// Append one content block's text (normal text or ACP `content` wrapper).
fn append_block_text(out: &mut String, value: &Value) {
    if let Some(text) = value.get("text").and_then(Value::as_str) {
        out.push_str(text);
    } else if let Some(text) = value.pointer("/content/text").and_then(Value::as_str) {
        out.push_str(text);
    }
}

/// Legacy per-agent subagent tags: some ACP agents (Qwen Code) flatten child
/// traffic into the parent and attach `_meta.parentToolCallId/subagentType` to
/// each update. Returns (parent tool call id, optional agent/profile).
pub(super) fn legacy_subagent_meta(params: &Value) -> Option<(String, Option<String>)> {
    let parent = params
        .pointer("/update/_meta/parentToolCallId")
        .or_else(|| params.pointer("/_meta/parentToolCallId"))
        .and_then(Value::as_str)?;
    let agent = params
        .pointer("/update/_meta/subagentType")
        .or_else(|| params.pointer("/_meta/subagentType"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Some((parent.to_string(), agent))
}

/// Apply an optional RFD patch field: omitted = unchanged, null = clear,
/// concrete value = replace.
pub(super) fn apply_patch_str(target: &mut Option<String>, value: Option<&Value>) {
    let Some(value) = value else { return };
    if value.is_null() {
        *target = None;
    } else if let Some(text) = value.as_str() {
        *target = Some(text.to_string());
    }
}

/// Append to a bounded recent list (newest kept) with a display cap.
pub(super) fn push_recent(list: &mut Vec<String>, value: String, max_chars: usize) {
    if value.trim().is_empty() {
        return;
    }
    if list.last() == Some(&value) {
        return;
    }
    list.push(truncate_display(&value, max_chars));
    if list.len() > 8 {
        list.drain(..list.len() - 8);
    }
}

/// Char-safe head truncation for display payloads.
fn truncate_display(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Association patches + child events collapse into one card snapshot;
    /// state maps running/requires_action/unknown → running and idle →
    /// completed/failed/cancelled by stop reason.
    #[test]
    fn maps_rfd_subagent_lifecycle() {
        let mut runtime = SubagentRuntime::default();
        apply_subagent_update(
            &mut runtime,
            &json!({
                "sessionId": "child_1",
                "title": "Test investigator",
                "description": "Investigates platform-specific failures.",
                "state": { "state": "running" }
            }),
        );
        let changed = ingest_subagent_events(
            &mut runtime,
            vec![
                HarnessEvent::TextDelta {
                    text: "checking windows".to_string(),
                },
                HarnessEvent::ToolCallStart {
                    tool_call_id: "call_1".to_string(),
                    name: Some("Bash".to_string()),
                    title: Some("Run focused test".to_string()),
                    tool_kind: Some("execute".to_string()),
                    arguments: Some("{}".to_string()),
                },
                HarnessEvent::ToolCallEnd {
                    tool_call_id: "call_1".to_string(),
                    ok: true,
                    content: Some("1 passed".to_string()),
                    error: None,
                    file_changes: Vec::new(),
                },
            ],
        );
        assert!(changed);
        let info = subagent_info("child_1", &runtime);
        assert_eq!(info.title.as_deref(), Some("Test investigator"));
        assert_eq!(
            info.task.as_deref(),
            Some("Investigates platform-specific failures.")
        );
        assert_eq!(info.status, "running");
        assert_eq!(info.recent_tools, vec!["Run focused test"]);
        assert_eq!(info.recent_output, vec!["checking windows"]);
        assert_eq!(info.result.as_deref(), Some("1 passed"));

        apply_subagent_update(
            &mut runtime,
            &json!({ "sessionId": "child_1", "state": { "state": "idle", "stopReason": "end_turn" } }),
        );
        assert_eq!(subagent_info("child_1", &runtime).status, "completed");

        apply_subagent_update(
            &mut runtime,
            &json!({
                "sessionId": "child_1",
                "state": { "state": "idle", "stopReason": "error", "error": { "message": "boom" } }
            }),
        );
        let info = subagent_info("child_1", &runtime);
        assert_eq!(info.status, "failed");
        assert_eq!(info.result.as_deref(), Some("boom"));
    }

    /// Qwen Code's legacy `_meta` tags: the parent tool call id becomes the
    /// card key, the profile label is optional.
    #[test]
    fn parses_legacy_subagent_meta() {
        let (parent, agent) = legacy_subagent_meta(&json!({
            "sessionId": "root",
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "hello" },
                "_meta": { "parentToolCallId": "toolu_task", "subagentType": "explorer" }
            }
        }))
        .expect("legacy meta");
        assert_eq!(parent, "toolu_task");
        assert_eq!(agent.as_deref(), Some("explorer"));

        assert!(legacy_subagent_meta(&json!({ "sessionId": "root" })).is_none());
    }

    /// Directed session messages carry ordinary content blocks; text extraction
    /// handles the chunk object shape and the whole-message array shape.
    #[test]
    fn extracts_session_message_text() {
        assert_eq!(
            content_text_of(Some(&json!({ "type": "text", "text": "hello" }))).as_deref(),
            Some("hello")
        );
        assert_eq!(
            content_text_of(Some(&json!([
                { "type": "text", "text": "a" },
                { "type": "content", "content": { "type": "text", "text": "b" } }
            ])))
            .as_deref(),
            Some("ab")
        );
        assert!(content_text_of(Some(&json!({ "type": "image" }))).is_none());
    }
}

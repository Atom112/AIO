//! `session/load` replay → display messages.
//!
//! The ACP driver buffers replay notifications instead of emitting them live;
//! this module turns that buffer into the chat `Message` model (same
//! `agentSteps` JSON the frontend assembles for a live turn) so a replayed
//! transcript renders exactly like a recorded one.

use crate::core::models::chat::Message;
use crate::plugins::harness::model::{
    HarnessEvent, HarnessFileChange, HarnessSubagentInfo, HarnessUsage,
};
use serde_json::Value;

/// Convert a `session/load` replay stream into display messages. ACP replays a
/// turn as `user_message_chunk` followed by `agent_thought_chunk`,
/// `agent_message_chunk`, tool-call updates and (for harnesses that expose
/// them) subagent state; this groups everything between user chunks into one
/// assistant message with the same `agentSteps` JSON the live frontend builds.
pub(super) fn build_history(events: &[HarnessEvent]) -> Vec<Message> {
    struct AssistantBuf {
        content: String,
        reasoning: String,
        steps: Vec<Value>,
        started_at: i64,
        usage: Option<HarnessUsage>,
    }

    impl AssistantBuf {
        fn new(ts: i64) -> Self {
            Self {
                content: String::new(),
                reasoning: String::new(),
                steps: Vec::new(),
                started_at: ts,
                usage: None,
            }
        }

        fn finish(self) -> Option<Message> {
            if self.content.is_empty() && self.reasoning.is_empty() && self.steps.is_empty() {
                return None;
            }
            Some(Message {
                id: Some(uuid::Uuid::new_v4().to_string()),
                role: "assistant".to_string(),
                content: serde_json::json!(self.content),
                reasoning: (!self.reasoning.is_empty()).then_some(self.reasoning),
                agent_steps: (!self.steps.is_empty()).then(|| Value::Array(self.steps)),
                agent_start_time: Some(self.started_at),
                context_tokens: self
                    .usage
                    .and_then(|u| u.context_tokens)
                    .and_then(|v| u32::try_from(v).ok()),
                ..Default::default()
            })
        }
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    fn tool_step(ts: i64, tool_call_id: &str, name: &str, arguments: &str) -> Value {
        serde_json::json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "type": "tool_call",
            "timestamp": ts,
            "status": "running",
            "toolCall": {
                "id": tool_call_id,
                "type": "function",
                "function": { "name": name, "arguments": arguments },
                "state": "calling"
            }
        })
    }

    fn step_status(raw: &str) -> &'static str {
        match raw {
            "completed" | "idle" => "complete",
            "failed" | "cancelled" | "aborted" | "error" => "error",
            _ => "running",
        }
    }

    fn tool_step_index(steps: &[Value], tool_call_id: &str) -> Option<usize> {
        steps.iter().position(|s| {
            s.get("type").and_then(Value::as_str) == Some("tool_call")
                && s.pointer("/toolCall/id").and_then(Value::as_str) == Some(tool_call_id)
        })
    }

    fn upsert_tool_start(
        steps: &mut Vec<Value>,
        ts: i64,
        tool_call_id: &str,
        name: &str,
        arguments: &str,
    ) {
        if let Some(i) = tool_step_index(steps, tool_call_id) {
            steps[i]["toolCall"]["function"]["name"] = serde_json::json!(name);
            steps[i]["toolCall"]["function"]["arguments"] = serde_json::json!(arguments);
        } else {
            steps.push(tool_step(ts, tool_call_id, name, arguments));
        }
    }

    fn patch_tool_running(
        steps: &mut [Value],
        tool_call_id: &str,
        status: Option<&str>,
        content: Option<&str>,
        raw_output: Option<&str>,
    ) {
        let Some(i) = tool_step_index(steps, tool_call_id) else {
            return;
        };
        let step = &mut steps[i];
        if let Some(text) = content.or(raw_output) {
            step["toolCall"]["content"] = serde_json::json!(text);
            step["toolCall"]["fullContent"] = serde_json::json!(text);
        }
        if let Some(raw) = status {
            let mapped = step_status(raw);
            step["status"] = serde_json::json!(mapped);
            step["toolCall"]["state"] = serde_json::json!(if mapped == "running" {
                "calling"
            } else {
                mapped
            });
        }
    }

    fn diff_summary(diff: &str) -> String {
        let mut added = 0usize;
        let mut removed = 0usize;
        for line in diff.split('\n') {
            if line.starts_with('+') && !line.starts_with("+++") {
                added += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                removed += 1;
            }
        }
        format!("+{added} -{removed}")
    }

    fn finish_tool(
        steps: &mut [Value],
        tool_call_id: &str,
        ok: bool,
        content: Option<&str>,
        error: Option<&str>,
        changes: &[HarnessFileChange],
    ) {
        let Some(i) = tool_step_index(steps, tool_call_id) else {
            return;
        };
        let step = &mut steps[i];
        let mapped = if ok { "complete" } else { "error" };
        step["status"] = serde_json::json!(mapped);
        step["toolCall"]["state"] = serde_json::json!(if ok { "success" } else { "error" });
        if let Some(text) = content {
            step["toolCall"]["content"] = serde_json::json!(text);
            step["toolCall"]["fullContent"] = serde_json::json!(text);
        }
        if let Some(text) = error {
            step["toolCall"]["error"] = serde_json::json!(text);
        }
        if !changes.is_empty() {
            let list: Vec<Value> = changes
                .iter()
                .map(|fc| {
                    serde_json::json!({
                        "filePath": fc.file_path,
                        "action": fc.action,
                        "diff": fc.diff,
                        "summary": diff_summary(&fc.diff)
                    })
                })
                .collect();
            step["toolCall"]["fileChanges"] = Value::Array(list);
        }
    }

    fn patch_subagent(steps: &mut Vec<Value>, info: &HarnessSubagentInfo, ts: i64) {
        let mapped = step_status(&info.status);
        let inner: Vec<Value> = info
            .recent_tools
            .iter()
            .rev()
            .take(8)
            .rev()
            .map(|tool| {
                serde_json::json!({
                    "id": uuid::Uuid::new_v4().to_string(),
                    "type": "tool_call",
                    "timestamp": ts,
                    "status": "complete",
                    "toolName": tool
                })
            })
            .chain(info.recent_output.iter().rev().take(8).rev().map(|text| {
                serde_json::json!({
                    "id": uuid::Uuid::new_v4().to_string(),
                    "type": "content",
                    "timestamp": ts,
                    "status": "complete",
                    "summary": text
                })
            }))
            .collect();
        let existing = steps.iter().position(|s| {
            s.get("type").and_then(Value::as_str) == Some("subagent")
                && s.get("subagentId").and_then(Value::as_str) == Some(info.id.as_str())
        });
        let step = match existing {
            Some(i) => &mut steps[i],
            None => {
                steps.push(serde_json::json!({
                    "id": uuid::Uuid::new_v4().to_string(),
                    "type": "subagent",
                    "timestamp": ts,
                    "status": mapped,
                    "subagentId": info.id,
                }));
                steps.last_mut().expect("pushed step")
            }
        };
        step["status"] = serde_json::json!(mapped);
        if let Some(agent) = &info.agent {
            step["subagentProfile"] = serde_json::json!(agent);
        }
        step["subagentName"] = serde_json::json!(info.title.as_ref().unwrap_or(&info.id));
        if let Some(task) = &info.task {
            step["subagentTask"] = serde_json::json!(task);
        }
        if let Some(duration) = info.duration_ms {
            step["duration"] = serde_json::json!(duration);
        }
        if !inner.is_empty() {
            step["subagentSteps"] = Value::Array(inner);
        }
        if let Some(result) = &info.result {
            step["subagentResult"] = serde_json::json!(result);
        }
    }

    fn push_user(out: &mut Vec<Message>, text: &mut String) {
        if text.trim().is_empty() {
            text.clear();
            return;
        }
        out.push(Message {
            id: Some(uuid::Uuid::new_v4().to_string()),
            role: "user".to_string(),
            content: serde_json::json!(text.trim()),
            ..Default::default()
        });
        text.clear();
    }

    fn flush_assistant(buffer: &mut Option<AssistantBuf>, out: &mut Vec<Message>) {
        if let Some(done) = buffer.take().and_then(AssistantBuf::finish) {
            out.push(done);
        }
    }

    let mut out: Vec<Message> = Vec::new();
    let mut user = String::new();
    let mut assistant: Option<AssistantBuf> = None;
    let mut ts = now_ms();

    for event in events {
        ts += 1;
        match event {
            HarnessEvent::UserTextDelta { text } => {
                // A new user message ends the previous assistant turn.
                flush_assistant(&mut assistant, &mut out);
                user.push_str(text);
            }
            HarnessEvent::TextDelta { text } => {
                push_user(&mut out, &mut user);
                let buf = assistant.get_or_insert_with(|| AssistantBuf::new(ts));
                buf.content.push_str(text);
            }
            HarnessEvent::ThinkingDelta { text } => {
                push_user(&mut out, &mut user);
                let buf = assistant.get_or_insert_with(|| AssistantBuf::new(ts));
                buf.reasoning.push_str(text);
            }
            HarnessEvent::ToolCallStart {
                tool_call_id,
                name,
                title,
                arguments,
                ..
            } => {
                push_user(&mut out, &mut user);
                let buf = assistant.get_or_insert_with(|| AssistantBuf::new(ts));
                let label = name.as_deref().or(title.as_deref()).unwrap_or("tool");
                upsert_tool_start(
                    &mut buf.steps,
                    ts,
                    tool_call_id,
                    label,
                    arguments.as_deref().unwrap_or("{}"),
                );
            }
            HarnessEvent::ToolCallUpdate {
                tool_call_id,
                status,
                content,
                raw_output,
            } => {
                push_user(&mut out, &mut user);
                let buf = assistant.get_or_insert_with(|| AssistantBuf::new(ts));
                patch_tool_running(
                    &mut buf.steps,
                    tool_call_id,
                    status.as_deref(),
                    content.as_deref(),
                    raw_output.as_deref(),
                );
            }
            HarnessEvent::ToolCallEnd {
                tool_call_id,
                ok,
                content,
                error,
                file_changes,
            } => {
                push_user(&mut out, &mut user);
                let buf = assistant.get_or_insert_with(|| AssistantBuf::new(ts));
                finish_tool(
                    &mut buf.steps,
                    tool_call_id,
                    *ok,
                    content.as_deref(),
                    error.as_deref(),
                    file_changes,
                );
            }
            HarnessEvent::SubagentUpdate { entries } => {
                push_user(&mut out, &mut user);
                let buf = assistant.get_or_insert_with(|| AssistantBuf::new(ts));
                for info in entries {
                    patch_subagent(&mut buf.steps, info, ts);
                }
            }
            HarnessEvent::UsageUpdate { usage } => {
                if let Some(buf) = assistant.as_mut() {
                    buf.usage = Some(usage.clone());
                }
            }
            HarnessEvent::Notice { message } => {
                push_user(&mut out, &mut user);
                let buf = assistant.get_or_insert_with(|| AssistantBuf::new(ts));
                buf.steps.push(serde_json::json!({
                    "id": uuid::Uuid::new_v4().to_string(),
                    "type": "content",
                    "timestamp": ts,
                    "status": "complete",
                    "contentText": message
                }));
            }
            HarnessEvent::TurnEnded { .. } => {
                push_user(&mut out, &mut user);
                flush_assistant(&mut assistant, &mut out);
            }
            HarnessEvent::SessionInfo { .. }
            | HarnessEvent::ConfigOptions { .. }
            | HarnessEvent::CommandsUpdate { .. }
            | HarnessEvent::TurnStarted
            | HarnessEvent::PlanUpdate { .. }
            | HarnessEvent::SubagentDelta { .. }
            | HarnessEvent::Error { .. } => {}
            HarnessEvent::History { .. } => {}
        }
    }
    push_user(&mut out, &mut user);
    flush_assistant(&mut assistant, &mut out);
    out
}

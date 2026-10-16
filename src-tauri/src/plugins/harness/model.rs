//! Harness domain model: the normalized event stream + approval payloads shared by
//! every CLI-harness adapter (ACP / Claude Code native / Codex native).
//!
//! Every adapter converts its native protocol into [`HarnessEvent`]; the frontend
//! maps that single stream onto the same `Message`/`AgentStep`/`ToolCallDisplay`
//! model the built-in chat pipeline uses, so the harness workbench renders with
//! the same components as the chat page. Wire shape: enum tag `kind` with
//! snake_case variant names, camelCase fields (matching the chat `Message`
//! serialization convention the frontend store mirrors).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Session key: (tool_id, harness session row id). A structured tuple avoids the
/// string-concat ambiguity of "a-b"+"c" == "a"+"b-c" (UUID-style ids contain "-").
pub type HarnessSessionKey = (String, String);

/// One plan/task-board entry pushed by the harness agent.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HarnessPlanEntry {
    /// Entry text (task description)
    pub content: String,
    /// Raw priority from the harness (high | medium | low | other)
    pub priority: String,
    /// Raw status from the harness (pending | in_progress | completed | cancelled | other)
    pub status: String,
}

/// One file change reported by a harness tool call.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HarnessFileChange {
    /// Absolute or workspace-relative file path as reported by the harness
    pub file_path: String,
    /// create | modify | delete
    pub action: String,
    /// Unified diff text (may be empty when the harness only reports the path)
    pub diff: String,
}

/// Usage / cost snapshot for a turn (all fields optional — adapters fill what the
/// native protocol reports; the frontend maps them onto the token bar).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct HarnessUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// Context tokens currently in use (ACP usage_update.used)
    pub context_tokens: Option<u64>,
    /// Total context window size (ACP usage_update.size)
    pub context_window: Option<u64>,
    pub cost_usd: Option<f64>,
}

/// The normalized harness event stream. `kind` tag values are snake_case; variant
/// fields are camelCase.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HarnessEvent {
    /// Session established on the harness side (id may arrive after spawn).
    #[serde(rename_all = "camelCase")]
    SessionInfo {
        harness_session_id: Option<String>,
        model: Option<String>,
    },
    /// A user prompt was accepted and the agent started a new turn.
    #[serde(rename_all = "camelCase")]
    TurnStarted,
    /// Streaming assistant text delta.
    #[serde(rename_all = "camelCase")]
    TextDelta { text: String },
    /// Streaming chain-of-thought delta.
    #[serde(rename_all = "camelCase")]
    ThinkingDelta { text: String },
    /// Tool call appeared or changed while still running (upsert by tool_call_id).
    #[serde(rename_all = "camelCase")]
    ToolCallStart {
        tool_call_id: String,
        name: Option<String>,
        title: Option<String>,
        /// Tool category from the harness (read | edit | execute | fetch | think | ...);
        /// named `tool_kind` — the serde tag of this enum is already `kind`.
        tool_kind: Option<String>,
        /// JSON-serialized tool input (display only)
        arguments: Option<String>,
    },
    /// Partial patch to a running tool call (upsert by tool_call_id).
    #[serde(rename_all = "camelCase")]
    ToolCallUpdate {
        tool_call_id: String,
        status: Option<String>,
        content: Option<String>,
        raw_output: Option<String>,
    },
    /// Tool call finished (final — closes the step).
    #[serde(rename_all = "camelCase")]
    ToolCallEnd {
        tool_call_id: String,
        /// true = success, false = failed
        ok: bool,
        content: Option<String>,
        error: Option<String>,
        file_changes: Vec<HarnessFileChange>,
    },
    /// Agent plan / task list replaced wholesale.
    #[serde(rename_all = "camelCase")]
    PlanUpdate { entries: Vec<HarnessPlanEntry> },
    /// Usage / cost update (may arrive mid-turn or at turn end).
    #[serde(rename_all = "camelCase")]
    UsageUpdate { usage: HarnessUsage },
    /// Turn finished (stop_reason: end_turn | cancelled | max_tokens | ...).
    #[serde(rename_all = "camelCase")]
    TurnEnded {
        stop_reason: String,
        error: Option<String>,
    },
    /// Agent notice / warning surfaced mid-turn (quota, deprecation, ...).
    #[serde(rename_all = "camelCase")]
    Notice { message: String },
    /// Driver-level error (spawn/handshake/transport); the session may be dead.
    #[serde(rename_all = "camelCase")]
    Error { message: String },
}

/// One selectable option in a harness approval request. ACP permission option
/// kinds are forwarded as-is; native adapters synthesize allow/deny/always.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HarnessApprovalOption {
    pub option_id: String,
    pub name: String,
    /// allow_once | allow_always | reject_once | reject_always | allow | deny | cancel
    pub kind: String,
}

/// An approval the harness agent is waiting on. The driver blocks until the user
/// answers (or the session dies); there is no timeout by design — the harness
/// semantics are "wait for the human".
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HarnessApprovalRequest {
    pub request_id: String,
    /// tool | command | plan | question (display hint)
    pub kind: String,
    pub title: Option<String>,
    pub tool_name: Option<String>,
    /// Raw tool arguments (JSON value, display only)
    pub arguments: Option<Value>,
    pub command: Option<String>,
    /// File path the preview diff belongs to (when the tool call carries one)
    pub file_path: Option<String>,
    pub preview_diff: Option<String>,
    pub options: Vec<HarnessApprovalOption>,
}

/// User decision for a harness approval.
#[derive(Debug, Clone)]
pub struct ApprovalDecision {
    /// The selected option id (None only when cancelled)
    pub option_id: Option<String>,
    /// True when the whole turn was cancelled instead of answering
    pub cancelled: bool,
}

/// Events the driver task pushes to the manager-owned emitter (which serializes
/// them into Tauri events for the frontend).
#[derive(Debug)]
pub enum DriverEvent {
    /// A normalized harness event for the frontend stream
    Event(HarnessEvent),
    /// Session run-state transition (connecting | running | idle | error)
    State {
        state: String,
        error: Option<String>,
    },
}

// ===== Frontend event payload wrappers (snake_case field names, matching the
// streamPayloads.ts convention for backend→frontend event envelopes) =====

/// `harness-event` payload.
#[derive(Serialize, Clone)]
pub struct HarnessEventPayload {
    pub tool_id: String,
    pub session_row_id: String,
    pub event: HarnessEvent,
}

/// `harness-state` payload.
#[derive(Serialize, Clone)]
pub struct HarnessStatePayload {
    pub tool_id: String,
    pub session_row_id: String,
    /// connecting | running | idle | error
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `harness-approval-requested` payload.
#[derive(Serialize, Clone)]
pub struct HarnessApprovalRequestedPayload {
    pub tool_id: String,
    pub session_row_id: String,
    pub request: HarnessApprovalRequest,
}

/// `harness-approval-expired` payload.
#[derive(Serialize, Clone)]
pub struct HarnessApprovalExpiredPayload {
    pub request_id: String,
}

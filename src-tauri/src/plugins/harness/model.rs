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

/// One slash command advertised by the agent (`available_commands_update`).
/// The command is executed by sending `/name [input]` as the next prompt —
/// ACP has no separate command-invocation method.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HarnessCommandInfo {
    pub name: String,
    pub description: String,
    /// Argument hint from the agent (`input.hint`), when the command takes input.
    pub input_hint: Option<String>,
}

/// One subagent reported by a harness tool call. Verified source: omp's `task`
/// tool updates carry `rawOutput.details.progress[]` (id / agent / status /
/// model / tokens / recent output) and its `wait` tool carries
/// `details.jobs[].resultText`. ACP itself has no subagent stream, so adapters
/// that expose subagents through structured tool output are mapped here.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HarnessSubagentInfo {
    /// Subagent id assigned by the harness (stable across updates).
    pub id: String,
    /// Parent tool call id (the `task` / `wait` call that reported it).
    pub tool_call_id: String,
    /// Agent/profile name (e.g. "sonic", "explorer").
    pub agent: Option<String>,
    /// Human-readable child title (ACP subagent RFD `title`); when absent the
    /// GUI falls back to the harness-assigned id.
    pub title: Option<String>,
    /// Assignment text (first line of the task the subagent was given).
    pub task: Option<String>,
    /// pending | running | completed | failed | aborted | cancelled (raw)
    pub status: String,
    pub model: Option<String>,
    pub thinking_level: Option<String>,
    pub duration_ms: Option<u64>,
    pub tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    /// Names/labels of the subagent's most recent tool calls.
    pub recent_tools: Vec<String>,
    /// The subagent's most recent output lines.
    pub recent_output: Vec<String>,
    /// Final result text (harnesses that report it, e.g. omp's `wait` job snapshot).
    pub result: Option<String>,
}

/// One existing session advertised by an ACP agent (`session/list`) that AIO
/// can adopt as a topic (history is replayed by `session/load` on connect).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HarnessAgentSessionInfo {
    pub session_id: String,
    pub title: Option<String>,
    pub updated_at: Option<String>,
    /// Agent-reported message count (`_meta.messageCount`), when available.
    pub message_count: Option<u64>,
    /// Already bound to an AIO topic under this tool/workspace.
    pub imported: bool,
}

/// Result of probing a tool's `session/list` capability.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HarnessAgentSessionList {
    /// False when the tool is a native adapter or does not advertise
    /// `sessionCapabilities.list`.
    pub supported: bool,
    pub sessions: Vec<HarnessAgentSessionInfo>,
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

/// One selectable value of a session-config option (model / thought level ...).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HarnessConfigValue {
    pub id: String,
    pub name: Option<String>,
}

/// A session-config option exposed for the GUI: model selector, thought-level
/// selector, or boolean toggle. ACP agents report these in session/new and
/// `config_option_update`; native adapters synthesize them from their own
/// surfaces (claude set_model/--effort, codex -m/-c per turn).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HarnessConfigOption {
    pub option_id: String,
    pub name: Option<String>,
    /// select | boolean
    pub kind: String,
    /// Semantic category hint (model | thought_level | model_config | mode | other) —
    /// ACP passes the agent's category through; native adapters synthesize it.
    pub category: Option<String>,
    /// select: currently selected value id
    pub current: Option<String>,
    /// select: available values
    pub values: Vec<HarnessConfigValue>,
    /// boolean: current on/off state
    pub boolean_value: Option<bool>,
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
    /// Session config options (model / thought-level selectors, toggles) —
    /// the full current set, replacing any previous report.
    #[serde(rename_all = "camelCase")]
    ConfigOptions { options: Vec<HarnessConfigOption> },
    /// Slash commands the agent currently accepts (replaces the previous set).
    /// Commands execute by sending `/name [input]` as the next prompt.
    #[serde(rename_all = "camelCase")]
    CommandsUpdate { commands: Vec<HarnessCommandInfo> },
    /// A user message replayed by `session/load` (history restore). Live turns
    /// ignore it — the GUI already renders the user's own input.
    #[serde(rename_all = "camelCase")]
    UserTextDelta { text: String },
    /// Current subagent states reported by a structured tool call
    /// (omp `task` / `wait`); upsert by id on the frontend.
    #[serde(rename_all = "camelCase")]
    SubagentUpdate { entries: Vec<HarnessSubagentInfo> },
    /// Complete display history replayed by `session/load`, already converted
    /// to the chat `Message` model and persisted by the driver. The frontend
    /// replaces the current message list with it (and drops any stale draft).
    #[serde(rename_all = "camelCase")]
    History {
        messages: Vec<crate::core::models::chat::Message>,
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
    /// Text streamed from a subagent run (Claude Code Task tool etc.).
    #[serde(rename_all = "camelCase")]
    SubagentDelta {
        /// Subagent display name when the harness reports one
        name: Option<String>,
        text: String,
    },
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

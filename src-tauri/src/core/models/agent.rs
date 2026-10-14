//! Agent domain models: Agent mode enum and custom subagent profiles.

use serde::{Deserialize, Serialize};

/// Agent execution mode.
///
/// The mode ladder (wire values are stable — they are persisted in the
/// assistants table; only the user-facing labels/semantics changed):
/// - `off`    → 「只读」: read-only agent (reads/search/web/think; mutations denied)
/// - `normal` → 「工作区内修改」: workspace edits without confirmation; deletes/commands still ask
/// - `auto`   → 「完全权限」: full autonomy inherent (sandbox auto-expansion + network
///   gate bypass + everything auto-allowed); risk confirmed on mode entry
/// - `danger` → 「本机全权」: Auto plus the OS sandbox disabled entirely — commands run
///   natively with user privileges. The DSH/Codex-YOLO tier: every entry shows an
///   explicit red warning; intended for disposable test machines / containers
/// - `plan`   → 「计划」: research + plan only
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum AgentMode {
    /// Read-only mode (formerly "chat"): reads/search/web allowed, mutations denied;
    /// without a project it degrades to pure chat (no tools)
    #[default]
    Off,
    /// Workspace-edit mode (formerly "normal"): file writes/edits/mkdir in the
    /// workspace run without confirmation; deletes / commands / git writes still ask
    Normal,
    /// Full-permission mode (formerly "auto"): everything auto-allowed — sandbox
    /// auto-expansion and the network-declaration bypass are inherent, no opt-in
    Auto,
    /// Machine-full-trust mode: identical permissions to Auto plus the OS sandbox
    /// disabled entirely (commands run natively). The explicit opt-in replacement
    /// for hand-editing sandboxWindows=off + sandboxRequired=false.
    Danger,
    /// Plan mode: presents a plan first, executes after user confirmation
    Plan,
}

impl AgentMode {
    /// Whether this mode grants inherent full autonomy (everything auto-allowed,
    /// network gate bypassed, untrusted-MCP gate bypassed): `auto` and `danger`.
    /// Sandbox behavior differs (`auto` keeps the OS sandbox, `danger` disables it).
    pub fn is_full_autonomy(&self) -> bool {
        matches!(self, AgentMode::Auto | AgentMode::Danger)
    }
}

// ====== Custom Subagent Profiles ======

/// User-defined subagent profile (stored in custom-subagent-profiles.json)
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CustomSubagentProfile {
    /// User-chosen unique identifier (alphanumeric plus hyphens)
    pub id: String,
    /// Display name
    pub name: String,
    /// Purpose description
    pub description: String,
    /// Allowed tool list (glob patterns); empty = all
    pub allowed_tools: Vec<String>,
    /// Denied tool list (glob patterns); empty = none
    pub denied_tools: Vec<String>,
    /// Extra system prompt (appended after the subagent base system prompt)
    pub system_prompt_extension: String,
    /// Whether MCP tools are injected (servers selected by the main agent). Default off.
    #[serde(default)]
    pub allow_mcp_tools: bool,
}

/// On-disk format of custom-subagent-profiles.json
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CustomSubagentProfilesFile {
    pub version: u32,
    pub updated_at: String,
    pub profiles: Vec<CustomSubagentProfile>,
}

// ====== Agent task board (main-agent task list) ======

/// Status of one agent task (snake_case: pending / in_progress / completed).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentTaskStatus {
    Pending,
    InProgress,
    Completed,
}

/// One tracked task on the main agent's task board (per assistant+topic, memory-only).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AgentTask {
    /// Short generated id ("task-xxxxxxxx") used by update_task
    pub id: String,
    /// One-line task title
    pub title: String,
    /// Current status
    pub status: AgentTaskStatus,
}

// ====== ask_user (agent question to the user) ======

/// One answered question (sent back by the frontend via respond_agent_question).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct QuestionAnswer {
    /// Which question this answers
    pub question_id: String,
    /// Chosen option label, or the free-form custom answer
    pub answer: String,
    /// Whether the answer was typed as free-form text
    pub is_custom: bool,
}

/// Final user response for one ask_user request (skipped = user let the agent decide).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct QuestionResponse {
    /// true = the user skipped; answers is empty and the agent decides on its own
    pub skipped: bool,
    /// One answer per question, in question order
    pub answers: Vec<QuestionAnswer>,
}

// ====== Engine Scanner ======

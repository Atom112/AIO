//! Agent domain models: Agent mode enum, workflows, and custom subagent profiles.

use serde::{Deserialize, Serialize};

/// Agent execution mode.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum AgentMode {
    /// Chat mode (no agent features)
    #[default]
    Off,
    /// Normal mode: file write/delete requires user confirmation
    Normal,
    /// Auto mode: skips confirmation, executes autonomously
    Auto,
    /// Plan mode: presents a plan first, executes after user confirmation
    Plan,
    /// Workflow mode: forces task decomposition into a workflow and auto-executes it
    Workflow,
}

/// Workflow step status
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowStepStatus {
    Pending,
    Running,
    Completed,
    Failed,
    /// Harness round 5 (failure gating): never started because an earlier step
    /// failed and aborted the chain (step-level continueOnError escapes this).
    Skipped,
}

/// One step of a workflow
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStep {
    /// Step identifier ("step-1", "step-2", ...)
    pub step_id: String,
    /// Subagent profile ID
    pub profile_id: String,
    /// Step display name
    pub name: String,
    /// Task description (input to the subagent)
    pub task_description: String,
    /// Whether this step keeps the chain running when IT fails (escape hatch for
    /// intentionally best-effort steps; a failed step still aborts later steps
    /// unless this is true)
    #[serde(default)]
    pub continue_on_error: bool,
    /// Current status
    pub status: WorkflowStepStatus,
    /// Step execution result
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// Start timestamp (ms)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    /// Duration (ms)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<u64>,
}

/// Complete workflow definition
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    /// UUID
    pub workflow_id: String,
    /// Workflow title
    pub title: String,
    /// Step list (linear order)
    pub steps: Vec<WorkflowStep>,
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

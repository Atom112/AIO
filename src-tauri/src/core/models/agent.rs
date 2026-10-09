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

// ====== Engine Scanner ======

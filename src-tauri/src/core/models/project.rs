//! Project and usage domain: project index, token usage stats, and subagent profile model overrides.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Project definition: a logical grouping unit bound to a filesystem directory.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    /// Absolute path of the bound filesystem directory
    pub path: String,
    pub created_at: String,
    pub updated_at: String,
    /// Assistant ID for this project (auto-generated at project creation)
    #[serde(rename = "assistantId")]
    pub assistant_id: String,
}

/// Project persistence index file (app_data_dir/projects.json).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProjectsFile {
    pub version: u32,
    pub updated_at: String,
    pub projects: BTreeMap<String, Project>,
}

// ====== Token usage log ======

/// Usage summary: token usage aggregated by day.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    /// Date (YYYY-MM-DD)
    pub date: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_input_tokens: i64,
    pub request_count: i64,
}

/// Usage summary aggregated by model.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummaryByModel {
    pub model_id: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_input_tokens: i64,
    pub request_count: i64,
}
// ====== Per-Profile Model Override ======

/// Model override config for a subagent profile.
/// Lets each profile (explorer / coder / general) specify its own model and connection info,
/// so subagents can use a different provider than the main agent.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProfileModelOverride {
    /// Subagent profile id ("explorer" | "coder" | "general")
    pub profile_id: String,
    /// Overriding model ID
    pub model_id: String,
    /// Overriding API URL
    pub api_url: String,
    /// Overriding API key
    pub api_key: String,
}

/// On-disk format of profile-model-overrides.json
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProfileModelOverridesFile {
    pub version: u32,
    pub updated_at: String,
    pub overrides: Vec<ProfileModelOverride>,
}

// ====== Workflow (Agent Workflow) ======

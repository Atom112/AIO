//! 项目与用量域：项目索引、token 用量统计、子智能体 profile 模型覆盖。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 项目定义：绑定到文件系统目录的逻辑分组单元。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    /// 绑定的文件系统目录绝对路径
    pub path: String,
    pub created_at: String,
    pub updated_at: String,
    /// 该项目的对应助理 ID（创建项目时自动生成）
    #[serde(rename = "assistantId")]
    pub assistant_id: String,
}

/// 项目持久化索引文件（app_data_dir/projects.json）。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProjectsFile {
    pub version: u32,
    pub updated_at: String,
    pub projects: BTreeMap<String, Project>,
}

// ====== Token 用量日志 ======

/// 用量摘要：按天聚合的 token 用量。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    /// 日期（YYYY-MM-DD）
    pub date: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_input_tokens: i64,
    pub request_count: i64,
}

/// 按模型聚合的用量摘要。
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

/// 子智能体 profile 的模型覆盖配置。
/// 允许为每个 profile（explorer / coder / general）独立指定模型及连接信息，
/// 使子智能体可以使用与主 Agent 不同的 provider。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProfileModelOverride {
    /// 子智能体 profile 标识（"explorer" | "coder" | "general"）
    pub profile_id: String,
    /// 覆盖的模型 ID
    pub model_id: String,
    /// 覆盖的 API 地址
    pub api_url: String,
    /// 覆盖的 API Key
    pub api_key: String,
}

/// profile-model-overrides.json 的磁盘格式
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProfileModelOverridesFile {
    pub version: u32,
    pub updated_at: String,
    pub overrides: Vec<ProfileModelOverride>,
}

// ====== Workflow (Agent Workflow) ======

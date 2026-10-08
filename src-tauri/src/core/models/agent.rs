//! Agent 域模型：Agent 模式枚举、工作流与自定义子智能体 profile。

use serde::{Deserialize, Serialize};

/// Agent 执行模式。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum AgentMode {
    /// 对话模式（不使用 Agent 功能）
    #[default]
    Off,
    /// 普通模式：文件写入/删除需要用户确认
    Normal,
    /// 自动模式：跳过确认，自主执行
    Auto,
    /// 计划模式：先列出计划，用户确认后再执行
    Plan,
    /// 工作流模式：强制拆解任务为工作流并自动执行
    Workflow,
}

/// 工作流步骤状态
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowStepStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

/// 工作流中的一个步骤
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStep {
    /// 步骤标识（"step-1", "step-2", ...）
    pub step_id: String,
    /// 子 Agent profile ID
    pub profile_id: String,
    /// 步骤显示名称
    pub name: String,
    /// 任务描述（给子 Agent 的输入）
    pub task_description: String,
    /// 当前状态
    pub status: WorkflowStepStatus,
    /// 步骤执行结果
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// 开始时间戳（ms）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    /// 耗时（ms）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<u64>,
}

/// 完整工作流定义
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    /// UUID
    pub workflow_id: String,
    /// 工作流标题
    pub title: String,
    /// 步骤列表（线性顺序）
    pub steps: Vec<WorkflowStep>,
}

// ====== Custom Subagent Profiles ======

/// 用户自定义的子智能体配置文件（存储在 custom-subagent-profiles.json）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CustomSubagentProfile {
    /// 用户指定的唯一标识（字母数字短横线）
    pub id: String,
    /// 显示名称
    pub name: String,
    /// 用途说明
    pub description: String,
    /// 允许的工具列表（glob 模式），空 = 全部
    pub allowed_tools: Vec<String>,
    /// 禁止的工具列表（glob 模式），空 = 无
    pub denied_tools: Vec<String>,
    /// 附加的系统提示词（追加到子 Agent 基础系统提示词之后）
    pub system_prompt_extension: String,
    /// 是否注入 MCP 工具（主 Agent 勾选的 server）。默认关闭。
    #[serde(default)]
    pub allow_mcp_tools: bool,
}

/// custom-subagent-profiles.json 的磁盘格式
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CustomSubagentProfilesFile {
    pub version: u32,
    pub updated_at: String,
    pub profiles: Vec<CustomSubagentProfile>,
}

// ====== Engine Scanner ======

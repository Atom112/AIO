//! 聊天域模型：消息/话题/助手/模型与流式载荷（AgentStep 相关载荷在 chat 域）。

use crate::core::models::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 激活模型的连接配置信息。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ActivatedModel {
    pub api_url: String,
    #[serde(default)]
    pub api_key: String,
    pub model_id: String,
    pub owned_by: String,
    /// 可选的本地路径，仅在本地运行模式下使用。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    /// 本地推理引擎类型标识，如 "llama_cpp", "vllm"。
    /// 旧配置无此字段时反序列化为 None，逻辑上视为 legacy llama.cpp 行为。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_type: Option<String>,
}

/// 处理 SSE (Server-Sent Events) 流式输出时的消息负载。
#[derive(Serialize, Clone)]
pub struct StreamPayload {
    pub assistant_id: String,
    pub topic_id: String,
    pub content: String,
    pub done: bool,
    /// done=true 时携带的错误信息（整轮因错误/取消结束时填充）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 本轮输入 tokens（服务端返回，仅 done=true 时有意义）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u32>,
    /// 本轮输出 tokens（服务端返回，仅 done=true 时有意义）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u32>,
    /// 上下文峰值 tokens：最后一轮 API 调用的 input_tokens，代表实际上下文窗口使用量。
    /// 用于前端进度条展示，区别于 input_tokens（跨轮累计，用于成本统计）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u32>,
    /// 缓存命中的输入 tokens（仅 done=true 时有意义）。
    /// 来源：OpenAI `usage.prompt_tokens_details.cached_tokens`；DeepSeek `usage.prompt_cache_hit_tokens`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u32>,
    /// 模型生成的图像元数据（done=true 且有图时携带）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<GeneratedImage>>,
}

/// 新一轮 LLM 调用开始时通知前端，前端据此 push 一条空 assistant 占位消息。
#[derive(Serialize, Clone)]
pub struct RoundStartPayload {
    pub assistant_id: String,
    pub topic_id: String,
    /// 第几轮（从 1 开始）
    pub round: u32,
}

/// 单个工具执行结果（后端执行完 MCP 工具后 emit，前端据此更新气泡状态并追加 role:tool 消息）。
#[derive(Serialize, Clone)]
pub struct ToolResultPayload {
    pub assistant_id: String,
    pub topic_id: String,
    pub tool_call_id: String,
    pub name: String,
    /// 给前端展示用的纯文本结果
    pub content: String,
    /// 原始结构化结果（ToolResultContent 数组）
    pub result: serde_json::Value,
    /// 是否为错误
    pub is_error: bool,
    /// 本次工具调用产生的文件变更（write_file/replace_in_file/delete_file 时非空）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_changes: Option<Vec<FileChange>>,
    /// 完整工具结果（前端 agentSteps 展示用，不截断）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_content: Option<String>,
}

/// 流式 tool_call 累积载荷（发往前端用，仅用于通知前端展示"调用中"气泡）。
///
/// 位于 core 层：plugins/provider（Anthropic 流式适配）需要构造它，
/// 而插件不应依赖命令层。
#[derive(Serialize, Clone)]
pub struct ToolCallPayload {
    pub assistant_id: String,
    pub topic_id: String,
    pub tool_call_id: String,
    pub name: String,
    pub arguments: String,
}

/// 单次文件修改记录（供前端展示 diff 预览 + 跳转 + 回滚）
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    /// 文件路径（相对于项目根目录）
    pub file_path: String,
    /// 操作类型: create | modify | delete
    pub action: String,
    /// unified diff 字符串（git diff 格式，含行号上下文）
    pub diff: String,
    /// 受影响行数概览（"+N -M"）
    pub summary: String,
}

/// 从 provider 实时拉取的单个模型信息（OpenAI-兼容 /v1/models 或厂商自定义端点）。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LiveModel {
    pub id: String,
    #[serde(default)]
    pub owned_by: String,
    /// 厂商返回的展示名（Google/Anthropic 有；OpenAI/Ollama 无 → None）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// 发布日期 YYYY-MM-DD（OpenAI 的 `created` 转；Anthropic 的 `created_at` 原样；Google/Ollama 无 → None）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub released_at: Option<String>,
}

/// 消息关联的附件元数据。旧记录只有 `name`，其余字段保持可选以兼容历史数据。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FileMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// 附件在磁盘上的绝对路径，用于前端渲染图片缩略图（convertFileSrc 资产 URL）；仅新数据可能携带
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_path: Option<String>,
}

/// 上传到应用附件目录后的完整元数据。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StoredAttachment {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size: u64,
    pub storage_path: String,
}

/// 模型生成的图像元数据（assistant 消息）。用于前端下载按钮与历史回放（重新展开为 image_url 块）。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedImage {
    pub name: String,
    pub mime_type: String,
    pub size: u64,
    pub storage_path: String,
}

/// 单条聊天消息模型。
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: Option<String>,
    pub role: String,
    pub content: serde_json::Value,
    #[serde(rename = "modelId", skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(rename = "displayFiles", skip_serializing_if = "Option::is_none")]
    pub display_files: Option<Vec<FileMeta>>,
    #[serde(rename = "displayText", skip_serializing_if = "Option::is_none")]
    pub display_text: Option<String>,
    /// role="tool" 时对应触发的 tool_call id
    #[serde(
        rename = "toolCallId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub tool_call_id: Option<String>,
    /// role="tool" 时为被调用的函数名；role="assistant" 携带 tool_calls 时为 "assistant"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// role="assistant" 时携带模型发起的工具调用请求（OpenAI 兼容格式）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// 模型原生思维链（GLM/DeepSeek-R1/Qwen3 等的 reasoning_content），仅 assistant 消息可能携带
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// 本轮输入 tokens（服务端返回，仅 assistant 消息有意义；旧数据缺省为 None）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u32>,
    /// 本轮输出 tokens（服务端返回，仅 assistant 消息有意义；旧数据缺省为 None）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u32>,
    /// 本轮缓存命中的输入 tokens（服务端返回，仅 assistant 消息有意义；旧数据缺省为 None）。
    /// 来源：OpenAI `usage.prompt_tokens_details.cached_tokens`；DeepSeek `usage.prompt_cache_hit_tokens`；
    /// Anthropic `message_start.usage.cache_read_input_tokens`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u32>,
    /// Agent 工作过程步骤（JSON 序列化的 AgentStep[]），跨重启持久化
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_steps: Option<serde_json::Value>,
    /// Agent 执行中的中间内容（流式文本暂存），跨重启持久化
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interim_content: Option<String>,
    /// Agent 开始执行时间戳（毫秒），跨重启持久化
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_start_time: Option<i64>,
    /// 父消息 ID（用于会话分支树），NULL = 主题根消息
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_message_id: Option<String>,
    /// 分支索引：从同一父消息分叉时递增（0 = 原始/主干）
    #[serde(default)]
    pub branch_index: i32,
    /// 模型生成的图像元数据（仅 assistant 消息），用于下载与历史回放
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<GeneratedImage>>,
    /// 完整工具执行结果（LLM 上下文中只包含截断版），仅 role=tool 消息有效，会话级内存字段不持久化到 SQLite
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_tool_result: Option<String>,
}

/// OpenAI 风格的工具调用（assistant 消息中）
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolCallFunction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "fileChanges")]
    pub file_changes: Option<Vec<FileChange>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToolCallFunction {
    pub name: String,
    /// JSON 字符串（OpenAI 规范要求）
    pub arguments: String,
}

/// OpenAI 风格的工具规范（在 `tools` 数组中发送给 LLM）
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToolSpec {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolFunctionSpec,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToolFunctionSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema
    pub parameters: serde_json::Value,
}

/// 按助手视角聚合的 MCP 工具集：扁平 `tools` 喂给 LLM，`tool_server_map` 供前端解析 toolName → serverId。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AssistantTools {
    pub tools: Vec<ToolSpec>,
    /// toolName → serverId（call_mcp_tool 时用，替代前端启发式查找）
    pub tool_server_map: HashMap<String, String>,
}

/// MCP 工具调用结果
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    pub content: Vec<ToolResultContent>,
    #[serde(default)]
    pub is_error: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToolResultContent {
    #[serde(rename = "type")]
    pub kind: String, // "text" | "image" | "resource"
    #[serde(flatten)]
    pub data: serde_json::Value,
}

// （已删除：ToolCallDelta / ToolCallFunctionDelta —— 零引用的流式增量类型，
//   实际流式累积使用 commands/llm 内部的 ToolCallAccum）

/// 包含历史记录的对话主题模型。
#[derive(Serialize, Deserialize, Clone)]
pub struct Topic {
    pub id: String,
    pub name: String,
    pub history: Vec<Message>,
    #[serde(default)]
    pub summary: Option<String>,
    /// 是否已经自动重命名过。新建话题为 `false`；首次对话结束后由前端置为 `true`，
    /// 之后不再触发自动重命名。旧配置 / 旧数据库行反序列化为 `false`，
    /// 由数据迁移在加载时统一修复。
    #[serde(default)]
    pub renamed: bool,
    /// 分支来源消息 ID（此话题从哪条消息分支而来）
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "branchedFromMessageId"
    )]
    pub branched_from_message_id: Option<String>,
    /// 父话题 ID（用于话题树结构）
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "parentTopicId"
    )]
    pub parent_topic_id: Option<String>,
}

/// AI 助手预设模型，包含系统提示词和相关的对话列表。
#[derive(Serialize, Deserialize, Clone)]
pub struct Assistant {
    pub id: String,
    pub name: String,
    pub prompt: String,
    /// 助手绑定的首选模型 ID（可选）。未设置时由前端回退到全局默认模型。
    /// 旧配置 / 旧数据库行反序列化为 None，逻辑上视为「使用全局默认」。
    #[serde(rename = "modelId", default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    /// 助手启用的 MCP server id 列表；空数组 = 该助手不使用任何 MCP 工具（opt-in 语义）。
    /// 旧数据库行 mcp_server_ids 列为 NULL → 反序列化为空 vec，等价于「未启用 MCP」。
    #[serde(
        rename = "mcpServerIds",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub mcp_server_ids: Vec<String>,
    /// 助手启用的 Skill id 列表；空数组表示不注入任何 Skill 指令。
    #[serde(rename = "skillIds", default, skip_serializing_if = "Vec::is_empty")]
    pub skill_ids: Vec<String>,
    /// 助手所属的项目 ID。None = 全局助手（不属于任何项目）。
    #[serde(rename = "projectId", default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Agent 执行模式。Off = 对话模式。旧数据缺省反序列化为 Off。
    #[serde(rename = "agentMode", default)]
    pub agent_mode: AgentMode,
    /// 助理类型（chat = 对话模式专属，project = 项目助理）
    /// 旧数据缺省反序列化为 "project"，再通过迁移将 default-assistant-id 修正为 "chat"。
    #[serde(rename = "assistantType", default = "default_assistant_type")]
    pub assistant_type: String,
    #[serde(default)]
    pub topics: Vec<Topic>,
}

/// 远程 API 返回的单个模型基础信息。
#[derive(Serialize, Deserialize, Clone)]
pub struct ModelInfo {
    pub id: String,
    pub owned_by: Option<String>,
}

/// 兼容 OpenAI 格式的模型列表响应。
#[derive(Serialize, Deserialize, Clone)]
pub struct ModelsResponse {
    pub data: Vec<ModelInfo>,
}

/// 旧数据无 assistant_type 时默认为 "project"
fn default_assistant_type() -> String {
    "project".into()
}

/// 定义各种数据模型，包括激活模型配置、消息结构、对话主题、AI 助手预设、远程模型信息以及全局应用配置。
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

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

/// 流式传输时携带的工具调用增量（累积 delta 后转成完整 ToolCall）
#[allow(dead_code)]
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToolCallDelta {
    pub index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function: Option<ToolCallFunctionDelta>,
}

#[allow(dead_code)]
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ToolCallFunctionDelta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

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

/// 应用程序全局配置。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AppConfig {
    #[serde(rename = "apiUrl")]
    pub api_url: String,
    #[serde(rename = "apiKey")]
    pub api_key: String,
    #[serde(rename = "defaultModel")]
    pub default_model: String,
    #[serde(rename = "localModelPath", default)]
    pub local_model_path: String,
    /// 工具调用失败时是否自动重试（默认 true）
    #[serde(default = "default_auto_retry_enabled")]
    pub auto_retry_enabled: bool,
    /// 自动重试次数上限（默认 2）
    #[serde(default = "default_auto_retry_count")]
    pub auto_retry_count: u32,
    /// 每次重试间隔（毫秒，默认 500）
    #[serde(default = "default_auto_retry_delay_ms")]
    pub auto_retry_delay_ms: u64,
    /// 跨会话记忆（项目知识持久化）。默认关闭，用户可在设置中开启。
    #[serde(default, rename = "knowledgeEnabled")]
    pub knowledge_enabled: bool,
    /// 系统自启。默认关闭。
    #[serde(default, rename = "autoStartEnabled")]
    pub auto_start_enabled: bool,
    /// 最大并发子智能体数量（None = 使用默认值 5）。用于限制 delegate_tasks 和多个 delegate_task 的并发数。
    #[serde(default, rename = "maxConcurrentSubagents")]
    pub max_concurrent_subagents: Option<u32>,
}

fn default_auto_retry_enabled() -> bool {
    true
}
fn default_auto_retry_count() -> u32 {
    2
}
fn default_auto_retry_delay_ms() -> u64 {
    500
}

// ====== MCP 服务器配置 ======

/// MCP 传输方式。serde tag = "transport"，按 transport 字段分发。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "transport", rename_all = "lowercase")]
pub enum McpTransport {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        /// 密钥值支持 `${KEYRING:account_id}` 占位（运行时反向解析）
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    Http {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
    StreamableHttp {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
}

/// MCP server 配置
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct McpServerConfig {
    pub id: String,
    pub display_name: String,
    pub transport: McpTransport,
    /// 工具白名单；空数组 = 全部启用
    #[serde(default)]
    pub enabled_tools: Vec<String>,
    /// 应用启动时是否自动连接（是否被某助手使用由 Assistant.mcp_server_ids 决定）
    #[serde(default)]
    pub auto_start: bool,
    /// 提示 UI 是否存在密钥存于 keyring
    #[serde(default)]
    pub has_stored_secret: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_catalog: Option<CatalogRef>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CatalogRef {
    pub catalog_id: String,
    pub source_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct McpCatalogInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub secret: bool,
    #[serde(default)]
    pub default_value: String,
    #[serde(default)]
    pub target: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct McpCatalogDelivery {
    pub id: String,
    pub kind: String,
    pub label: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub inputs: Vec<McpCatalogInput>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct McpCatalogServer {
    pub id: String,
    pub name: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub repository_url: String,
    #[serde(default)]
    pub website_url: String,
    #[serde(default)]
    pub deliveries: Vec<McpCatalogDelivery>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpCatalogPage {
    #[serde(default)]
    pub servers: Vec<McpCatalogServer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct McpCatalogInstallRequest {
    pub server: McpCatalogServer,
    pub delivery_id: String,
    #[serde(default)]
    pub values: BTreeMap<String, String>,
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
}

/// MCP server 运行时状态
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum McpStatus {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Error,
}

/// MCP server 状态信息（暴露给前端）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct McpServerStatusInfo {
    pub id: String,
    pub status: McpStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default)]
    pub tool_count: usize,
    #[serde(default)]
    pub resource_count: usize,
    #[serde(default)]
    pub prompt_count: usize,
}

/// MCP server 持久化文件
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct McpServersFile {
    pub version: u32,
    pub updated_at: String,
    pub servers: BTreeMap<String, McpServerConfig>,
}

impl Default for McpServersFile {
    fn default() -> Self {
        Self {
            version: 1,
            updated_at: String::new(),
            servers: BTreeMap::new(),
        }
    }
}

// ====== Skill 配置 ======

/// Skill 来源类型。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum SkillSource {
    /// 手动创建或旧数据（缺省值）
    #[default]
    Local,
    /// 从 skills.sh 市场下载
    Market,
    /// 从 npm/npx 生态导入
    Npx,
}

/// 可复用的助手系统指令模块。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkillConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub content: String,
    /// Skill 来源。旧数据缺省反序列化为 Local。
    #[serde(default)]
    pub source: SkillSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_slug: Option<String>,
    #[serde(default)]
    pub installs: u64,
    /// npx 包名（source = Npx 时必填），如 "@anthropic-ai/skill-docx"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npx_package: Option<String>,
    /// npx 包的已安装版本号
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npx_version: Option<String>,
    /// npx 执行命令（默认等于 npx_package）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npx_command: Option<String>,
}

/// Skill 持久化文件。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkillsFile {
    pub version: u32,
    pub updated_at: String,
    pub skills: BTreeMap<String, SkillConfig>,
}

impl Default for SkillsFile {
    fn default() -> Self {
        Self {
            version: 1,
            updated_at: String::new(),
            skills: BTreeMap::new(),
        }
    }
}

/// skills.sh 市场中的 Skill 摘要。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MarketSkill {
    pub id: String,
    pub name: String,
    pub owner: String,
    pub repo: String,
    pub slug: String,
    #[serde(default)]
    pub description: String,
    pub source_url: String,
    #[serde(default)]
    pub installs: u64,
    #[serde(default)]
    pub installs_label: String,
    #[serde(default)]
    pub weekly_installs: Vec<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

/// skills.sh 官方主题分类。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkillMarketCategory {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub skill_count: usize,
}

// ====== Agent 模式配置 ======

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

/// 旧数据无 assistant_type 时默认为 "project"
fn default_assistant_type() -> String {
    "project".into()
}

/// MCP server 初始化握手返回的服务端信息
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct McpServerInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// 服务端声明的能力（tools / resources / prompts）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<McpCapabilities>,
}

/// MCP 服务端能力声明（从 initialize 响应解析）
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompts: Option<serde_json::Value>,
}

// ====== MCP Resources ======

/// MCP 资源元数据（resources/list 返回条目）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

/// resources/read 返回值
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReadResourceResult {
    pub contents: Vec<ResourceContent>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ResourceContent {
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
}

// ====== MCP Prompts ======

/// MCP 提示词元数据（prompts/list 返回条目）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct McpPrompt {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Vec<PromptArgument>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PromptArgument {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

/// prompts/get 返回值
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GetPromptResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub messages: Vec<PromptMessage>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PromptMessage {
    pub role: String,
    pub content: serde_json::Value,
}

// ====== 项目配置 ======

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
    pub request_count: i64,
}

/// 按模型聚合的用量摘要。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummaryByModel {
    pub model_id: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
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

/// 引擎安装检测结果：是否已安装在系统上及版本号。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EngineInstallInfo {
    pub installed: bool,
    pub version: Option<String>,
}

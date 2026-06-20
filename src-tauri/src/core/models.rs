/// 定义各种数据模型，包括激活模型配置、消息结构、对话主题、AI 助手预设、远程模型信息以及全局应用配置。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 激活模型的连接配置信息。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ActivatedModel {
    pub api_url: String,
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
}

/// 从 provider 实时拉取的单个模型信息（OpenAI-兼容 /v1/models 或厂商自定义端点）。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LiveModel {
    pub id: String,
    pub owned_by: String,
    /// 厂商返回的展示名（Google/Anthropic 有；OpenAI/Ollama 无 → None）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// 发布日期 YYYY-MM-DD（OpenAI 的 `created` 转；Anthropic 的 `created_at` 原样；Google/Ollama 无 → None）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub released_at: Option<String>,
}

/// 消息中包含的附件元数据。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FileMeta {
    pub name: String,
}

/// 单条聊天消息模型。
#[derive(Serialize, Deserialize, Clone)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
}

/// OpenAI 风格的工具调用（assistant 消息中）
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolCallFunction,
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

/// MCP 工具调用结果
#[derive(Serialize, Deserialize, Clone, Debug)]
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
#[derive(Serialize, Deserialize, Debug)]
pub struct AppConfig {
    #[serde(rename = "apiUrl")]
    pub api_url: String,
    #[serde(rename = "apiKey")]
    pub api_key: String,
    #[serde(rename = "defaultModel")]
    pub default_model: String,
    #[serde(rename = "localModelPath", default)]
    pub local_model_path: String,
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
    pub enabled: bool,
    pub transport: McpTransport,
    /// 工具白名单；空数组 = 全部启用
    #[serde(default)]
    pub enabled_tools: Vec<String>,
    #[serde(default)]
    pub auto_start: bool,
    /// 提示 UI 是否存在密钥存于 keyring
    #[serde(default)]
    pub has_stored_secret: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_catalog: Option<CatalogRef>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CatalogRef {
    pub catalog_id: String,
    pub source_id: String,
}

/// MCP server 运行时状态
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpStatus {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

impl Default for McpStatus {
    fn default() -> Self {
        Self::Disconnected
    }
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

/// MCP server 初始化握手返回的服务端信息
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct McpServerInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

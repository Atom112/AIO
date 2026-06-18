/// 定义各种数据模型，包括激活模型配置、消息结构、对话主题、AI 助手预设、远程模型信息以及全局应用配置。

use serde::{Deserialize, Serialize};

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
}

/// 包含历史记录的对话主题模型。
#[derive(Serialize, Deserialize, Clone)]
pub struct Topic {
    pub id: String,
    pub name: String,
    pub history: Vec<Message>,
    #[serde(default)]
    pub summary: Option<String>,
}

/// AI 助手预设模型，包含系统提示词和相关的对话列表。
#[derive(Serialize, Deserialize, Clone)]
pub struct Assistant {
    pub id: String,
    pub name: String,
    pub prompt: String,
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

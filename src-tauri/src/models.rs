//! # 数据模型与协议定义
//!
//! **功能描述**:
//! 定义了应用程序中所有核心协议结构体，包括 AI 助手配置、对话层级（Topic/Message）以及前端所需的 JSON 交互模型。
//!
//! **数据流流向**:
//! 1. **反序列化 (Inbound)**: 接收来自前端的请求参数或从数据库读取的 JSON 字符串，转化为 Rust 结构体。
//! 2. **序列化 (Outbound)**: 将 Rust 对象（如 AI 的回复、加载的配置）序列化为 JSON 传回前端或写入本地存储。
//! 3. **网络传输**: `ModelsResponse` 等结构映射了外部 LLM API 的 API 响应格式。

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
}

/// 处理 SSE (Server-Sent Events) 流式输出时的消息负载。
#[derive(Serialize, Clone)]
pub struct StreamPayload {
    pub assistant_id: String,
    pub topic_id: String,
    pub content: String,
    pub done: bool,
}

/// 消息中包含的附件元数据。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FileMeta {
    pub name: String,
}

/// 单条聊天消息模型。
#[derive(Serialize, Deserialize, Clone)]
pub struct Message {
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
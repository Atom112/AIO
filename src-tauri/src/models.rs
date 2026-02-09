//! # 数据模型模块
//!
//! 该模块定义了应用程序中使用的核心数据结构，涵盖了从 AI 模型配置、
//! 聊天消息历史、会话管理到应用程序全局配置的所有序列化和反序列化模型。
//!
//! 主要配合 `serde` 库进行 JSON 数据解析，适用于 Tauri 前后端通信。

use serde::{Deserialize, Serialize};

/// 代表当前已激活并准备使用的 AI 模型配置。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ActivatedModel {
    /// 模型的 API 端点地址。
    pub api_url: String,
    /// 授权使用的 API 密钥。
    pub api_key: String,
    /// 模型的唯一标识符（如 "gpt-4"）。
    pub model_id: String,
    /// 模型的所有方或提供商。
    pub owned_by: String,
    /// 本地模型路径（仅针对本地运行的模型）。
    /// 如果为 `None`，在序列化时将跳过此字段。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
}

/// 流式输出时发送给前端的数据负载。
#[derive(Serialize, Clone)]
pub struct StreamPayload {
    /// 关联的助手 ID。
    pub assistant_id: String,
    /// 关联的对话主题 ID。
    pub topic_id: String,
    /// 当前推送的文本片段内容。
    pub content: String,
    /// 标识流式输出是否已经结束。
    pub done: bool,
}

/// 文件的元数据信息。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FileMeta {
    /// 文件名。
    pub name: String,
}

/// 代表对话中的一条消息。
#[derive(Serialize, Deserialize, Clone)]
pub struct Message {
    /// 消息发送者的角色（如 "user"、"assistant" 或 "system"）。
    pub role: String,
    /// 消息的正文内容。
    pub content: serde_json::Value,
    #[serde(rename = "modelId", skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    /// 在 UI 上显示的附件文件列表。
    /// 映射自前端的 `displayFiles` 字段。
    #[serde(rename = "displayFiles", skip_serializing_if = "Option::is_none")]
    pub display_files: Option<Vec<FileMeta>>,
    /// 经过处理或格式化后用于显示的文本内容。
    /// 映射自前端的 `displayText` 字段。
    #[serde(rename = "displayText", skip_serializing_if = "Option::is_none")]
    pub display_text: Option<String>,
}

/// 代表一个具体的对话主题或会话线程。
#[derive(Serialize, Deserialize, Clone)]
pub struct Topic {
    /// 会话的唯一 ID。
    pub id: String,
    /// 会话的标题或名称。
    pub name: String,
    /// 属于该主题的历史消息列表。
    pub history: Vec<Message>,
    #[serde(default)]
    pub summary: Option<String>,
}

/// 代表一个 AI 助手预设。
///
/// 助手包含特定的回复逻辑（提示词）以及独立的对话列表。
#[derive(Serialize, Deserialize, Clone)]
pub struct Assistant {
    /// 助手的唯一 ID。
    pub id: String,
    /// 助手的名称。
    pub name: String,
    /// 系统级提示词（System Prompt），定义助手的行为准则。
    pub prompt: String,
    /// 该助手下属的所有对话主题。
    #[serde(default)]
    pub topics: Vec<Topic>,
}

/// 从远程 API 获取的模型基础信息。
#[derive(Serialize, Deserialize, Clone)]
pub struct ModelInfo {
    /// 模型 ID。
    pub id: String,
    /// 模型的所有方。
    pub owned_by: Option<String>,
}

/// 远程 API 获取模型列表的响应包装结构。
#[derive(Serialize, Deserialize, Clone)]
pub struct ModelsResponse {
    /// 包含所有可选模型的列表。
    pub data: Vec<ModelInfo>,
}

/// 应用程序的全局配置文件结构。
#[derive(Serialize, Deserialize, Debug)]
pub struct AppConfig {
    /// 默认的 API 服务基地址。
    #[serde(rename = "apiUrl")]
    pub api_url: String,
    /// 默认的 API 访问授权密钥。
    #[serde(rename = "apiKey")]
    pub api_key: String,
    /// 默认选中的模型 ID。
    #[serde(rename = "defaultModel")]
    pub default_model: String,
    /// 本地模型的存放路径，默认为空字符串。
    #[serde(rename = "localModelPath", default)]
    pub local_model_path: String,
}

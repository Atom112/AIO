//! Chat domain models: messages/topics/assistants/models and streaming payloads (AgentStep payloads live in the chat domain).

use crate::core::models::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Connection configuration for an activated model.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ActivatedModel {
    pub api_url: String,
    #[serde(default)]
    pub api_key: String,
    pub model_id: String,
    pub owned_by: String,
    /// Optional local path, only used in local-run mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    /// Local inference engine type identifier, e.g. "llama_cpp", "vllm".
    /// Old configs without this field deserialize to None, treated as legacy llama.cpp behavior.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_type: Option<String>,
}

/// Message payload for SSE (Server-Sent Events) streaming output.
#[derive(Serialize, Clone)]
pub struct StreamPayload {
    pub assistant_id: String,
    pub topic_id: String,
    pub content: String,
    pub done: bool,
    /// Error message carried when done=true (set when the whole round ends in error or cancellation)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Input tokens for this round (server-reported; only meaningful when done=true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u32>,
    /// Output tokens for this round (server-reported; only meaningful when done=true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u32>,
    /// Peak context tokens: input_tokens of the last API call, i.e. actual context window usage.
    /// Drives the frontend progress bar; distinct from input_tokens (cumulative across rounds, for cost stats).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u32>,
    /// Cache-hit input tokens (only meaningful when done=true).
    /// Source: OpenAI `usage.prompt_tokens_details.cached_tokens`; DeepSeek `usage.prompt_cache_hit_tokens`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u32>,
    /// Metadata of model-generated images (carried when done=true and images exist)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<GeneratedImage>>,
}

/// Notifies the frontend that a new LLM round started, so it pushes an empty assistant placeholder message.
#[derive(Serialize, Clone)]
pub struct RoundStartPayload {
    pub assistant_id: String,
    pub topic_id: String,
    /// Round number (1-based)
    pub round: u32,
}

/// Single tool execution result (emitted after the backend runs an MCP tool; the frontend updates the bubble status and appends a role:tool message).
#[derive(Serialize, Clone)]
pub struct ToolResultPayload {
    pub assistant_id: String,
    pub topic_id: String,
    pub tool_call_id: String,
    pub name: String,
    /// Plain-text result for frontend display
    pub content: String,
    /// Raw structured result (ToolResultContent array)
    pub result: serde_json::Value,
    /// Whether this is an error
    pub is_error: bool,
    /// File changes produced by this tool call (non-empty for write_file/replace_in_file/delete_file)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_changes: Option<Vec<FileChange>>,
    /// Full tool result (for frontend agentSteps display, untruncated)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_content: Option<String>,
}

/// Streaming tool_call accumulation payload (sent to the frontend only to drive the "calling" bubble).
///
/// Lives in the core layer: plugins/provider (Anthropic streaming adapter) must construct it,
/// and plugins must not depend on the command layer.
#[derive(Serialize, Clone)]
pub struct ToolCallPayload {
    pub assistant_id: String,
    pub topic_id: String,
    pub tool_call_id: String,
    pub name: String,
    pub arguments: String,
}

/// Single file modification record (frontend uses it for diff preview, jump-to-file, and rollback)
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    /// File path (relative to the project root)
    pub file_path: String,
    /// Action type: create | modify | delete
    pub action: String,
    /// Unified diff string (git diff format, with line-number context)
    pub diff: String,
    /// Affected-line overview ("+N -M")
    pub summary: String,
}

/// A single model fetched live from a provider (OpenAI-compatible /v1/models or a vendor-specific endpoint).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LiveModel {
    pub id: String,
    #[serde(default)]
    pub owned_by: String,
    /// Vendor-reported display name (present for Google/Anthropic; absent for OpenAI/Ollama → None)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Release date YYYY-MM-DD (converted from OpenAI `created`; Anthropic `created_at` as-is; absent for Google/Ollama → None)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub released_at: Option<String>,
}

/// Attachment metadata for a message. Legacy records carry only `name`; other fields stay optional for backward compatibility.
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
    /// Absolute on-disk path of the attachment, used by the frontend to render image thumbnails (convertFileSrc asset URL); only newer records carry it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_path: Option<String>,
}

/// Full metadata after uploading into the app attachment directory.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StoredAttachment {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size: u64,
    pub storage_path: String,
}

/// Metadata of a model-generated image (assistant message). Used for the frontend download button and history replay (re-expanded into an image_url block).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedImage {
    pub name: String,
    pub mime_type: String,
    pub size: u64,
    pub storage_path: String,
}

/// A single chat message.
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
    /// Tool_call id that produced this message when role="tool"
    #[serde(
        rename = "toolCallId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub tool_call_id: Option<String>,
    /// Invoked function name when role="tool"; "assistant" when role="assistant" carries tool_calls
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Tool call requests initiated by the model when role="assistant" (OpenAI-compatible format)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Model-native chain of thought (reasoning_content from GLM/DeepSeek-R1/Qwen3 etc.); only assistant messages may carry it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// Input tokens for this round (server-reported; only meaningful on assistant messages; defaults to None for legacy data)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u32>,
    /// Output tokens for this round (server-reported; only meaningful on assistant messages; defaults to None for legacy data)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u32>,
    /// Cache-hit input tokens for this round (server-reported; only meaningful on assistant messages; legacy default None).
    /// Source: OpenAI `usage.prompt_tokens_details.cached_tokens`; DeepSeek `usage.prompt_cache_hit_tokens`;
    /// Anthropic `message_start.usage.cache_read_input_tokens`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u32>,
    /// Agent work steps (JSON-serialized AgentStep[]), persisted across restarts
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_steps: Option<serde_json::Value>,
    /// Interim content during agent execution (streaming text staging), persisted across restarts
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interim_content: Option<String>,
    /// Agent start timestamp (milliseconds), persisted across restarts
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_start_time: Option<i64>,
    /// Parent message ID (for the conversation branch tree); NULL = topic root message
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_message_id: Option<String>,
    /// Branch index: increments when branching from the same parent message (0 = original/trunk)
    #[serde(default)]
    pub branch_index: i32,
    /// Model-generated image metadata (assistant messages only), used for download and history replay
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<GeneratedImage>>,
    /// Full tool execution result (the LLM context only holds the truncated version); valid only for role=tool messages; session-memory field, not persisted to SQLite
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_tool_result: Option<String>,
}

/// OpenAI-style tool call (in assistant messages)
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
    /// JSON string (required by the OpenAI spec)
    pub arguments: String,
}

/// OpenAI-style tool spec (sent to the LLM in the `tools` array)
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

/// MCP tools aggregated per assistant: the flat `tools` list feeds the LLM; `tool_server_map` resolves toolName → serverId for the frontend.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AssistantTools {
    pub tools: Vec<ToolSpec>,
    /// toolName → serverId (used by call_mcp_tool instead of frontend heuristic lookup)
    pub tool_server_map: HashMap<String, String>,
}

/// MCP tool invocation result
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

// Streaming tool_call accumulation is centralized in commands/llm's ToolCallAccum.

/// A conversation topic with its message history.
#[derive(Serialize, Deserialize, Clone)]
pub struct Topic {
    pub id: String,
    pub name: String,
    pub history: Vec<Message>,
    #[serde(default)]
    pub summary: Option<String>,
    /// Whether the topic has been auto-renamed. New topics are `false`; the frontend sets `true`
    /// after the first conversation ends, disabling further auto-rename. Legacy configs / DB rows
    /// deserialize as `false` and are fixed up by data migration at load time.
    #[serde(default)]
    pub renamed: bool,
    /// Source message ID of the branch (which message this topic branched from)
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "branchedFromMessageId"
    )]
    pub branched_from_message_id: Option<String>,
    /// Parent topic ID (for the topic tree structure)
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "parentTopicId"
    )]
    pub parent_topic_id: Option<String>,
}

/// AI assistant preset with a system prompt and its conversation topics.
#[derive(Serialize, Deserialize, Clone)]
pub struct Assistant {
    pub id: String,
    pub name: String,
    pub prompt: String,
    /// Preferred model ID bound to the assistant (optional). When unset, the frontend falls back to the global default model.
    /// Legacy configs / DB rows deserialize to None, meaning "use the global default".
    #[serde(rename = "modelId", default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    /// MCP server ids enabled for the assistant; an empty array = the assistant uses no MCP tools (opt-in semantics).
    /// Legacy DB rows with NULL mcp_server_ids deserialize to an empty vec, equivalent to "MCP disabled".
    #[serde(
        rename = "mcpServerIds",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub mcp_server_ids: Vec<String>,
    /// Skill ids enabled for the assistant; an empty array means no Skill instructions are injected.
    #[serde(rename = "skillIds", default, skip_serializing_if = "Vec::is_empty")]
    pub skill_ids: Vec<String>,
    /// Project the assistant belongs to. None = global assistant (not tied to any project).
    #[serde(rename = "projectId", default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Agent execution mode. Off = plain chat mode. Legacy data defaults to Off.
    #[serde(rename = "agentMode", default)]
    pub agent_mode: AgentMode,
    /// Assistant type (chat = chat-mode only, project = project assistant).
    /// Legacy data defaults to "project"; migration fixes default-assistant-id to "chat".
    #[serde(rename = "assistantType", default = "default_assistant_type")]
    pub assistant_type: String,
    #[serde(default)]
    pub topics: Vec<Topic>,
}

/// Basic info of a single model returned by a remote API.
#[derive(Serialize, Deserialize, Clone)]
pub struct ModelInfo {
    pub id: String,
    pub owned_by: Option<String>,
}

/// OpenAI-compatible model list response.
#[derive(Serialize, Deserialize, Clone)]
pub struct ModelsResponse {
    pub data: Vec<ModelInfo>,
}

/// Defaults to "project" when legacy data lacks assistant_type
fn default_assistant_type() -> String {
    "project".into()
}

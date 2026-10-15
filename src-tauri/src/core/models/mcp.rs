//! MCP domain models: server config, catalog marketplace, status, and resource/prompt protocol types.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// MCP transport. serde tag = "transport"; dispatch on the transport field.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "transport", rename_all = "lowercase")]
pub enum McpTransport {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        /// Secret values support the `${KEYRING:account_id}` placeholder (resolved back at runtime)
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

/// MCP server configuration
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct McpServerConfig {
    pub id: String,
    pub display_name: String,
    pub transport: McpTransport,
    /// Tool allowlist; empty array = all enabled
    #[serde(default)]
    pub enabled_tools: Vec<String>,
    /// Whether to auto-connect at app startup (whether an assistant uses it is decided by Assistant.mcp_server_ids)
    #[serde(default)]
    pub auto_start: bool,
    /// Hints the UI that a secret is stored in the keyring
    #[serde(default)]
    pub has_stored_secret: bool,
    /// Whether trusted: tools of untrusted servers require per-call approval even in Auto/Workflow mode
    /// (prevents autonomous mode from blanket-approving unknown external tools). Default false; opted in via server settings.
    #[serde(default)]
    pub trusted: bool,
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
    /// Install scope: `Some(project_id)` writes the server into the project's
    /// `.aio/mcp-servers.json` (and records trust — the install dialog click IS the
    /// user authorization); `None` installs to the global config.
    #[serde(default)]
    pub project_id: Option<String>,
}

/// MCP server runtime status
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

/// MCP server status info (exposed to the frontend)
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

/// MCP server persistence file
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

// ====== Skill configuration ======

/// Server info returned by the MCP initialize handshake
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct McpServerInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Capabilities declared by the server (tools / resources / prompts)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<McpCapabilities>,
}

/// MCP server capability declaration (parsed from the initialize response)
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

/// MCP resource metadata (entry returned by resources/list)
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

/// Return value of resources/read
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

/// MCP prompt metadata (entry returned by prompts/list)
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

/// Return value of prompts/get
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

// ====== Project configuration ======

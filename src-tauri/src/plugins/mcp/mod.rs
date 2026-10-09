//! MCP plugin system entry point
//!
//! Design follows the `LocalEnginePlugin` pattern of the [`crate::plugins::engine`] module:
//! - each transport implements the [`McpServerPlugin`] trait
//! - [`McpServerManager`] centrally manages all plugins and dispatches by transport type
//! - persisted at `$APPDATA/com.loch.aio/mcp-servers.json` (separate from provider-configs.json)

pub mod connection;
pub mod convert;
pub mod error;
pub mod http;
pub mod stdio;

use crate::core::models::ToolResult;
use crate::core::models::*;
use crate::plugins::mcp::connection::McpConnection;
pub use crate::plugins::mcp::error::{McpError, McpResult};
use async_trait::async_trait;
use dashmap::DashMap;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// MCP transport plugin trait
#[async_trait]
pub trait McpServerPlugin: Send + Sync {
    /// Unique identifier: stdio / http / streamable_http
    fn identifier(&self) -> &'static str;

    /// Start an MCP server connection
    async fn start(&self, app: AppHandle, config: &McpServerConfig) -> McpResult<McpConnection>;

    /// MCP protocol handshake: initialize
    async fn initialize(&self, conn: &McpConnection) -> McpResult<McpServerInfo>;

    /// List the tools provided by the server
    async fn list_tools(&self, conn: &McpConnection) -> McpResult<Vec<ToolSpec>>;

    /// Call a tool
    async fn call_tool(
        &self,
        conn: &McpConnection,
        name: &str,
        arguments: Value,
        timeout: Duration,
    ) -> McpResult<ToolResult>;

    /// List the resources provided by the server
    async fn list_resources(&self, _conn: &McpConnection) -> McpResult<Vec<McpResource>> {
        Ok(Vec::new())
    }

    /// Read a resource
    async fn read_resource(
        &self,
        _conn: &McpConnection,
        _uri: &str,
    ) -> McpResult<ReadResourceResult> {
        Err(McpError::Unimplemented("resources/read 未实现".into()))
    }

    /// List the prompts provided by the server
    async fn list_prompts(&self, _conn: &McpConnection) -> McpResult<Vec<McpPrompt>> {
        Ok(Vec::new())
    }

    /// Get a prompt
    async fn get_prompt(
        &self,
        _conn: &McpConnection,
        _name: &str,
        _arguments: Option<Value>,
    ) -> McpResult<GetPromptResult> {
        Err(McpError::Unimplemented("prompts/get 未实现".into()))
    }

    /// Close the connection
    async fn stop(&self, conn: McpConnection) -> McpResult<()>;
}

/// Default tool call timeout
/// Maximum number of tools allowed per server (guards against tools/list explosion)
pub const MAX_TOOLS_PER_SERVER: usize = 100;

/// MCP plugin registry
pub struct McpServerManager {
    plugins: HashMap<String, Arc<dyn McpServerPlugin>>,
}

impl McpServerManager {
    /// Construct and register all built-in plugins
    pub fn builtin() -> Self {
        let mut mgr = Self {
            plugins: HashMap::new(),
        };
        mgr.register(Arc::new(stdio::StdioPlugin));
        mgr.register(Arc::new(http::HttpPlugin::new()));
        // Register the Streamable HTTP transport (the enum variant previously had no plugin, so any such config failed at startup)
        mgr.register(Arc::new(http::HttpPlugin::streamable()));
        mgr
    }

    /// Register a new transport plugin
    pub fn register(&mut self, plugin: Arc<dyn McpServerPlugin>) {
        self.plugins.insert(plugin.identifier().to_string(), plugin);
    }

    /// Get a plugin by transport string
    pub fn get(&self, id: &str) -> Option<Arc<dyn McpServerPlugin>> {
        self.plugins.get(id).cloned()
    }

    /// List the identifiers of all registered plugins
    pub fn list(&self) -> Vec<String> {
        self.plugins.keys().cloned().collect()
    }
}

// ====== MCP runtime state ======

/// MCP server connection pool: server_id -> McpConnection
/// Lock policy: single lock (same as LocalEngineState) to avoid nested deadlocks
pub struct McpServerState(
    pub parking_lot::Mutex<std::collections::HashMap<String, Arc<McpConnection>>>,
);

impl Default for McpServerState {
    fn default() -> Self {
        Self(parking_lot::Mutex::new(std::collections::HashMap::new()))
    }
}

impl McpServerState {
    pub fn lock(
        &self,
    ) -> parking_lot::MutexGuard<'_, std::collections::HashMap<String, Arc<McpConnection>>> {
        self.0.lock()
    }
}

/// In-flight MCP tool calls: call_id -> JoinHandle<Result<ToolResult, McpError>>
/// When the user hits stop, all entries are iterated and aborted
pub struct McpRequestManager(
    pub Arc<DashMap<String, JoinHandle<std::result::Result<ToolResult, McpError>>>>,
);

impl McpRequestManager {
    pub fn new() -> Self {
        Self(Arc::new(DashMap::new()))
    }

    /// Abort all in-flight calls
    pub fn abort_all(&self) {
        for entry in self.0.iter() {
            entry.value().abort();
        }
        self.0.clear();
    }
}

/// Approval context (basis for deriving a session rule from "always allow this session")
#[derive(Clone, Debug)]
pub struct ApprovalContext {
    pub server_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
}

/// Pending tool-call approvals: approval_id -> (oneshot::Sender<bool>, approval context)
/// The channel is signaled when the frontend calls `respond_tool_approval(approval_id, approved, always?)`;
/// always=Some("session") derives a session-level allow rule from the context.
pub struct PendingApprovals(pub Arc<DashMap<String, (oneshot::Sender<bool>, ApprovalContext)>>);

impl PendingApprovals {
    pub fn new() -> Self {
        Self(Arc::new(DashMap::new()))
    }

    /// Insert a pending approval item and return its approval_id
    pub fn insert(&self, tx: oneshot::Sender<bool>, ctx: ApprovalContext) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.0.insert(id.clone(), (tx, ctx));
        id
    }

    /// Remove and return the channel and approval context (consumed once)
    pub fn remove(&self, id: &str) -> Option<(oneshot::Sender<bool>, ApprovalContext)> {
        self.0.remove(id).map(|(_, v)| v)
    }
}

// ====== Persistence ======

const MCP_FILE: &str = "mcp-servers.json";

fn mcp_file_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join(MCP_FILE))
}

pub fn load_mcp_servers(app: &AppHandle) -> McpServersFile {
    let Some(p) = mcp_file_path(app) else {
        return McpServersFile::default();
    };
    if !p.exists() {
        return McpServersFile::default();
    }
    std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str::<McpServersFile>(&s).ok())
        .unwrap_or_default()
}

pub fn save_mcp_servers(app: &AppHandle, file: &McpServersFile) -> McpResult<()> {
    let p = mcp_file_path(app).ok_or_else(|| McpError::Server("无法获取 AppData 目录".into()))?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(file)?;
    std::fs::write(&p, json)?;
    Ok(())
}

// ====== Project-level MCP file operations ======

/// Load the project-level mcp-servers.json (returns empty if the file does not exist).
pub fn load_project_mcp_servers(project_path: &str) -> McpServersFile {
    let p = std::path::PathBuf::from(project_path)
        .join(".aio")
        .join("mcp-servers.json");
    if !p.exists() {
        return McpServersFile::default();
    }
    std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str::<McpServersFile>(&s).ok())
        .unwrap_or_default()
}

/// Save the project-level mcp-servers.json.
pub fn save_project_mcp_servers(project_path: &str, file: &McpServersFile) -> McpResult<()> {
    let p = std::path::PathBuf::from(project_path)
        .join(".aio")
        .join("mcp-servers.json");
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(file)?;
    std::fs::write(&p, json)?;
    Ok(())
}

/// Merge global + project MCP configs (project entries with the same ID override global ones).
pub fn merge_mcp_configs(
    global: &McpServersFile,
    project: &McpServersFile,
) -> BTreeMap<String, McpServerConfig> {
    let mut merged = global.servers.clone();
    for (id, cfg) in &project.servers {
        merged.insert(id.clone(), cfg.clone());
    }
    merged
}

/// List the merged server configs (global + project, project takes precedence).
pub fn list_configs_merged(
    app: &AppHandle,
    project_id: Option<&str>,
) -> McpResult<Vec<McpServerConfig>> {
    let global = load_mcp_servers(app);
    let project = match project_id {
        Some(pid) => {
            let project_path = resolve_project_path(app, pid)?;
            load_project_mcp_servers(&project_path)
        }
        None => McpServersFile::default(),
    };
    let merged = merge_mcp_configs(&global, &project);
    Ok(merged.into_values().collect())
}

/// Resolve a project path by project_id (implementation lives in utils::project_index:
/// typed resolution + Windows extended-path prefix normalization).
pub(crate) fn resolve_project_path(app: &AppHandle, project_id: &str) -> McpResult<String> {
    crate::utils::project_index::project_path(app, project_id).map_err(McpError::Server)
}

pub fn upsert_config(app: &AppHandle, config: McpServerConfig) -> McpResult<()> {
    let mut file = load_mcp_servers(app);
    file.servers.insert(config.id.clone(), config);
    file.updated_at = now_timestamp();
    save_mcp_servers(app, &file)
}

/// Upsert into the project-level file.
pub fn upsert_project_config(project_path: &str, config: McpServerConfig) -> McpResult<()> {
    let mut file = load_project_mcp_servers(project_path);
    file.servers.insert(config.id.clone(), config);
    file.updated_at = now_timestamp();
    save_project_mcp_servers(project_path, &file)
}

pub fn remove_config(app: &AppHandle, id: &str) -> McpResult<()> {
    let mut file = load_mcp_servers(app);
    file.servers.remove(id);
    file.updated_at = now_timestamp();
    save_mcp_servers(app, &file)
}

/// Remove from the project-level file.
pub fn remove_project_config(project_path: &str, id: &str) -> McpResult<()> {
    let mut file = load_project_mcp_servers(project_path);
    file.servers.remove(id);
    file.updated_at = now_timestamp();
    save_project_mcp_servers(project_path, &file)
}

/// Look up an MCP server config (merged view: project-level first).
pub fn get_config_merged(
    app: &AppHandle,
    id: &str,
    project_id: Option<&str>,
) -> Option<McpServerConfig> {
    // 1. With project_id -> resolve the exact project
    if let Some(pid) = project_id {
        if let Ok(project_path) = resolve_project_path(app, pid) {
            let project = load_project_mcp_servers(&project_path);
            if let Some(cfg) = project.servers.get(id) {
                return Some(cfg.clone());
            }
        }
    }
    // 2. Global lookup
    let global = load_mcp_servers(app);
    global.servers.get(id).cloned()
}

/// Lightweight timestamp (Unix seconds; avoids pulling in chrono).
/// Implementation lives in utils::time.
fn now_timestamp() -> String {
    crate::utils::time::now_unix_secs_string()
}

// ====== Helpers: resolving env placeholders ======

/// Check whether any env field contains a keyring placeholder
#[allow(dead_code)]
pub fn env_has_keyring_placeholder(env: &BTreeMap<String, String>) -> bool {
    env.values().any(|v| v.contains("${KEYRING:"))
}

/// Resolve keyring placeholders in env to real values
/// Called only by the stdio transport before spawning the child process
pub fn resolve_env_placeholders(
    app: &AppHandle,
    server_id: &str,
    env: &BTreeMap<String, String>,
) -> McpResult<BTreeMap<String, String>> {
    let mut resolved = BTreeMap::new();
    for (k, v) in env {
        let actual = if let Some(start) = v.find("${KEYRING:") {
            if let Some(end) = v[start..].find('}') {
                let account_full = &v[start + 9..start + end];
                // account_full looks like "mcp-server-{server_id}-env-{env_key}"
                let value = crate::core::secure_store::get(app, account_full)
                    .map_err(|e| {
                        McpError::Server(format!("读取 keyring {} 失败: {}", account_full, e))
                    })?
                    .ok_or_else(|| {
                        McpError::Server(format!("keyring 中未找到密钥: {}", account_full))
                    })?;
                let prefix = v[..start].to_string();
                let suffix = v[start + end + 1..].to_string();
                format!("{}{}{}", prefix, value, suffix)
            } else {
                v.clone()
            }
        } else {
            v.clone()
        };
        // Prevent the keyring account name from being passed to the child process: substitute the server_id placeholder
        let actual = actual.replace(&"${SERVER_ID}".to_string(), server_id);
        resolved.insert(k.clone(), actual);
    }
    Ok(resolved)
}

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

    /// Pool key of a server under its effective config (MCP-12): `"{id}::{fp16}"`,
    /// where the fingerprint is the canonical transport digest from the trust store.
    /// Project-level overlays can give one server id different effective transports
    /// per project; a plain-id key would hand one project another project's
    /// connection (and a restart in one project would kill the other's). The
    /// fingerprint covers exactly the fields that decide "connect where / run what",
    /// so display fields like enabled_tools never fork the connection.
    pub fn pool_key(id: &str, config: &McpServerConfig) -> String {
        let fp = crate::core::mcp_trust::config_fingerprint(config);
        let short: String = fp.chars().take(16).collect();
        format!("{id}::{short}")
    }

    /// Insert a connection keyed by (id, effective-config fingerprint).
    pub fn insert_for_config(&self, id: &str, config: &McpServerConfig, conn: Arc<McpConnection>) {
        self.0.lock().insert(Self::pool_key(id, config), conn);
    }

    /// The connection for `id` under exactly this effective config (None when the
    /// pool holds only a differently-configured variant — that is the point).
    pub fn get_for_config(&self, id: &str, config: &McpServerConfig) -> Option<Arc<McpConnection>> {
        self.0.lock().get(&Self::pool_key(id, config)).cloned()
    }

    /// Remove the connection for `id` under exactly this effective config (restart
    /// semantics: only the same variant is replaced, other projects' variants stay).
    pub fn remove_for_config(
        &self,
        id: &str,
        config: &McpServerConfig,
    ) -> Option<Arc<McpConnection>> {
        self.0.lock().remove(&Self::pool_key(id, config))
    }

    /// Remove every pool entry belonging to `id` (all config variants), returning them.
    pub fn remove_all_variants(&self, id: &str) -> Vec<Arc<McpConnection>> {
        let mut map = self.0.lock();
        let prefix = format!("{id}::");
        let targets: Vec<String> = map
            .keys()
            .filter(|k| k.as_str() == id || k.starts_with(&prefix))
            .cloned()
            .collect();
        targets.into_iter().filter_map(|k| map.remove(&k)).collect()
    }

    /// Any connection for `id` (config-agnostic fallback for informational calls
    /// that carry no project context: resources/prompts lookups from the UI).
    pub fn get_any(&self, id: &str) -> Option<Arc<McpConnection>> {
        let map = self.0.lock();
        if let Some(conn) = map.get(id) {
            return Some(conn.clone());
        }
        let prefix = format!("{id}::");
        map.keys()
            .find(|k| k.starts_with(&prefix))
            .and_then(|k| map.get(k).cloned())
    }

    /// Distinct server ids currently connected (fingerprint suffixes stripped).
    pub fn distinct_ids(&self) -> Vec<String> {
        let map = self.0.lock();
        let mut ids: Vec<String> = Vec::new();
        for key in map.keys() {
            let id = key.split("::").next().unwrap_or(key);
            if !ids.iter().any(|x| x == id) {
                ids.push(id.to_string());
            }
        }
        ids
    }
}

/// In-flight MCP tool calls: call_id -> AbortHandle
/// Entries stay registered for the whole await; when the user hits stop (or the server
/// stops / the window closes) the tasks are aborted for real instead of being detached
/// (the historical insert-then-remove made abort_all iterate an empty table).
pub struct McpRequestManager(pub Arc<DashMap<String, tokio::task::AbortHandle>>);

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

    /// Abort one server's in-flight calls (call ids carry the `mcp:{server_id}:` prefix).
    pub fn abort_server(&self, server_id: &str) {
        let prefix = format!("mcp:{server_id}:");
        let ids: Vec<String> = self
            .0
            .iter()
            .filter(|entry| entry.key().starts_with(&prefix))
            .map(|entry| entry.key().clone())
            .collect();
        for id in ids {
            if let Some((_, abort)) = self.0.remove(&id) {
                abort.abort();
            }
        }
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
    let json = serde_json::to_string_pretty(file)?;
    crate::core::fs_util::atomic_write(&p, json.as_bytes())?;
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
    let json = serde_json::to_string_pretty(file)?;
    crate::core::fs_util::atomic_write(&p, json.as_bytes())?;
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

/// `${KEYRING:account}` 占位符前缀（共 10 字节，冒号位于偏移 9）。
/// 历史实现按 `start + 9` 切片使 account 带前导冒号，所有 MCP 密钥解析必然失败。
const KEYRING_PREFIX: &str = "${KEYRING:";

/// 解析值中的**全部** `${KEYRING:account}` 占位符（纯函数，便于测试）。
///
/// - `lookup` 返回 `None` → `Err(account)`，由调用方生成用户可见错误；
/// - 未闭合的占位符原样保留（与历史行为一致）；
/// - 值可含多个占位符（历史实现只解析第一个）。
pub(crate) fn resolve_keyring_placeholders(
    value: &str,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find(KEYRING_PREFIX) {
        out.push_str(&rest[..start]);
        let after = &rest[start + KEYRING_PREFIX.len()..];
        let Some(end) = after.find('}') else {
            // 未闭合：剩余原文原样保留
            out.push_str(&rest[start..]);
            return Ok(out);
        };
        let account = &after[..end];
        let Some(secret) = lookup(account) else {
            return Err(account.to_string());
        };
        out.push_str(&secret);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Resolve keyring placeholders in env (stdio) / headers (HTTP) to real values.
/// Called by both transports before use; every placeholder must resolve.
pub fn resolve_env_placeholders(
    app: &AppHandle,
    server_id: &str,
    env: &BTreeMap<String, String>,
) -> McpResult<BTreeMap<String, String>> {
    let mut resolved = BTreeMap::new();
    for (k, v) in env {
        let actual = resolve_keyring_placeholders(v, |account| {
            crate::core::secure_store::get(app, account).ok().flatten()
        })
        .map_err(|account| {
            McpError::Server(crate::ierr!(
                "keyring 中未找到密钥: {}",
                "Secret not found in the keyring: {}",
                account
            ))
        })?;
        // Prevent the keyring account name from being passed to the child process: substitute the server_id placeholder
        let actual = actual.replace("${SERVER_ID}", server_id);
        resolved.insert(k.clone(), actual);
    }
    Ok(resolved)
}

// ====== Project-level MCP trust gate (P0-1 / MCP-1) ======

/// 判定项目级条目是否为 **AIO 自己生成**的内置文件系统 server：命令必须逐字
/// 指向当前进程可执行文件、参数恰为 `--fs-server <项目路径>`。仓库可以伪造同名
/// id，但只要命令/参数与当前进程不匹配就照常走信任门——豁免的是 AIO 写入的
/// 初始化条目，不是这个 id 本身。
fn is_self_generated_fs_server(cfg: &McpServerConfig, project_path: &str) -> bool {
    if cfg.id != crate::core::ids::BUILTIN_FS_SERVER_ID {
        return false;
    }
    let McpTransport::Stdio { command, args, .. } = &cfg.transport else {
        return false;
    };
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let exe_matches = std::path::Path::new(command) == exe
        || std::fs::canonicalize(command)
            .ok()
            .zip(std::fs::canonicalize(&exe).ok())
            .is_some_and(|(a, b)| a == b);
    if !exe_matches || args.len() != 2 || args[0] != "--fs-server" {
        return false;
    }
    std::path::Path::new(&args[1]) == std::path::Path::new(project_path)
        || std::fs::canonicalize(&args[1])
            .ok()
            .zip(std::fs::canonicalize(project_path).ok())
            .is_some_and(|(a, b)| a == b)
}

/// 项目级服务器信任存储的落点（$APPDATA/mcp-trust.json，仓库之外）。
fn trust_store(app: &AppHandle) -> crate::core::mcp_trust::McpTrustStore {
    let dir = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir());
    crate::core::mcp_trust::McpTrustStore::open(&dir)
}

/// 项目级配置中尚未信任的服务器（前端据此弹确认；autoStart 对它们一律跳过）。
/// AIO 自身生成的内置 fs server 条目（见 [`is_self_generated_fs_server`]）视为已信任。
pub fn untrusted_project_servers(app: &AppHandle, project_path: &str) -> Vec<McpServerConfig> {
    let store = trust_store(app);
    load_project_mcp_servers(project_path)
        .servers
        .into_values()
        .filter(|cfg| {
            if is_self_generated_fs_server(cfg, project_path) {
                return false;
            }
            !store.is_trusted(
                project_path,
                &cfg.id,
                &crate::core::mcp_trust::config_fingerprint(cfg),
            )
        })
        .collect()
}

/// 标记一个项目级服务器为已信任（记录当前配置指纹；仓库配置变更后自动失效）。
/// 返回被标记的配置；项目文件中不存在该 id 时报错。
pub fn trust_project_server(
    app: &AppHandle,
    project_path: &str,
    server_id: &str,
) -> McpResult<McpServerConfig> {
    let file = load_project_mcp_servers(project_path);
    let cfg = file.servers.get(server_id).cloned().ok_or_else(|| {
        McpError::Server(crate::ierr!(
            "项目 .aio/mcp-servers.json 中不存在服务器: {}",
            "Server not found in the project's .aio/mcp-servers.json: {}",
            server_id
        ))
    })?;
    let fingerprint = crate::core::mcp_trust::config_fingerprint(&cfg);
    trust_store(app)
        .set_trusted(project_path, server_id, &fingerprint)
        .map_err(McpError::Server)?;
    Ok(cfg)
}

/// 用户经 AIO 设置界面**写入**项目级配置时顺带记录信任（用户意图即授权；
/// 仓库文件永远不会走这条路径——只有 add_mcp_server 的 UI 流程会）。
pub fn mark_project_server_trusted(app: &AppHandle, project_path: &str, cfg: &McpServerConfig) {
    let fingerprint = crate::core::mcp_trust::config_fingerprint(cfg);
    if let Err(e) = trust_store(app).set_trusted(project_path, &cfg.id, &fingerprint) {
        tracing::warn!("[mcp-trust] 记录项目级 server 信任失败: {e}");
    }
}

/// `start_mcp_server` 的信任门：生效配置来自项目文件（id 存在于其中）且未信任 → false。
/// 全局配置、AIO 自身生成的内置 fs server、已信任的项目配置返回 true。
pub fn project_server_spawn_allowed(
    app: &AppHandle,
    project_id: Option<&str>,
    id: &str,
) -> McpResult<bool> {
    let Some(pid) = project_id else {
        return Ok(true);
    };
    let project_path = resolve_project_path(app, pid)?;
    let Some(cfg) = load_project_mcp_servers(&project_path)
        .servers
        .get(id)
        .cloned()
    else {
        return Ok(true);
    };
    if is_self_generated_fs_server(&cfg, &project_path) {
        return Ok(true);
    }
    let store = trust_store(app);
    let trusted = store.is_trusted(
        &project_path,
        id,
        &crate::core::mcp_trust::config_fingerprint(&cfg),
    );
    Ok(trusted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_generated_fs_server_detection() {
        fn stdio_cfg(id: &str, command: &str, args: Vec<String>) -> McpServerConfig {
            McpServerConfig {
                id: id.into(),
                display_name: "fs".into(),
                transport: McpTransport::Stdio {
                    command: command.into(),
                    args,
                    env: BTreeMap::new(),
                    cwd: None,
                },
                enabled_tools: vec![],
                auto_start: false,
                has_stored_secret: false,
                trusted: false,
                from_catalog: None,
            }
        }
        let exe = std::env::current_exe().expect("当前 exe 路径");
        let project = std::env::temp_dir();
        let project_str = project.to_string_lossy().into_owned();
        // AIO 自生成的条目：命令=当前 exe、参数=--fs-server <项目> → 豁免
        let ok = stdio_cfg(
            crate::core::ids::BUILTIN_FS_SERVER_ID,
            &exe.to_string_lossy(),
            vec!["--fs-server".into(), project_str.clone()],
        );
        assert!(is_self_generated_fs_server(&ok, &project_str));
        // 仓库伪造同名 id 但命令不同 → 不豁免（走信任门）
        let forged = stdio_cfg(
            crate::core::ids::BUILTIN_FS_SERVER_ID,
            "powershell",
            vec!["-Command".into(), "evil".into()],
        );
        assert!(!is_self_generated_fs_server(&forged, &project_str));
        // 正常服务器 id → 不豁免
        let other = stdio_cfg("some-server", &exe.to_string_lossy(), vec![]);
        assert!(!is_self_generated_fs_server(&other, &project_str));
    }

    /// MCP-12: pool keys fork per effective-config variant — same config is stable,
    /// a different transport for the same id gets a different key (so one project's
    /// connection is never handed to / restarted by another project).
    #[test]
    fn pool_key_separates_config_variants() {
        fn cfg(id: &str, command: &str) -> McpServerConfig {
            McpServerConfig {
                id: id.into(),
                display_name: "x".into(),
                transport: McpTransport::Stdio {
                    command: command.into(),
                    args: vec![],
                    env: BTreeMap::new(),
                    cwd: None,
                },
                enabled_tools: vec![],
                auto_start: false,
                has_stored_secret: false,
                trusted: false,
                from_catalog: None,
            }
        }
        let a1 = McpServerState::pool_key("srv", &cfg("srv", "run-a"));
        let a2 = McpServerState::pool_key("srv", &cfg("srv", "run-a"));
        let b = McpServerState::pool_key("srv", &cfg("srv", "run-b"));
        assert_eq!(a1, a2, "相同生效配置必须命中同一池条目");
        assert_ne!(a1, b, "不同 transport 的同 id 服务器必须分池");
        assert!(a1.starts_with("srv::"), "键保留 id 前缀便于清理: {a1}");
        assert_eq!(a1.len(), "srv::".len() + 16, "指纹取短前缀即可区分");
        // 展示字段（enabled_tools）不参与指纹：连接身份只取决于 transport
        let mut display_only = cfg("srv", "run-a");
        display_only.enabled_tools = vec!["other-tool".into()];
        assert_eq!(
            McpServerState::pool_key("srv", &display_only),
            a1,
            "enabled_tools 差异不应分裂连接"
        );
    }

    #[test]
    fn keyring_placeholder_account_has_no_leading_colon() {
        // 回归：历史 off-by-one 使 account 带 ":" 前缀，所有密钥必失败
        let out = resolve_keyring_placeholders("${KEYRING:mcp-server-x-env-K}", |account| {
            assert_eq!(account, "mcp-server-x-env-K");
            Some("SECRET".to_string())
        })
        .expect("占位符应解析成功");
        assert_eq!(out, "SECRET");
    }

    #[test]
    fn keyring_placeholder_multiple_with_affixes() {
        let out = resolve_keyring_placeholders("pre-${KEYRING:a}-mid-${KEYRING:b}-post", |a| {
            if a == "a" {
                Some("A".to_string())
            } else if a == "b" {
                Some("B".to_string())
            } else {
                None
            }
        })
        .expect("多占位符应全部解析");
        assert_eq!(out, "pre-A-mid-B-post");
    }

    #[test]
    fn keyring_placeholder_missing_secret_reports_account() {
        let err =
            resolve_keyring_placeholders("${KEYRING:gone}", |_| None).expect_err("缺密钥应报错");
        assert_eq!(err, "gone");
    }

    #[test]
    fn keyring_placeholder_unclosed_left_as_is() {
        let out = resolve_keyring_placeholders("x${KEYRING:a", |_| None).expect("未闭合应原样保留");
        assert_eq!(out, "x${KEYRING:a");
    }

    #[test]
    fn value_without_placeholder_unchanged() {
        let out = resolve_keyring_placeholders("plain-value", |_| None).expect("无占位符应原样");
        assert_eq!(out, "plain-value");
    }
}

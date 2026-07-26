//! MCP 插件系统入口
//!
//! 设计参考 [`crate::plugins::engine`] 模块的 `LocalEnginePlugin` 模式：
//! - 各传输实现 [`McpServerPlugin`] trait
//! - [`McpServerManager`] 集中管理所有插件，按 transport 类型分发
//! - 持久化在 `$APPDATA/com.loch.aio/mcp-servers.json`（独立于 provider-configs.json）

pub mod connection;
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

/// MCP 传输插件 trait
#[async_trait]
pub trait McpServerPlugin: Send + Sync {
    /// 唯一标识符：stdio / http / streamable_http
    fn identifier(&self) -> &'static str;

    /// 启动一个 MCP server 连接
    async fn start(&self, app: AppHandle, config: &McpServerConfig) -> McpResult<McpConnection>;

    /// MCP 协议握手：initialize
    async fn initialize(&self, conn: &McpConnection) -> McpResult<McpServerInfo>;

    /// 获取 server 提供的工具列表
    async fn list_tools(&self, conn: &McpConnection) -> McpResult<Vec<ToolSpec>>;

    /// 调用一个工具
    async fn call_tool(
        &self,
        conn: &McpConnection,
        name: &str,
        arguments: Value,
        timeout: Duration,
    ) -> McpResult<ToolResult>;

    /// 获取 server 提供的资源列表
    async fn list_resources(&self, _conn: &McpConnection) -> McpResult<Vec<McpResource>> {
        Ok(Vec::new())
    }

    /// 读取一个资源
    async fn read_resource(
        &self,
        _conn: &McpConnection,
        _uri: &str,
    ) -> McpResult<ReadResourceResult> {
        Err(McpError::Unimplemented("resources/read 未实现".into()))
    }

    /// 获取 server 提供的提示词列表
    async fn list_prompts(&self, _conn: &McpConnection) -> McpResult<Vec<McpPrompt>> {
        Ok(Vec::new())
    }

    /// 获取一个提示词
    async fn get_prompt(
        &self,
        _conn: &McpConnection,
        _name: &str,
        _arguments: Option<Value>,
    ) -> McpResult<GetPromptResult> {
        Err(McpError::Unimplemented("prompts/get 未实现".into()))
    }

    /// 关闭连接
    async fn stop(&self, conn: McpConnection) -> McpResult<()>;
}

/// 默认调用工具超时
#[allow(dead_code)]
pub const DEFAULT_TOOL_TIMEOUT: Duration = Duration::from_secs(30);

/// 单个 server 最多允许的工具数（防御 tools/list 爆炸）
pub const MAX_TOOLS_PER_SERVER: usize = 100;

/// 单个工具结果最大体积（64KB）
#[allow(dead_code)]
pub const MAX_TOOL_RESULT_BYTES: usize = 64 * 1024;

/// 5 轮工具调用上限（防 LLM 死循环）
#[allow(dead_code)]
pub const MAX_TOOL_CALL_ROUNDS: usize = 5;

/// MCP 插件注册中心
pub struct McpServerManager {
    plugins: HashMap<String, Arc<dyn McpServerPlugin>>,
}

impl McpServerManager {
    /// 构造并注册所有内置插件
    pub fn builtin() -> Self {
        let mut mgr = Self {
            plugins: HashMap::new(),
        };
        mgr.register(Arc::new(stdio::StdioPlugin));
        mgr.register(Arc::new(http::HttpPlugin));
        mgr
    }

    /// 注册一个新传输插件
    pub fn register(&mut self, plugin: Arc<dyn McpServerPlugin>) {
        self.plugins.insert(plugin.identifier().to_string(), plugin);
    }

    /// 按 transport 字符串获取插件
    pub fn get(&self, id: &str) -> Option<Arc<dyn McpServerPlugin>> {
        self.plugins.get(id).cloned()
    }

    /// 列出所有已注册插件的 identifier
    pub fn list(&self) -> Vec<String> {
        self.plugins.keys().cloned().collect()
    }
}

// ====== MCP 运行时状态 ======

/// MCP 服务器连接池：server_id → McpConnection
/// 锁策略：与 LocalEngineState 一致，单锁避免嵌套死锁
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

/// 在途 MCP 工具调用：call_id → JoinHandle<Result<ToolResult, McpError>>
/// 用户点停止时遍历 abort 所有
pub struct McpRequestManager(
    pub Arc<DashMap<String, JoinHandle<std::result::Result<ToolResult, McpError>>>>,
);

impl McpRequestManager {
    pub fn new() -> Self {
        Self(Arc::new(DashMap::new()))
    }

    /// 中止所有在途调用
    pub fn abort_all(&self) {
        for entry in self.0.iter() {
            entry.value().abort();
        }
        self.0.clear();
    }
}

/// 待处理的工具调用审批：approval_id → oneshot::Sender<bool>
/// 前端调用 `respond_tool_approval(approval_id, approved)` 时触发对应 channel。
pub struct PendingApprovals(pub Arc<DashMap<String, oneshot::Sender<bool>>>);

impl PendingApprovals {
    pub fn new() -> Self {
        Self(Arc::new(DashMap::new()))
    }

    /// 插入一个待审批项，返回 approval_id
    pub fn insert(&self, tx: oneshot::Sender<bool>) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.0.insert(id.clone(), tx);
        id
    }

    /// 移除并返回对应 channel（消费一次）
    pub fn remove(&self, id: &str) -> Option<oneshot::Sender<bool>> {
        self.0.remove(id).map(|(_, v)| v)
    }
}

// ====== 持久化 ======

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

// ====== 项目级 MCP 文件操作 ======

/// 加载项目级 mcp-servers.json（不存在则返回空）。
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

/// 保存项目级 mcp-servers.json。
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

/// 合并全局 + 项目的 MCP configs（项目同 ID 覆盖全局）。
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

/// 列出合并后的 server 列表（全局 + 项目，项目优先）。
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

/// 通过 project_id 解析项目路径（复用 skill.rs 中的逻辑）。
pub(crate) fn resolve_project_path(app: &AppHandle, project_id: &str) -> McpResult<String> {
    let idx_path = app
        .path()
        .app_data_dir()
        .map_err(|e| McpError::Server(format!("获取 AppData 目录失败: {}", e)))?
        .join("projects.json");
    let content = std::fs::read_to_string(&idx_path)
        .map_err(|e| McpError::Server(format!("读取项目索引失败: {}", e)))?;
    let file: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| McpError::Server(format!("解析项目索引失败: {}", e)))?;
    file["projects"][project_id]["path"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| McpError::Server(format!("项目 {} 不存在", project_id)))
}

pub fn upsert_config(app: &AppHandle, config: McpServerConfig) -> McpResult<()> {
    let mut file = load_mcp_servers(app);
    file.servers.insert(config.id.clone(), config);
    file.updated_at = now_timestamp();
    save_mcp_servers(app, &file)
}

/// Upsert 到项目级文件。
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

/// 从项目级文件删除。
pub fn remove_project_config(project_path: &str, id: &str) -> McpResult<()> {
    let mut file = load_project_mcp_servers(project_path);
    file.servers.remove(id);
    file.updated_at = now_timestamp();
    save_project_mcp_servers(project_path, &file)
}

/// 查找 MCP server 配置（合并视图：项目级优先）。
pub fn get_config_merged(
    app: &AppHandle,
    id: &str,
    project_id: Option<&str>,
) -> Option<McpServerConfig> {
    // 1. 有 project_id → 精确查找项目
    if let Some(pid) = project_id {
        if let Ok(project_path) = resolve_project_path(app, pid) {
            let project = load_project_mcp_servers(&project_path);
            if let Some(cfg) = project.servers.get(id) {
                return Some(cfg.clone());
            }
        }
    }
    // 2. 全局查找
    let global = load_mcp_servers(app);
    global.servers.get(id).cloned()
}

/// 轻量级时间戳（秒级 Unix time，不引入 chrono 依赖）
fn now_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}", secs)
}

// ====== 工具函数：解析 env 占位符 ======

/// 检查 env 字段中是否包含 keyring 占位符
#[allow(dead_code)]
pub fn env_has_keyring_placeholder(env: &BTreeMap<String, String>) -> bool {
    env.values().any(|v| v.contains("${KEYRING:"))
}

/// 把 env 中的 keyring 占位符解析为真实值
/// 仅供 stdio transport 在启动子进程前调用
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
                // account_full 形如 "mcp-server-{server_id}-env-{env_key}"
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
        // 防止把 keyring account 名误传给子进程：server_id 占位
        let actual = actual.replace(&format!("${{SERVER_ID}}"), server_id);
        resolved.insert(k.clone(), actual);
    }
    Ok(resolved)
}

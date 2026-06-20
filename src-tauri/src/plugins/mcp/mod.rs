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

use crate::core::models::*;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager};

pub use connection::McpConnection;
pub use error::{McpError, McpResult};

/// MCP 传输插件 trait
#[async_trait]
pub trait McpServerPlugin: Send + Sync {
    /// 唯一标识符：stdio / http / streamable_http
    fn identifier(&self) -> &'static str;

    /// 启动一个 MCP server 连接
    async fn start(
        &self,
        app: AppHandle,
        config: &McpServerConfig,
    ) -> McpResult<McpConnection>;

    /// MCP 协议握手：initialize
    async fn initialize(
        &self,
        conn: &McpConnection,
    ) -> McpResult<McpServerInfo>;

    /// 获取 server 提供的工具列表
    async fn list_tools(
        &self,
        conn: &McpConnection,
    ) -> McpResult<Vec<ToolSpec>>;

    /// 调用一个工具
    async fn call_tool(
        &self,
        conn: &McpConnection,
        name: &str,
        arguments: Value,
        timeout: Duration,
    ) -> McpResult<ToolResult>;

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
        self.plugins
            .insert(plugin.identifier().to_string(), plugin);
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
    let p = mcp_file_path(app)
        .ok_or_else(|| McpError::Server("无法获取 AppData 目录".into()))?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(file)?;
    std::fs::write(&p, json)?;
    Ok(())
}

/// 列出当前配置文件中所有 server（按 id 排序）
pub fn list_configs(app: &AppHandle) -> Vec<McpServerConfig> {
    load_mcp_servers(app)
        .servers
        .values()
        .cloned()
        .collect()
}

pub fn upsert_config(app: &AppHandle, config: McpServerConfig) -> McpResult<()> {
    let mut file = load_mcp_servers(app);
    file.servers.insert(config.id.clone(), config);
    file.updated_at = now_timestamp();
    save_mcp_servers(app, &file)
}

pub fn remove_config(app: &AppHandle, id: &str) -> McpResult<()> {
    let mut file = load_mcp_servers(app);
    file.servers.remove(id);
    file.updated_at = now_timestamp();
    save_mcp_servers(app, &file)
}

pub fn get_config(app: &AppHandle, id: &str) -> Option<McpServerConfig> {
    load_mcp_servers(app).servers.get(id).cloned()
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
                let value = crate::core::secure_store::get(
                    app,
                    account_full,
                )
                .map_err(|e| McpError::Server(format!("读取 keyring {} 失败: {}", account_full, e)))?
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

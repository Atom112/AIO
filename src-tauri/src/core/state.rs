/// 全局 Tauri 状态定义

use dashmap::DashMap;
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;

/// 管理活跃的 LLM 流式任务
/// 键格式为 "{assistant_id}-{topic_id}"
pub struct StreamManager(pub Arc<DashMap<String, JoinHandle<()>>>);

/// 包装 SQLite 数据库连接
pub struct DbState(pub std::sync::Mutex<rusqlite::Connection>);

/// 本地引擎进程内部状态（M11：合并为单锁避免死锁）
#[derive(Default)]
pub struct LocalEngineInner {
    /// 当前引擎类型标识，如 "llama_cpp"
    pub engine_type: String,
    /// 子进程句柄
    pub child_process: Option<std::process::Child>,
}

/// 当前运行的本地推理引擎进程状态
pub struct LocalEngineState(pub Mutex<LocalEngineInner>);

impl LocalEngineState {
    pub fn new() -> Self {
        Self(Mutex::new(LocalEngineInner::default()))
    }

    pub fn lock(&self) -> std::sync::MutexGuard<'_, LocalEngineInner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

// ====== MCP 状态 ======

use crate::core::models::ToolResult;
use crate::plugins::mcp::connection::McpConnection;
use crate::plugins::mcp::error::McpError;

/// MCP 服务器连接池：server_id → McpConnection
/// 锁策略：与 LocalEngineState 一致，单锁避免嵌套死锁
pub struct McpServerState(pub Mutex<std::collections::HashMap<String, Arc<McpConnection>>>);

impl Default for McpServerState {
    fn default() -> Self {
        Self(Mutex::new(std::collections::HashMap::new()))
    }
}

impl McpServerState {
    /// 锁中毒时仍能拿到内部数据
    pub fn lock(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, Arc<McpConnection>>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
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

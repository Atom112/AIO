/// 全局 Tauri 状态定义

use dashmap::DashMap;
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// 管理活跃的 LLM 流式任务
/// 键格式为 "{assistant_id}-{topic_id}"
/// 每个任务携带一个 CancellationToken：停止时调用 cancel()（而非 abort()），
/// 保证任务能在 select! 分支优雅退出并执行 epilogue（emit done + 移除自身）。
pub struct StreamManager(pub Arc<DashMap<String, (JoinHandle<()>, CancellationToken)>>);

/// 包装 SQLite 数据库连接
pub struct DbState(pub parking_lot::Mutex<rusqlite::Connection>);

/// 本地引擎进程内部状态（M11：合并为单锁避免死锁）
#[derive(Default)]
pub struct LocalEngineInner {
    /// 当前引擎类型标识，如 "llama_cpp"
    pub engine_type: String,
    /// 子进程句柄
    pub child_process: Option<std::process::Child>,
}

impl Drop for LocalEngineInner {
    fn drop(&mut self) {
        if let Some(mut child) = self.child_process.take() {
            tracing::debug!("[engine] Drop LocalEngineInner — 清理残留子进程");
            let _ = child.kill();
        }
    }
}

/// 当前运行的本地推理引擎进程状态
pub struct LocalEngineState(pub Mutex<LocalEngineInner>);

impl LocalEngineState {
    pub fn new() -> Self {
        Self(Mutex::new(LocalEngineInner::default()))
    }

    pub fn lock(&self) -> parking_lot::MutexGuard<'_, LocalEngineInner> {
        self.0.lock()
    }
}

// ====== MCP 状态 ======

use crate::core::models::ToolResult;
use crate::plugins::mcp::connection::McpConnection;
use crate::plugins::mcp::error::McpError;

/// MCP 服务器连接池：server_id → McpConnection
/// 锁策略：与 LocalEngineState 一致，单锁避免嵌套死锁
pub struct McpServerState(pub parking_lot::Mutex<std::collections::HashMap<String, Arc<McpConnection>>>);

impl Default for McpServerState {
    fn default() -> Self {
        Self(parking_lot::Mutex::new(std::collections::HashMap::new()))
    }
}

impl McpServerState {
    pub fn lock(&self) -> parking_lot::MutexGuard<'_, std::collections::HashMap<String, Arc<McpConnection>>> {
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
use tokio::sync::oneshot;
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

/// 后台运行的子智能体句柄：subagent_id → JoinHandle
/// 用于 fire-and-forget 子智能体的生命周期管理。
pub struct SubagentHandles(
    pub DashMap<String, tokio::task::JoinHandle<Result<crate::core::models::ToolResult, String>>>,
);

impl SubagentHandles {
    pub fn new() -> Self {
        Self(DashMap::new())
    }
}

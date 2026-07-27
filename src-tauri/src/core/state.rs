/// 全局 Tauri 状态定义
use dashmap::DashMap;
use parking_lot::Mutex;
use std::collections::HashMap;
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

/// 本地引擎进程内部状态
#[derive(Default)]
pub struct LocalEngineInner {
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

/// 当前运行的本地推理引擎进程状态（支持多引擎并发）。
/// key = engine_type (e.g. "llama_cpp", "vllm")
pub struct LocalEngineState(pub Mutex<HashMap<String, LocalEngineInner>>);

impl LocalEngineState {
    pub fn new() -> Self {
        Self(Mutex::new(HashMap::new()))
    }

    pub fn lock(&self) -> parking_lot::MutexGuard<'_, HashMap<String, LocalEngineInner>> {
        self.0.lock()
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

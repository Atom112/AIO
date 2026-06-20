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

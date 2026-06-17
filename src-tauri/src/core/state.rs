/// 全局 Tauri 状态定义

use dashmap::DashMap;
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;

/// 管理活跃的 LLM 流式任务
/// 键格式为 "{assistant_id}-{topic_id}"
pub struct StreamManager(pub Arc<DashMap<String, JoinHandle<()>>>);

/// 包装 SQLite 数据库连接
pub struct DbState(pub std::sync::Mutex<rusqlite::Connection>);

/// 当前运行的本地推理引擎进程状态
pub struct LocalEngineState {
    /// 当前引擎类型标识，如 "llama_cpp"
    pub engine_type: Mutex<String>,
    /// 子进程句柄
    pub child_process: Mutex<Option<std::process::Child>>,
}

impl LocalEngineState {
    pub fn new() -> Self {
        Self {
            engine_type: Mutex::new(String::new()),
            child_process: Mutex::new(None),
        }
    }
}

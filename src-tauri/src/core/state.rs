/// Global Tauri state definitions
use crate::core::models::{AgentTask, QuestionResponse};
use dashmap::DashMap;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Manages active LLM streaming tasks.
/// Key is an (assistant_id, topic_id) tuple — a structured key avoids the string-concat
/// ambiguity of "a-b"+"c" == "a"+"b-c" (UUIDs contain "-").
/// Each task carries a CancellationToken: stopping calls cancel() (not abort()),
/// so the task can exit gracefully in its select! branch and run its epilogue
/// (emit done + remove itself).
/// Task key: (assistant_id, topic_id).
pub type StreamTaskKey = (String, String);
/// One active LLM streaming task entry (JoinHandle + cancellation token).
pub type StreamTaskEntry = (JoinHandle<()>, CancellationToken);

/// Concurrent map of active LLM streaming tasks.
pub struct StreamManager(pub Arc<DashMap<StreamTaskKey, StreamTaskEntry>>);

/// Wraps the SQLite database connection
pub struct DbState(pub parking_lot::Mutex<rusqlite::Connection>);

/// Internal state of a local engine process
#[derive(Default)]
pub struct LocalEngineInner {
    /// Child process handle
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

/// Currently running local inference engine processes (multiple engines may run concurrently).
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

/// Background subagent entry: task handle + standalone cancellation token (child of the parent token, cancellable independently).
pub type SubagentEntry = (tokio::task::JoinHandle<()>, CancellationToken);

/// Handles of background subagents: subagent_id → (JoinHandle, CancellationToken).
/// Used to manage fire-and-forget subagent lifecycles and stop them individually (stop_subagent command).
pub struct SubagentHandles(pub DashMap<String, SubagentEntry>);

impl SubagentHandles {
    pub fn new() -> Self {
        Self(DashMap::new())
    }
}

/// 后台子智能体的运行终态（pull 通道读取；终态只在 fire-and-forget 闭包收尾处写入，
/// stop_subagent 仅取消令牌、由闭包统一收尾，避免双写竞态）。
#[derive(Clone)]
pub enum SubagentRunStatus {
    Running,
    Done(String),
    Failed(String),
    Cancelled,
}

/// 单个后台子智能体的状态/结果条目。
#[derive(Clone)]
pub struct SubagentOutcomeEntry {
    pub profile_name: String,
    /// 任务摘要（前 100 字符，供通知与查询结果展示）
    pub task_summary: String,
    pub status: SubagentRunStatus,
}

/// 待注入主会话的完成通知（push 队列载荷；outcome_text 已按终态渲染）。
#[derive(Clone)]
pub struct SubagentNotice {
    pub subagent_id: String,
    pub profile_name: String,
    pub task_summary: String,
    pub outcome_text: String,
}

/// 后台子智能体完成登记：
/// - `outcomes`：结果缓存（get_subagent_result 读取，不消费）；
/// - `pending`：待注入通知队列（主循环回合开始/每轮 drain，consume-once），
///   key = (assistant_id, topic_id)；
/// - `notify`：任一后台子智能体到达终态时 notify_waiters，唤醒阻塞等待的
///   get_subagent_result（粗粒度惊群 + 短轮询兜底，醒来各自重查 outcomes）。
///
/// 内存语义：后台子智能体随 app 退出而死，队列不持久化（与 handoff 文档一致）。
pub struct SubagentCompletions {
    pub outcomes: DashMap<String, SubagentOutcomeEntry>,
    pub pending: DashMap<(String, String), std::collections::VecDeque<SubagentNotice>>,
    pub notify: tokio::sync::Notify,
}

impl SubagentCompletions {
    pub fn new() -> Self {
        Self {
            outcomes: DashMap::new(),
            pending: DashMap::new(),
            notify: tokio::sync::Notify::new(),
        }
    }
}

/// Main-agent task board: (assistant_id, topic_id) → ordered task list.
///
/// Memory-only (cleared on app exit, like SubagentCompletions); the frontend mirrors it
/// via the `agent-tasks-changed` event. Mutations happen only through the
/// `create_tasks` / `update_task` tools (commands/llm/tasks.rs).
pub struct AgentTaskBoard(pub Arc<DashMap<(String, String), Vec<AgentTask>>>);

impl AgentTaskBoard {
    pub fn new() -> Self {
        Self(Arc::new(DashMap::new()))
    }

    /// Current task list for the key (empty when none).
    pub fn tasks(&self, key: &(String, String)) -> Vec<AgentTask> {
        self.0
            .get(key)
            .map(|v| v.value().clone())
            .unwrap_or_default()
    }

    /// Replaces the whole list (create_tasks path: clear_existing or append).
    pub fn set_tasks(&self, key: (String, String), tasks: Vec<AgentTask>) {
        self.0.insert(key, tasks);
    }
}

/// Pending ask_user requests: request_id → oneshot sender for the user's response.
///
/// Mirrors PendingApprovals: the ask_user tool inserts a sender and awaits it; the
/// frontend resolves it via the `respond_agent_question` command. Token cancellation and
/// the wait timeout remove the entry to avoid leaks.
pub struct PendingAgentQuestions(pub Arc<DashMap<String, oneshot::Sender<QuestionResponse>>>);

impl PendingAgentQuestions {
    pub fn new() -> Self {
        Self(Arc::new(DashMap::new()))
    }

    /// Insert a pending question request and return its request_id.
    pub fn insert(&self, tx: oneshot::Sender<QuestionResponse>) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.0.insert(id.clone(), tx);
        id
    }

    /// Remove and return the sender (consumed once by respond_agent_question).
    pub fn remove(&self, id: &str) -> Option<oneshot::Sender<QuestionResponse>> {
        self.0.remove(id).map(|(_, v)| v)
    }
}

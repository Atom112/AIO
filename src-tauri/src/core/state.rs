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
/// One active LLM streaming task entry: (JoinHandle, cancellation token, turn_id).
/// `turn_id` identifies the generation (LOOP-1): a replacement turn's epilogue must
/// only remove ITS OWN entry — after the 10s replacement wait times out, the old
/// task can still be stuck in a long command and its keyed remove would otherwise
/// delete the NEW task's entry (orphan: stop can't find it, double main loops).
pub type StreamTaskEntry = (JoinHandle<()>, CancellationToken, String);

/// Outcome of [`StreamManager::stop_with_escalation`].
pub enum StopOutcome {
    /// No active task for the key (frontend state drifted; nothing to do).
    NotFound,
    /// The task acknowledged cancellation and removed its own entry within the grace window.
    Graceful,
    /// The task ignored cancellation past the grace window; its entry was removed and
    /// the JoinHandle aborted. Carries the aborted generation's `turn_id` so the caller
    /// can run that generation's terminal bookkeeping (checkpoints + done event).
    Aborted(String),
}

/// Concurrent map of active LLM streaming tasks.
///
/// 阶段5（报告第3项：并发 run_agent_turn 换代）：换代协议此前内联在两个命令层
/// 调用点（call_llm_stream / run_agent_turn 各一份 remove→cancel→wait + 注册 +
/// 身份注销）——协议本体收进本类型成为单一来源，并发换代语义随附集成测试
/// （generation_tests：窗口内协作退出 / 越窗滞留旧代不腐蚀新代条目 / 身份不匹配
/// 注销零效果）。
#[derive(Clone)]
pub struct StreamManager(pub Arc<DashMap<StreamTaskKey, StreamTaskEntry>>);

impl StreamManager {
    pub fn new() -> Self {
        Self(Arc::new(DashMap::new()))
    }

    /// Registers a running stream task under `key` (assistant_id, topic_id).
    pub fn register(
        &self,
        key: StreamTaskKey,
        handle: JoinHandle<()>,
        token: CancellationToken,
        turn_id: String,
    ) {
        self.0.insert(key, (handle, token, turn_id));
    }

    /// stop_stream path: cancels the current task for `key` (cancel only, never abort —
    /// the epilogue is guaranteed and the task removes its own entry via [`remove_own`]).
    /// Returns whether a task was found.
    pub fn cancel(&self, key: &StreamTaskKey) -> bool {
        match self.0.get(key) {
            Some(entry) => {
                entry.1.cancel();
                true
            }
            None => false,
        }
    }

    /// All active stream keys — the frontend reconciles its stop-button stream
    /// ownership on startup (a webview reload loses the module-level keys while
    /// backend tasks keep running, leaving nothing for stop to target).
    pub fn keys(&self) -> Vec<StreamTaskKey> {
        self.0.iter().map(|e| e.key().clone()).collect()
    }

    /// Two-phase stop (mainstream parity: cooperative cancel, then a hard abort after
    /// a grace window — cf. Codex's drain budget): cancellation gives the task the
    /// chance to run its epilogue; if the entry is still present after `grace`, the
    /// task is stuck in an await that does not observe the token (a blocking wait, a
    /// sync mutex, or a regression), so it is aborted and the caller runs the terminal
    /// bookkeeping (checkpoints + terminal done event) in its place.
    ///
    /// The abort is identity-matched: if a replacement generation registered under the
    /// same key during the grace window, it is left running untouched.
    pub async fn stop_with_escalation(
        &self,
        key: &StreamTaskKey,
        grace: std::time::Duration,
    ) -> StopOutcome {
        let Some(turn_id) = self.0.get(key).map(|e| e.2.clone()) else {
            return StopOutcome::NotFound;
        };
        self.cancel(key);
        let deadline = tokio::time::Instant::now() + grace;
        loop {
            if !self.0.contains_key(key) {
                return StopOutcome::Graceful;
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                break;
            }
            tokio::time::sleep((deadline - now).min(std::time::Duration::from_millis(50))).await;
        }
        if let Some((_, (handle, _, _))) = self.0.remove_if(key, |_, (_, _, tid)| *tid == turn_id) {
            handle.abort();
            StopOutcome::Aborted(turn_id)
        } else {
            // The entry vanished at the last moment, or a new generation owns the key now.
            StopOutcome::Graceful
        }
    }

    /// Generation replacement (LOOP-1): takes the OLD entry out, cancels it, and
    /// awaits its epilogue bounded by `wait`. The old task's self-removal is
    /// identity-matched ([`remove_own`]), so even a task that outlives `wait` —
    /// e.g. stuck in a 300s command — can no longer corrupt the replacement's entry.
    pub async fn replace_generation(&self, key: &StreamTaskKey, wait: std::time::Duration) {
        if let Some((_, (old_handle, old_token, _))) = self.0.remove(key) {
            old_token.cancel();
            let _ = tokio::time::timeout(wait, old_handle).await;
        }
    }

    /// Identity-matched self-removal (epilogue): removes the entry only when it still
    /// belongs to THIS turn generation. After a replacement raced past its wait
    /// window, the entry under `key` may already belong to the NEW task — a keyed
    /// remove would orphan it (stop could never find it again). Returns whether the
    /// caller's own entry was actually removed.
    pub fn remove_own(&self, key: &StreamTaskKey, turn_id: &str) -> bool {
        self.0
            .remove_if(key, |_, (_, _, entry_turn)| entry_turn == turn_id)
            .is_some()
    }
}

#[cfg(test)]
mod generation_tests {
    //! 并发 run_agent_turn 换代（报告阶段5第3项）。
    //!
    //! 协议本体在 [`StreamManager`]（阶段5 从两个命令层调用点收敛而来），此处
    //! 直接以真实 spawn 任务 + CancellationToken 驱动完整换代语义——
    //! start_paused 使时间自动推进，睡眠时序确定且瞬时。

    use super::*;
    use std::time::Duration;

    /// 旧代协作退出：cancel 后在窗口内完成收尾与自注销；换代后注册的新代
    /// 可被 stop 找到并取消（无孤儿）。
    #[tokio::test(start_paused = true)]
    async fn replacement_cancels_cooperative_old_generation_within_window() {
        let mgr = StreamManager::new();
        let key = ("assistant-1".to_string(), "topic-1".to_string());

        // Old task: waits on its token, then runs its epilogue (identity self-removal).
        let old_token = CancellationToken::new();
        let token_wait = old_token.clone();
        let mgr_inner = mgr.clone();
        let key_c = key.clone();
        let old_handle = tokio::spawn(async move {
            token_wait.cancelled().await;
            // Epilogue
            assert!(mgr_inner.remove_own(&key_c, "turn-old"));
        });
        mgr.register(key.clone(), old_handle, old_token, "turn-old".to_string());

        // Replacement: old task exits promptly → entry already gone when the window closes.
        mgr.replace_generation(&key, Duration::from_secs(10)).await;
        assert!(mgr.0.get(&key).is_none(), "协作旧代应在窗口内完成自注销");

        // New generation registers; stop must find and cancel it.
        let new_token = CancellationToken::new();
        let new_handle = tokio::spawn(std::future::pending::<()>());
        mgr.register(
            key.clone(),
            new_handle,
            new_token.clone(),
            "turn-new".to_string(),
        );
        assert!(mgr.cancel(&key), "stop 应找到新代条目");
        assert!(new_token.is_cancelled(), "stop 应取消新代令牌");
    }

    /// 旧代越窗滞留（如卡在 300s 命令）：换代的等待窗口超时后新代立即注册——
    /// 滞留旧代随后才跑收尾，其身份注销必须不腐蚀新代条目；stop 仍能找到新代。
    /// 这是 LOOP-1 的核心回归：历史的按键注销会把新代条目删成孤儿。
    #[tokio::test(start_paused = true)]
    async fn outliving_old_generation_cannot_corrupt_replacement_entry() {
        let mgr = StreamManager::new();
        let key = ("assistant-1".to_string(), "topic-2".to_string());

        // Old task: stuck in a "long command" (sleep), THEN runs its late epilogue.
        let old_token = CancellationToken::new();
        let mgr_inner = mgr.clone();
        let key_c = key.clone();
        let old_handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await; // stuck past the window
            let _removed = mgr_inner.remove_own(&key_c, "turn-old");
            // 身份不匹配：返回 false，新代条目必须毫发无损
            assert!(!_removed, "滞留旧代的注销不应命中任何条目");
        });
        mgr.register(key.clone(), old_handle, old_token, "turn-old".to_string());

        // Replacement waits only 50ms → the stuck old task outlives the window.
        mgr.replace_generation(&key, Duration::from_millis(50))
            .await;

        // New generation registers immediately.
        let new_token = CancellationToken::new();
        let new_handle = tokio::spawn(std::future::pending::<()>());
        mgr.register(
            key.clone(),
            new_handle,
            new_token.clone(),
            "turn-new".to_string(),
        );

        // Let the outliving old task run its late epilogue (auto-advanced time).
        tokio::time::sleep(Duration::from_millis(400)).await;

        let entry = mgr.0.get(&key);
        assert!(entry.is_some(), "新代条目不得被滞留旧代腐蚀删除");
        assert_eq!(entry.as_ref().unwrap().2, "turn-new");
        // stop_stream still finds the NEW generation (no orphan)
        assert!(mgr.cancel(&key));
        assert!(new_token.is_cancelled());
    }

    /// 身份注销矩阵：持有者注销命中（返回 true），他人 turn_id 注销零效果
    /// （返回 false 且条目保留）——旧按键注销无身份校验，任何旧代收尾都能
    /// 删掉新代条目，本断言固化为该缺陷的回归防线。
    #[tokio::test]
    async fn identity_mismatch_removal_is_inert() {
        let mgr = StreamManager::new();
        let key = ("assistant-2".to_string(), "topic-3".to_string());
        let token = CancellationToken::new();
        let handle = tokio::spawn(std::future::pending::<()>());
        mgr.register(key.clone(), handle, token, "turn-new".to_string());

        // A late OLD generation's epilogue (foreign turn id): inert.
        assert!(!mgr.remove_own(&key, "turn-old"));
        assert!(
            mgr.0.get(&key).is_some(),
            "身份不匹配的注销不得删除新代条目"
        );
        assert_eq!(mgr.0.get(&key).unwrap().2, "turn-new");

        // The owner's own removal works.
        assert!(mgr.remove_own(&key, "turn-new"));
        assert!(mgr.0.get(&key).is_none(), "持有者注销应清空条目");
        assert!(!mgr.cancel(&key), "清空后 stop 返回 false");
    }

    /// 两阶段停止：协作任务在宽限期内自注销 → Graceful，不触发 abort。
    #[tokio::test(start_paused = true)]
    async fn stop_with_escalation_graceful_within_window() {
        let mgr = StreamManager::new();
        let key = ("assistant-3".to_string(), "topic-1".to_string());
        let token = CancellationToken::new();
        let token_wait = token.clone();
        let mgr_inner = mgr.clone();
        let key_c = key.clone();
        let handle = tokio::spawn(async move {
            token_wait.cancelled().await;
            assert!(mgr_inner.remove_own(&key_c, "turn-coop"));
        });
        mgr.register(key.clone(), handle, token, "turn-coop".to_string());

        let outcome = mgr.stop_with_escalation(&key, Duration::from_secs(2)).await;
        assert!(
            matches!(outcome, StopOutcome::Graceful),
            "协作任务应走 Graceful 通道"
        );
        assert!(mgr.0.get(&key).is_none());
    }

    /// 两阶段停止：任务卡在永不观察 token 的 await（如阻塞互斥）→ 宽限期后
    /// 升级 abort，条目移除并带回 turn_id（调用方代跑终末簿记）。
    #[tokio::test(start_paused = true)]
    async fn stop_with_escalation_aborts_stuck_task() {
        let mgr = StreamManager::new();
        let key = ("assistant-3".to_string(), "topic-2".to_string());
        let token = CancellationToken::new();
        // Stuck task: never observes the token (std::future::pending).
        let handle = tokio::spawn(std::future::pending::<()>());
        mgr.register(key.clone(), handle, token.clone(), "turn-stuck".to_string());

        let outcome = mgr.stop_with_escalation(&key, Duration::from_secs(2)).await;
        match outcome {
            StopOutcome::Aborted(turn_id) => assert_eq!(turn_id, "turn-stuck"),
            _ => panic!("卡死任务必须升级为 Aborted"),
        }
        assert!(token.is_cancelled(), "协作取消仍应先发出");
        assert!(mgr.0.get(&key).is_none(), "abort 后条目必须移除");
    }

    /// 两阶段停止的身份防线：宽限期内换代注册了新代 → 升级 abort 不得误杀新代。
    #[tokio::test(start_paused = true)]
    async fn stop_with_escalation_never_aborts_replacement_generation() {
        let mgr = StreamManager::new();
        let key = ("assistant-3".to_string(), "topic-3".to_string());
        // Old stuck generation.
        let old_handle = tokio::spawn(std::future::pending::<()>());
        mgr.register(
            key.clone(),
            old_handle,
            CancellationToken::new(),
            "turn-old".to_string(),
        );

        let mgr_stop = mgr.clone();
        let key_stop = key.clone();
        let stop_fut = tokio::spawn(async move {
            mgr_stop
                .stop_with_escalation(&key_stop, Duration::from_secs(2))
                .await
        });

        // Mid-grace: a replacement takes over the key (replace_generation semantics).
        tokio::time::sleep(Duration::from_secs(1)).await;
        let new_token = CancellationToken::new();
        let new_handle = tokio::spawn(std::future::pending::<()>());
        mgr.register(
            key.clone(),
            new_handle,
            new_token.clone(),
            "turn-new".to_string(),
        );

        let outcome = stop_fut.await.expect("stop task panicked");
        assert!(
            matches!(outcome, StopOutcome::Graceful),
            "升级 abort 不得命中新代（应落回 Graceful）"
        );
        let entry = mgr.0.get(&key);
        assert!(entry.is_some(), "新代条目必须保留");
        assert_eq!(entry.as_ref().unwrap().2, "turn-new");
        assert!(!new_token.is_cancelled(), "新代令牌不得被取消");
    }

    /// 无活动流时停止为 NotFound（前端状态漂移的无害路径）。
    #[tokio::test]
    async fn stop_with_escalation_not_found() {
        let mgr = StreamManager::new();
        let key = ("assistant-9".to_string(), "topic-9".to_string());
        let outcome = mgr
            .stop_with_escalation(&key, Duration::from_millis(10))
            .await;
        assert!(matches!(outcome, StopOutcome::NotFound));
    }
}

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
            // ENG-06: kill 后 wait，避免 Unix 僵尸进程
            let _ = child.kill();
            let _ = child.wait();
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

//! Tool dispatch classification: read-only tools in the same response block run concurrently;
//! mutating tools stay serial in original order.
//!
//! Semantics follow Claude Code / Codex CLI: batched tool calls in one assistant message are
//! independent and may run concurrently; side-effecting calls (file writes, command execution,
//! Git writes, MCP (side effects unknown)) keep original order, avoiding races and approval UX
//! confusion.
//!
//! Result backfill (role:tool messages) follows original tool_calls index order; API semantics unchanged.

use crate::core::models::{AgentMode, SkillConfig, ToolSpec};
use crate::core::subagent;
use crate::utils::file_tools;
use crate::utils::git_tools;
use crate::utils::lsp_agent_tools;
use crate::utils::web_tools;
use std::collections::HashMap;
use tauri::{AppHandle, Manager};

use super::tool_specs::{artifact_read_tool_spec, skill_read_tool_spec};
use crate::core::tool_names;

/// Whether a **builtin** tool is safe to run concurrently (purely read-only, no side
/// effects, no approval dialog on the normal path). Only meaningful for calls with
/// server_id == BUILTIN_SERVER_ID; all MCP tools are always serial (side effects unknown).
pub(crate) fn is_parallel_safe(tool_name: &str) -> bool {
    matches!(
        tool_name,
        // file read/search
        tool_names::READ_FILE | tool_names::LIST_DIRECTORY | "search_files" | tool_names::SEARCH_CONTENT
            // web read-only
            | tool_names::WEB_FETCH | tool_names::WEB_SEARCH
            // git read-only
            | "git_status" | "git_diff" | "git_log"
            // LSP queries
            | "read_lints" | "lsp_definition" | "lsp_references" | "lsp_hover" | "lsp_symbols"
            // memory read-only
            | tool_names::RECALL | tool_names::SEARCH_MEMORY | "search_code"
            // other read-only / side-effect-free
            | tool_names::PROJECT_MAP | tool_names::THINK
    )
}

/// One execution outcome for the tool failure breaker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolOutcome {
    /// Executed successfully — resets the tool's consecutive counter
    Success,
    /// Environment-blocked failure (missing command / permission denied / missing
    /// dependency — text signature via `shell_tools::is_env_blocked_failure`):
    /// counts toward the disable threshold.
    EnvBlocked,
    /// Business failure (the command/tool RAN and returned nonzero — e.g. failing
    /// tests): never counts toward the threshold (harness round 3, P1-1: the
    /// model legitimately retries while fixing real failures; the round cap
    /// bounds that loop).
    Business,
}

/// Per-turn tool circuit breaker (harness-session finding: a tester subagent
/// burned its entire 25-round budget retrying environment-blocked operations).
///
/// A tool that fails with [`ToolOutcome::EnvBlocked`] CONSECUTIVELY
/// `threshold` times is disabled for the rest of the turn: further calls
/// short-circuit with [`ToolFailureBreaker::blocked_message`] instead of
/// executing. A success resets that tool's counter; a business failure leaves
/// it unchanged (unless already disabled — disabled is terminal for the turn).
/// Counts are per tool NAME, per turn (main loop and each subagent loop carry
/// their own instance).
pub(crate) struct ToolFailureBreaker {
    consecutive: HashMap<String, u32>,
    disabled: std::collections::HashSet<String>,
    threshold: u32,
}

impl Default for ToolFailureBreaker {
    fn default() -> Self {
        Self::new(0)
    }
}

impl ToolFailureBreaker {
    /// Default consecutive-env-failure threshold (harness round 3 evidence: the
    /// tester's 6th probe carried no new information; 5 alternatives had failed).
    pub const DEFAULT_THRESHOLD: u32 = 6;

    /// `configured` 0 → default; clamped to >= 2 (a threshold of 1 would disable
    /// on the first failure, leaving the model no chance to vary its approach).
    pub(crate) fn new(configured: u32) -> Self {
        let threshold = if configured == 0 {
            Self::DEFAULT_THRESHOLD
        } else {
            configured.max(2)
        };
        Self {
            consecutive: HashMap::new(),
            disabled: std::collections::HashSet::new(),
            threshold,
        }
    }

    #[cfg(test)]
    pub(crate) fn threshold(&self) -> u32 {
        self.threshold
    }

    pub(crate) fn is_disabled(&self, tool: &str) -> bool {
        self.disabled.contains(tool)
    }

    /// Records one execution outcome. Returns `Some(n)` — the current consecutive
    /// ENV-failure count — when it enters the warning band (`warn_from <= n <
    /// threshold`), so the caller can append an early-warning line to the result
    /// text (harness round 3, P1-1 suggestion: "失败的预警").
    pub(crate) fn record(&mut self, tool: &str, outcome: ToolOutcome) -> Option<u32> {
        if self.disabled.contains(tool) {
            return None; // terminal for the turn
        }
        match outcome {
            ToolOutcome::Success => {
                self.consecutive.remove(tool);
                None
            }
            ToolOutcome::Business => None, // never counts toward the disable
            ToolOutcome::EnvBlocked => {
                let n = self.consecutive.entry(tool.to_string()).or_insert(0);
                *n += 1;
                let n = *n;
                if n >= self.threshold {
                    tracing::warn!(
                        "[breaker] 工具 {} 连续环境类失败 {} 次，本回合内已停用",
                        tool,
                        n
                    );
                    self.disabled.insert(tool.to_string());
                    None
                } else if n >= self.warn_from() {
                    Some(n)
                } else {
                    None
                }
            }
        }
    }

    /// First warning-band count (3 before the threshold for the default 6).
    fn warn_from(&self) -> u32 {
        self.threshold.saturating_sub(3).max(1)
    }

    /// Early-warning line appended to the result text in the warning band.
    pub(crate) fn warn_message(&self, tool: &str, n: u32) -> String {
        format!(
            "\n\n[!] 注意：{tool} 已连续环境类失败 {n} 次；再连续失败 {} 次将在本回合内停用（通常是环境阻断：缺少解释器/依赖、权限被拒）。请考虑改用其他方式。",
            self.threshold - n
        )
    }

    /// Short-circuit result text for a call to a disabled tool.
    pub(crate) fn blocked_message(&self, tool: &str) -> String {
        format!(
            "工具 {tool} 在本回合已连续环境类失败 {} 次，已停用至回合结束（通常是环境阻断：缺少解释器/依赖、权限被拒、资源不可用）。请改用其他方式完成该部分，或在总结中如实报告无法完成；不要继续重试。",
            self.threshold
        )
    }
}

/// Classifies one tool execution into a breaker outcome from the error flag and
/// the result text (env-class via `shell_tools::is_env_blocked_failure`; needs
/// the ANSI-decoded console output — harness round 3).
pub(crate) fn classify_outcome(is_error: bool, result_text: &str) -> ToolOutcome {
    if !is_error {
        ToolOutcome::Success
    } else if crate::utils::shell_tools::is_env_blocked_failure(result_text) {
        ToolOutcome::EnvBlocked
    } else {
        ToolOutcome::Business
    }
}

/// Cross-thread tool-call statistics for one subagent run (harness round 5:
/// workflow failure gating consumes this — a step whose tool calls ALL failed
/// is a hard environment failure even though the subagent returned a summary).
#[derive(Default)]
pub(crate) struct ToolCallStats {
    total: std::sync::atomic::AtomicUsize,
    failed: std::sync::atomic::AtomicUsize,
}

impl ToolCallStats {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Records one tool-call outcome (atomic: the subagent loop's parallel phase
    /// runs read-only tools concurrently while serial results land interleaved).
    pub(crate) fn record(&self, is_error: bool) {
        use std::sync::atomic::Ordering::Relaxed;
        self.total.fetch_add(1, Relaxed);
        if is_error {
            self.failed.fetch_add(1, Relaxed);
        }
    }

    pub(crate) fn total(&self) -> usize {
        self.total.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn failed(&self) -> usize {
        self.failed.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Hard-failure signature: at least one call happened and every single one
    /// errored (the round-5 explorer step: its only read_file failed, yet the
    /// step was counted "completed" and the chain continued on garbage).
    pub(crate) fn all_failed(&self) -> bool {
        self.total() > 0 && self.failed() == self.total()
    }
}

/// Policy for tools coming from the built-in filesystem MCP server (`BUILTIN_FS_SERVER_ID`).
///
/// Why filtering is mandatory: the fs server runs as a separate stdio subprocess rooted
/// at the main project directory. Its tool calls therefore bypass the per-file write-lock
/// choke point (`execute_locked_file_tool`), the last-writer/hashline bookkeeping, and
/// worktree isolation — injecting them would let agents sidestep the whole multi-agent
/// write-coordination layer (the server itself remains for external MCP clients).
pub(crate) enum FsServerPolicy {
    /// Subagent assembly: drop every fs-server tool. The builtin file tools are always
    /// injected per profile (with locking), so every fs-server variant is a redundant,
    /// lock-free duplicate that would also escape the subagent's worktree.
    DropAll,
    /// Main-agent assembly: drop mutating file tools unconditionally (they bypass the
    /// write-lock choke point; hybrid/full provide locked builtin equivalents and the
    /// orchestrator design delegates mutations); drop read tools only when the builtin
    /// set already provides the same name (dedupe — the strict orchestrator keeps its
    /// legacy direct read access through the server).
    DropMutatingAndDuplicates,
}

/// Merge MCP tools into (tool set, tool-to-server map), handling name conflicts.
///
/// On a conflict (name already taken by a builtin tool or another server) the tool is
/// renamed to mcp__<server>__<tool> (Claude Code convention); if the renamed name still
/// conflicts it is skipped with a warning. Tools from the built-in filesystem server are
/// filtered per `fs_policy` BEFORE the rename logic (a lock-free `write_file` variant
/// must never shadow or duplicate the locked builtin path). Pure function, shared by the
/// main loop (build_main_tool_set) and subagent (build_subagent_tools) tool-set assembly.
pub(crate) fn merge_mcp_tools(
    tools: &mut Vec<ToolSpec>,
    tool_server_map: &mut std::collections::HashMap<String, String>,
    mcp: &crate::core::models::AssistantTools,
    fs_policy: FsServerPolicy,
) {
    for mut spec in mcp.tools.clone() {
        let original = spec.function.name.clone();
        let Some(server_id) = mcp.tool_server_map.get(&original).cloned() else {
            continue;
        };
        if server_id == crate::core::ids::BUILTIN_FS_SERVER_ID {
            match fs_policy {
                FsServerPolicy::DropAll => continue,
                FsServerPolicy::DropMutatingAndDuplicates => {
                    if file_tools::is_mutating_file_tool(&original) {
                        continue;
                    }
                    // Read variant duplicates the locked builtin tool -> dedupe, no rename
                    if tool_server_map.contains_key(&original) {
                        continue;
                    }
                }
            }
        }
        let mut exposed = original.clone();
        if tool_server_map.contains_key(&exposed) {
            exposed = format!("mcp__{}__{}", server_id, original);
            if tool_server_map.contains_key(&exposed) {
                tracing::warn!(
                    "[tools] MCP 工具名冲突且重命名后仍冲突，已跳过: {}/{}",
                    server_id,
                    original
                );
                continue;
            }
        }
        spec.function.name = exposed.clone();
        tool_server_map.insert(exposed, server_id);
        tools.push(spec);
    }
}

/// Main-loop tool-set assembly result.
pub(crate) struct MainToolSet {
    pub(crate) tools: Vec<ToolSpec>,
    /// tool name to server id (MCP prefix / `__builtin__` placeholder)
    pub(crate) tool_server_map: HashMap<String, String>,
    /// whether project memory is effective for this run (reused by system-prompt injection)
    pub(crate) memory_effective: bool,
}

/// Assemble the main-loop tool set by agent mode:
/// - Plan: research tools only (think + project_map + web)
/// - Normal/Auto/Workflow: orchestration tools (delegate_task(s) + create_workflow + think + web + project_map)
///   x builtin tool set (per tool_access: hybrid default = read-only tools direct + mutations
///   via subagent; orchestrator = legacy strict orchestrator; full = all builtin tools)
/// - chat + web search: web tools only
/// - plain chat: no tools
///
/// Injected additionally on demand: MCP tools (servers enabled on the assistant), memory
/// tools (global default x project-level override), read_skill (when Skills are enabled),
/// read_artifact (recovery channel for compressed archives).
#[allow(
    clippy::too_many_arguments,
    reason = "工具集装配的显式依赖注入，避免隐式全局状态"
)]
pub(crate) fn build_main_tool_set(
    app: &AppHandle,
    agent_mode: &AgentMode,
    web_search_enabled: bool,
    project_id: Option<&str>,
    skills: &[SkillConfig],
    memory_enabled_global: bool,
    mcp: Option<&crate::core::models::AssistantTools>,
    tool_access: &str,
) -> MainToolSet {
    let is_agent_mode = *agent_mode != AgentMode::Off;
    let (mut tools, mut tool_server_map) = if *agent_mode == AgentMode::Plan {
        // Plan mode: research tools only (think + project_map + web)
        let mut plan_tools: Vec<ToolSpec> = Vec::new();
        let mut plan_map: HashMap<String, String> = HashMap::new();

        // think
        let think_spec = crate::utils::think::tool_spec();
        plan_map.insert(
            think_spec.function.name.clone(),
            crate::core::ids::BUILTIN_SERVER_ID.into(),
        );
        plan_tools.push(think_spec);

        // project_map
        let pm_spec = crate::utils::project_map::tool_spec();
        plan_map.insert(
            pm_spec.function.name.clone(),
            crate::core::ids::BUILTIN_SERVER_ID.into(),
        );
        plan_tools.push(pm_spec);

        // web tools
        for spec in web_tools::get_web_tool_specs() {
            plan_map.insert(
                spec.function.name.clone(),
                crate::core::ids::BUILTIN_SERVER_ID.into(),
            );
            plan_tools.push(spec);
        }

        // Task board + ask_user: research tracking and clarification are legitimate in
        // Plan mode (metadata only; never touch files)
        for spec in crate::commands::llm::tasks::get_agent_task_tool_specs() {
            plan_map.insert(
                spec.function.name.clone(),
                crate::core::ids::BUILTIN_SERVER_ID.into(),
            );
            plan_tools.push(spec);
        }

        (plan_tools, plan_map)
    } else if is_agent_mode {
        // Normal/Auto/Workflow: orchestration tools only
        let mut orch_tools: Vec<ToolSpec> = Vec::new();
        let mut orch_map: HashMap<String, String> = HashMap::new();

        // delegate_task
        let delegate_spec = subagent::delegate_task_tool_spec();
        orch_map.insert(
            delegate_spec.function.name.clone(),
            crate::core::ids::BUILTIN_SERVER_ID.into(),
        );
        orch_tools.push(delegate_spec);

        // delegate_tasks (batch parallel)
        let delegate_batch_spec = subagent::delegate_tasks_tool_spec();
        orch_map.insert(
            delegate_batch_spec.function.name.clone(),
            crate::core::ids::BUILTIN_SERVER_ID.into(),
        );
        orch_tools.push(delegate_batch_spec);

        // get_subagent_result (background delegate pull channel)
        let get_result_spec = subagent::get_subagent_result_tool_spec();
        orch_map.insert(
            get_result_spec.function.name.clone(),
            crate::core::ids::BUILTIN_SERVER_ID.into(),
        );
        orch_tools.push(get_result_spec);

        // create_workflow
        let workflow_spec = subagent::create_workflow_tool_spec();
        orch_map.insert(
            workflow_spec.function.name.clone(),
            crate::core::ids::BUILTIN_SERVER_ID.into(),
        );
        orch_tools.push(workflow_spec);

        // think
        let think_spec = crate::utils::think::tool_spec();
        orch_map.insert(
            think_spec.function.name.clone(),
            crate::core::ids::BUILTIN_SERVER_ID.into(),
        );
        orch_tools.push(think_spec);

        // web tools
        for spec in web_tools::get_web_tool_specs() {
            orch_map.insert(
                spec.function.name.clone(),
                crate::core::ids::BUILTIN_SERVER_ID.into(),
            );
            orch_tools.push(spec);
        }

        // project_map
        let pm_spec = crate::utils::project_map::tool_spec();
        orch_map.insert(
            pm_spec.function.name.clone(),
            crate::core::ids::BUILTIN_SERVER_ID.into(),
        );
        orch_tools.push(pm_spec);

        // Task board (create_tasks / update_task / list_tasks) + ask_user: the
        // orchestrator's progress-tracking and user-interaction tools (main agent only)
        for spec in crate::commands::llm::tasks::get_agent_task_tool_specs() {
            orch_map.insert(
                spec.function.name.clone(),
                crate::core::ids::BUILTIN_SERVER_ID.into(),
            );
            orch_tools.push(spec);
        }

        // Builtin tool set: the main agent holds builtin tools directly, removing the
        // subagent round-trip latency for even a single file read. hybrid (default) =
        // read-only set (is_parallel_safe is the read-only predicate); full = all builtin
        // tools (Claude Code shape); orchestrator = legacy (strict orchestrator, orchestration tools only).
        let project_root = file_tools::resolve_project_root(app, project_id).ok();
        let git_ok = project_root
            .as_deref()
            .map(git_tools::is_git_repo)
            .unwrap_or(false);
        let builtin_specs: Vec<ToolSpec> = file_tools::get_file_tool_specs()
            .into_iter()
            .chain(
                git_tools::get_git_tool_specs()
                    .into_iter()
                    .filter(|_| git_ok),
            )
            .chain(lsp_agent_tools::get_all_lsp_tool_specs())
            .collect();
        let include = |name: &str| match tool_access {
            "orchestrator" => false,
            "full" => true,
            // hybrid: read-only set
            _ => is_parallel_safe(name),
        };
        for spec in builtin_specs {
            if include(&spec.function.name) {
                orch_map.insert(
                    spec.function.name.clone(),
                    crate::core::ids::BUILTIN_SERVER_ID.into(),
                );
                orch_tools.push(spec);
            }
        }
        // Command execution tool: direct access only in full mode (under hybrid,
        // execute_command is a mutating op and stays delegated; debugger/general profiles can run it)
        if tool_access == "full" {
            let shell_spec = crate::utils::shell_tools::get_command_tool_spec();
            orch_map.insert(
                shell_spec.function.name.clone(),
                crate::core::ids::BUILTIN_SERVER_ID.into(),
            );
            orch_tools.push(shell_spec);
        }

        (orch_tools, orch_map)
    } else if web_search_enabled {
        // inject web tools only (web search in chat mode)
        let web_specs = web_tools::get_web_tool_specs();
        let mut mcp_map = std::collections::HashMap::new();
        for spec in &web_specs {
            mcp_map.insert(
                spec.function.name.clone(),
                crate::core::ids::BUILTIN_SERVER_ID.into(),
            );
        }
        (web_specs, mcp_map)
    } else {
        (Vec::new(), std::collections::HashMap::new())
    };
    // MCP tool injection: agent modes (Normal/Auto/Workflow) inject tools from the servers
    // enabled on the assistant; Plan mode keeps a pure research tool set (MCP tool side
    // effects unknown, they fall back to Ask). On a name conflict (taken by a builtin tool
    // or another server) the tool is renamed to mcp__<server>__<tool>, keeping routing deterministic.
    if is_agent_mode && *agent_mode != AgentMode::Plan {
        if let Some(mcp) = mcp {
            merge_mcp_tools(
                &mut tools,
                &mut tool_server_map,
                mcp,
                FsServerPolicy::DropMutatingAndDuplicates,
            );
        }
    }
    // Memory tools: global default + per-project override decide enablement; when enabled the memory tools are injected, otherwise the legacy knowledge tools are used
    let mut memory_effective = false;
    if is_agent_mode {
        if let Some(pid) = project_id {
            let root = file_tools::resolve_project_root(app, Some(pid)).ok();
            let override_enabled = root
                .and_then(|r| {
                    app.state::<crate::services::memory::MemoryStoreManager>()
                        .open(&r)
                        .ok()
                })
                .and_then(|s| s.project_enabled().ok())
                .flatten();
            memory_effective = override_enabled.unwrap_or(memory_enabled_global);
            if memory_effective {
                for spec in crate::services::memory::tools::get_memory_tool_specs() {
                    tool_server_map.insert(
                        spec.function.name.clone(),
                        crate::core::ids::BUILTIN_SERVER_ID.into(),
                    );
                    tools.push(spec);
                }
            }
        }
    }
    // Skills on demand - only the read_skill tool is injected; the model reads full contents on demand
    if is_agent_mode && !skills.is_empty() {
        let spec = skill_read_tool_spec();
        tool_server_map.insert(
            spec.function.name.clone(),
            crate::core::ids::BUILTIN_SERVER_ID.into(),
        );
        tools.push(spec);
    }
    // history-archive read tool (recovery channel for compression fallback archives)
    if is_agent_mode {
        let spec = artifact_read_tool_spec();
        tool_server_map.insert(
            spec.function.name.clone(),
            crate::core::ids::BUILTIN_SERVER_ID.into(),
        );
        tools.push(spec);
    }
    MainToolSet {
        tools,
        tool_server_map,
        memory_effective,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_tools_are_parallel_safe() {
        for name in [
            tool_names::READ_FILE,
            tool_names::LIST_DIRECTORY,
            "search_files",
            tool_names::SEARCH_CONTENT,
            tool_names::WEB_FETCH,
            tool_names::WEB_SEARCH,
            "git_status",
            "git_diff",
            "git_log",
            "read_lints",
            "lsp_definition",
            "lsp_references",
            "lsp_hover",
            "lsp_symbols",
            tool_names::RECALL,
            tool_names::SEARCH_MEMORY,
            "search_code",
            tool_names::PROJECT_MAP,
            tool_names::THINK,
        ] {
            assert!(is_parallel_safe(name), "{name} 应为并行安全");
        }
    }

    #[test]
    fn mutating_tools_stay_serial() {
        for name in [
            tool_names::WRITE_FILE,
            tool_names::REPLACE_IN_FILE,
            tool_names::DELETE_FILE,
            tool_names::MAKE_DIRECTORY,
            tool_names::EXECUTE_COMMAND,
            "git_add",
            "git_commit",
            tool_names::REMEMBER,
            "update_memory",
            "forget_memory",
            // orchestration tools: each goes through its own dedicated path
            tool_names::DELEGATE_TASK,
            tool_names::DELEGATE_TASKS,
            tool_names::CREATE_WORKFLOW,
            tool_names::GET_SUBAGENT_RESULT,
            // unknown tool
            "some_mcp_tool",
            "",
        ] {
            assert!(!is_parallel_safe(name), "{name} 必须串行执行");
        }
    }

    /// Build a minimal ToolSpec (only the name participates in merge semantics).
    fn spec_named(name: &str) -> ToolSpec {
        serde_json::from_value(serde_json::json!({
            "type": "function",
            "function": {
                "name": name,
                "description": "test",
                "parameters": {"type": "object", "properties": {}}
            }
        }))
        .unwrap()
    }

    fn mcp_set(entries: &[(&str, &str)]) -> crate::core::models::AssistantTools {
        crate::core::models::AssistantTools {
            tools: entries.iter().map(|(n, _)| spec_named(n)).collect(),
            tool_server_map: entries
                .iter()
                .map(|(n, s)| (n.to_string(), s.to_string()))
                .collect(),
        }
    }

    /// No conflict: the original name is kept, mapped to its server.
    #[test]
    fn merge_mcp_tools_no_conflict() {
        let mut tools = vec![spec_named(tool_names::READ_FILE)];
        let mut map = std::collections::HashMap::from([(
            tool_names::READ_FILE.to_string(),
            "builtin".to_string(),
        )]);
        merge_mcp_tools(
            &mut tools,
            &mut map,
            &mcp_set(&[("mcp_query", "srv1")]),
            FsServerPolicy::DropMutatingAndDuplicates,
        );
        assert_eq!(tools.len(), 2);
        assert_eq!(map.get("mcp_query").map(String::as_str), Some("srv1"));
        assert_eq!(tools[1].function.name, "mcp_query");
    }

    /// Conflict with a builtin tool: renamed to mcp__<server>__<tool>.
    #[test]
    fn merge_mcp_tools_conflict_with_builtin_renames() {
        let mut tools = vec![spec_named(tool_names::READ_FILE)];
        let mut map = std::collections::HashMap::from([(
            tool_names::READ_FILE.to_string(),
            "builtin".to_string(),
        )]);
        merge_mcp_tools(
            &mut tools,
            &mut map,
            &mcp_set(&[(tool_names::READ_FILE, "srv1")]),
            FsServerPolicy::DropMutatingAndDuplicates,
        );
        assert_eq!(
            map.get("mcp__srv1__read_file").map(String::as_str),
            Some("srv1")
        );
        // the original name still maps to builtin
        assert_eq!(
            map.get(tool_names::READ_FILE).map(String::as_str),
            Some("builtin")
        );
    }

    /// Two servers offering the same tool name: the first keeps the original name, later ones are renamed.
    #[test]
    fn merge_mcp_tools_two_servers_same_name() {
        let mut tools = Vec::new();
        let mut map = std::collections::HashMap::new();
        merge_mcp_tools(
            &mut tools,
            &mut map,
            &mcp_set(&[("search", "srv1")]),
            FsServerPolicy::DropMutatingAndDuplicates,
        );
        merge_mcp_tools(
            &mut tools,
            &mut map,
            &mcp_set(&[("search", "srv2")]),
            FsServerPolicy::DropMutatingAndDuplicates,
        );
        assert_eq!(map.get("search").map(String::as_str), Some("srv1"));
        assert_eq!(
            map.get("mcp__srv2__search").map(String::as_str),
            Some("srv2")
        );
    }

    /// Still conflicting after rename: the tool is skipped.
    #[test]
    fn merge_mcp_tools_double_conflict_skips() {
        let mut tools = Vec::new();
        let mut map = std::collections::HashMap::from([(
            "mcp__srv1__search".to_string(),
            "other".to_string(),
        )]);
        merge_mcp_tools(
            &mut tools,
            &mut map,
            &mcp_set(&[("search", "srv1")]),
            FsServerPolicy::DropMutatingAndDuplicates,
        );
        // "search" is absent (unclaimed) -> no conflict, the original name is kept
        assert_eq!(map.get("search").map(String::as_str), Some("srv1"));

        // original name taken + rename target taken -> skip
        let mut map2 = std::collections::HashMap::from([
            ("search".to_string(), "builtin".to_string()),
            ("mcp__srv1__search".to_string(), "other".to_string()),
        ]);
        let mut tools2 = Vec::new();
        merge_mcp_tools(
            &mut tools2,
            &mut map2,
            &mcp_set(&[("search", "srv1")]),
            FsServerPolicy::DropMutatingAndDuplicates,
        );
        assert_eq!(tools2.len(), 0);
        assert_eq!(map2.len(), 2, "映射不应新增");
    }

    /// P0-1 regression: the built-in fs server's MUTATING tools must never enter the
    /// main agent's tool set under any tool_access mode — they run in a subprocess rooted
    /// at the main project and would bypass the per-file write-lock choke point.
    #[test]
    fn fs_server_mutating_tools_never_reach_the_main_agent() {
        for (tool_access, builtin_mutating_injected) in
            [("hybrid", false), ("full", true), ("orchestrator", false)]
        {
            // Mirror the three modes' builtin mutating-tool presence
            let mut map = std::collections::HashMap::new();
            let mut tools = Vec::new();
            if builtin_mutating_injected {
                map.insert(tool_names::WRITE_FILE.to_string(), "builtin".to_string());
                tools.push(spec_named(tool_names::WRITE_FILE));
            }
            merge_mcp_tools(
                &mut tools,
                &mut map,
                &mcp_set(&[
                    (
                        tool_names::WRITE_FILE,
                        crate::core::ids::BUILTIN_FS_SERVER_ID,
                    ),
                    (
                        tool_names::REPLACE_IN_FILE,
                        crate::core::ids::BUILTIN_FS_SERVER_ID,
                    ),
                    (
                        tool_names::DELETE_FILE,
                        crate::core::ids::BUILTIN_FS_SERVER_ID,
                    ),
                    (
                        tool_names::MAKE_DIRECTORY,
                        crate::core::ids::BUILTIN_FS_SERVER_ID,
                    ),
                ]),
                FsServerPolicy::DropMutatingAndDuplicates,
            );
            assert!(
                map.values()
                    .all(|v| v.as_str() != crate::core::ids::BUILTIN_FS_SERVER_ID),
                "{tool_access}: 变更类文件工具不得路由到 fs server"
            );
            if builtin_mutating_injected {
                assert_eq!(
                    map.get(tool_names::WRITE_FILE).map(String::as_str),
                    Some("builtin"),
                    "{tool_access}: 内置 write_file 不得被覆盖"
                );
            }
        }
    }

    /// Main policy dedupes fs-server read tools against their locked builtin equivalents
    /// (hybrid/full) but keeps them when only the server provides reads (orchestrator).
    #[test]
    fn fs_server_read_tools_dedupe_only_when_builtin_provides() {
        // hybrid/full: builtin read present -> fs variant dropped, NOT renamed
        let mut map = std::collections::HashMap::from([(
            tool_names::READ_FILE.to_string(),
            "builtin".to_string(),
        )]);
        let mut tools = vec![spec_named(tool_names::READ_FILE)];
        merge_mcp_tools(
            &mut tools,
            &mut map,
            &mcp_set(&[(
                tool_names::READ_FILE,
                crate::core::ids::BUILTIN_FS_SERVER_ID,
            )]),
            FsServerPolicy::DropMutatingAndDuplicates,
        );
        assert_eq!(tools.len(), 1, "重复读工具应去重");
        assert_eq!(map.len(), 1, "不应产生 mcp__ 改名条目");

        // orchestrator: no builtin read -> legacy direct read through the server is kept
        let mut map2 = std::collections::HashMap::new();
        let mut tools2 = Vec::new();
        merge_mcp_tools(
            &mut tools2,
            &mut map2,
            &mcp_set(&[(
                tool_names::READ_FILE,
                crate::core::ids::BUILTIN_FS_SERVER_ID,
            )]),
            FsServerPolicy::DropMutatingAndDuplicates,
        );
        assert_eq!(
            map2.get(tool_names::READ_FILE).map(String::as_str),
            Some(crate::core::ids::BUILTIN_FS_SERVER_ID)
        );
    }

    /// P1-3 regression: subagent assembly drops EVERY fs-server tool — the server runs
    /// rooted at the main project, so its calls would escape the subagent's worktree
    /// isolation and the write locks.
    #[test]
    fn fs_server_tools_never_reach_subagents() {
        let mut map = std::collections::HashMap::from([(
            tool_names::READ_FILE.to_string(),
            "builtin".to_string(),
        )]);
        let mut tools = vec![spec_named(tool_names::READ_FILE)];
        merge_mcp_tools(
            &mut tools,
            &mut map,
            &mcp_set(&[
                (
                    tool_names::READ_FILE,
                    crate::core::ids::BUILTIN_FS_SERVER_ID,
                ),
                (
                    tool_names::WRITE_FILE,
                    crate::core::ids::BUILTIN_FS_SERVER_ID,
                ),
                ("search_files", crate::core::ids::BUILTIN_FS_SERVER_ID),
            ]),
            FsServerPolicy::DropAll,
        );
        assert_eq!(tools.len(), 1, "fs server 工具应全部被丢弃");
        assert_eq!(map.len(), 1, "映射不应新增");
    }

    /// The filter is scoped to the built-in fs server: a USER server offering a
    /// write_file tool follows the normal rename rules untouched.
    #[test]
    fn user_server_write_tools_are_not_filtered() {
        let mut map = std::collections::HashMap::new();
        let mut tools = Vec::new();
        merge_mcp_tools(
            &mut tools,
            &mut map,
            &mcp_set(&[(tool_names::WRITE_FILE, "my-git-helper")]),
            FsServerPolicy::DropMutatingAndDuplicates,
        );
        assert_eq!(
            map.get(tool_names::WRITE_FILE).map(String::as_str),
            Some("my-git-helper")
        );
    }

    // ====== ToolFailureBreaker ======

    use super::ToolOutcome;

    /// Reaching the threshold of ENV failures disables the tool for the rest of
    /// the turn (harness round 3 reproduced: 6 consecutive nosuchcmd probes).
    #[test]
    fn breaker_disables_after_consecutive_env_failures() {
        let mut b = ToolFailureBreaker::default();
        for i in 0..5 {
            b.record("execute_command", ToolOutcome::EnvBlocked);
            assert!(!b.is_disabled("execute_command"), "第 {i} 次不应停用");
        }
        b.record("execute_command", ToolOutcome::EnvBlocked); // 6th consecutive
        assert!(
            b.is_disabled("execute_command"),
            "连续 6 次环境类失败应停用"
        );
        // Disabled is terminal for the turn: a success record keeps it disabled.
        b.record("execute_command", ToolOutcome::Success);
        assert!(b.is_disabled("execute_command"));
    }

    /// Business failures (command ran, nonzero exit) NEVER trip the breaker —
    /// harness round 3, P1-1: `pytest` failing while the model fixes real bugs
    /// is legitimate retry behavior bounded by the round cap.
    #[test]
    fn breaker_ignores_business_failures() {
        let mut b = ToolFailureBreaker::default();
        for _ in 0..50 {
            b.record("execute_command", ToolOutcome::Business);
            assert!(!b.is_disabled("execute_command"));
        }
        // Interleaved business failures do not even advance the env counter.
        b.record("execute_command", ToolOutcome::EnvBlocked);
        b.record("execute_command", ToolOutcome::Business);
        b.record("execute_command", ToolOutcome::EnvBlocked);
        b.record("execute_command", ToolOutcome::Business);
        assert!(!b.is_disabled("execute_command"), "环境计数应只有 2");
    }

    /// A success resets the consecutive counter — interleaved env failures never trip.
    #[test]
    fn breaker_success_resets_counter() {
        let mut b = ToolFailureBreaker::default();
        for _ in 0..20 {
            b.record("del", ToolOutcome::EnvBlocked);
            b.record("del", ToolOutcome::Success); // intermittent failures are not environment blocking
            assert!(!b.is_disabled("del"));
        }
    }

    /// Per-tool isolation: one blocked tool does not affect others.
    #[test]
    fn breaker_is_per_tool() {
        let mut b = ToolFailureBreaker::default();
        for _ in 0..6 {
            b.record("python_probe", ToolOutcome::EnvBlocked);
        }
        assert!(b.is_disabled("python_probe"));
        assert!(!b.is_disabled("other_tool"));
        b.record("other_tool", ToolOutcome::EnvBlocked);
        assert!(!b.is_disabled("other_tool"));
    }

    /// The blocked message names the tool and the threshold.
    #[test]
    fn breaker_message_names_tool_and_threshold() {
        let b = ToolFailureBreaker::default();
        let msg = b.blocked_message("execute_command");
        assert!(msg.contains("execute_command"));
        assert!(msg.contains("6"));
    }

    /// Configurable threshold (harness round 3, P1-1 suggestion 1): explicit
    /// values apply (clamped to >= 2); 0 keeps the default.
    #[test]
    fn breaker_threshold_is_configurable() {
        let mut b = ToolFailureBreaker::new(3);
        assert_eq!(b.threshold(), 3);
        b.record("t", ToolOutcome::EnvBlocked);
        b.record("t", ToolOutcome::EnvBlocked);
        assert!(!b.is_disabled("t"), "阈值 3：2 次不应停用");
        b.record("t", ToolOutcome::EnvBlocked);
        assert!(b.is_disabled("t"), "阈值 3：3 次应停用");
        // Clamp: a threshold of 1 would disable on first failure — disallowed.
        let b1 = ToolFailureBreaker::new(1);
        assert_eq!(b1.threshold(), 2, "阈值应钳制到 2");
        // 0 → default
        assert_eq!(ToolFailureBreaker::new(0).threshold(), 6);
    }

    /// Early warning (harness round 3, P1-1 suggestion 2): from the 3rd
    /// consecutive env failure the caller receives the count to append a
    /// warning line; the message names remaining budget.
    #[test]
    fn breaker_warns_before_trip() {
        let mut b = ToolFailureBreaker::default();
        assert!(
            b.record("t", ToolOutcome::EnvBlocked).is_none(),
            "第 1 次不预警"
        );
        assert!(
            b.record("t", ToolOutcome::EnvBlocked).is_none(),
            "第 2 次不预警"
        );
        let n = b
            .record("t", ToolOutcome::EnvBlocked)
            .expect("第 3 次应预警");
        assert_eq!(n, 3);
        let warn = b.warn_message("t", n);
        assert!(warn.contains("t"));
        assert!(warn.contains("3"));
        // 4th and 5th stay in the warning band (Some with the count)
        assert_eq!(b.record("t", ToolOutcome::EnvBlocked), Some(4));
        assert_eq!(b.record("t", ToolOutcome::EnvBlocked), Some(5));
        // 6th trips: disables and returns None (the disable message replaces warnings)
        assert!(b.record("t", ToolOutcome::EnvBlocked).is_none());
        assert!(b.is_disabled("t"));
    }

    // ====== ToolCallStats (harness round 5, workflow failure gating) ======

    /// all_failed is the hard-failure signature: >0 calls and every one errored.
    #[test]
    fn stats_all_failed_signature() {
        let s = ToolCallStats::new();
        assert!(!s.all_failed(), "无调用不算失败");
        s.record(true);
        assert!(
            s.all_failed(),
            "唯一调用失败 = 硬失败（round-5 explorer 场景）"
        );
        s.record(false);
        assert!(!s.all_failed(), "有成功调用则不判硬失败");
        assert_eq!(s.total(), 2);
        assert_eq!(s.failed(), 1);
    }

    /// Mixed outcomes across threads (atomicity smoke).
    #[test]
    fn stats_concurrent_mixed_outcomes() {
        let s = std::sync::Arc::new(ToolCallStats::new());
        let mut joins = Vec::new();
        for t in 0..4 {
            let s2 = s.clone();
            joins.push(std::thread::spawn(move || {
                for i in 0..100 {
                    s2.record((t * 100 + i) % 3 == 0);
                }
            }));
        }
        for j in joins {
            j.join().unwrap();
        }
        assert_eq!(s.total(), 400);
        assert_eq!(s.failed(), 134); // per thread: 34 of 100 (i%3==0)
    }
}

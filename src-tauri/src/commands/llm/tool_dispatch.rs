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

/// Whether a **builtin** tool is safe to run concurrently (purely read-only, no side
/// effects, no approval dialog on the normal path). Only meaningful for calls with
/// server_id == BUILTIN_SERVER_ID; all MCP tools are always serial (side effects unknown).
pub(crate) fn is_parallel_safe(tool_name: &str) -> bool {
    matches!(
        tool_name,
        // file read/search
        "read_file" | "list_directory" | "search_files" | "search_content"
            // web read-only
            | "web_fetch" | "web_search"
            // git read-only
            | "git_status" | "git_diff" | "git_log"
            // LSP queries
            | "read_lints" | "lsp_definition" | "lsp_references" | "lsp_hover" | "lsp_symbols"
            // memory read-only
            | "recall" | "search_memory" | "search_code"
            // other read-only / side-effect-free
            | "project_map" | "think"
    )
}

/// Merge MCP tools into (tool set, tool-to-server map), handling name conflicts.
///
/// On a conflict (name already taken by a builtin tool or another server) the tool is
/// renamed to mcp__<server>__<tool> (Claude Code convention); if the renamed name still
/// conflicts it is skipped with a warning. Pure function, shared by the main loop
/// (build_main_tool_set) and subagent (build_subagent_tools) tool-set assembly.
pub(crate) fn merge_mcp_tools(
    tools: &mut Vec<ToolSpec>,
    tool_server_map: &mut std::collections::HashMap<String, String>,
    mcp: &crate::core::models::AssistantTools,
) {
    for mut spec in mcp.tools.clone() {
        let original = spec.function.name.clone();
        let Some(server_id) = mcp.tool_server_map.get(&original).cloned() else {
            continue;
        };
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
            merge_mcp_tools(&mut tools, &mut tool_server_map, mcp);
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
            "read_file",
            "list_directory",
            "search_files",
            "search_content",
            "web_fetch",
            "web_search",
            "git_status",
            "git_diff",
            "git_log",
            "read_lints",
            "lsp_definition",
            "lsp_references",
            "lsp_hover",
            "lsp_symbols",
            "recall",
            "search_memory",
            "search_code",
            "project_map",
            "think",
        ] {
            assert!(is_parallel_safe(name), "{name} 应为并行安全");
        }
    }

    #[test]
    fn mutating_tools_stay_serial() {
        for name in [
            "write_file",
            "replace_in_file",
            "delete_file",
            "make_directory",
            "execute_command",
            "git_add",
            "git_commit",
            "remember",
            "update_memory",
            "forget_memory",
            // orchestration tools: each goes through its own dedicated path
            "delegate_task",
            "delegate_tasks",
            "create_workflow",
            "get_subagent_result",
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
        let mut tools = vec![spec_named("read_file")];
        let mut map =
            std::collections::HashMap::from([("read_file".to_string(), "builtin".to_string())]);
        merge_mcp_tools(&mut tools, &mut map, &mcp_set(&[("mcp_query", "srv1")]));
        assert_eq!(tools.len(), 2);
        assert_eq!(map.get("mcp_query").map(String::as_str), Some("srv1"));
        assert_eq!(tools[1].function.name, "mcp_query");
    }

    /// Conflict with a builtin tool: renamed to mcp__<server>__<tool>.
    #[test]
    fn merge_mcp_tools_conflict_with_builtin_renames() {
        let mut tools = vec![spec_named("read_file")];
        let mut map =
            std::collections::HashMap::from([("read_file".to_string(), "builtin".to_string())]);
        merge_mcp_tools(&mut tools, &mut map, &mcp_set(&[("read_file", "srv1")]));
        assert_eq!(
            map.get("mcp__srv1__read_file").map(String::as_str),
            Some("srv1")
        );
        // the original name still maps to builtin
        assert_eq!(map.get("read_file").map(String::as_str), Some("builtin"));
    }

    /// Two servers offering the same tool name: the first keeps the original name, later ones are renamed.
    #[test]
    fn merge_mcp_tools_two_servers_same_name() {
        let mut tools = Vec::new();
        let mut map = std::collections::HashMap::new();
        merge_mcp_tools(&mut tools, &mut map, &mcp_set(&[("search", "srv1")]));
        merge_mcp_tools(&mut tools, &mut map, &mcp_set(&[("search", "srv2")]));
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
        merge_mcp_tools(&mut tools, &mut map, &mcp_set(&[("search", "srv1")]));
        // "search" is absent (unclaimed) -> no conflict, the original name is kept
        assert_eq!(map.get("search").map(String::as_str), Some("srv1"));

        // original name taken + rename target taken -> skip
        let mut map2 = std::collections::HashMap::from([
            ("search".to_string(), "builtin".to_string()),
            ("mcp__srv1__search".to_string(), "other".to_string()),
        ]);
        let mut tools2 = Vec::new();
        merge_mcp_tools(&mut tools2, &mut map2, &mcp_set(&[("search", "srv1")]));
        assert_eq!(tools2.len(), 0);
        assert_eq!(map2.len(), 2, "映射不应新增");
    }
}

//! 工具分发分类：同一响应块内的只读工具并发执行，变更类工具保持顺序串行。
//!
//! 语义对齐 Claude Code / Codex CLI：模型在一个 assistant 消息里批量发出的
//! 工具调用视为相互独立，可并发执行；副作用类（文件写入、命令执行、Git 写、
//! MCP——副作用未知）保持原始顺序串行，避免竞态与审批 UX 混乱。
//!
//! 结果回填（role:tool 消息）始终按原始 tool_calls 索引顺序，API 语义不变。

use crate::core::models::{AgentMode, SkillConfig, ToolSpec};
use crate::core::subagent;
use crate::utils::file_tools;
use crate::utils::git_tools;
use crate::utils::lsp_agent_tools;
use crate::utils::web_tools;
use std::collections::HashMap;
use tauri::{AppHandle, Manager};

use super::tool_specs::{artifact_read_tool_spec, skill_read_tool_spec};

/// 判定一个**内置**工具是否可安全并发执行（纯只读、无副作用、无审批弹窗
/// 的常态路径）。仅对 server_id == BUILTIN_SERVER_ID 的工具调用有意义；
/// 全部 MCP 工具一律串行（副作用未知）。
pub(crate) fn is_parallel_safe(tool_name: &str) -> bool {
    matches!(
        tool_name,
        // 文件读取/搜索
        "read_file" | "list_directory" | "search_files" | "search_content"
            // Web 只读
            | "web_fetch" | "web_search"
            // Git 只读
            | "git_status" | "git_diff" | "git_log"
            // LSP 查询
            | "read_lints" | "lsp_definition" | "lsp_references" | "lsp_hover" | "lsp_symbols"
            // 记忆只读
            | "recall" | "search_memory" | "search_code"
            // 其他只读 / 无副作用
            | "project_map" | "think"
    )
}

/// 将 MCP 工具合并进（工具集, tool→server 映射），处理命名冲突。
///
/// 冲突（内置工具或其他 server 已占用该名）时重命名为 mcp__<server>__<tool>
///（对齐 Claude Code 惯例）；重命名后仍冲突则跳过并告警。纯函数，
/// 主循环（build_main_tool_set）与子代理（build_subagent_tools）的工具集装配共用。
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

/// 主循环工具集装配结果。
pub(crate) struct MainToolSet {
    pub(crate) tools: Vec<ToolSpec>,
    /// 工具名 → server 标识（MCP 前缀 / `__builtin__` 占位）
    pub(crate) tool_server_map: HashMap<String, String>,
    /// 项目记忆是否对本次运行生效（供后续系统提示词注入判定复用）
    pub(crate) memory_effective: bool,
}

/// 按 Agent 模式组装主循环工具集：
/// - Plan：仅研究工具（think + project_map + web）
/// - Normal/Auto/Workflow：编排工具（delegate_task(s) + create_workflow + think + web + project_map）
///   × 内置工具集（tool_access 决定：hybrid 默认 = 只读工具直达 + 变更走子代理；
///   orchestrator = 旧行为严格编排者；full = 全部内置工具）
/// - 对话 + 联网：仅 web 工具
/// - 纯对话：无工具
///
/// 并按需附加注入：MCP 工具（助手勾选的 server）、记忆工具（全局默认 × 项目级覆盖）、
/// read_skill（有已启用 Skills 时）、read_artifact（压缩归档恢复通道）。
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
        // Plan 模式：仅研究工具（think + project_map + web）
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
        // Normal/Auto/Workflow: 仅编排工具
        let mut orch_tools: Vec<ToolSpec> = Vec::new();
        let mut orch_map: HashMap<String, String> = HashMap::new();

        // delegate_task
        let delegate_spec = subagent::delegate_task_tool_spec();
        orch_map.insert(
            delegate_spec.function.name.clone(),
            crate::core::ids::BUILTIN_SERVER_ID.into(),
        );
        orch_tools.push(delegate_spec);

        // delegate_tasks（批量并行）
        let delegate_batch_spec = subagent::delegate_tasks_tool_spec();
        orch_map.insert(
            delegate_batch_spec.function.name.clone(),
            crate::core::ids::BUILTIN_SERVER_ID.into(),
        );
        orch_tools.push(delegate_batch_spec);

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

        // 内置工具集（P2.3）：主 Agent 直接持有内置工具，消除"读一个文件也要
        // 一次子代理往返"的延迟税。hybrid（默认）= 只读集合（is_parallel_safe
        // 即只读判定）；full = 全部内置工具（Claude Code 形态）；orchestrator =
        // 旧行为（严格编排者，仅上面的编排工具）。
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
            // hybrid：只读集合
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
        // 命令执行工具：仅 full 模式直达（hybrid 下 execute_command 属于变更操作，
        // 保持委托子代理——debugger/general profile 可执行）
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
        // 仅注入 Web 工具（对话模式下联网搜索）
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
    // MCP 工具注入（P2.2）：Agent 模式（Normal/Auto/Workflow）注入助手勾选 server 的
    // 工具；Plan 模式保持纯研究工具集（MCP 工具副作用未知，落入兜底 Ask）。
    // 工具名冲突（内置工具或其他 server 已占用）时重命名为 mcp__<server>__<tool>，
    // 使主循环的 tool_server_map 路由保持确定性。
    if is_agent_mode && *agent_mode != AgentMode::Plan {
        if let Some(mcp) = mcp {
            merge_mcp_tools(&mut tools, &mut tool_server_map, mcp);
        }
    }
    // 记忆工具：全局默认 + 每项目覆盖决定是否启用；启用时注入记忆工具，否则回退旧知识工具
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
    // Skills 按需注入 — 仅注入 read_skill 工具，由模型按需读取完整内容
    if is_agent_mode && !skills.is_empty() {
        let spec = skill_read_tool_spec();
        tool_server_map.insert(
            spec.function.name.clone(),
            crate::core::ids::BUILTIN_SERVER_ID.into(),
        );
        tools.push(spec);
    }
    // 历史归档读取工具（压缩回退归档的恢复通道）
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
            // 编排类：走各自的专用路径
            "delegate_task",
            "delegate_tasks",
            "create_workflow",
            // 未知工具
            "some_mcp_tool",
            "",
        ] {
            assert!(!is_parallel_safe(name), "{name} 必须串行执行");
        }
    }

    /// 构造一个最小 ToolSpec（仅名称参与 merge 语义）。
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

    /// 无冲突：原名保留，映射到对应 server。
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

    /// 与内置工具冲突：重命名为 mcp__<server>__<tool>。
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
        // 原名仍指向内置
        assert_eq!(map.get("read_file").map(String::as_str), Some("builtin"));
    }

    /// 两个 server 提供同名工具：先到者保留原名，后到者重命名。
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

    /// 重命名后仍冲突：跳过该工具。
    #[test]
    fn merge_mcp_tools_double_conflict_skips() {
        let mut tools = Vec::new();
        let mut map = std::collections::HashMap::from([(
            "mcp__srv1__search".to_string(),
            "other".to_string(),
        )]);
        merge_mcp_tools(&mut tools, &mut map, &mcp_set(&[("search", "srv1")]));
        // "search" 不存在（未占用）→ 不冲突，原名保留
        assert_eq!(map.get("search").map(String::as_str), Some("srv1"));

        // 占用原名 + 占用重命名目标 → 跳过
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

//! 子智能体与工作流编排：delegate_task(s)、execute_workflow、execute_subagent 及其辅助。

use crate::commands::llm::*;
use crate::core::events as ev;
use futures_util::stream::FuturesUnordered;
use futures_util::StreamExt;
use serde_json::json;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, Window};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
/// 构建子 Agent 的消息列表（系统提示词 + 任务描述 + 上下文提示）
pub(crate) fn build_subagent_messages(
    profile: &subagent::SubagentProfile,
    task_desc: &str,
    context_files: &[String],
    project_root: &str,
    extra_context: Option<&str>,
    locale: &str,
) -> Vec<serde_json::Value> {
    let system_prompt = if locale == "en-US" {
        format!(
            "You are an AIO subagent with the role \"{}\".\nWorking directory: {}\nComplete the task efficiently and summarize the result at the end.\n\n{}\n\nImportant: end with a concise work summary including changed file paths and key findings.",
            profile.name, project_root, profile.system_prompt_extension,
        )
    } else {
        format!(
            "你是 AIO 的子智能体，类型为「{}」。\n工作目录: {}\n请高效完成子任务并在最后总结工作成果。\n\n{}\n\n重要：请在回复末尾给出简洁的工作总结（含修改的文件路径和关键发现）。回复请保持精炼，重点突出核心成果，避免冗长叙述。",
            profile.name, project_root, profile.system_prompt_extension,
        )
    };
    let task_prompt = if locale == "en-US" {
        format!(
            "Subtask:\n{}\n\nBegin now and finish with a work summary.",
            task_desc
        )
    } else {
        format!(
            "子任务:\n{}\n\n请开始执行。完成后在最后给出工作总结。",
            task_desc
        )
    };

    let mut msgs: Vec<serde_json::Value> = vec![
        json!({"role": "system", "content": system_prompt}),
        json!({"role": "user", "content": task_prompt}),
    ];

    if !context_files.is_empty() {
        let files_hint = context_files
            .iter()
            .map(|f| format!("- {}", f))
            .collect::<Vec<_>>()
            .join("\n");
        msgs.push(json!({
            "role": "user",
            "content": if locale == "en-US" {
                format!("The following files may be relevant. Read them as needed:\n{}", files_hint)
            } else {
                format!("提示：以下文件可能与任务相关，请根据需要读取：\n{}", files_hint)
            }
        }));
    }

    if let Some(ctx) = extra_context {
        if !ctx.is_empty() {
            msgs.push(json!({
                "role": "user",
                "content": format!("{}\n{}", if locale == "en-US" { "[Shared context]" } else { "[共享上下文]" }, ctx)
            }));
        }
    }

    msgs
}

/// 提取 ToolResult 的首个文本内容（无文本时序列化整个 content）。
fn tool_result_text(tr: &ToolResult) -> String {
    tr.content
        .iter()
        .find(|c| c.kind == "text")
        .and_then(|c| c.data.get("text"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| serde_json::to_string(&tr.content).unwrap_or_default())
}

/// 构建子 Agent 的受限工具列表（根据 profile 过滤）。
///
/// 仅返回工具规格；子智能体的内置工具统一走 `__builtin__` 通道，
/// 主循环的 tool_map 在此无用（历史上多算了一份 8 条 HashMap 并丢弃）。
pub(crate) fn build_subagent_tools(
    profile: &subagent::SubagentProfile,
    project_root: &str,
    mcp: Option<&crate::core::models::AssistantTools>,
) -> (Vec<ToolSpec>, std::collections::HashMap<String, String>) {
    // 注入和主 Agent 相同的内置工具，但在执行时按 profile 过滤
    let mut tools: Vec<ToolSpec> = Vec::new();
    let mut server_map: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    // 始终注入的文件工具（但 profile 会限制哪些可执行）
    for spec in file_tools::get_file_tool_specs() {
        if profile.is_tool_allowed(&spec.function.name) {
            server_map.insert(
                spec.function.name.clone(),
                crate::core::ids::BUILTIN_SERVER_ID.into(),
            );
            tools.push(spec);
        }
    }
    // 命令执行工具
    if profile.is_tool_allowed("execute_command") {
        let spec = shell_tools::get_command_tool_spec();
        server_map.insert(
            spec.function.name.clone(),
            crate::core::ids::BUILTIN_SERVER_ID.into(),
        );
        tools.push(spec);
    }
    // Web 工具
    for spec in web_tools::get_web_tool_specs() {
        if profile.is_tool_allowed(&spec.function.name) {
            server_map.insert(
                spec.function.name.clone(),
                crate::core::ids::BUILTIN_SERVER_ID.into(),
            );
            tools.push(spec);
        }
    }
    // Git 工具
    for spec in git_tools::get_git_tool_specs() {
        if profile.is_tool_allowed(&spec.function.name) && is_git_repo(project_root) {
            server_map.insert(
                spec.function.name.clone(),
                crate::core::ids::BUILTIN_SERVER_ID.into(),
            );
            tools.push(spec);
        }
    }
    // LSP 工具（所有 5 个）
    for spec in lsp_agent_tools::get_all_lsp_tool_specs() {
        if profile.is_tool_allowed(&spec.function.name) {
            server_map.insert(
                spec.function.name.clone(),
                crate::core::ids::BUILTIN_SERVER_ID.into(),
            );
            tools.push(spec);
        }
    }
    // 记忆只读工具（子智能体可检索项目记忆与代码；执行时按记忆开关判定）
    for spec in crate::services::memory::tools::get_readonly_memory_specs() {
        if profile.is_tool_allowed(&spec.function.name) {
            server_map.insert(
                spec.function.name.clone(),
                crate::core::ids::BUILTIN_SERVER_ID.into(),
            );
            tools.push(spec);
        }
    }

    // MCP 工具注入（P2.2）：仅 allow_mcp_tools 的 profile（默认仅 general 与用户自定义
    // 开启项）。工具名冲突（内置工具或其他 server 占用）时重命名 mcp__<server>__<tool>，
    // 与主循环同规则；MCP 工具不受 allowed_tools 列表约束（门控即 allow_mcp_tools
    // 标志本身），denied_tools 仍可显式排除。
    if let Some(mcp) = mcp {
        if profile.allow_mcp_tools {
            // denied_tools 显式排除（按原名过滤；重命名后的排除为尽力而为）
            let filtered = crate::core::models::AssistantTools {
                tools: mcp
                    .tools
                    .iter()
                    .filter(|s| !profile.is_tool_denied(&s.function.name))
                    .cloned()
                    .collect(),
                tool_server_map: mcp.tool_server_map.clone(),
            };
            merge_mcp_tools(&mut tools, &mut server_map, &filtered);
        }
    }

    (tools, server_map)
}

/// 处理 delegate_task 工具调用（从主 Agent 循环中调用）。
///
/// 解析参数、验证权限、获取 profile，然后委托给 execute_subagent 执行。
#[allow(
    clippy::too_many_arguments,
    reason = "agent delegation keeps inherited execution context explicit"
)]
pub(crate) async fn handle_delegate_task(
    window: &Window,
    app: &AppHandle,
    client: &reqwest::Client,
    api_url: &str,
    api_key: &str,
    model: &str,
    parent_assistant_id: &str,
    parent_topic_id: &str,
    arguments: &serde_json::Value,
    project_id: Option<&str>,
    agent_mode: &AgentMode,
    semaphore: Arc<Semaphore>,
    token: &CancellationToken,
    profile_model_overrides: &[ProfileModelOverride],
    custom_profiles: &[crate::core::models::CustomSubagentProfile],
    mcp_server_ids: &[String],
    locale: &str,
) -> Result<ToolResult, String> {
    // AGT-11：单发/后台 delegate_task 也受全局并发上限约束（与批量 delegate_tasks 共享同一信号量）。
    let _permit = semaphore
        .acquire()
        .await
        .map_err(|_| "并发控制信号量已关闭".to_string())?;

    // 解析参数
    let profile_id = arguments["profile"].as_str().unwrap_or("general");
    let task_desc = arguments["task"].as_str().unwrap_or("").to_string();
    if task_desc.is_empty() {
        return Err("delegate_task 缺少必填参数 'task'".into());
    }
    let profile = subagent::find_profile(profile_id, custom_profiles)
        .ok_or_else(|| format!("未知的子智能体类型: '{}'，可用: explorer, coder, general, architect, debugger, reviewer, writer, tester, requirements, 或自定义角色 ID", profile_id))?;

    let context_files: Vec<String> = arguments["context_files"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // 解析项目根目录
    let project_root = file_tools::resolve_project_root(app, project_id)?;

    // 权限检查：非 Auto 模式下需要用户确认
    if *agent_mode != AgentMode::Auto {
        let custom_rules = permission::load_permissions(Some(&project_root)).rules;
        let action = permission::check_permission(
            "delegate_task",
            crate::core::ids::BUILTIN_FS_SERVER_ID,
            arguments,
            agent_mode,
            &custom_rules,
        );
        match action {
            PermissionAction::Deny => {
                return Err("delegate_task 在当前模式下被安全策略禁止".into());
            }
            PermissionAction::Ask => {
                let pending = app.state::<PendingApprovals>();
                let reason = format!(
                    "启动子智能体\n\n类型: {}\n任务: {}\n\n确认？",
                    profile.name,
                    truncate_chars_head(&task_desc, 150),
                );
                let approval_fut = crate::commands::mcp::request_tool_approval(
                    app,
                    pending.inner(),
                    crate::core::ids::BUILTIN_FS_SERVER_ID,
                    "delegate_task",
                    arguments,
                    &reason,
                    None,
                    None,
                );
                tokio::select! {
                    _ = token.cancelled() => return Err("cancelled".into()),
                    res = approval_fut => res?,
                }
            }
            PermissionAction::Allow => {}
        }
    }

    // 执行子 Agent
    execute_single_delegate(
        window,
        app,
        client,
        api_url,
        api_key,
        model,
        parent_assistant_id,
        parent_topic_id,
        profile_id,
        &task_desc,
        &context_files,
        None, // extra_context — 单数调用不注入共享上下文
        project_id,
        agent_mode,
        token,
        profile_model_overrides,
        custom_profiles,
        mcp_server_ids,
        locale,
    )
    .await
}

/// 单个子智能体执行的核心逻辑（被 `handle_delegate_task` 和 `handle_delegate_tasks` 共用）。
///
/// 负责：解析 profile、模型覆盖、项目根目录，然后调用 execute_subagent。
/// 不包含权限检查——由调用方负责。
#[allow(
    clippy::too_many_arguments,
    reason = "agent delegation keeps inherited execution context explicit"
)]
pub(crate) async fn execute_single_delegate(
    window: &Window,
    app: &AppHandle,
    client: &reqwest::Client,
    api_url: &str,
    api_key: &str,
    model: &str,
    parent_assistant_id: &str,
    parent_topic_id: &str,
    profile_id: &str,
    task_desc: &str,
    context_files: &[String],
    extra_context: Option<&str>,
    project_id: Option<&str>,
    agent_mode: &AgentMode,
    token: &CancellationToken,
    profile_model_overrides: &[ProfileModelOverride],
    custom_profiles: &[crate::core::models::CustomSubagentProfile],
    mcp_server_ids: &[String],
    locale: &str,
) -> Result<ToolResult, String> {
    let profile = subagent::find_profile(profile_id, custom_profiles)
        .ok_or_else(|| format!("未知的子智能体类型: '{}'，可用: explorer, coder, general, architect, debugger, reviewer, writer, tester, requirements, 或自定义角色 ID", profile_id))?;

    // 模型解析：per-profile override 优先，否则使用主模型
    let override_info = profile_model_overrides
        .iter()
        .find(|o| o.profile_id == profile.id);
    let (resolved_api_url, resolved_api_key, resolved_model) = if let Some(ov) = override_info {
        (
            ov.api_url.as_str(),
            ov.api_key.as_str(),
            ov.model_id.as_str(),
        )
    } else {
        (api_url, api_key, model)
    };

    // 解析项目根目录
    let project_root = file_tools::resolve_project_root(app, project_id)?;

    execute_subagent(
        window,
        app,
        client,
        resolved_api_url,
        resolved_api_key,
        resolved_model,
        parent_assistant_id,
        parent_topic_id,
        &profile,
        task_desc,
        context_files,
        extra_context,
        &project_root,
        project_id,
        agent_mode,
        token,
        mcp_server_ids,
        locale,
    )
    .await
}

/// 处理 delegate_tasks 批量工具调用。
///
/// 解析 tasks 数组和可选的 context，整体检查一次权限，
/// 然后使用 FuturesUnordered 并行执行所有子智能体。
/// 返回所有子任务结果的汇总报告。
#[allow(
    clippy::too_many_arguments,
    reason = "parallel delegation keeps inherited execution context explicit"
)]
pub(crate) async fn handle_delegate_tasks(
    window: &Window,
    app: &AppHandle,
    client: &reqwest::Client,
    api_url: &str,
    api_key: &str,
    model: &str,
    parent_assistant_id: &str,
    parent_topic_id: &str,
    arguments: &serde_json::Value,
    project_id: Option<&str>,
    agent_mode: &AgentMode,
    token: &CancellationToken,
    profile_model_overrides: &[ProfileModelOverride],
    custom_profiles: &[crate::core::models::CustomSubagentProfile],
    mcp_server_ids: &[String],
    semaphore: Arc<Semaphore>,
    locale: &str,
) -> Result<ToolResult, String> {
    // 解析 tasks 数组
    let tasks_array = arguments["tasks"]
        .as_array()
        .ok_or_else(|| "delegate_tasks 缺少必填参数 'tasks'（应为数组）".to_string())?;

    if tasks_array.is_empty() {
        return Err("delegate_tasks: 'tasks' 数组不能为空".into());
    }
    if tasks_array.len() > 10 {
        return Err("delegate_tasks: 'tasks' 数组最多 10 个元素".into());
    }

    let shared_context = arguments["context"]
        .as_str()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());

    // 验证每个 task 的结构
    for (idx, task_obj) in tasks_array.iter().enumerate() {
        let profile = task_obj["profile"].as_str();
        let task = task_obj["task"].as_str();
        if profile.is_none() || profile.unwrap().is_empty() {
            return Err(format!(
                "delegate_tasks: 第 {} 个 task 缺少必填字段 'profile'",
                idx + 1
            ));
        }
        if task.is_none() || task.unwrap().is_empty() {
            return Err(format!(
                "delegate_tasks: 第 {} 个 task 缺少必填字段 'task'",
                idx + 1
            ));
        }
    }

    // 权限检查：整个批量调用一次审批（非 Auto 模式）
    if *agent_mode != AgentMode::Auto {
        // 解析项目根目录用于加载权限规则
        let project_root = file_tools::resolve_project_root(app, project_id)?;
        let custom_rules = permission::load_permissions(Some(&project_root)).rules;
        let action = permission::check_permission(
            "delegate_tasks",
            crate::core::ids::BUILTIN_FS_SERVER_ID,
            arguments,
            agent_mode,
            &custom_rules,
        );
        match action {
            PermissionAction::Deny => {
                return Err("delegate_tasks 在当前模式下被安全策略禁止".into());
            }
            PermissionAction::Ask => {
                let pending = app.state::<PendingApprovals>();
                let profiles_summary: Vec<String> = tasks_array
                    .iter()
                    .map(|t| {
                        format!(
                            "  - {}: {}",
                            t["profile"].as_str().unwrap_or("?"),
                            truncate_chars_head(t["task"].as_str().unwrap_or(""), 80)
                        )
                    })
                    .collect();
                let reason = format!(
                    "批量启动 {} 个子智能体:\n{}\n\n确认？",
                    tasks_array.len(),
                    profiles_summary.join("\n"),
                );
                let approval_fut = crate::commands::mcp::request_tool_approval(
                    app,
                    pending.inner(),
                    crate::core::ids::BUILTIN_FS_SERVER_ID,
                    "delegate_tasks",
                    arguments,
                    &reason,
                    None,
                    None,
                );
                tokio::select! {
                    _ = token.cancelled() => return Err("cancelled".into()),
                    res = approval_fut => res?,
                }
            }
            PermissionAction::Allow => {}
        }
    }

    // 并发执行所有子任务
    let mut unordered = FuturesUnordered::new();

    for task_obj in tasks_array.iter() {
        let profile_id = task_obj["profile"]
            .as_str()
            .unwrap_or("general")
            .to_string();
        let task_desc = task_obj["task"].as_str().unwrap_or("").to_string();
        let task_context_files: Vec<String> = task_obj["context_files"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let extra_ctx = shared_context.clone();

        let window_clone = window.clone();
        let app_clone = app.clone();
        let client_clone = client.clone();
        let api_url_clone = api_url.to_string();
        let api_key_clone = api_key.to_string();
        let model_clone = model.to_string();
        let parent_aid = parent_assistant_id.to_string();
        let parent_tid = parent_topic_id.to_string();
        let pid_clone = project_id.map(|s| s.to_string());
        let token_clone = token.clone();
        let overrides_clone = profile_model_overrides.to_vec();
        let customs_clone = custom_profiles.to_vec();
        let mcp_ids_clone = mcp_server_ids.to_vec();
        let sem_clone = semaphore.clone();
        let locale_clone = locale.to_string();
        let mode_clone = agent_mode.clone();

        unordered.push(async move {
            let _permit = sem_clone
                .acquire()
                .await
                .map_err(|_| "并发控制信号量已关闭".to_string())?;

            execute_single_delegate(
                &window_clone,
                &app_clone,
                &client_clone,
                &api_url_clone,
                &api_key_clone,
                &model_clone,
                &parent_aid,
                &parent_tid,
                &profile_id,
                &task_desc,
                &task_context_files,
                extra_ctx.as_deref(),
                pid_clone.as_deref(),
                &mode_clone,
                &token_clone,
                &overrides_clone,
                &customs_clone,
                &mcp_ids_clone,
                &locale_clone,
            )
            .await
        });
    }

    // 收集所有结果
    let mut successes: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    while let Some(result) = unordered.next().await {
        match result {
            Ok(tr) => {
                let summary = tr
                    .content
                    .iter()
                    .find(|c| c.kind == "text")
                    .and_then(|c| c.data.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("(done)");
                successes.push(summary.to_string());
            }
            Err(e) => {
                failures.push(e);
            }
        }
    }

    // 构造汇总报告
    let total = successes.len() + failures.len();
    let mut report = format!("[批量委托完成] 共 {} 个子任务", total);

    if !successes.is_empty() {
        report.push_str(&format!("\n\n成功 ({}):", successes.len()));
        for (i, s) in successes.iter().enumerate() {
            report.push_str(&format!("\n--- 子任务 {} ---\n{}\n", i + 1, s));
        }
    }
    if !failures.is_empty() {
        report.push_str(&format!("\n\n失败 ({}):", failures.len()));
        for (i, f) in failures.iter().enumerate() {
            report.push_str(&format!("\n- 子任务 {} 错误: {}", i + 1, f));
        }
    }

    Ok(ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({"text": report}),
        }],
        is_error: false,
    })
}
/// 解析工作流步骤的权限模式：配置 "auto" → Auto（旧行为）；其余（含 "inherit" 与
/// 未知值）→ 继承父回合模式（fail-safe，Normal 下写操作仍逐次审批）。
fn resolve_workflow_step_mode(cfg_val: &str, parent: &AgentMode) -> AgentMode {
    if cfg_val.eq_ignore_ascii_case("auto") {
        AgentMode::Auto
    } else {
        parent.clone()
    }
}

/// 执行一个工作流（按顺序依次运行每个步骤的子智能体）。
///
/// 每个步骤独立调用 `handle_delegate_task`，前一步的输出作为上下文追加到下一步的任务描述中。
/// 通过 Tauri 事件向前端推送工作流进度（workflow-start / workflow-step-start / workflow-step-complete / workflow-complete）。
#[allow(
    clippy::too_many_arguments,
    reason = "workflow orchestration keeps inherited execution context explicit"
)]
pub(crate) async fn execute_workflow(
    window: &Window,
    app: &AppHandle,
    client: &reqwest::Client,
    api_url: &str,
    api_key: &str,
    model: &str,
    parent_assistant_id: &str,
    parent_topic_id: &str,
    workflow: &Workflow,
    project_id: Option<&str>,
    semaphore: Arc<Semaphore>,
    custom_profiles: &[CustomSubagentProfile],
    profile_model_overrides: &[ProfileModelOverride],
    mcp_server_ids: &[String],
    token: &CancellationToken,
    locale: &str,
    parent_mode: &AgentMode,
) -> Result<ToolResult, String> {
    let total_steps = workflow.steps.len();

    // 工作流步骤权限模式：默认继承父回合（修复此前静默强制 Auto 的权限旁路——
    // 从 Normal 模式启动的工作流曾自动放行全部写操作）；配置 "auto" 保持旧行为。
    let step_mode = resolve_workflow_step_mode(
        &crate::core::config::load_app_config(app)
            .map(|c| c.workflow_step_mode)
            .unwrap_or_default(),
        parent_mode,
    );

    // 发射 workflow-start 事件
    let _ = window.emit(
        ev::WORKFLOW_START,
        serde_json::json!({
            "workflowId": workflow.workflow_id,
            "title": workflow.title,
            "steps": workflow.steps.iter().map(|s| serde_json::json!({
                "stepId": s.step_id,
                "profileId": s.profile_id,
                "name": s.name,
                "status": "pending",
            })).collect::<Vec<_>>(),
        }),
    );

    let mut context = String::new();
    let mut all_results: Vec<String> = Vec::new();

    for (idx, step) in workflow.steps.iter().enumerate() {
        if token.is_cancelled() {
            let _ = window.emit(
                ev::WORKFLOW_COMPLETE,
                serde_json::json!({ "workflowId": workflow.workflow_id }),
            );
            return Err("工作流被用户取消".into());
        }

        // 发射 workflow-step-start 事件
        let _ = window.emit(
            ev::WORKFLOW_STEP_START,
            serde_json::json!({ "stepId": step.step_id }),
        );

        let start = std::time::Instant::now();

        // 构建任务描述：基础任务 + 上下文
        let mut task_desc = step.task_description.clone();
        if !context.is_empty() {
            task_desc.push_str("\n\n== 前期工作上下文 ==\n");
            task_desc.push_str(&context);
        }

        // 构建 arguments JSON（与 delegate_task 格式一致）
        let arguments = serde_json::json!({
            "profile": step.profile_id,
            "task": task_desc,
        });

        // 调用 handle_delegate_task 执行该步骤
        let result = handle_delegate_task(
            window,
            app,
            client,
            api_url,
            api_key,
            model,
            parent_assistant_id,
            parent_topic_id,
            &arguments,
            project_id,
            &step_mode, // 步骤权限模式：默认继承父回合（见 resolve_workflow_step_mode）
            semaphore.clone(),
            token,
            profile_model_overrides,
            custom_profiles,
            mcp_server_ids,
            locale,
        )
        .await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(tr) => {
                let step_result_text = tr
                    .content
                    .iter()
                    .find(|c| c.kind == "text")
                    .and_then(|c| c.data.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("(done)")
                    .to_string();

                // 追加到上下文（头部+尾部截断，防止多步工作流的上下文无限累积）
                context.push_str(&format!(
                    "\n## 步骤 {} ({}): {}\n{}",
                    idx + 1,
                    step.name,
                    step.profile_id,
                    truncate_chars_head_tail(&step_result_text, 1200)
                ));
                all_results.push(format!(
                    "步骤 {} ({}): {}",
                    idx + 1,
                    step.name,
                    step_result_text
                ));

                // 发射 workflow-step-complete 事件
                let _ = window.emit(
                    ev::WORKFLOW_STEP_COMPLETE,
                    serde_json::json!({
                        "stepId": step.step_id,
                        "status": "completed",
                        "duration": duration_ms,
                    }),
                );
            }
            Err(e) => {
                context.push_str(&format!(
                    "\n## 步骤 {} ({}): [失败] {}",
                    idx + 1,
                    step.name,
                    e
                ));
                all_results.push(format!("步骤 {} ({}): [失败] {}", idx + 1, step.name, e));

                // 发射 workflow-step-complete 事件（失败）
                let _ = window.emit(
                    ev::WORKFLOW_STEP_COMPLETE,
                    serde_json::json!({
                        "stepId": step.step_id,
                        "status": "failed",
                        "duration": duration_ms,
                    }),
                );

                // 步骤失败不中断整个工作流，继续执行后续步骤
            }
        }
    }

    // 发射 workflow-complete 事件
    let _ = window.emit(
        ev::WORKFLOW_COMPLETE,
        serde_json::json!({ "workflowId": workflow.workflow_id }),
    );

    // 构建最终结果
    let summary = format!(
        "工作流 '{}' 执行完成（{}/{} 步骤）\n\n{}",
        workflow.title,
        total_steps,
        total_steps,
        all_results.join("\n\n")
    );

    Ok(ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: serde_json::json!({ "text": summary }),
        }],
        is_error: false,
    })
}

/// 执行子 Agent 循环（从主 Agent 的工具执行循环中调用）。
///
/// 创建独立的 LLM 上下文，注入子 Agent 系统提示词 + 任务描述，
/// 运行最多 `max_rounds` 轮流式调用 + 工具执行，完成后返回结果总结。
#[allow(
    clippy::too_many_arguments,
    reason = "subagent execution keeps inherited parent context explicit"
)]
pub(crate) async fn execute_subagent(
    window: &Window,
    app: &AppHandle,
    client: &reqwest::Client,
    api_url: &str,
    api_key: &str,
    model: &str,
    parent_assistant_id: &str,
    parent_topic_id: &str,
    profile: &subagent::SubagentProfile,
    task_desc: &str,
    context_files: &[String],
    extra_context: Option<&str>,
    project_root: &str,
    project_id: Option<&str>,
    agent_mode: &AgentMode,
    token: &CancellationToken,
    mcp_server_ids: &[String],
    locale: &str,
) -> Result<ToolResult, String> {
    let subagent_id = uuid::Uuid::new_v4().to_string();
    let model_to_use = profile.model_override.as_deref().unwrap_or(model);

    // 通知前端：子 Agent 已启动
    let _ = window.emit(
        ev::SUBAGENT_START,
        SubagentStartPayload {
            parent_assistant_id: parent_assistant_id.to_string(),
            parent_topic_id: parent_topic_id.to_string(),
            subagent_id: subagent_id.clone(),
            profile_id: profile.id.clone(),
            profile_name: profile.name.clone(),
            task_summary: truncate_chars_head(task_desc, 150),
        },
    );

    let mut sub_msgs = build_subagent_messages(
        profile,
        task_desc,
        context_files,
        project_root,
        extra_context,
        locale,
    );
    // MCP 工具回接（P2.2）：allow_mcp_tools 的 profile 注入主 Agent 勾选 server 的工具
    let mcp_tools: Option<crate::core::models::AssistantTools> =
        if profile.allow_mcp_tools && !mcp_server_ids.is_empty() {
            let mgr = app.state::<crate::plugins::mcp::McpServerManager>();
            let mcp_state = app.state::<crate::plugins::mcp::McpServerState>();
            crate::commands::mcp::list_mcp_tools_for_assistant_inner(
                app,
                mgr.inner(),
                mcp_state.inner(),
                mcp_server_ids.to_vec(),
                project_id.map(|s| s.to_string()),
            )
            .await
            .ok()
        } else {
            None
        };
    let (sub_tools, sub_tool_server_map) =
        build_subagent_tools(profile, project_root, mcp_tools.as_ref());
    let tools_slice: Option<&[ToolSpec]> = if sub_tools.is_empty() {
        None
    } else {
        Some(&sub_tools)
    };

    let mut round: usize = 0;
    let mut final_text = String::new();
    // 输出截断（finish_reason="length"）续写重试次数（最多 1 次）
    let mut length_retries: u32 = 0;
    // 上下文溢出自动恢复次数（压缩后重试）
    let mut sub_overflow_retries: u32 = 0;
    // 子 Agent 预算（AgentLimits：失控保护 100 轮；用户显式设置 max_tool_rounds 时以其为准）。
    // 历史 bug：此前硬编码且完全忽略用户配置。
    let app_cfg = crate::core::config::load_app_config(app).unwrap_or_default();
    let limits = crate::commands::llm::AgentLimits::for_subagent(&app_cfg);
    // 只读工具并发开关（与主循环共用同一配置项）
    let parallel_enabled = app_cfg.parallel_tool_execution;

    loop {
        round += 1;
        if token.is_cancelled() {
            break;
        }
        if round > limits.max_rounds as usize {
            // 命中失控保护：把已完成的工作整理成有效返回，而不是丢弃
            final_text = build_subagent_progress_summary(&sub_msgs, limits.max_rounds as usize);
            break;
        }

        // 流式调用前：发射心跳告知前端子 Agent 工作中
        let _ = window.emit(
            ev::SUBAGENT_STEP,
            SubagentStepPayload {
                parent_assistant_id: parent_assistant_id.to_string(),
                parent_topic_id: parent_topic_id.to_string(),
                subagent_id: subagent_id.clone(),
                round: round as u32,
                step_type: "streaming".to_string(),
                summary: format!("第 {} 轮 LLM 调用中...", round),
            },
        );

        // 流式调用一轮
        let result = stream_one_round(
            window,
            token,
            client,
            api_url.to_string(),
            api_key,
            model_to_use,
            &sub_msgs,
            tools_slice,
            parent_assistant_id,
            parent_topic_id,
            true, // suppress events for sub-agent (uses subagent-* events instead)
            None, // 子 Agent 图像输出 v1 不支持：不入盘、不重写，仅保留文本
            false,
            None, // 子 Agent 输出上限由 round cap 兜底，不额外注入 max_tokens
            app_cfg.model_call_retries,
        )
        .await;

        match result {
            Ok(rr) => {
                // 追加 assistant 消息到子 Agent 上下文
                let mut asst_obj = serde_json::Map::new();
                asst_obj.insert("role".into(), json!("assistant"));
                asst_obj.insert("content".into(), rr.api_content.clone());
                if !rr.tool_calls.is_empty() {
                    let tcs: Vec<serde_json::Value> = rr
                        .tool_calls
                        .iter()
                        .map(|tc| {
                            json!({
                                "id": tc.id,
                                "type": "function",
                                "function": { "name": tc.name, "arguments": tc.arguments },
                            })
                        })
                        .collect();
                    asst_obj.insert("tool_calls".into(), json!(tcs));
                }
                sub_msgs.push(serde_json::Value::Object(asst_obj));

                // 发射子 Agent 步骤事件（通知前端）
                let step_content = if rr.content.is_empty() {
                    "[工具调用]".to_string()
                } else {
                    truncate_chars_head(&rr.content, 300)
                };
                let _ = window.emit(
                    ev::SUBAGENT_STEP,
                    SubagentStepPayload {
                        parent_assistant_id: parent_assistant_id.to_string(),
                        parent_topic_id: parent_topic_id.to_string(),
                        subagent_id: subagent_id.clone(),
                        round: round as u32,
                        step_type: if rr.tool_calls.is_empty() {
                            "content".to_string()
                        } else {
                            "tool_call".to_string()
                        },
                        summary: step_content,
                    },
                );

                // 输出被截断（finish_reason="length"）且无工具调用：追加续写指令后重试一次
                if rr.finish_reason.as_deref() == Some("length")
                    && rr.tool_calls.is_empty()
                    && length_retries < 1
                {
                    length_retries += 1;
                    sub_msgs.push(json!({
                        "role": "system",
                        "content": "[输出被截断] 请直接继续完成上一条回复中未完成的内容，不要重复已经输出的部分。"
                    }));
                    continue;
                }

                // 无工具调用 → 子 Agent 完成
                if rr.tool_calls.is_empty() {
                    final_text = rr.content;
                    break;
                }

                // 执行工具调用：与主循环同语义 —— 只读工具并发（FuturesUnordered），
                // 变更类工具按原始顺序串行；结果按 tool_calls 索引序回填（API 语义不变）。
                // 阶段 1：串行执行变更类工具 + 收集只读工具 future
                let mut tool_results: Vec<Option<(String,)>> = vec![None; rr.tool_calls.len()];
                let mut read_futures: Vec<(
                    usize,
                    futures_util::future::BoxFuture<'static, Result<ToolResult, String>>,
                )> = Vec::new();

                for (i, tc) in rr.tool_calls.iter().enumerate() {
                    if token.is_cancelled() {
                        break;
                    }
                    let args_val: serde_json::Value =
                        serde_json::from_str(&tc.arguments).unwrap_or(json!({}));

                    // 再次确认工具被允许（双重保险）。MCP 工具的门控是注入时的
                    // allow_mcp_tools 标志（allowed_tools 列表不约束 MCP 名），
                    // 但 denied_tools 显式排除仍然生效。
                    let is_mcp_tool = sub_tool_server_map
                        .get(&tc.name)
                        .map(|s| s.as_str() != crate::core::ids::BUILTIN_SERVER_ID)
                        .unwrap_or(false);
                    let allowed = if is_mcp_tool {
                        !profile.is_tool_denied(&tc.name)
                    } else {
                        profile.is_tool_allowed(&tc.name)
                    };
                    if !allowed {
                        tool_results[i] = Some((format!(
                            "[已阻止] 子 Agent ({}) 不允许使用工具: {}",
                            profile.id, tc.name
                        ),));
                        continue;
                    }

                    if parallel_enabled && is_parallel_safe(&tc.name) {
                        // 只读工具：收集为并发 future（阶段 2 统一等待）
                        let app_c = app.clone();
                        let tn = tc.name.clone();
                        let args_c = args_val.clone();
                        let pid = project_id.map(|p| p.to_string());
                        let mode = agent_mode.clone();
                        let tok = token.clone();
                        let fut = Box::pin(async move {
                            execute_builtin_tool(&app_c, &tn, &args_c, pid.as_deref(), &mode, &tok)
                                .await
                        });
                        read_futures.push((i, fut));
                        continue;
                    }

                    // 串行执行：MCP 工具走重试包装 + execute_tool_call；内置工具直连
                    let tool_result = if is_mcp_tool {
                        let app_c2 = app.clone();
                        let tn = tc.name.clone();
                        let args_c = args_val.clone();
                        let sid = sub_tool_server_map
                            .get(&tc.name)
                            .cloned()
                            .unwrap_or_default();
                        let pid = project_id.map(|p| p.to_string());
                        let mode = agent_mode.clone();
                        let tok = token.clone();
                        execute_tool_with_retry(
                            || {
                                let app3 = app_c2.clone();
                                let tn3 = tn.clone();
                                let args3 = args_c.clone();
                                let sid3 = sid.clone();
                                let pid3 = pid.clone();
                                let mode3 = mode.clone();
                                let tok3 = tok.clone();
                                async move {
                                    crate::commands::mcp::execute_tool_call(
                                        &app3,
                                        &sid3,
                                        &tn3,
                                        args3,
                                        pid3.as_deref(),
                                        &mode3,
                                        &tok3,
                                    )
                                    .await
                                }
                            },
                            &app_cfg,
                            token,
                        )
                        .await
                    } else {
                        execute_builtin_tool(
                            app, &tc.name, &args_val, project_id,
                            agent_mode, // 子 Agent 继承父 agent_mode，写/删/执行按父模式走审批
                            token,
                        )
                        .await
                    };

                    let content_text = match tool_result {
                        Ok(tr) => tool_result_text(&tr),
                        Err(e) => format!("[Error] {}", e),
                    };
                    let _ = window.emit(
                        ev::SUBAGENT_STEP,
                        SubagentStepPayload {
                            parent_assistant_id: parent_assistant_id.to_string(),
                            parent_topic_id: parent_topic_id.to_string(),
                            subagent_id: subagent_id.clone(),
                            round: round as u32,
                            step_type: format!("tool_result:{}", tc.name),
                            summary: truncate_chars_head(&content_text, 200),
                        },
                    );
                    tool_results[i] = Some((content_text,));
                }

                // 阶段 2：并发等待只读工具，完成即发射
                if !read_futures.is_empty() {
                    let mut unordered = FuturesUnordered::new();
                    for (i, fut) in read_futures {
                        unordered.push(async move {
                            let result = fut.await;
                            (i, result)
                        });
                    }
                    while let Some((i, result)) = unordered.next().await {
                        let content_text = match result {
                            Ok(tr) => tool_result_text(&tr),
                            Err(e) => format!("[Error] {}", e),
                        };
                        let tc = &rr.tool_calls[i];
                        let _ = window.emit(
                            ev::SUBAGENT_STEP,
                            SubagentStepPayload {
                                parent_assistant_id: parent_assistant_id.to_string(),
                                parent_topic_id: parent_topic_id.to_string(),
                                subagent_id: subagent_id.clone(),
                                round: round as u32,
                                step_type: format!("tool_result:{}", tc.name),
                                summary: truncate_chars_head(&content_text, 200),
                            },
                        );
                        tool_results[i] = Some((content_text,));
                    }
                }

                // 阶段 3：按索引序回填 tool 消息（空结果占位 + 截断，与主循环一致）
                for (i, tc) in rr.tool_calls.iter().enumerate() {
                    if let Some((content_text,)) = &tool_results[i] {
                        let truncated_content = if content_text.trim().is_empty() {
                            "[无结果]".to_string()
                        } else {
                            truncate_tool_result(content_text)
                        };
                        sub_msgs.push(json!({
                            "role": "tool",
                            "content": truncated_content,
                            "tool_call_id": tc.id,
                            "name": tc.name,
                        }));
                    }
                }

                // 子 Agent 上下文安全阀：估算 token 超保守预算时压缩（无需模型窗口信息，
                // 对绝大多数 ≥64k 窗口的模型都安全；压缩保留最近 token 预算）
                let sub_est: usize =
                    estimate_context_tokens(model_to_use, &sub_msgs, limits.context_budget);
                if sub_est > limits.context_budget && sub_msgs.len() > 4 {
                    let _ = compress_sub_context(
                        window,
                        client,
                        api_url,
                        api_key,
                        model_to_use,
                        &mut sub_msgs,
                        parent_assistant_id,
                        parent_topic_id,
                        token,
                        limits.keep_recent_tokens,
                    )
                    .await;
                }
            }
            Err(e) => {
                if e == "cancelled" {
                    break;
                }
                // 上下文溢出自动恢复：压缩后重试一次
                if sub_overflow_retries < 1 && is_context_overflow_error(&e) && sub_msgs.len() > 4 {
                    sub_overflow_retries += 1;
                    let res = compress_sub_context(
                        window,
                        client,
                        api_url,
                        api_key,
                        model_to_use,
                        &mut sub_msgs,
                        parent_assistant_id,
                        parent_topic_id,
                        token,
                        limits.keep_recent_tokens,
                    )
                    .await;
                    if res.is_ok() {
                        tracing::warn!("[subagent overflow] 上下文溢出，已压缩并重试: {}", e);
                        continue;
                    }
                }
                // API 错误：记录并返回
                let _ = window.emit(
                    ev::SUBAGENT_ERROR,
                    SubagentErrorPayload {
                        parent_assistant_id: parent_assistant_id.to_string(),
                        parent_topic_id: parent_topic_id.to_string(),
                        subagent_id: subagent_id.clone(),
                        error: e.clone(),
                    },
                );
                return Err(format!("子 Agent API 错误: {}", e));
            }
        }
    }

    // 子 Agent 完成：发射 done 事件
    let result_text = if final_text.is_empty() {
        // 无最终文本时回退为已完成工作摘要，保证主 Agent 始终拿到有效返回
        build_subagent_progress_summary(&sub_msgs, limits.max_rounds as usize)
    } else {
        final_text
    };

    let _ = window.emit(
        ev::SUBAGENT_DONE,
        SubagentDonePayload {
            parent_assistant_id: parent_assistant_id.to_string(),
            parent_topic_id: parent_topic_id.to_string(),
            subagent_id: subagent_id.clone(),
            profile_name: profile.name.clone(),
            result: result_text.clone(),
        },
    );

    Ok(ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({"text": format!(
                "[子智能体「{}」工作报告]\n\n{}",
                profile.name,
                result_text,
            )}),
        }],
        is_error: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_step_mode_inherits_by_default() {
        // 默认（"inherit"）、空串与未知值都回落到父回合模式 —— fail-safe
        assert_eq!(
            resolve_workflow_step_mode("inherit", &AgentMode::Normal),
            AgentMode::Normal
        );
        assert_eq!(
            resolve_workflow_step_mode("", &AgentMode::Normal),
            AgentMode::Normal
        );
        assert_eq!(
            resolve_workflow_step_mode("whatever", &AgentMode::Plan),
            AgentMode::Plan
        );
    }

    #[test]
    fn workflow_step_mode_auto_opt_in() {
        // 显式配置 "auto"（大小写不敏感）才保持旧行为
        assert_eq!(
            resolve_workflow_step_mode("auto", &AgentMode::Normal),
            AgentMode::Auto
        );
        assert_eq!(
            resolve_workflow_step_mode("Auto", &AgentMode::Workflow),
            AgentMode::Auto
        );
    }
}

// ===== 子 Agent 事件载荷定义 =====

#[derive(Serialize, Clone)]
pub(crate) struct SubagentStartPayload {
    parent_assistant_id: String,
    parent_topic_id: String,
    subagent_id: String,
    profile_id: String,
    profile_name: String,
    task_summary: String,
}

#[derive(Serialize, Clone)]
pub(crate) struct SubagentStepPayload {
    parent_assistant_id: String,
    parent_topic_id: String,
    subagent_id: String,
    round: u32,
    step_type: String,
    summary: String,
}

#[derive(Serialize, Clone)]
pub(crate) struct SubagentDonePayload {
    parent_assistant_id: String,
    parent_topic_id: String,
    subagent_id: String,
    profile_name: String,
    result: String,
}

#[derive(Serialize, Clone)]
pub(crate) struct SubagentErrorPayload {
    parent_assistant_id: String,
    parent_topic_id: String,
    subagent_id: String,
    error: String,
}

/// 子 Agent 压缩早期上下文：预算超限安全阀与溢出重试共用（take-出→压缩→回填）。
#[allow(
    clippy::too_many_arguments,
    reason = "subagent compression reuses the active stream request context"
)]
async fn compress_sub_context(
    window: &Window,
    client: &reqwest::Client,
    api_url: &str,
    api_key: &str,
    model: &str,
    sub_msgs: &mut Vec<serde_json::Value>,
    parent_assistant_id: &str,
    parent_topic_id: &str,
    token: &CancellationToken,
    keep_recent_tokens: usize,
) -> Result<(), String> {
    let mut msgs = std::mem::take(sub_msgs);
    let r = compress_context(
        window,
        client,
        api_url,
        api_key,
        model,
        &mut msgs,
        parent_assistant_id,
        parent_topic_id,
        token,
        keep_recent_tokens,
        "subagent",
    )
    .await;
    *sub_msgs = msgs;
    r
}

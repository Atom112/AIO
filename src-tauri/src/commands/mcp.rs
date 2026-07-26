//! MCP Tauri Commands
//!
//! 提供前端调用的 9 个命令；MCP 运行时状态由 [`McpServerState`] 持有。

use crate::core::models::*;
use crate::core::permission::{self, PermissionAction};
use crate::core::secure_store;
use crate::plugins::mcp::{self, McpServerManager, McpServerPlugin, McpRequestManager, McpServerState, PendingApprovals};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::oneshot;

const TOOL_CALL_TIMEOUT: Duration = Duration::from_secs(30);

fn emit_status(
    app: &AppHandle,
    id: &str,
    status: McpStatus,
    message: Option<String>,
    tool_count: usize,
) {
    emit_status_full(app, id, status, message, tool_count, 0, 0);
}

fn emit_status_full(
    app: &AppHandle,
    id: &str,
    status: McpStatus,
    message: Option<String>,
    tool_count: usize,
    resource_count: usize,
    prompt_count: usize,
) {
    let _ = app.emit(
        "mcp-server-status",
        McpServerStatusInfo {
            id: id.to_string(),
            status,
            message,
            tool_count,
            resource_count,
            prompt_count,
        },
    );
}

#[tauri::command]
pub async fn list_mcp_servers(
    app: AppHandle,
    project_id: Option<String>,
) -> Result<Vec<McpServerConfig>, String> {
    mcp::list_configs_merged(&app, project_id.as_deref()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn add_mcp_server(
    app: AppHandle,
    config: McpServerConfig,
    project_id: Option<String>,
) -> Result<(), String> {
    match project_id {
        Some(ref pid) => {
            let project_path = resolve_project_path_mcp(&app, pid)?;
            mcp::upsert_project_config(&project_path, config).map_err(|e| e.to_string())
        }
        None => mcp::upsert_config(&app, config).map_err(|e| e.to_string()),
    }
}

/// MCP 专用的项目路径解析。
fn resolve_project_path_mcp(app: &AppHandle, project_id: &str) -> Result<String, String> {
    mcp::resolve_project_path(app, project_id).map_err(|e| e.to_string())
}

/// 解析项目路径（返回 Option，不报错）。
fn resolve_project_path_mcp_opt(app: &AppHandle, project_id: &str) -> Option<String> {
    mcp::resolve_project_path(app, project_id).ok()
}

/// Store one MCP environment variable or HTTP header secret in the system keyring.
#[tauri::command]
pub async fn save_mcp_server_secret(
    app: AppHandle,
    server_id: String,
    target: String,
    key: String,
    value: String,
) -> Result<String, String> {
    if server_id.trim().is_empty() || key.trim().is_empty() || value.is_empty() {
        return Err("服务器、密钥名称和值不能为空".into());
    }
    if target != "env" && target != "header" {
        return Err("不支持的密钥目标".into());
    }
    let account = format!("mcp-server-{}-{}-{}", server_id, target, key);
    secure_store::set(&app, &account, &value).map_err(|error| error.to_string())?;
    Ok(format!("${{KEYRING:{}}}", account))
}

#[tauri::command]
pub async fn remove_mcp_server(
    app: AppHandle,
    mgr: State<'_, McpServerManager>,
    state: State<'_, McpServerState>,
    requests: State<'_, McpRequestManager>,
    id: String,
    project_id: Option<String>,
) -> Result<(), String> {
    // 复用 stop_mcp_server 的清理逻辑：abort 在途调用 + plugin.stop（清理 HTTP_STATES）
    {
        let prefix = format!("mcp:{}:", id);
        requests.0.retain(|k, _| !k.starts_with(&prefix));
    }
    let conn_opt = {
        let mut map = state.lock();
        map.remove(&id)
    };
    if let Some(conn) = conn_opt {
        let transport_id = conn.transport_kind.clone();
        if let Some(plugin) = mgr.get(&transport_id) {
            let conn_owned = Arc::try_unwrap(conn).unwrap_or_else(|arc| (*arc).clone());
            let _ = plugin.stop(conn_owned).await;
        }
    }
    // 清理 keyring 中该 server 的所有 env 密钥
    let cfg = mcp::get_config_merged(&app, &id, project_id.as_deref());
    if let Some(cfg) = cfg {
        let entries = match &cfg.transport {
            crate::core::models::McpTransport::Stdio { env, .. } => Some(("env", env)),
            crate::core::models::McpTransport::Http { headers, .. }
            | crate::core::models::McpTransport::StreamableHttp { headers, .. } => {
                Some(("header", headers))
            }
        };
        if let Some((target, values)) = entries {
            for (key, value) in values {
                let legacy = secure_store::accounts::mcp_server_env(&id, key);
                let _ = secure_store::delete(&app, &legacy);
                if value.contains("${KEYRING:") {
                    let account = format!("mcp-server-{}-{}-{}", id, target, key);
                    let _ = secure_store::delete(&app, &account);
                }
            }
        }
    }
    match project_id {
        Some(ref pid) => {
            let project_path = resolve_project_path_mcp(&app, pid)?;
            mcp::remove_project_config(&project_path, &id).map_err(|e| e.to_string())?
        }
        None => mcp::remove_config(&app, &id).map_err(|e| e.to_string())?,
    };
    emit_status(&app, &id, McpStatus::Disconnected, None, 0);
    Ok(())
}

#[tauri::command]
pub async fn start_mcp_server(
    app: AppHandle,
    mgr: State<'_, McpServerManager>,
    state: State<'_, McpServerState>,
    id: String,
    project_id: Option<String>,
) -> Result<Vec<ToolSpec>, String> {
    emit_status(&app, &id, McpStatus::Connecting, None, 0);

    let config = mcp::get_config_merged(&app, &id, project_id.as_deref())
        .ok_or_else(|| format!("未找到 MCP server: {}", id))?;

    let transport_id = match &config.transport {
        McpTransport::Stdio { .. } => "stdio",
        McpTransport::Http { .. } => "http",
        McpTransport::StreamableHttp { .. } => "streamable_http",
    };

    let plugin = mgr
        .get(transport_id)
        .ok_or_else(|| format!("未注册 transport 插件: {}", transport_id))?;

    let conn = plugin
        .start(app.clone(), &config)
        .await
        .map_err(|e| format!("启动 MCP server 失败: {}", e))?;
    let conn = Arc::new(conn);

    // 协议握手
    let server_info = plugin
        .initialize(&conn)
        .await
        .map_err(|e| format!("MCP initialize 失败: {}", e))?;

    // 拉取工具列表
    let tools = plugin
        .list_tools(&conn)
        .await
        .map_err(|e| format!("MCP tools/list 失败: {}", e))?;

    // 应用 enabled_tools 白名单
    let filtered: Vec<ToolSpec> = if config.enabled_tools.is_empty() {
        tools.clone()
    } else {
        tools
            .iter()
            .filter(|t| config.enabled_tools.contains(&t.function.name))
            .cloned()
            .collect()
    };

    // 仅当服务端声明了 resources 能力时才拉取资源列表
    let (_resources, resource_count) = if server_info.capabilities.as_ref().and_then(|c| c.resources.as_ref()).is_some() {
        match plugin.list_resources(&conn).await {
            Ok(r) => {
                let count = r.len();
                (Some(r), count)
            }
            Err(e) => {
                tracing::debug!("MCP {} resources/list 失败: {}", id, e);
                (None, 0)
            }
        }
    } else {
        (None, 0)
    };

    // 仅当服务端声明了 prompts 能力时才拉取提示词列表
    let (_prompts, prompt_count) = if server_info.capabilities.as_ref().and_then(|c| c.prompts.as_ref()).is_some() {
        match plugin.list_prompts(&conn).await {
            Ok(p) => {
                let count = p.len();
                (Some(p), count)
            }
            Err(e) => {
                tracing::debug!("MCP {} prompts/list 失败: {}", id, e);
                (None, 0)
            }
        }
    } else {
        (None, 0)
    };

    {
        let mut map = state.lock();
        map.insert(id.clone(), conn);
    }

    emit_status_full(&app, &id, McpStatus::Connected, None, filtered.len(), resource_count, prompt_count);
    Ok(filtered)
}

#[tauri::command]
pub async fn stop_mcp_server(
    app: AppHandle,
    mgr: State<'_, McpServerManager>,
    state: State<'_, McpServerState>,
    requests: State<'_, McpRequestManager>,
    id: String,
) -> Result<(), String> {
    // 先 abort 该 server 的在途调用
    let prefix = format!("mcp:{}:", id);
    requests.0.retain(|k, _| !k.starts_with(&prefix));

    let conn_opt = {
        let mut map = state.lock();
        map.remove(&id)
    };
    if let Some(conn) = conn_opt {
        let transport_id = conn.transport_kind.clone();
        if let Some(plugin) = mgr.get(&transport_id) {
            // stop 需要 owned conn，这里 unwrap Arc
            let conn_owned = Arc::try_unwrap(conn).unwrap_or_else(|arc| {
                // 仍有其他引用：强行 take 一个新连接很复杂，这里直接 clone
                // 实际上 plugin::stop 通常只调用 close()，可接受共享
                (*arc).clone()
            });
            let _ = plugin.stop(conn_owned).await;
        }
    }
    emit_status(&app, &id, McpStatus::Disconnected, None, 0);
    Ok(())
}

#[tauri::command]
pub async fn list_mcp_server_status(
    state: State<'_, McpServerState>,
) -> Result<HashMap<String, McpServerStatusInfo>, String> {
    let map = state.lock();
    let mut out = HashMap::new();
    for id in map.keys() {
        out.insert(
            id.clone(),
            McpServerStatusInfo {
                id: id.clone(),
                status: McpStatus::Connected,
                message: None,
                tool_count: 0,
                resource_count: 0,
                prompt_count: 0,
            },
        );
    }
    Ok(out)
}

#[tauri::command]
pub async fn list_mcp_tools(
    app: AppHandle,
    mgr: State<'_, McpServerManager>,
    state: State<'_, McpServerState>,
    project_id: Option<String>,
) -> Result<Vec<ToolSpec>, String> {
    // 1) 取出所有 server_id 后立即释放锁
    let ids: Vec<String> = {
        let map = state.lock();
        map.keys().cloned().collect()
    };

    // 2) 收集 (id, conn, cfg, plugin) 全部准备好后再 await
    let mut jobs: Vec<(
        McpServerConfig,
        Arc<dyn McpServerPlugin>,
        Arc<crate::plugins::mcp::connection::McpConnection>,
    )> = Vec::new();
    for id in ids {
        let cfg = match mcp::get_config_merged(&app, &id, project_id.as_deref()) {
            Some(c) => c,
            None => continue,
        };
        let transport_id = match &cfg.transport {
            McpTransport::Stdio { .. } => "stdio",
            McpTransport::Http { .. } => "http",
            McpTransport::StreamableHttp { .. } => "streamable_http",
        };
        let Some(plugin) = mgr.get(transport_id) else {
            continue;
        };
        let conn = {
            let map = state.lock();
            map.get(&id).cloned()
        };
        if let Some(conn) = conn {
            jobs.push((cfg, plugin, conn));
        }
    }
    drop(state);
    drop(mgr);

    // 3) 顺序调用（避免在循环中跨 await 持锁）
    let mut all_tools = Vec::new();
    for (cfg, plugin, conn) in jobs {
        if let Ok(tools) = plugin.list_tools(&conn).await {
            let filtered: Vec<ToolSpec> = if cfg.enabled_tools.is_empty() {
                tools
            } else {
                tools
                    .into_iter()
                    .filter(|t| cfg.enabled_tools.contains(&t.function.name))
                    .collect()
            };
            all_tools.extend(filtered);
        }
    }
    Ok(all_tools)
}

/// 按助手启用的 MCP server 列表聚合工具。
///
/// - `mcp_server_ids`：前端传入该助手勾选的 server id 列表（opt-in，空 = 无工具）。
/// - 仅遍历这些 id 中当前已连接的 server，应用各自 `enabled_tools` 白名单。
/// - 返回扁平 `tools`（直接喂给 LLM）+ `tool_server_map`（toolName → serverId，供 call_mcp_tool 解析），
///   替代前端不可靠的启发式查找。
#[tauri::command]
pub async fn list_mcp_tools_for_assistant(
    app: AppHandle,
    mgr: State<'_, McpServerManager>,
    state: State<'_, McpServerState>,
    mcp_server_ids: Vec<String>,
    project_id: Option<String>,
) -> Result<AssistantTools, String> {
    list_mcp_tools_for_assistant_inner(&app, mgr.inner(), state.inner(), mcp_server_ids, project_id).await
}

/// 内部实现：供 `run_agent_turn` 后端复用，避免再次走 Tauri State 解包。
pub(crate) async fn list_mcp_tools_for_assistant_inner(
    app: &AppHandle,
    mgr: &McpServerManager,
    state: &McpServerState,
    mcp_server_ids: Vec<String>,
    project_id: Option<String>,
) -> Result<AssistantTools, String> {
    // 1) 收集入参 id 中已连接的 (cfg, plugin, conn)，先取出 conn 后立即释放锁
    let mut jobs: Vec<(
        McpServerConfig,
        Arc<dyn McpServerPlugin>,
        Arc<crate::plugins::mcp::connection::McpConnection>,
    )> = Vec::new();
    for id in mcp_server_ids {
        let cfg = match mcp::get_config_merged(app, &id, project_id.as_deref()) {
            Some(c) => c,
            None => continue,
        };
        let transport_id = match &cfg.transport {
            McpTransport::Stdio { .. } => "stdio",
            McpTransport::Http { .. } => "http",
            McpTransport::StreamableHttp { .. } => "streamable_http",
        };
        let Some(plugin) = mgr.get(transport_id) else {
            continue;
        };
        let conn = {
            let map = state.lock();
            map.get(&id).cloned()
        };
        if let Some(conn) = conn {
            jobs.push((cfg, plugin, conn));
        }
    }

    // 2) 顺序拉取工具，应用白名单，构造 tools + tool_server_map
    let mut tools = Vec::new();
    let mut tool_server_map: HashMap<String, String> = HashMap::new();
    for (cfg, plugin, conn) in jobs {
        if let Ok(server_tools) = plugin.list_tools(&conn).await {
            let filtered: Vec<ToolSpec> = if cfg.enabled_tools.is_empty() {
                server_tools
            } else {
                server_tools
                    .into_iter()
                    .filter(|t| cfg.enabled_tools.contains(&t.function.name))
                    .collect()
            };
            for t in &filtered {
                tool_server_map.insert(t.function.name.clone(), cfg.id.clone());
            }
            tools.extend(filtered);
        }
    }
    Ok(AssistantTools {
        tools,
        tool_server_map,
    })
}

/// 修改 `call_mcp_tool` 以加入权限检查（防御层）。
/// 前端应预先通过 `check_tool_permission` 检查权限，但后端也做二次检查确保安全。
#[tauri::command]
pub async fn call_mcp_tool(
    app: AppHandle,
    server_id: String,
    tool_name: String,
    arguments: Value,
    project_id: Option<String>,
    // 当前 agent 模式（如 "normal", "auto", "plan", "off"），提供则进行权限检查
    agent_mode: Option<String>,
) -> Result<ToolResult, String> {
    let mode = match agent_mode.as_deref() {
        Some("normal") => AgentMode::Normal,
        Some("auto") => AgentMode::Auto,
        Some("plan") => AgentMode::Plan,
        _ => AgentMode::Off,
    };
    // call_mcp_tool 路径无 CancellationToken，用一个新的（永不取消）
    let token = tokio_util::sync::CancellationToken::new();
    execute_tool_call(
        &app,
        &server_id,
        &tool_name,
        arguments,
        project_id.as_deref(),
        &mode,
        &token,
    )
    .await
}

/// 执行单个 MCP 工具调用：白名单 → 权限/审批 → spawn 调用 → 取结果。
///
/// 供 `run_agent_turn`（带 CancellationToken，可被用户停止打断挂起审批）与
/// `call_mcp_tool`（无取消，用独立 token）复用。MCP 全局状态通过 `AppHandle` 解析，
/// 避免借用逃逸到 `tokio::spawn`。
///
/// - `call_id` 带 uuid，避免并发同名工具调用互相覆盖 handle（旧 bug：两个 `read_file` 共享 call_id
///   导致一个报"已被中止"、一个泄漏）。
/// - 审批等待用 `select!` 包 `request_tool_approval` 与 `token.cancelled()`，
///   用户停止时立即打断挂起审批。
pub(crate) async fn execute_tool_call(
    app: &AppHandle,
    server_id: &str,
    tool_name: &str,
    arguments: Value,
    project_id: Option<&str>,
    agent_mode: &AgentMode,
    token: &tokio_util::sync::CancellationToken,
) -> Result<ToolResult, String> {
    let mgr = app.state::<McpServerManager>();
    let state = app.state::<McpServerState>();
    let requests = app.state::<McpRequestManager>();
    let pending = app.state::<PendingApprovals>();

    let cfg = mcp::get_config_merged(app, server_id, project_id)
        .ok_or_else(|| format!("未找到 MCP server: {}", server_id))?;
    if !cfg.enabled_tools.is_empty() && !cfg.enabled_tools.contains(&tool_name.to_string()) {
        return Err(format!(
            "工具 {} 不在 server {} 的白名单中",
            tool_name, server_id
        ));
    }

    // 权限检查（后端防御层）
    if *agent_mode != AgentMode::Off {
        let project_path = project_id.and_then(|pid| resolve_project_path_mcp_opt(app, pid));
        let custom_rules = project_path
            .as_ref()
            .map(|p| permission::load_permissions(Some(p)).rules)
            .unwrap_or_default();
        let action = permission::check_permission(tool_name, server_id, &arguments, agent_mode, &custom_rules);

        match action {
            PermissionAction::Deny => {
                return Err(format!(
                    "工具 '{}' 已在当前模式下被安全策略禁止执行（Deny）。\n如需执行，请切换到自动模式或在项目权限设置中添加 allow 规则。",
                    tool_name
                ));
            }
            PermissionAction::Ask => {
                // 需要用户确认；用 select! 让取消可打断挂起审批
                let reason = format!("工具 '{}' 需要您的确认才能执行", tool_name);
                let approval_fut = request_tool_approval(app, pending.inner(), server_id, tool_name, &arguments, &reason, None, None);
                tokio::select! {
                    _ = token.cancelled() => return Err("cancelled".into()),
                    res = approval_fut => res?,
                }
            }
            PermissionAction::Allow => {}
        }
    }

    let transport_id = match &cfg.transport {
        McpTransport::Stdio { .. } => "stdio",
        McpTransport::Http { .. } => "http",
        McpTransport::StreamableHttp { .. } => "streamable_http",
    };
    let plugin = mgr
        .get(transport_id)
        .ok_or_else(|| format!("未注册 transport: {}", transport_id))?;
    let conn = {
        let map = state.lock();
        map.get(server_id)
            .cloned()
            .ok_or_else(|| format!("MCP server 未连接: {}", server_id))?
    };

    // 用 call_id 跟踪；带 uuid 保证唯一，避免并发同名调用互相覆盖 handle。
    let call_id = format!("mcp:{}:{}:{}", server_id, tool_name, uuid::Uuid::new_v4());
    let plugin_arc = plugin.clone();
    let conn_arc = conn.clone();
    let tool_for_task = tool_name.to_string();
    let handle = tokio::spawn(async move {
        plugin_arc
            .call_tool(&conn_arc, &tool_for_task, arguments, TOOL_CALL_TIMEOUT)
            .await
    });
    requests.0.insert(call_id.clone(), handle);

    // 等待结果或取消
    let entry = requests.0.remove(&call_id);
    let handle: tokio::task::JoinHandle<
        std::result::Result<ToolResult, crate::plugins::mcp::error::McpError>,
    > = match entry {
        Some((_, h)) => h,
        None => return Err("调用已被中止".into()),
    };
    tokio::select! {
        _ = token.cancelled() => Err("cancelled".into()),
        res = handle => match res {
            Ok(r) => r.map_err(|e: crate::plugins::mcp::error::McpError| e.to_string()),
            Err(e) => Err(format!("工具调用任务 join 失败: {}", e)),
        },
    }
}

#[tauri::command]
pub async fn test_mcp_server_connection(
    app: AppHandle,
    mgr: State<'_, McpServerManager>,
    config: McpServerConfig,
) -> Result<Vec<ToolSpec>, String> {
    let transport_id = match &config.transport {
        McpTransport::Stdio { .. } => "stdio",
        McpTransport::Http { .. } => "http",
        McpTransport::StreamableHttp { .. } => "streamable_http",
    };
    let plugin = mgr
        .get(transport_id)
        .ok_or_else(|| format!("未注册 transport: {}", transport_id))?;
    let conn = plugin
        .start(app.clone(), &config)
        .await
        .map_err(|e| e.to_string())?;
    plugin.initialize(&conn).await.map_err(|e| e.to_string())?;
    let tools = plugin.list_tools(&conn).await.map_err(|e| e.to_string())?;
    // 测试完即关
    let _ = plugin.stop(conn).await;
    Ok(tools)
}

// ====== 权限检查 ======

/// 权限检查结果（返回给前端）
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PermissionCheckResult {
    pub action: String, // "allow" | "ask" | "deny"
    /// 当 action = "ask" 时，包含审批请求 ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_id: Option<String>,
    /// 拒绝/需要确认的原因说明
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// 检查工具调用的权限（预检）。
///
/// 前端在 `handleToolCall` 中先调用此命令，根据返回的 `action` 决定是否执行：
/// - `"allow"` — 可直接调用 `call_mcp_tool`
/// - `"ask"` — 需显示审批对话框，用户批准后再调用 `call_mcp_tool`
/// - `"deny"` — 工具被禁止，跳过执行
#[tauri::command]
pub async fn check_tool_permission(
    app: AppHandle,
    tool_name: String,
    server_id: String,
    arguments: Value,
    agent_mode: String,
    project_id: Option<String>,
) -> Result<PermissionCheckResult, String> {
    let mode = match agent_mode.as_str() {
        "normal" => AgentMode::Normal,
        "auto" => AgentMode::Auto,
        "plan" => AgentMode::Plan,
        _ => AgentMode::Off,
    };

    // 加载项目级自定义规则
    let project_path = project_id.as_ref().and_then(|pid| resolve_project_path_mcp_opt(&app, pid));
    let custom_rules = project_path
        .as_ref()
        .map(|p| permission::load_permissions(Some(p)).rules)
        .unwrap_or_default();

    let action = permission::check_permission(&tool_name, &server_id, &arguments, &mode, &custom_rules);

    match action {
        PermissionAction::Deny => Ok(PermissionCheckResult {
            action: "deny".into(),
            approval_id: None,
            reason: Some(format!("工具 '{}' 在当前模式下已被禁止执行", tool_name)),
        }),
        PermissionAction::Allow => Ok(PermissionCheckResult {
            action: "allow".into(),
            approval_id: None,
            reason: None,
        }),
        PermissionAction::Ask => Ok(PermissionCheckResult {
            action: "ask".into(),
            approval_id: None,
            reason: Some(format!("工具 '{}' 需要您的确认才能执行", tool_name)),
        }),
    }
}



/// 审批结果负载（发送给前端的事件）
///
/// 注意：字段名保持 **snake_case**，与其它 LLM 流式事件（`llm-tool-call`/`llm-chunk`）
/// 保持一致。旧的 `rename_all="camelCase"` 会把 `approval_id` 序列化成 `approvalId`，
/// 而前端按 `approval_id` 解构，导致全部 `undefined`、`respond_tool_approval` 永远失败、
/// normal 模式每个写/删操作 60s 超时后报错。
#[derive(Serialize, Clone, Debug)]
pub struct ApprovalRequestPayload {
    pub approval_id: String,
    pub server_id: String,
    pub tool_name: String,
    pub arguments: Value,
    pub reason: String,
    /// 文件变更预览（unified diff），仅文件操作工具携带
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_diff: Option<String>,
    /// 受影响的文件路径
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
}

/// 发起一个需要用户确认的工具调用。
///
/// 在 `execute_tool_call` 中检测到权限为 `Ask` 时，会通过此流程：
/// 1. 发送 `tool-approval-requested` 事件到前端
/// 2. 阻塞等待前端调用 `respond_tool_approval`
/// 3. 用户批准 → 继续执行；用户拒绝 → 返回错误
///
/// 超时分支会从 `PendingApprovals` 移除 sender，避免泄漏（旧 bug：超时后 sender 永留 map）。
pub(crate) async fn request_tool_approval(
    app: &AppHandle,
    pending: &PendingApprovals,
    server_id: &str,
    tool_name: &str,
    arguments: &Value,
    reason: &str,
    preview_diff: Option<String>,
    file_path: Option<String>,
) -> Result<(), String> {
    let (tx, rx) = oneshot::channel::<bool>();
    let approval_id = pending.insert(tx);

    // 发送事件到前端
    let _ = app.emit(
        "tool-approval-requested",
        ApprovalRequestPayload {
            approval_id: approval_id.clone(),
            server_id: server_id.to_string(),
            tool_name: tool_name.to_string(),
            arguments: arguments.clone(),
            reason: reason.to_string(),
            preview_diff,
            file_path,
        },
    );

    // 等待前端响应（60s 超时）；超时/通道关闭时移除泄漏的 sender
    match tokio::time::timeout(Duration::from_secs(60), rx).await {
        Ok(Ok(true)) => Ok(()),       // 用户批准
        Ok(Ok(false)) => {
            // 拒绝：sender 已被消费，无需移除
            Err("用户已拒绝此操作".into())
        }
        Ok(Err(_)) => {
            // channel 关闭：移除泄漏的 sender
            pending.remove(&approval_id);
            Err("审批通道意外关闭".into())
        }
        Err(_) => {
            // 超时：移除泄漏的 sender，否则永久驻留 PendingApprovals
            pending.remove(&approval_id);
            Err("等待用户确认超时（60s）".into())
        }
    }
}

/// 前端调用此命令来响应工具调用审批。
#[tauri::command]
pub async fn respond_tool_approval(
    pending: State<'_, PendingApprovals>,
    approval_id: String,
    approved: bool,
) -> Result<(), String> {
    let tx = pending.remove(&approval_id)
        .ok_or_else(|| format!("审批请求 {} 不存在或已过期", approval_id))?;
    tx.send(approved).map_err(|_| "发送审批结果失败".into())
}

/// 列出指定 MCP server 的资源列表。
#[tauri::command]
pub async fn list_mcp_resources(
    mgr: State<'_, McpServerManager>,
    state: State<'_, McpServerState>,
    id: String,
) -> Result<Vec<McpResource>, String> {
    let conn = {
        let map = state.lock();
        map.get(&id).cloned().ok_or_else(|| format!("MCP server 未连接: {}", id))?
    };
    let plugin = mgr
        .get(&conn.transport_kind)
        .ok_or_else(|| format!("未注册 transport 插件: {}", conn.transport_kind))?;
    plugin
        .list_resources(&conn)
        .await
        .map_err(|e| format!("resources/list 失败: {}", e))
}

/// 读取指定 MCP server 的一个资源。
#[tauri::command]
pub async fn read_mcp_resource(
    mgr: State<'_, McpServerManager>,
    state: State<'_, McpServerState>,
    id: String,
    uri: String,
) -> Result<ReadResourceResult, String> {
    let conn = {
        let map = state.lock();
        map.get(&id).cloned().ok_or_else(|| format!("MCP server 未连接: {}", id))?
    };
    let plugin = mgr
        .get(&conn.transport_kind)
        .ok_or_else(|| format!("未注册 transport 插件: {}", conn.transport_kind))?;
    plugin
        .read_resource(&conn, &uri)
        .await
        .map_err(|e| format!("resources/read 失败: {}", e))
}

/// 列出指定 MCP server 的提示词列表。
#[tauri::command]
pub async fn list_mcp_prompts(
    mgr: State<'_, McpServerManager>,
    state: State<'_, McpServerState>,
    id: String,
) -> Result<Vec<McpPrompt>, String> {
    let conn = {
        let map = state.lock();
        map.get(&id).cloned().ok_or_else(|| format!("MCP server 未连接: {}", id))?
    };
    let plugin = mgr
        .get(&conn.transport_kind)
        .ok_or_else(|| format!("未注册 transport 插件: {}", conn.transport_kind))?;
    plugin
        .list_prompts(&conn)
        .await
        .map_err(|e| format!("prompts/list 失败: {}", e))
}

/// 获取指定 MCP server 的一个提示词。
#[tauri::command]
pub async fn get_mcp_prompt(
    mgr: State<'_, McpServerManager>,
    state: State<'_, McpServerState>,
    id: String,
    name: String,
    arguments: Option<Value>,
) -> Result<GetPromptResult, String> {
    let conn = {
        let map = state.lock();
        map.get(&id).cloned().ok_or_else(|| format!("MCP server 未连接: {}", id))?
    };
    let plugin = mgr
        .get(&conn.transport_kind)
        .ok_or_else(|| format!("未注册 transport 插件: {}", conn.transport_kind))?;
    plugin
        .get_prompt(&conn, &name, arguments)
        .await
        .map_err(|e| format!("prompts/get 失败: {}", e))
}

/// 测试用：列出已注册 transport 插件 identifier
#[tauri::command]
pub async fn list_mcp_transports(mgr: State<'_, McpServerManager>) -> Result<Vec<String>, String> {
    Ok(mgr.list().into_iter().map(|s| s.to_string()).collect())
}

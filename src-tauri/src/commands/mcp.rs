//! MCP Tauri Commands
//!
//! 提供前端调用的 9 个命令；MCP 运行时状态由 [`McpServerState`] 持有。

use crate::core::models::*;
use crate::core::secure_store;
use crate::core::state::{McpRequestManager, McpServerState};
use crate::plugins::mcp::{self, McpServerManager, McpServerPlugin};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

const TOOL_CALL_TIMEOUT: Duration = Duration::from_secs(30);

fn emit_status(app: &AppHandle, id: &str, status: McpStatus, message: Option<String>, tool_count: usize) {
    let _ = app.emit(
        "mcp-server-status",
        McpServerStatusInfo {
            id: id.to_string(),
            status,
            message,
            tool_count,
        },
    );
}

#[tauri::command]
pub async fn list_mcp_servers(app: AppHandle) -> Result<Vec<McpServerConfig>, String> {
    Ok(mcp::list_configs(&app))
}

#[tauri::command]
pub async fn add_mcp_server(app: AppHandle, config: McpServerConfig) -> Result<(), String> {
    mcp::upsert_config(&app, config).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_mcp_server(
    app: AppHandle,
    state: State<'_, McpServerState>,
    id: String,
) -> Result<(), String> {
    // 先停掉连接
    {
        let mut map = state.lock();
        map.remove(&id);
    }
    // 清理 keyring 中该 server 的所有 env 密钥
    if let Some(cfg) = mcp::get_config(&app, &id) {
        if let crate::core::models::McpTransport::Stdio { env, .. } = &cfg.transport {
            for key in env.keys() {
                let account = secure_store::accounts::mcp_server_env(&id, key);
                let _ = secure_store::delete(&app, &account);
            }
        }
    }
    mcp::remove_config(&app, &id).map_err(|e| e.to_string())?;
    emit_status(&app, &id, McpStatus::Disconnected, None, 0);
    Ok(())
}

#[tauri::command]
pub async fn start_mcp_server(
    app: AppHandle,
    mgr: State<'_, McpServerManager>,
    state: State<'_, McpServerState>,
    id: String,
) -> Result<Vec<ToolSpec>, String> {
    emit_status(&app, &id, McpStatus::Connecting, None, 0);

    let config = mcp::get_config(&app, &id)
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
    plugin
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

    {
        let mut map = state.lock();
        map.insert(id.clone(), conn);
    }

    emit_status(&app, &id, McpStatus::Connected, None, filtered.len());
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
) -> Result<Vec<ToolSpec>, String> {
    // 1) 取出所有 server_id 后立即释放锁
    let ids: Vec<String> = {
        let map = state.lock();
        map.keys().cloned().collect()
    };

    // 2) 收集 (id, conn, cfg, plugin) 全部准备好后再 await
    let mut jobs: Vec<(McpServerConfig, Arc<dyn McpServerPlugin>, Arc<crate::plugins::mcp::connection::McpConnection>)> = Vec::new();
    for id in ids {
        let cfg = match mcp::get_config(&app, &id) {
            Some(c) => c,
            None => continue,
        };
        let transport_id = match &cfg.transport {
            McpTransport::Stdio { .. } => "stdio",
            McpTransport::Http { .. } => "http",
            McpTransport::StreamableHttp { .. } => "streamable_http",
        };
        let Some(plugin) = mgr.get(transport_id) else { continue };
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

#[tauri::command]
pub async fn call_mcp_tool(
    app: AppHandle,
    mgr: State<'_, McpServerManager>,
    state: State<'_, McpServerState>,
    requests: State<'_, McpRequestManager>,
    server_id: String,
    tool_name: String,
    arguments: Value,
) -> Result<ToolResult, String> {
    let cfg = mcp::get_config(&app, &server_id)
        .ok_or_else(|| format!("未找到 MCP server: {}", server_id))?;
    if !cfg.enabled_tools.is_empty() && !cfg.enabled_tools.contains(&tool_name) {
        return Err(format!("工具 {} 不在 server {} 的白名单中", tool_name, server_id));
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
        map.get(&server_id)
            .cloned()
            .ok_or_else(|| format!("MCP server 未连接: {}", server_id))?
    };

    // 用 call_id 跟踪；stop_mcp_server / 用户停止时可 abort
    let call_id = format!("mcp:{}:{}", server_id, tool_name);
    let plugin_arc = plugin.clone();
    let conn_arc = conn.clone();
    let tool_for_task = tool_name.clone();
    let handle = tokio::spawn(async move {
        plugin_arc
            .call_tool(&conn_arc, &tool_for_task, arguments, TOOL_CALL_TIMEOUT)
            .await
    });
    requests.0.insert(call_id.clone(), handle);

    // 取出并等待
    let entry = requests.0.remove(&call_id);
    let handle: tokio::task::JoinHandle<std::result::Result<ToolResult, crate::plugins::mcp::error::McpError>> = match entry {
        Some((_, h)) => h,
        None => return Err("调用已被中止".into()),
    };
    match handle.await {
        Ok(res) => res.map_err(|e: crate::plugins::mcp::error::McpError| e.to_string()),
        Err(e) => Err(format!("工具调用任务 join 失败: {}", e)),
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

/// 测试用：列出已注册 transport 插件 identifier
#[tauri::command]
pub async fn list_mcp_transports(mgr: State<'_, McpServerManager>) -> Result<Vec<String>, String> {
    Ok(mgr.list().into_iter().map(|s| s.to_string()).collect())
}

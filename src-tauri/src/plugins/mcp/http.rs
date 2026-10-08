//! MCP HTTP+SSE 传输
//!
//! 设计决策：HTTP 插件**不**走 McpConnection 的 pending-map 抽象（HTTP 是
//! 请求-响应模式，不需要后台 reader 任务）。`start()` 返回的 McpConnection 仅是
//! 满足 trait 形状的句柄（其 transport 为空操作实现），真实调用直接用 reqwest。
//!
//! 每个 server 的传输状态（URL/头/客户端/id 计数）存放在 **本插件实例**的
//! states 表中，由 McpServerManager 持有插件实例管理生命周期；
//! `stop()` 会移除对应条目，错误路径不再残留。
//!
//! Streamable SSE 长连接在 P2 完整实现。

use super::error::{McpError, McpResult};
use crate::core::models::{
    GetPromptResult, McpPrompt, McpResource, McpServerConfig, McpServerInfo, ReadResourceResult,
    ToolResult, ToolSpec,
};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;

/// MCP HTTP+SSE 传输插件。
///
/// 状态按 server_id 存放在**本插件实例**的表中（而不是进程级全局静态表）：
/// 生命周期由插件实例（归 McpServerManager 所有）管理，错误路径不再残留条目。
pub struct HttpPlugin {
    states: Arc<parking_lot::Mutex<std::collections::HashMap<String, HttpTransportState>>>,
    /// true = Streamable HTTP 传输（identifier "streamable_http"，接受
    /// StreamableHttp 配置，管理 Mcp-Session-Id 会话头）；false = 旧 "http"。
    streamable: bool,
}

impl HttpPlugin {
    pub fn new() -> Self {
        Self {
            states: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
            streamable: false,
        }
    }

    /// Streamable HTTP 传输插件（P4.2：补齐此前缺失的注册，消除死枚举）。
    pub fn streamable() -> Self {
        Self {
            states: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
            streamable: true,
        }
    }
}

impl Default for HttpPlugin {
    fn default() -> Self {
        Self::new()
    }
}

/// 构造带超时的 reqwest 客户端
fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// HTTP transport 内部状态：url + headers + id 计数器 + 会话 id（Streamable）
#[derive(Clone)]
pub struct HttpTransportState {
    pub base_url: String,
    pub headers: Arc<BTreeMap<String, String>>,
    pub client: Client,
    pub next_id: Arc<AtomicU64>,
    pub closed: Arc<AtomicBool>,
    /// Streamable HTTP 会话：initialize 响应的 Mcp-Session-Id，
    /// 后续请求原样回传（会话过期由服务端报错，客户端透传错误即可）
    pub session_id: Arc<parking_lot::Mutex<Option<String>>>,
}

impl HttpTransportState {
    fn endpoint(&self) -> String {
        let trimmed = self.base_url.trim_end_matches('/');
        if trimmed.ends_with("/mcp") || trimmed.ends_with("/sse") {
            trimmed.to_string()
        } else {
            format!("{}/mcp", trimmed)
        }
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }
}

#[async_trait]
impl super::McpServerPlugin for HttpPlugin {
    fn identifier(&self) -> &'static str {
        if self.streamable {
            "streamable_http"
        } else {
            "http"
        }
    }

    async fn start(
        &self,
        app: AppHandle,
        config: &McpServerConfig,
    ) -> McpResult<super::connection::McpConnection> {
        let (url, headers) = match &config.transport {
            crate::core::models::McpTransport::Http { url, headers } if !self.streamable => {
                (url.clone(), headers.clone())
            }
            crate::core::models::McpTransport::StreamableHttp { url, headers }
                if self.streamable =>
            {
                (url.clone(), headers.clone())
            }
            _ => {
                return Err(McpError::Server(
                    "HttpPlugin 收到不匹配的 transport 配置".into(),
                ))
            }
        };

        // SSRF 防护：统一 URL 校验（MCP server 可能运行在本地，允许 localhost）
        let url = crate::utils::url_validation::validate_http_url(
            &url,
            &crate::utils::url_validation::HttpUrlOptions::local_engine(),
        )
        .map_err(|e| McpError::TransportStartup(format!("MCP HTTP server URL 非法: {e}")))?;

        // HTTP transport 不需要 McpConnection 的 pending map 机制，
        // 但为了满足 trait，我们仍返回一个 dummy 连接。
        // 真实调用走 HttpTransportState 路径。
        let headers = super::resolve_env_placeholders(&app, &config.id, &headers)?;
        let state = HttpTransportState {
            base_url: url,
            headers: Arc::new(headers),
            client: http_client(),
            next_id: Arc::new(AtomicU64::new(1)),
            closed: Arc::new(AtomicBool::new(false)),
            session_id: Arc::new(parking_lot::Mutex::new(None)),
        };
        let dummy_transport = Box::new(DummyHttpTransport {
            closed: state.closed.clone(),
        });
        let conn =
            super::connection::McpConnection::new(&config.id, self.identifier(), dummy_transport);
        // 把真实状态挂到 conn.server_id 之外的某个可达地方
        // 这里用 thread_local 简化：把 state 存到全局 HashMap
        self.states.lock().insert(config.id.clone(), state);
        Ok(conn)
    }

    async fn initialize(
        &self,
        conn: &super::connection::McpConnection,
    ) -> McpResult<McpServerInfo> {
        let state = self
            .states
            .lock()
            .get(&conn.server_id)
            .cloned()
            .ok_or_else(|| McpError::NotConnected(conn.server_id.clone()))?;
        let id = state.next_id();
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "aio", "version": env!("CARGO_PKG_VERSION") }
            }
        });
        let v = post_json(&state, &body).await?;
        // send notifications/initialized
        let _ = post_json(
            &state,
            &json!({
                "jsonrpc": "2.0", "method": "notifications/initialized"
            }),
        )
        .await;
        let info_val = v
            .get("result")
            .and_then(|r| r.get("serverInfo"))
            .cloned()
            .unwrap_or(json!({}));
        let name = info_val
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let version = info_val
            .get("version")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let capabilities = v
            .get("result")
            .and_then(|r| r.get("capabilities"))
            .and_then(|c| serde_json::from_value(c.clone()).ok());
        Ok(McpServerInfo {
            name,
            version,
            capabilities,
        })
    }

    async fn list_tools(
        &self,
        conn: &super::connection::McpConnection,
    ) -> McpResult<Vec<ToolSpec>> {
        let state = self
            .states
            .lock()
            .get(&conn.server_id)
            .cloned()
            .ok_or_else(|| McpError::NotConnected(conn.server_id.clone()))?;
        let id = state.next_id();
        let body = json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/list", "params": {}
        });
        let v = post_json(&state, &body).await?;
        let tools = v
            .get("result")
            .and_then(|r| r.get("tools"))
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();
        // 协议转换收敛在 convert 模块（stdio/http 单实现）
        Ok(super::convert::tools_to_specs(tools))
    }

    async fn call_tool(
        &self,
        conn: &super::connection::McpConnection,
        name: &str,
        arguments: Value,
        timeout: Duration,
    ) -> McpResult<ToolResult> {
        let state = self
            .states
            .lock()
            .get(&conn.server_id)
            .cloned()
            .ok_or_else(|| McpError::NotConnected(conn.server_id.clone()))?;
        let id = state.next_id();
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        });
        let v = tokio::time::timeout(timeout, post_json(&state, &body))
            .await
            .map_err(|_| McpError::Timeout(timeout))??;
        let result = v.get("result").cloned().unwrap_or(json!({}));
        Ok(super::convert::parse_call_tool_result(&result))
    }

    async fn list_resources(
        &self,
        conn: &super::connection::McpConnection,
    ) -> McpResult<Vec<McpResource>> {
        let state = self
            .states
            .lock()
            .get(&conn.server_id)
            .cloned()
            .ok_or_else(|| McpError::NotConnected(conn.server_id.clone()))?;
        let id = state.next_id();
        let body = json!({ "jsonrpc": "2.0", "id": id, "method": "resources/list", "params": {} });
        let v = post_json(&state, &body).await?;
        let resources = v
            .get("result")
            .and_then(|r| r.get("resources"))
            .and_then(|a| a.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(super::convert::parse_resources(resources))
    }

    async fn read_resource(
        &self,
        conn: &super::connection::McpConnection,
        uri: &str,
    ) -> McpResult<ReadResourceResult> {
        let state = self
            .states
            .lock()
            .get(&conn.server_id)
            .cloned()
            .ok_or_else(|| McpError::NotConnected(conn.server_id.clone()))?;
        let id = state.next_id();
        let body = json!({
            "jsonrpc": "2.0", "id": id, "method": "resources/read",
            "params": { "uri": uri }
        });
        let v = post_json(&state, &body).await?;
        let result = v.get("result").cloned().unwrap_or(json!({}));
        serde_json::from_value(result).map_err(McpError::from)
    }

    async fn list_prompts(
        &self,
        conn: &super::connection::McpConnection,
    ) -> McpResult<Vec<McpPrompt>> {
        let state = self
            .states
            .lock()
            .get(&conn.server_id)
            .cloned()
            .ok_or_else(|| McpError::NotConnected(conn.server_id.clone()))?;
        let id = state.next_id();
        let body = json!({ "jsonrpc": "2.0", "id": id, "method": "prompts/list", "params": {} });
        let v = post_json(&state, &body).await?;
        let prompts = v
            .get("result")
            .and_then(|r| r.get("prompts"))
            .and_then(|a| a.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(super::convert::parse_prompts(prompts))
    }

    async fn get_prompt(
        &self,
        conn: &super::connection::McpConnection,
        name: &str,
        arguments: Option<Value>,
    ) -> McpResult<GetPromptResult> {
        let state = self
            .states
            .lock()
            .get(&conn.server_id)
            .cloned()
            .ok_or_else(|| McpError::NotConnected(conn.server_id.clone()))?;
        let id = state.next_id();
        let mut params = json!({ "name": name });
        if let Some(args) = arguments {
            params["arguments"] = args;
        }
        let body = json!({
            "jsonrpc": "2.0", "id": id, "method": "prompts/get", "params": params
        });
        let v = post_json(&state, &body).await?;
        let result = v.get("result").cloned().unwrap_or(json!({}));
        serde_json::from_value(result).map_err(McpError::from)
    }

    async fn stop(&self, conn: super::connection::McpConnection) -> McpResult<()> {
        self.states.lock().remove(&conn.server_id);
        Ok(())
    }
}

async fn post_json(state: &HttpTransportState, body: &Value) -> McpResult<Value> {
    if state.closed.load(Ordering::SeqCst) {
        return Err(McpError::Server("HTTP transport 已关闭".into()));
    }
    let url = state.endpoint();
    let mut req = state
        .client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .json(body);
    for (k, v) in state.headers.iter() {
        req = req.header(k, v);
    }
    // Streamable HTTP 会话：后续请求回传 initialize 拿到的 Mcp-Session-Id
    if let Some(sid) = state.session_id.lock().clone() {
        req = req.header("Mcp-Session-Id", sid);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| McpError::Server(format!("HTTP POST: {}", e)))?;
    let status = resp.status();
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    // 捕获服务端下发/更新的会话 id（initialize 与会话续期响应都会带）
    if let Some(sid) = resp
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
    {
        *state.session_id.lock() = Some(sid);
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let truncated = if body.len() > 512 {
            &body[..body.floor_char_boundary(512)]
        } else {
            &body
        };
        return Err(McpError::Server(format!(
            "MCP HTTP {}: {}",
            status, truncated
        )));
    }
    if content_type.starts_with("text/event-stream") {
        // SSE 解析（P4.2 补全）：按空行分块；块内多行 data 按 SSE 规范以 \n 连接；
        // event:/id:/注释行忽略；\r\n 容忍。取首个含 data 的块（请求-响应模式
        // 下服务端把 JSON-RPC 响应作为首个事件返回）。
        use futures_util::StreamExt;
        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();
        while let Some(item) = stream.next().await {
            let chunk = item.map_err(|e| McpError::Server(format!("SSE read: {}", e)))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            // 以空行（\n\n 或 \r\n\r\n）为块边界
            let block_end = buffer.find("\n\n").or_else(|| buffer.find("\r\n\r\n"));
            if let Some(pos) = block_end {
                let block: String = buffer[..pos].to_string();
                let mut data_lines: Vec<String> = Vec::new();
                for line in block.lines() {
                    let line = line.trim_end_matches('\r');
                    if let Some(rest) = line.strip_prefix("data:") {
                        data_lines.push(rest.trim_start().to_string());
                    }
                    // event: / id: / :注释 行按规范忽略
                }
                if !data_lines.is_empty() {
                    let data = data_lines.join("\n");
                    if !data.is_empty() {
                        return serde_json::from_str::<Value>(&data)
                            .map_err(|e| McpError::Server(format!("SSE JSON: {}", e)));
                    }
                }
                // 无 data 的块（纯通知/心跳）继续读下一块
                buffer.drain(..pos);
            }
        }
        Err(McpError::Server("SSE 流意外关闭".into()))
    } else {
        resp.json::<Value>()
            .await
            .map_err(|e| McpError::Server(format!("JSON 解析: {}", e)))
    }
}

/// Dummy transport：仅用于满足 McpConnection 抽象
struct DummyHttpTransport {
    closed: Arc<AtomicBool>,
}

#[async_trait]
impl super::connection::McpTransport for DummyHttpTransport {
    async fn send(&self, _payload: &str) -> McpResult<()> {
        // HTTP 模式不走这里
        Err(McpError::Unimplemented(
            "HTTP 模式应直接走 HttpTransportState".into(),
        ))
    }
    async fn close(&self) -> McpResult<()> {
        self.closed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

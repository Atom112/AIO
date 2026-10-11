//! MCP HTTP+SSE transport
//!
//! Design decision: the HTTP plugin does **not** use McpConnection's pending-map
//! abstraction (HTTP is request-response and needs no background reader task).
//! The McpConnection returned by `start()` is merely a handle satisfying the trait
//! shape (its transport is a no-op implementation); real calls go directly through reqwest.
//!
//! Per-server transport state (URL/headers/client/id counter) lives in the states
//! map of **this plugin instance**, whose lifecycle is managed by McpServerManager;
//! `stop()` removes the entry, so error paths leave no residue.
//!
//! Streamable SSE long connections are fully implemented.

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

/// MCP HTTP+SSE transport plugin.
///
/// State is stored per server_id in the map of **this plugin instance** (not a
/// process-level global static table): the lifecycle is managed by the plugin
/// instance (owned by McpServerManager), so error paths leave no stale entries.
pub struct HttpPlugin {
    states: Arc<parking_lot::Mutex<std::collections::HashMap<String, HttpTransportState>>>,
    /// true = Streamable HTTP transport (identifier "streamable_http", accepts
    /// StreamableHttp config, manages the Mcp-Session-Id session header); false = legacy "http".
    streamable: bool,
}

impl HttpPlugin {
    pub fn new() -> Self {
        Self {
            states: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
            streamable: false,
        }
    }

    /// Streamable HTTP transport plugin (registers the transport so the StreamableHttp config variant is not dead code).
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

/// Builds a reqwest client with timeouts.
///
/// MCP-8: redirects are REFUSED. The reqwest default follows up to 10 redirects and
/// forwards custom auth headers (X-API-Key etc. — only Authorization/Cookie are
/// stripped cross-host) to the redirect target; an https→http downgrade would hand
/// the API key to an attacker-controlled host.
fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// HTTP transport internal state: url + headers + id counter + session id (Streamable)
#[derive(Clone)]
pub struct HttpTransportState {
    pub base_url: String,
    pub headers: Arc<BTreeMap<String, String>>,
    pub client: Client,
    pub next_id: Arc<AtomicU64>,
    pub closed: Arc<AtomicBool>,
    /// Streamable HTTP session: the Mcp-Session-Id from the initialize response,
    /// echoed back on subsequent requests (session expiry is a server-side error; the client just forwards it)
    pub session_id: Arc<parking_lot::Mutex<Option<String>>>,
    /// MCP-22: which transport kind this state serves — `false` = legacy HTTP+SSE,
    /// `true` = streamable HTTP. Drives the kind-appropriate endpoint suffix for
    /// bare-host base URLs (`/sse` vs `/mcp`).
    pub streamable: bool,
}

impl HttpTransportState {
    /// MCP-22: endpoint derivation is kind-aware and never silent.
    /// - an explicit path ending in `/mcp` or `/sse` is kept verbatim;
    /// - a BARE host (no path) gets the kind-appropriate suffix — `/sse` for the
    ///   legacy HTTP+SSE transport, `/mcp` for streamable HTTP — logged at debug;
    /// - any other explicit path is used AS IS (the historic code silently rewrote
    ///   `https://host/custom/endpoint` into `https://host/custom/endpoint/mcp`).
    fn endpoint(&self) -> String {
        let trimmed = self.base_url.trim_end_matches('/');
        if trimmed.ends_with("/mcp") || trimmed.ends_with("/sse") {
            return trimmed.to_string();
        }
        let has_explicit_path = url::Url::parse(trimmed)
            .map(|u| !u.path().is_empty() && u.path() != "/")
            .unwrap_or(true);
        if has_explicit_path {
            tracing::debug!(
                "[mcp:http] 使用配置的完整端点（未追加路径后缀）: {}",
                trimmed
            );
            return trimmed.to_string();
        }
        let suffix = if self.streamable { "/mcp" } else { "/sse" };
        tracing::debug!(
            "[mcp:http] 基础 URL 未含路径，按 {} transport 追加 {}: {}{}",
            if self.streamable {
                "Streamable HTTP"
            } else {
                "HTTP+SSE"
            },
            suffix,
            trimmed,
            suffix
        );
        format!("{trimmed}{suffix}")
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }
}

/// MCP-19: our client generates u64 JSON-RPC ids, but some servers echo them back
/// as strings ("1"). Correlation parses both encodings of the same number; any
/// other shape yields None (missing-id leniency is preserved by the callers).
fn rpc_id_as_u64(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse::<u64>().ok(),
        _ => None,
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

        // SSRF protection: unified URL validation (MCP servers may run locally, so localhost is allowed)
        let url = crate::utils::url_validation::validate_http_url(
            &url,
            &crate::utils::url_validation::HttpUrlOptions::local_engine(),
        )
        .map_err(|e| McpError::TransportStartup(format!("MCP HTTP server URL 非法: {e}")))?;

        // The HTTP transport does not need McpConnection's pending-map mechanism,
        // but a dummy connection is still returned to satisfy the trait.
        // Real calls go through the HttpTransportState path.
        let headers = super::resolve_env_placeholders(&app, &config.id, &headers)?;
        let state = HttpTransportState {
            base_url: url,
            headers: Arc::new(headers),
            client: http_client(),
            next_id: Arc::new(AtomicU64::new(1)),
            closed: Arc::new(AtomicBool::new(false)),
            session_id: Arc::new(parking_lot::Mutex::new(None)),
            streamable: self.streamable,
        };
        let dummy_transport = Box::new(DummyHttpTransport {
            closed: state.closed.clone(),
        });
        let conn =
            super::connection::McpConnection::new(&config.id, self.identifier(), dummy_transport);
        // Stash the real state somewhere reachable other than conn.server_id
        // Simplified: the state is stored in the plugin's states HashMap
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
                // MCP-20: shared MCP_PROTOCOL_VERSION constant (two transports)
                "protocolVersion": super::MCP_PROTOCOL_VERSION,
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
        // Protocol conversion is centralized in the convert module (single implementation shared by stdio/http)
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
    // MCP-19: accept numeric-string ids echoed by lenient servers
    let expect_id = body.get("id").and_then(rpc_id_as_u64);
    let mut req = state
        .client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .json(body);
    for (k, v) in state.headers.iter() {
        req = req.header(k, v);
    }
    // Streamable HTTP session: subsequent requests echo back the Mcp-Session-Id obtained from initialize
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
    // Capture the session id issued/updated by the server (present in both initialize and session-renewal responses)
    if let Some(sid) = resp
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
    {
        *state.session_id.lock() = Some(sid);
    }
    // MCP-8: redirect refused (the custom auth headers above must never be forwarded
    // to a redirect target; https→http downgrade would leak the key)
    if status.is_redirection() {
        let target = resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("<unknown>");
        return Err(McpError::Server(crate::ierr!(
            "MCP 端点重定向被拒绝（目标: {}）。请将服务器配置更新为最终地址",
            "MCP endpoint redirect refused (target: {}). Update the server config to the final URL",
            target
        )));
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
        // SSE parsing: split into blocks on blank lines; multiple data lines within a
        // block are joined with \n per the SSE spec; event:/id:/comment lines are
        // ignored; \r\n is tolerated. MCP-4: blocks carrying notifications (a
        // "method" field) or a non-matching id are skipped — only the response for
        // OUR request is returned; error responses surface as errors.
        use futures_util::StreamExt;
        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();
        while let Some(item) = stream.next().await {
            let chunk = item.map_err(|e| McpError::Server(format!("SSE read: {}", e)))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            // MCP-5: consume EVERY complete block in the buffer, delimiter included
            // (the historic drain(..pos) left the "\n\n" separator in place, so the
            // next find hit 0 forever and a server that opens with a heartbeat/
            // comment block hung with a never-draining buffer).
            while let Some((block, consumed)) = next_sse_block(&buffer) {
                let block = block.to_string();
                buffer.drain(..consumed);
                if let Some(data) = sse_block_data(&block) {
                    if let Some(v) = classify_sse_data(&data, expect_id)? {
                        return Ok(v);
                    }
                }
            }
        }
        Err(McpError::Server("SSE 流意外关闭".into()))
    } else {
        let v: Value = resp
            .json()
            .await
            .map_err(|e| McpError::Server(format!("JSON 解析: {}", e)))?;
        // MCP-4: HTTP 200 + {"error":{...}} used to degrade into an empty-result
        // fake success; the id must match the request (missing id tolerated for
        // lenient servers, mismatched id is a protocol error).
        if v.get("method").is_some() {
            return Err(McpError::Server("MCP 返回了通知而非响应".into()));
        }
        if v.get("error").is_some() {
            return Err(rpc_error(&v));
        }
        let resp_id = v.get("id").and_then(rpc_id_as_u64);
        if resp_id.is_some() && resp_id != expect_id {
            return Err(McpError::Server(format!(
                "MCP 响应 id ({resp_id:?}) 与请求 ({expect_id:?}) 不匹配"
            )));
        }
        Ok(v)
    }
}

/// MCP-4: 把 JSON-RPC 响应中的 error 对象转成 [`McpError::Remote`]。
fn rpc_error(v: &Value) -> McpError {
    let err = v.get("error").cloned().unwrap_or(json!({}));
    let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(-32603) as i32;
    let message = err
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("unknown error")
        .to_string();
    McpError::Remote { code, message }
}

/// MCP-5: 从缓冲取下一个完整 SSE 块，返回 (块内容, 含分隔符应消费的长度)。
/// `None` = 缓冲中尚无完整块。**消费长度必须包含分隔符**——只 drain 到边界
/// 会让分隔符残留、后续 find 恒命中 0，缓冲只增不消。
fn next_sse_block(buffer: &str) -> Option<(&str, usize)> {
    if let Some(p) = buffer.find("\r\n\r\n") {
        Some((&buffer[..p], p + 4))
    } else {
        buffer.find("\n\n").map(|p| (&buffer[..p], p + 2))
    }
}

/// 提取一个 SSE 块的 data 载荷（多行 `data:` 按 SSE 规范以 `\n` 拼接）。
/// 无 data 行（心跳/注释/纯 event 行）返回 `None`。
fn sse_block_data(block: &str) -> Option<String> {
    let mut data_lines: Vec<String> = Vec::new();
    for line in block.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start().to_string());
        }
    }
    if data_lines.is_empty() {
        None
    } else {
        Some(data_lines.join("\n"))
    }
}

/// MCP-4: 判定一个 SSE data 块是否为期待中的响应。
/// `Ok(Some(v))` = 命中的响应；`Ok(None)` = 跳过（通知块 / id 不匹配 /
/// 畸形 JSON 仅告警）；`Err` = 远端错误响应。
fn classify_sse_data(data: &str, expect_id: Option<u64>) -> McpResult<Option<Value>> {
    let Ok(v) = serde_json::from_str::<Value>(data) else {
        tracing::warn!(
            "[mcp:http] 无法解析的 SSE data 块: {}",
            data.chars().take(120).collect::<String>()
        );
        return Ok(None);
    };
    if v.get("method").is_some() {
        return Ok(None); // notification (e.g. notifications/message) — not a response
    }
    if v.get("error").is_some() {
        return Err(rpc_error(&v));
    }
    let resp_id = v.get("id").and_then(rpc_id_as_u64);
    if resp_id.is_none() || resp_id == expect_id {
        Ok(Some(v))
    } else {
        Ok(None)
    }
}

/// Dummy transport: exists only to satisfy the McpConnection abstraction
struct DummyHttpTransport {
    closed: Arc<AtomicBool>,
}

#[async_trait]
impl super::connection::McpTransport for DummyHttpTransport {
    async fn send(&self, _payload: &str) -> McpResult<()> {
        // Not used in HTTP mode
        Err(McpError::Unimplemented(
            "HTTP 模式应直接走 HttpTransportState".into(),
        ))
    }
    async fn close(&self) -> McpResult<()> {
        self.closed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MCP-5 回归：心跳/注释块先到时，块消费必须连同分隔符一起删除——
    /// 历史实现 drain(..pos) 留下 "\n\n"，之后 find 恒命中 0、缓冲只增不消，
    /// 先发心跳的服务器 100% 连接失败。
    #[test]
    fn sse_blocks_drain_including_delimiter() {
        // 心跳块 + 响应块（模拟服务器先发 :keep-alive 再发 data）
        let mut buffer =
            ": keep-alive\n\ndata: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{}}\n\n".to_string();
        // 第一个块：心跳（无 data）
        let (block, consumed) = next_sse_block(&buffer).expect("应切出第一块");
        assert_eq!(block, ": keep-alive");
        assert!(sse_block_data(block).is_none());
        buffer.drain(..consumed);
        // 关键回归点：第二块仍能正常切出（分隔符已被消费）
        let (block2, consumed2) = next_sse_block(&buffer).expect("分隔符残留会卡死在此");
        assert!(block2.starts_with("data:"));
        buffer.drain(..consumed2);
        assert!(buffer.is_empty(), "全部块消费后缓冲应清空");
        // \r\n 分隔符变体
        let crlf = "data: x\r\n\r\nrest";
        let (_, consumed) = next_sse_block(crlf).expect("CRLF 分隔符应命中");
        assert_eq!(&crlf[consumed..], "rest");
    }

    /// SSE 多行 data 按规范以 \n 拼接。
    #[test]
    fn sse_block_data_joins_multiline() {
        let block = "event: message\ndata: {\"a\":\ndata: 1}";
        assert_eq!(sse_block_data(block).as_deref(), Some("{\"a\":\n1}"));
        assert!(sse_block_data(": comment only").is_none());
    }

    /// MCP-4 回归：通知块跳过、error 响应浮出、id 匹配才返回。
    #[test]
    fn sse_data_classification() {
        // 通知（带 method）→ 跳过
        let notification =
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{}}";
        assert!(classify_sse_data(notification, Some(7)).unwrap().is_none());
        // 期待 id 的响应 → 命中
        let response = "{\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"tools\":[]}}";
        assert!(classify_sse_data(response, Some(7)).unwrap().is_some());
        // 别的 id 的响应 → 跳过
        let other = "{\"jsonrpc\":\"2.0\",\"id\":99,\"result\":{}}";
        assert!(classify_sse_data(other, Some(7)).unwrap().is_none());
        // error 响应 → 报错（此前 200+error 被当空结果假成功）
        let error_resp =
            "{\"jsonrpc\":\"2.0\",\"id\":7,\"error\":{\"code\":-32000,\"message\":\"boom\"}}";
        let err = classify_sse_data(error_resp, Some(7)).unwrap_err();
        match err {
            McpError::Remote { code, message } => {
                assert_eq!(code, -32000);
                assert_eq!(message, "boom");
            }
            other => panic!("应为 Remote 错误: {other:?}"),
        }
        // 畸形 JSON → 跳过不致命
        assert!(classify_sse_data("not json", Some(7)).unwrap().is_none());
    }

    /// MCP-19: servers that echo the request id back as a STRING ("7") must still
    /// correlate — the historic u64-only parse dropped the id to None, and in the
    /// SSE classifier None was treated as "ours" (mismatched string ids slipped
    /// through as false hits).
    #[test]
    fn string_echoed_rpc_ids_correlate() {
        assert_eq!(rpc_id_as_u64(&json!(7)), Some(7));
        assert_eq!(rpc_id_as_u64(&json!("7")), Some(7));
        assert_eq!(rpc_id_as_u64(&json!("x")), None);
        assert_eq!(rpc_id_as_u64(&json!(null)), None);
        // SSE classifier: matching string id → hit
        let hit = "{\"jsonrpc\":\"2.0\",\"id\":\"7\",\"result\":{}}";
        assert!(classify_sse_data(hit, Some(7)).unwrap().is_some());
        // SSE classifier: OTHER request's string id → correctly skipped now
        let other = "{\"jsonrpc\":\"2.0\",\"id\":\"99\",\"result\":{}}";
        assert!(classify_sse_data(other, Some(7)).unwrap().is_none());
    }

    /// MCP-22: endpoint derivation is kind-aware and never silently rewrites an
    /// explicit path; only a bare host gets the transport-appropriate suffix.
    #[test]
    fn endpoint_derivation_is_kind_aware_and_explicit_paths_kept() {
        let mk = |streamable: bool, base: &str| HttpTransportState {
            base_url: base.to_string(),
            headers: Arc::new(BTreeMap::new()),
            client: http_client(),
            next_id: Arc::new(AtomicU64::new(1)),
            closed: Arc::new(AtomicBool::new(false)),
            session_id: Arc::new(parking_lot::Mutex::new(None)),
            streamable,
        };
        // Explicit /sse and /mcp endpoints kept verbatim
        assert_eq!(
            mk(true, "https://s.example/mcp").endpoint(),
            "https://s.example/mcp"
        );
        assert_eq!(
            mk(false, "https://s.example/sse").endpoint(),
            "https://s.example/sse"
        );
        // Bare host: streamable appends /mcp, legacy appends /sse
        assert_eq!(
            mk(true, "https://s.example").endpoint(),
            "https://s.example/mcp"
        );
        assert_eq!(
            mk(false, "https://s.example").endpoint(),
            "https://s.example/sse"
        );
        // Bare host with trailing slash: normalized then suffixed
        assert_eq!(
            mk(false, "https://s.example/").endpoint(),
            "https://s.example/sse"
        );
        // Explicit custom path: kept AS IS (historic code appended /mcp silently)
        assert_eq!(
            mk(true, "https://s.example/custom/endpoint").endpoint(),
            "https://s.example/custom/endpoint"
        );
        // Localhost + port variants
        assert_eq!(
            mk(true, "http://127.0.0.1:3333").endpoint(),
            "http://127.0.0.1:3333/mcp"
        );
        assert_eq!(
            mk(false, "http://127.0.0.1:3333").endpoint(),
            "http://127.0.0.1:3333/sse"
        );
    }
}

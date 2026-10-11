//! MCP HTTP+SSE transport
//!
//! 阶段4 第3项：请求统一走 [`HttpPlugin::rpc_call`]——与 stdio 的
//! `McpConnection::request` 同语义的单一 JSON-RPC 出口（共享
//! `JsonRpcRequest`/`JsonRpcNotification` 编解码、id 关联、超时、result
//! 剥壳、错误映射——MCP-4/5/19 类两传输分叉的温床从结构上收敛）。
//! 传输层的 HTTP 循环（重定向拒绝 / SSE 块消费 / 会话头）仍在 `post_json`。
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

    /// 阶段4 第3项：HTTP 传输的统一 JSON-RPC 出口（与 stdio 的
    /// `McpConnection::request` 同语义）。此前 7 个方法各持一份手搓
    /// json! 请求体 + 各自的 envelope 拆包——正是 MCP-4/5/19 类分叉的
    /// 温床。收敛后单一实现负责：state 查找、id 生成、**共享
    /// `JsonRpcRequest` 编解码**（两传输线格式单一来源）、超时、result
    /// 剥壳（缺 result → Null，同 stdio）。错误语义在 [`rpc_error`]
    /// 对齐（-32601 → MethodNotFound，此前 http 一律 Remote）。
    async fn rpc_call(
        &self,
        conn: &super::connection::McpConnection,
        method: &str,
        params: Value,
        dur: Duration,
    ) -> McpResult<Value> {
        let state = self
            .states
            .lock()
            .get(&conn.server_id)
            .cloned()
            .ok_or_else(|| McpError::NotConnected(conn.server_id.clone()))?;
        let id = state.next_id();
        // Shared codec with stdio: JsonRpcRequest serializes to the identical wire shape
        // the hand-built json! bodies used to approximate per call site.
        let body = serde_json::to_value(super::connection::JsonRpcRequest::new(
            id,
            method,
            Some(params),
        ))
        .map_err(|e| McpError::JsonRpc(e.to_string()))?;
        let v = tokio::time::timeout(dur, post_json(&state, &body))
            .await
            .map_err(|_| McpError::Timeout(dur))??;
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    }

    /// 阶段4 第3项：HTTP 通知出口——与 [`rpc_call`] 同享编解码单一来源
    /// （`JsonRpcNotification`），此前 initialize 的 notifications/initialized
    /// 是手搓 json!。
    async fn rpc_notify(
        &self,
        conn: &super::connection::McpConnection,
        method: &str,
    ) -> McpResult<()> {
        let state = self
            .states
            .lock()
            .get(&conn.server_id)
            .cloned()
            .ok_or_else(|| McpError::NotConnected(conn.server_id.clone()))?;
        let n = super::connection::JsonRpcNotification {
            jsonrpc: "2.0",
            method: method.to_string(),
            params: None,
        };
        let body = serde_json::to_value(&n).map_err(|e| McpError::JsonRpc(e.to_string()))?;
        // Servers answer notifications with 202/empty or an SSE stream of zero
        // response blocks; post_json errors here are expected and non-fatal.
        let _ = post_json(&state, &body).await;
        Ok(())
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
        let result = self
            .rpc_call(
                conn,
                "initialize",
                json!({
                    // MCP-20: shared MCP_PROTOCOL_VERSION constant (two transports)
                    "protocolVersion": super::MCP_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "aio", "version": env!("CARGO_PKG_VERSION") }
                }),
                Duration::from_secs(60),
            )
            .await?;
        // notifications/initialized completes the handshake
        let _ = self.rpc_notify(conn, "notifications/initialized").await;
        // 阶段4: initialize 解析并入 convert——HTTP 传输的响应多一层 JSON-RPC
        // envelope，rpc_call 已剥 result（stdio/http 不再各持一份解析）
        Ok(super::convert::parse_server_info(&result))
    }

    async fn list_tools(
        &self,
        conn: &super::connection::McpConnection,
    ) -> McpResult<Vec<ToolSpec>> {
        let result = self
            .rpc_call(conn, "tools/list", json!({}), Duration::from_secs(60))
            .await?;
        let tools = result
            .get("tools")
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
        let result = self
            .rpc_call(
                conn,
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
                timeout,
            )
            .await?;
        Ok(super::convert::parse_call_tool_result(&result))
    }

    async fn list_resources(
        &self,
        conn: &super::connection::McpConnection,
    ) -> McpResult<Vec<McpResource>> {
        let result = self
            .rpc_call(conn, "resources/list", json!({}), Duration::from_secs(60))
            .await?;
        let resources = result
            .get("resources")
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
        let result = self
            .rpc_call(
                conn,
                "resources/read",
                json!({ "uri": uri }),
                Duration::from_secs(60),
            )
            .await?;
        serde_json::from_value(result).map_err(McpError::from)
    }

    async fn list_prompts(
        &self,
        conn: &super::connection::McpConnection,
    ) -> McpResult<Vec<McpPrompt>> {
        let result = self
            .rpc_call(conn, "prompts/list", json!({}), Duration::from_secs(60))
            .await?;
        let prompts = result
            .get("prompts")
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
        let mut params = json!({ "name": name });
        if let Some(args) = arguments {
            params["arguments"] = args;
        }
        let result = self
            .rpc_call(conn, "prompts/get", params, Duration::from_secs(60))
            .await?;
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
            // 阶段5: the block-drain loop lives in drain_sse_blocks (exercised by
            // the malicious/edge-server transcript tests below).
            if let Some(v) = drain_sse_blocks(&mut buffer, expect_id)? {
                return Ok(v);
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

/// MCP-4: 把 JSON-RPC 响应中的 error 对象转成 [`McpError`]。
/// 阶段4 第3项：错误语义与 stdio 的 `McpConnection::request` 对齐——
/// -32601 → [`McpError::MethodNotFound`]（此前 http 一律映射 Remote，
/// 是两传输间的活语义分叉）；其余错误码 → `Remote`。
fn rpc_error(v: &Value) -> McpError {
    let err = v.get("error").cloned().unwrap_or(json!({}));
    let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(-32603) as i32;
    let message = err
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("unknown error")
        .to_string();
    if code == -32601 {
        return McpError::MethodNotFound(message);
    }
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

/// 阶段5: drains every complete SSE block currently buffered and returns the
/// response for `expect_id` when one arrives. Extracted from post_json so the
/// full MCP-4/MCP-5/MCP-19 consumption pattern (heartbeat-first servers,
/// notifications, foreign/absent/string ids, error blocks) is testable against
/// hostile transcripts without a live HTTP server.
///
/// Consumed bytes — delimiter included, the MCP-5 regression — are drained from
/// `buffer`; `Ok(None)` = nothing more to do (stream ended / awaiting more bytes).
pub(crate) fn drain_sse_blocks(
    buffer: &mut String,
    expect_id: Option<u64>,
) -> McpResult<Option<Value>> {
    while let Some((block, consumed)) = next_sse_block(buffer) {
        let block = block.to_string();
        buffer.drain(..consumed);
        if let Some(data) = sse_block_data(&block) {
            if let Some(v) = classify_sse_data(&data, expect_id)? {
                return Ok(Some(v));
            }
        }
    }
    Ok(None)
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

    /// 阶段5 集成测试：恶意/边缘 server 台本。按 post_json 的真实消费模式——
    /// 逐 chunk 喂入 drain_sse_blocks（跨 TCP 分段边界），台本依次包含：
    /// 心跳块（MCP-4：先发心跳的服务器必须能连上）→ 日志通知块（跳过）→
    /// 他人数字 id 响应（跳过）→ 他人字符串 id 响应（MCP-19：跳过）→
    /// 我方响应（命中返回）。结尾带一个不完整块，验证它留在缓冲不被误消费。
    #[test]
    fn malicious_edge_server_transcript_drains_correctly() {
        let transcript = concat!(
            ": keep-alive\r\n\r\n",
            "event: log\r\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{}}\r\n\r\n",
            "data: {\"jsonrpc\":\"2.0\",\"id\":98,\"result\":{\"noise\":true}}\r\n\r\n",
            "data: {\"jsonrpc\":\"2.0\",\"id\":\"99\",\"result\":{\"noise\":true}}\r\n\r\n",
            "data: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"tools\":[{\"name\":\"read\"}]}}\r\n\r\n",
            "data: {\"jsonrpc\":\"2.0\",\"id\":\"partial-bl",
        );
        // Feed in 7-byte chunks: blocks straddle chunk boundaries repeatedly.
        // The whole transcript is fed (draining stops after the first hit) so the
        // trailing partial block lands in the buffer exactly as in production.
        let mut buffer = String::new();
        let mut found = None;
        for chunk in transcript.as_bytes().chunks(7) {
            buffer.push_str(&String::from_utf8_lossy(chunk));
            if found.is_none() {
                if let Some(v) = drain_sse_blocks(&mut buffer, Some(7)).unwrap() {
                    found = Some(v);
                }
            }
        }
        let v = found.expect("台本中我方响应必须被命中");
        assert_eq!(v["id"], 7, "命中我方数字 id 响应");
        assert_eq!(v["result"]["tools"][0]["name"], "read");
        // The trailing partial block must remain buffered (not consumed).
        assert!(
            buffer.contains("partial-bl"),
            "不完整块应留在缓冲: {buffer:?}"
        );
    }

    /// 阶段5 集成测试：我方 id 的 error 块浮出为 Remote 错误（PROV-2 类语义），
    /// 即使前面隔着多个心跳块——截断/失败不得被当成功吞掉。
    #[test]
    fn error_block_surfaces_through_heartbeats() {
        let transcript = concat!(
            ": ping\r\n\r\n: ping\r\n\r\n",
            "data: {\"jsonrpc\":\"2.0\",\"id\":7,\"error\":{\"code\":-32000,\"message\":\"boom\"}}\r\n\r\n",
        );
        let mut buffer = String::new();
        buffer.push_str(transcript);
        let err = drain_sse_blocks(&mut buffer, Some(7)).unwrap_err();
        match err {
            McpError::Remote { code, message } => {
                assert_eq!(code, -32000);
                assert_eq!(message, "boom");
            }
            other => panic!("应为 Remote 错误: {other:?}"),
        }
        assert!(buffer.is_empty(), "错误块应被完整消费");
    }

    /// 阶段4 第3项回归：错误语义与 stdio 对齐——-32601 (Method not found)
    /// 映射 [`McpError::MethodNotFound`]，其余错误码仍是 Remote。
    /// 此前 http 一律 Remote，是两传输间的活语义分叉。
    #[test]
    fn rpc_error_maps_32601_to_method_not_found_like_stdio() {
        let method_missing = json!({
            "jsonrpc": "2.0", "id": 7,
            "error": { "code": -32601, "message": "resources/list not supported" }
        });
        match rpc_error(&method_missing) {
            McpError::MethodNotFound(msg) => assert_eq!(msg, "resources/list not supported"),
            other => panic!("-32601 应映射 MethodNotFound: {other:?}"),
        }
        // 其他错误码保持 Remote
        let server_err = json!({
            "jsonrpc": "2.0", "id": 8,
            "error": { "code": -32000, "message": "boom" }
        });
        match rpc_error(&server_err) {
            McpError::Remote { code, message } => {
                assert_eq!(code, -32000);
                assert_eq!(message, "boom");
            }
            other => panic!("非 -32601 应保持 Remote: {other:?}"),
        }
        // 缺 code/message 的畸形 error 对象：默认 -32603/unknown error
        match rpc_error(&json!({ "error": {} })) {
            McpError::Remote { code, message } => {
                assert_eq!(code, -32603);
                assert_eq!(message, "unknown error");
            }
            other => panic!("畸形 error 应降级默认值: {other:?}"),
        }
    }

    /// 阶段5 集成测试：流在我方响应到来前结束 → drain 返回 Ok(None)（调用方
    /// 以"SSE 流意外关闭"报错）；全部完整块均已消费、缓冲只剩无 data 的残留。
    #[test]
    fn drain_returns_none_when_stream_ends_without_our_response() {
        let transcript =
            ": only heartbeats\r\n\r\ndata: {\"jsonrpc\":\"2.0\",\"id\":99,\"result\":{}}\r\n\r\n";
        let mut buffer = String::new();
        buffer.push_str(transcript);
        assert!(drain_sse_blocks(&mut buffer, Some(7)).unwrap().is_none());
        assert!(buffer.is_empty(), "所有完整块（含他人 id）都应被消费");
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

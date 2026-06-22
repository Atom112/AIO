//! MCP HTTP+SSE 传输
//!
//! 设计决策：HTTP 插件**不**走 McpConnection 抽象（HTTP 是请求-响应模式，
//! 不需要后台 reader 任务）。`start()` 返回的 McpConnection 主要是为了满足 trait，
//! 实际 list_tools / call_tool 等方法直接用 reqwest 调用 server。
//!
//! Streamable SSE 长连接在 P2 完整实现。

use super::error::{McpError, McpResult};
use crate::core::models::{
    McpServerConfig, McpServerInfo, ToolResult, ToolResultContent, ToolSpec,
};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;

pub struct HttpPlugin;

/// 构造带超时的 reqwest 客户端
fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// HTTP transport 内部状态：url + headers + id 计数器
#[derive(Clone)]
pub struct HttpTransportState {
    pub base_url: String,
    pub headers: Arc<BTreeMap<String, String>>,
    pub client: Client,
    pub next_id: Arc<AtomicU64>,
    pub closed: Arc<AtomicBool>,
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
        "http"
    }

    async fn start(
        &self,
        app: AppHandle,
        config: &McpServerConfig,
    ) -> McpResult<super::connection::McpConnection> {
        let (url, headers) = match &config.transport {
            crate::core::models::McpTransport::Http { url, headers } => {
                (url.clone(), headers.clone())
            }
            _ => {
                return Err(McpError::Server(
                    "HttpPlugin 收到非 http transport 配置".into(),
                ))
            }
        };

        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(McpError::TransportStartup(format!(
                "MCP HTTP server URL 协议非法: {}",
                url
            )));
        }

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
        };
        let dummy_transport = Box::new(DummyHttpTransport {
            closed: state.closed.clone(),
        });
        let conn = super::connection::McpConnection::new(&config.id, "http", dummy_transport);
        // 把真实状态挂到 conn.server_id 之外的某个可达地方
        // 这里用 thread_local 简化：把 state 存到全局 HashMap
        HTTP_STATES.lock().insert(config.id.clone(), state);
        Ok(conn)
    }

    async fn initialize(
        &self,
        conn: &super::connection::McpConnection,
    ) -> McpResult<McpServerInfo> {
        let state = HTTP_STATES
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
                "protocolVersion": "2024-11-05",
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
        Ok(McpServerInfo { name, version })
    }

    async fn list_tools(
        &self,
        conn: &super::connection::McpConnection,
    ) -> McpResult<Vec<ToolSpec>> {
        let state = HTTP_STATES
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
        // MCP 协议返回 { name, description, inputSchema }，需转换为 OpenAI
        // function calling 格式 { type: "function", function: { name, description, parameters } }
        let specs: Vec<ToolSpec> = tools
            .into_iter()
            .filter_map(|t| {
                let name = t.get("name")?.as_str()?.to_string();
                let description = t.get("description")?.as_str()?.to_string();
                let parameters = t.get("inputSchema").cloned().unwrap_or(json!({}));
                Some(ToolSpec {
                    kind: "function".to_string(),
                    function: crate::core::models::ToolFunctionSpec {
                        name,
                        description,
                        parameters,
                    },
                })
            })
            .take(super::MAX_TOOLS_PER_SERVER)
            .collect();
        Ok(specs)
    }

    async fn call_tool(
        &self,
        conn: &super::connection::McpConnection,
        name: &str,
        arguments: Value,
        timeout: Duration,
    ) -> McpResult<ToolResult> {
        let state = HTTP_STATES
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
        let content_val = result.get("content").cloned().unwrap_or(json!([]));
        let content: Vec<ToolResultContent> = match content_val {
            Value::Array(arr) => arr
                .into_iter()
                .filter_map(|x| serde_json::from_value(x).ok())
                .collect(),
            _ => vec![],
        };
        let is_error = result
            .get("isError")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        Ok(ToolResult { content, is_error })
    }

    async fn stop(&self, conn: super::connection::McpConnection) -> McpResult<()> {
        HTTP_STATES.lock().remove(&conn.server_id);
        Ok(())
    }
}

/// 内部全局表：server_id → HttpTransportState
/// 由于 start/list_tools 不在同一个 async 上下文，用 parking_lot::Mutex
static HTTP_STATES: once_cell::sync::Lazy<
    parking_lot::Mutex<std::collections::HashMap<String, HttpTransportState>>,
> = once_cell::sync::Lazy::new(|| parking_lot::Mutex::new(std::collections::HashMap::new()));

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
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let truncated = if body.len() > 512 {
            &body[..512]
        } else {
            &body
        };
        return Err(McpError::Server(format!(
            "MCP HTTP {}: {}",
            status, truncated
        )));
    }
    if content_type.starts_with("text/event-stream") {
        // 简化：从 SSE 块中提取首个 "data:" 行的 JSON
        use futures_util::StreamExt;
        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();
        while let Some(item) = stream.next().await {
            let chunk = item.map_err(|e| McpError::Server(format!("SSE read: {}", e)))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            if let Some(pos) = buffer.find("\n\n") {
                let block = &buffer[..pos];
                for line in block.lines() {
                    if let Some(rest) = line.strip_prefix("data:") {
                        let data = rest.trim();
                        if !data.is_empty() {
                            return serde_json::from_str::<Value>(data)
                                .map_err(|e| McpError::Server(format!("SSE JSON: {}", e)));
                        }
                    }
                }
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

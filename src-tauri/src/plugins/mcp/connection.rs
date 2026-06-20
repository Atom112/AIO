//! MCP 连接抽象 + JSON-RPC 2.0 编解码
//!
//! 设计要点：
//! - McpConnection 不关心底层传输（stdio / http+sse / streamable-http）
//! - 每个连接内部维护一个 `DashMap<u64, oneshot::Sender>`，将 JSON-RPC request 的
//!   id 关联到对应的 response 接收端
//! - request/notify 共用 `send_raw`；request 会返回 Future<Output=Response>

use super::error::{McpError, McpResult};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::time::timeout;

/// JSON-RPC 2.0 Request
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JsonRpcRequest {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub fn new(id: u64, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method: method.into(),
            params,
        }
    }
}

/// JSON-RPC 2.0 Response
#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcErrorPayload>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JsonRpcErrorPayload {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// JSON-RPC 2.0 Notification（无 id，无需响应）
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JsonRpcNotification {
    pub jsonrpc: &'static str,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// 内部：解析一行 JSON 后返回的中间结果
#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum Incoming {
    Response(JsonRpcResponse),
    Notification(JsonRpcNotification),
}

/// 通用 MCP 连接抽象。传输层只需实现 [`McpTransport`] trait。
#[derive(Clone)]
pub struct McpConnection {
    pub server_id: String,
    pub transport_kind: String,
    /// 内部状态（传输层句柄、id 关联表等）
    pub(crate) inner: Arc<McpConnectionInner>,
}

// 显式 Send + Sync 标记：所有内部字段都是 Send + Sync
unsafe impl Send for McpConnection {}
unsafe impl Sync for McpConnection {}

pub(crate) struct McpConnectionInner {
    pub pending: DashMap<u64, oneshot::Sender<JsonRpcResponse>>,
    pub next_id: AtomicU64,
    /// 传输层抽象
    pub transport: Box<dyn McpTransport>,
}

impl McpConnection {
    /// 由传输层调用：构造一个 McpConnection
    pub fn new(server_id: impl Into<String>, transport_kind: impl Into<String>, transport: Box<dyn McpTransport>) -> Self {
        Self {
            server_id: server_id.into(),
            transport_kind: transport_kind.into(),
            inner: Arc::new(McpConnectionInner {
                pending: DashMap::new(),
                next_id: AtomicU64::new(1),
                transport,
            }),
        }
    }

    /// 发送 JSON-RPC request 并等待响应（带超时）
    pub async fn request(
        &self,
        method: impl Into<String>,
        params: Option<Value>,
        dur: Duration,
    ) -> McpResult<Value> {
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let req = JsonRpcRequest::new(id, method, params);
        let (tx, rx) = oneshot::channel();
        self.inner.pending.insert(id, tx);
        // 序列化失败也不留 pending
        let payload = serde_json::to_string(&req)?;
        let send_res = self.inner.transport.send(&payload).await;
        if let Err(e) = send_res {
            self.inner.pending.remove(&id);
            return Err(e);
        }
        let resp = match timeout(dur, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_canceled)) => {
                self.inner.pending.remove(&id);
                return Err(McpError::JsonRpc("oneshot canceled (sender dropped)".into()));
            }
            Err(_) => {
                self.inner.pending.remove(&id);
                return Err(McpError::Timeout(dur));
            }
        };
        if let Some(err) = resp.error {
            // JSON-RPC -32601 = Method not found；-32000 ~ -32099 = server error
            if err.code == -32601 {
                return Err(McpError::MethodNotFound(err.message));
            }
            return Err(McpError::Remote {
                code: err.code,
                message: err.message,
            });
        }
        Ok(resp.result.unwrap_or(Value::Null))
    }

    /// 发送 JSON-RPC notification（无 id，不等响应）
    pub async fn notify(&self, method: impl Into<String>, params: Option<Value>) -> McpResult<()> {
        let n = JsonRpcNotification {
            jsonrpc: "2.0",
            method: method.into(),
            params,
        };
        let payload = serde_json::to_string(&n)?;
        self.inner.transport.send(&payload).await
    }

    /// 主动处理一行来自 server 的 JSON（由传输层在收到数据时调用）
    /// 若是 Response → 投递给 pending；若是 Notification → 忽略（MCP v1 暂不处理）
    pub(crate) fn dispatch_line(&self, line: &str) {
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return,
        };
        if v.get("id").is_some() && v.get("method").is_none() {
            if let Ok(resp) = serde_json::from_value::<JsonRpcResponse>(v) {
                if let Some((_, tx)) = self.inner.pending.remove(&resp.id) {
                    let _ = tx.send(resp);
                }
            }
        }
        // Notifications 暂不消费
    }

    /// 关闭连接
    pub async fn close(&self) -> McpResult<()> {
        self.inner.transport.close().await
    }
}

/// 传输层抽象
#[async_trait::async_trait]
pub trait McpTransport: Send + Sync {
    /// 发送一行 JSON（自动附加换行）
    async fn send(&self, payload: &str) -> McpResult<()>;
    /// 关闭连接（kill 子进程 / 关闭 HTTP 连接）
    async fn close(&self) -> McpResult<()>;
}

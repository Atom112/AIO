//! MCP 连接抽象 + JSON-RPC 2.0 编解码
//!
//! 设计要点：
//! - McpConnection 不关心底层传输（stdio / http+sse / streamable-http）
//! - 每个连接内部维护一个 `DashMap<u64, oneshot::Sender>`，将 JSON-RPC request 的
//!   id 关联到对应的 response 接收端
//! - request/notify 共用 `send_raw`；request 会返回 Future<Output=Response>

use super::error::{McpError, McpResult};
use crate::utils::jsonrpc::{PendingMap, WaitError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

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

// 说明：McpConnection 的字段（String + Arc<McpConnectionInner>）本身都是 Send + Sync，
// 自动推导即可。此前的手写 unsafe impl 会遮蔽编译器校验——若将来加入非 Send 字段将静默越界，
// 故移除，交由类型系统保证。

pub(crate) struct McpConnectionInner {
    /// JSON-RPC 待响应请求表（与 LSP 客户端共用的 utils::jsonrpc::PendingMap）
    pub pending: PendingMap<JsonRpcResponse>,
    /// 传输层抽象
    pub transport: Box<dyn McpTransport>,
}

impl McpConnection {
    /// 由传输层调用：构造一个 McpConnection
    pub fn new(
        server_id: impl Into<String>,
        transport_kind: impl Into<String>,
        transport: Box<dyn McpTransport>,
    ) -> Self {
        Self {
            server_id: server_id.into(),
            transport_kind: transport_kind.into(),
            inner: Arc::new(McpConnectionInner {
                pending: PendingMap::new(),
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
        let (id, rx) = self.inner.pending.register();
        let req = JsonRpcRequest::new(id, method, params);
        // 序列化失败也不留 pending
        let payload = serde_json::to_string(&req)?;
        let send_res = self.inner.transport.send(&payload).await;
        if let Err(e) = send_res {
            self.inner.pending.cancel(id);
            return Err(e);
        }
        let resp = match self.inner.pending.wait(id, rx, dur).await {
            Ok(r) => r,
            Err(WaitError::Canceled) => {
                return Err(McpError::JsonRpc(
                    "oneshot canceled (sender dropped)".into(),
                ))
            }
            Err(WaitError::Timeout(d)) => return Err(McpError::Timeout(d)),
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

    /// 主动处理一行来自 server 的 JSON（由传输层在收到数据时调用）。
    /// 若是 Response → 投递给 pending；若是 Notification → 返回 (method, params)
    /// 交给调用方消费（P4.1：tools/list_changed 等此前被静默丢弃）。
    pub(crate) fn dispatch_line(&self, line: &str) -> Option<(String, Value)> {
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return None,
        };
        if v.get("id").is_some() && v.get("method").is_none() {
            if let Ok(resp) = serde_json::from_value::<JsonRpcResponse>(v) {
                if let Some(tx) = self.inner.pending.take(resp.id) {
                    let _ = tx.send(resp);
                }
            }
            return None;
        }
        // Notification：有 method 无 id
        if let Some(method) = v.get("method").and_then(|m| m.as_str()) {
            let params = v.get("params").cloned().unwrap_or(Value::Null);
            return Some((method.to_string(), params));
        }
        None
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

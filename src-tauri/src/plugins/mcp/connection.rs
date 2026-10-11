//! MCP connection abstraction + JSON-RPC 2.0 codec
//!
//! Design notes:
//! - McpConnection is agnostic of the underlying transport (stdio / http+sse / streamable-http)
//! - Each connection internally keeps a `DashMap<u64, oneshot::Sender>` that maps a JSON-RPC
//!   request's id to the corresponding response receiver
//! - request/notify share `send_raw`; request returns Future<Output=Response>

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

/// MCP-19: JSON-RPC ids are typed `u64` internally, but real servers echo them
/// back as JSON strings ("id":"7"). Deserializing strictly as u64 made stdio
/// drop those responses silently (the request then hung until its timeout).
/// Numeric strings are accepted and parsed; anything else stays a hard parse
/// error (an unparseable id cannot correlate to any pending request anyway).
fn deserialize_rpc_id<'de, D>(d: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Value::deserialize(d)?;
    match v {
        Value::Number(n) => n
            .as_u64()
            .ok_or_else(|| serde::de::Error::custom("JSON-RPC id must be a u64")),
        Value::String(s) => s
            .parse::<u64>()
            .map_err(|_| serde::de::Error::custom("JSON-RPC string id is not numeric")),
        _ => Err(serde::de::Error::custom(
            "JSON-RPC id must be a number or a numeric string",
        )),
    }
}

/// JSON-RPC 2.0 Response
#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(deserialize_with = "deserialize_rpc_id")]
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

/// JSON-RPC 2.0 Notification (no id, no response expected)
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JsonRpcNotification {
    pub jsonrpc: &'static str,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// Internal: intermediate result after parsing one JSON line
#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum Incoming {
    Response(JsonRpcResponse),
    Notification(JsonRpcNotification),
}

/// Generic MCP connection abstraction. Transports only need to implement the [`McpTransport`] trait.
#[derive(Clone)]
pub struct McpConnection {
    pub server_id: String,
    pub transport_kind: String,
    /// Internal state (transport handle, id correlation table, etc.)
    pub(crate) inner: Arc<McpConnectionInner>,
}

// Note: McpConnection's fields (String + Arc<McpConnectionInner>) are Send + Sync, so
// auto-derivation suffices; a hand-written unsafe impl would mask compiler checks and
// silently accept future non-Send fields. Rely on the type system instead.

pub(crate) struct McpConnectionInner {
    /// Pending JSON-RPC request table (utils::jsonrpc::PendingMap, shared with the LSP client)
    pub pending: PendingMap<JsonRpcResponse>,
    /// Transport abstraction
    pub transport: Box<dyn McpTransport>,
}

impl McpConnection {
    /// Called by the transport layer: constructs a McpConnection
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

    /// MCP-10: instance identity — clones share one `inner` Arc, so this is true
    /// exactly when two handles belong to the same underlying transport. Used by
    /// the pool's self-healing removal to drop ONLY the dead instance (another
    /// project's live variant of the same server id must stay).
    pub fn same_connection(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    /// Send a JSON-RPC request and await the response (with timeout)
    pub async fn request(
        &self,
        method: impl Into<String>,
        params: Option<Value>,
        dur: Duration,
    ) -> McpResult<Value> {
        let (id, rx) = self.inner.pending.register();
        let req = JsonRpcRequest::new(id, method, params);
        // Do not leave a pending entry behind on serialization failure
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
            // JSON-RPC -32601 = Method not found; -32000 ~ -32099 = server error
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

    /// Send a JSON-RPC notification (no id, does not wait for a response)
    pub async fn notify(&self, method: impl Into<String>, params: Option<Value>) -> McpResult<()> {
        let n = JsonRpcNotification {
            jsonrpc: "2.0",
            method: method.into(),
            params,
        };
        let payload = serde_json::to_string(&n)?;
        self.inner.transport.send(&payload).await
    }

    /// Handle one line of JSON received from the server (called by the transport on incoming data).
    /// Response → delivered to pending; Notification → returns (method, params)
    /// for the caller to consume (e.g. tools/list_changed must not be dropped silently).
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
        // Notification: has method, no id
        if let Some(method) = v.get("method").and_then(|m| m.as_str()) {
            let params = v.get("params").cloned().unwrap_or(Value::Null);
            return Some((method.to_string(), params));
        }
        None
    }

    /// Close the connection
    pub async fn close(&self) -> McpResult<()> {
        self.inner.transport.close().await
    }
}

/// Transport abstraction
#[async_trait::async_trait]
pub trait McpTransport: Send + Sync {
    /// Send one line of JSON (newline appended automatically)
    async fn send(&self, payload: &str) -> McpResult<()>;
    /// Close the connection (kill the child process / close the HTTP connection)
    async fn close(&self) -> McpResult<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoopTransport;
    #[async_trait::async_trait]
    impl McpTransport for NoopTransport {
        async fn send(&self, _payload: &str) -> McpResult<()> {
            Ok(())
        }
        async fn close(&self) -> McpResult<()> {
            Ok(())
        }
    }

    /// MCP-19 回归：服务器以 JSON 字符串回显 id（"id":"1"）时必须能关联到
    /// pending 请求——历史上严格 u64 反序列化失败后该行被静默丢弃，
    /// 请求挂到超时。非数字字符串 id 仍然无法关联（保持丢弃语义）。
    #[tokio::test]
    async fn string_echoed_rpc_ids_are_correlated_on_stdio() {
        let conn = McpConnection::new("test", "stdio", Box::new(NoopTransport));
        let (id, rx) = conn.inner.pending.register();
        conn.dispatch_line(&format!(
            r#"{{"jsonrpc":"2.0","id":"{id}","result":{{"ok":true}}}}"#
        ));
        let resp = conn
            .inner
            .pending
            .wait(id, rx, std::time::Duration::from_millis(100))
            .await
            .expect("string-echoed id must reach the pending entry");
        assert_eq!(resp.id, id);
        assert!(resp.result.is_some());
    }
}

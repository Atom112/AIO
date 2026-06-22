//! MCP 模块统一错误类型

use thiserror::Error;

#[derive(Debug, Error)]
pub enum McpError {
    #[error("MCP 服务器未连接: {0}")]
    NotConnected(String),

    #[error("MCP 传输启动失败: {0}")]
    TransportStartup(String),

    #[error("JSON-RPC 协议错误: {0}")]
    JsonRpc(String),

    #[error("JSON 序列化错误: {0}")]
    Json(#[from] serde_json::Error),

    #[error("I/O 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("超时（{0:?}）")]
    Timeout(std::time::Duration),

    #[error("JSON-RPC 远端错误 {code}: {message}")]
    Remote { code: i32, message: String },

    #[error("MCP 服务器返回错误: {0}")]
    Server(String),

    #[error("MCP 协议方法未找到: {0}")]
    MethodNotFound(String),

    #[error("未实现: {0}")]
    Unimplemented(String),
}

pub type McpResult<T> = std::result::Result<T, McpError>;

//! LSP plugin error types

use std::time::Duration;

/// Result type alias for LSP operations
pub type LspResult<T> = Result<T, LspError>;

/// LSP plugin error
#[derive(Debug, thiserror::Error)]
pub enum LspError {
    /// Transport startup failure
    #[error("传输启动失败: {0}")]
    TransportStartup(String),

    /// Transport I/O error
    #[error("传输错误: {0}")]
    Transport(String),

    /// Protocol error
    #[error("协议错误: {0}")]
    Protocol(String),

    /// Request timeout
    #[error("请求超时 ({0:?})")]
    Timeout(Duration),

    /// Error returned by the remote server
    #[error("远程错误 [code={code}]: {message}")]
    Remote { code: i32, message: String },

    /// Serialization/deserialization error
    #[error("序列化错误: {0}")]
    Serialization(#[from] serde_json::Error),

    /// I/O error
    #[error("I/O 错误: {0}")]
    Io(#[from] std::io::Error),
}

//! LSP 插件错误类型

use std::time::Duration;

/// LSP 操作结果类型别名
pub type LspResult<T> = Result<T, LspError>;

/// LSP 插件错误
#[derive(Debug, thiserror::Error)]
pub enum LspError {
    /// 传输层启动失败
    #[error("传输启动失败: {0}")]
    TransportStartup(String),

    /// 传输层 I/O 错误
    #[error("传输错误: {0}")]
    Transport(String),

    /// 协议错误
    #[error("协议错误: {0}")]
    Protocol(String),

    /// 请求超时
    #[error("请求超时 ({0:?})")]
    Timeout(Duration),

    /// 远程服务器返回错误
    #[error("远程错误 [code={code}]: {message}")]
    Remote { code: i32, message: String },

    /// 不支持的操作
    #[error("不支持: {0}")]
    Unsupported(String),

    /// 序列化/反序列化错误
    #[error("序列化错误: {0}")]
    Serialization(#[from] serde_json::Error),

    /// I/O 错误
    #[error("I/O 错误: {0}")]
    Io(#[from] std::io::Error),
}

//! 云端后端 HTTP 客户端与错误类型
//!
//! 所有 Tauri 命令都应通过 [`http_client`] 拿客户端，禁止散落构造 [`reqwest::Client`]。

use std::time::Duration;
use thiserror::Error;

/// 云端后端统一错误类型（命令层在边界做 `.map_err(|e| e.to_string())`）
#[derive(Debug, Error)]
pub enum CloudBackendError {
    #[error("构造 HTTP 客户端失败: {0}")]
    ClientBuild(String),

    #[error("网络请求失败: {0}")]
    Request(#[from] reqwest::Error),

    #[error("服务端返回 HTTP {status}: {message}")]
    Server { status: u16, message: String },
}

pub type CbResult<T> = std::result::Result<T, CloudBackendError>;

/// 构造一个带超时配置的 reqwest 客户端（单例式使用）
///
/// - 连接超时：5s
/// - 请求总超时：15s
/// - User-Agent：固定标识
pub fn http_client() -> CbResult<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .user_agent("AIO-Desktop/0.4 (cloud-backend)")
        .build()
        .map_err(|e| CloudBackendError::ClientBuild(e.to_string()))
}

/// 把非 2xx 响应统一翻译为 [`CloudBackendError::Server`]
pub async fn ensure_success(resp: reqwest::Response) -> CbResult<reqwest::Response> {
    let status = resp.status();
    if status.is_success() {
        Ok(resp)
    } else {
        let body = resp.text().await.unwrap_or_default();
        // 截断长 body 避免日志/前端被巨型响应撑爆
        let truncated: String = body.chars().take(512).collect();
        Err(CloudBackendError::Server {
            status: status.as_u16(),
            message: truncated,
        })
    }
}

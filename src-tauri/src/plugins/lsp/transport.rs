//! LSP 传输层 — Content-Length 帧编码/解码
//!
//! LSP 协议使用类似 HTTP 的帧格式，与 MCP 的换行分隔 JSON 不同：
//! ```text
//! Content-Length: <byte_count>\r\n
//! Content-Type: application/vscode-jsonrpc; charset=utf-8\r\n
//! \r\n
//! <json body>
//! ```
//!
//! 本模块提供：
//! - `encode_message`: 将 JSON 字符串封装为 LSP 帧
//! - `LspFrameReader`: 从 BufReader 中读取并解析 LSP 帧

use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::ChildStdout;

/// 将 JSON body 编码为 LSP Content-Length 帧
pub fn encode_message(body: &str) -> String {
    let len = body.as_bytes().len();
    format!("Content-Length: {}\r\n\r\n{}", len, body)
}

/// 从 child stdout 读取单帧 LSP 消息
///
/// 返回完整的 JSON body 字符串，EOF 时返回 None。
pub async fn read_frame(reader: &mut BufReader<ChildStdout>) -> std::io::Result<Option<String>> {
    // 1. 读取头部行直到空行（\r\n\r\n）
    let mut headers = String::new();
    let mut content_length: Option<usize> = None;

    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            // EOF
            return if headers.is_empty() { Ok(None) } else { Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "incomplete LSP header")) };
        }
        // 空行（\r\n 或 \n）表示头部结束
        if line == "\r\n" || line == "\n" {
            break;
        }
        // 解析 Content-Length: N
        if let Some(len_str) = line
            .to_lowercase()
            .strip_prefix("content-length:")
            .map(|s| s.trim().to_string())
        {
            content_length = Some(
                len_str
                    .parse::<usize>()
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("invalid Content-Length: {}", e)))?,
            );
        }
        headers.push_str(&line);
    }

    let len = content_length
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "missing Content-Length header"))?;

    // 2. 读取指定长度的 body
    let mut body_buf = vec![0u8; len];
    reader.read_exact(&mut body_buf).await?;
    let body = String::from_utf8(body_buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("invalid UTF-8 in LSP message: {}", e)))?;

    Ok(Some(body))
}

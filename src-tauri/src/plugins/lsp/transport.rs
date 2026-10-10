//! LSP transport - Content-Length frame encoding/decoding
//!
//! The LSP protocol uses an HTTP-like frame format, unlike MCP's newline-delimited JSON:
//! ```text
//! Content-Length: <byte_count>\r\n
//! Content-Type: application/vscode-jsonrpc; charset=utf-8\r\n
//! \r\n
//! <json body>
//! ```
//!
//! This module provides:
//! - `encode_message`: wrap a JSON string into an LSP frame
//! - `LspFrameReader`: read and parse LSP frames from a BufReader

use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::ChildStdout;

/// Encode a JSON body as an LSP Content-Length frame
pub fn encode_message(body: &str) -> String {
    let len = body.len();
    format!("Content-Length: {}\r\n\r\n{}", len, body)
}

/// Read a single LSP message frame from child stdout
///
/// Returns the full JSON body string, or None on EOF.
pub async fn read_frame(reader: &mut BufReader<ChildStdout>) -> std::io::Result<Option<String>> {
    // 1. Read header lines until the blank line (\r\n\r\n)
    let mut headers = String::new();
    let mut content_length: Option<usize> = None;

    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            // EOF
            return if headers.is_empty() {
                Ok(None)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "incomplete LSP header",
                ))
            };
        }
        // A blank line (\r\n or \n) marks the end of the headers
        if line == "\r\n" || line == "\n" {
            break;
        }
        // Parse Content-Length: N
        if let Some(len_str) = line
            .to_lowercase()
            .strip_prefix("content-length:")
            .map(|s| s.trim().to_string())
        {
            content_length = Some(len_str.parse::<usize>().map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("invalid Content-Length: {}", e),
                )
            })?);
        }
        headers.push_str(&line);
    }

    let len = content_length.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "missing Content-Length header",
        )
    })?;

    // 2. Read the body of the given length
    let mut body_buf = vec![0u8; len];
    reader.read_exact(&mut body_buf).await?;
    let body = String::from_utf8(body_buf).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid UTF-8 in LSP message: {}", e),
        )
    })?;

    Ok(Some(body))
}

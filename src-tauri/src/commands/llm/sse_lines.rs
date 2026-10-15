//! SSE byte-level line buffering — single implementation shared by the main agent
//! stream (`commands/llm/mod.rs`) and the BTW side-question stream (`btw.rs`).
//!
//! PROV-1: decoding each TCP chunk with `from_utf8_lossy` turns a multi-byte UTF-8
//! character split across chunks into U+FFFD (Chinese streaming garbled into the
//! persisted history). The fix is buffering BYTES and decoding only complete lines:
//! `\n` (0x0A) never appears inside a multi-byte UTF-8 sequence, so every line cut at
//! `\n` is valid UTF-8.
//!
//! LOOP-9: a data line without a trailing newline would grow the buffer unboundedly
//! (a hostile server sending an infinite comment-less line); `push` errors past the
//! configured cap.

/// Byte-level SSE line buffer with a hard size cap.
pub(crate) struct SseLineBuffer {
    buf: Vec<u8>,
    limit: usize,
}

impl SseLineBuffer {
    /// New buffer; `limit` caps the retained bytes of an unfinished line.
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            buf: Vec::new(),
            limit,
        }
    }

    /// Appends a raw chunk. Errors when an unfinished line exceeds the cap
    /// (LOOP-9 runaway-line guard) — the buffer is left as-is; the caller aborts.
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Result<(), String> {
        self.buf.extend_from_slice(chunk);
        if self.buf.len() > self.limit {
            return Err(crate::ierr!(
                "SSE 行缓冲超限（{} 字节）：服务器发送了无换行的超长数据行",
                "SSE line buffer exceeded {} bytes: the server sent an overlong line without a newline",
                self.limit
            ));
        }
        Ok(())
    }

    /// Takes the next complete line (trimmed). `None` when no `\n` is buffered.
    /// Decodes lossily as a backstop — per the invariant above a complete line is
    /// valid UTF-8, so lossy never fires on well-formed input.
    pub(crate) fn next_line(&mut self) -> Option<String> {
        let pos = self.buf.iter().position(|&b| b == b'\n')?;
        let line = String::from_utf8_lossy(&self.buf[..pos]).trim().to_string();
        self.buf.drain(..pos + 1);
        Some(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 阶段5 集成测试：SSE 跨 chunk UTF-8 切割流。把一段含中文与 4 字节 emoji
    /// 的多行流按**每个可能的字节边界**切成两段（外加逐字节喂入的极端情形），
    /// 重组出的行必须与整段喂入完全一致——任何边界上出现 U+FFFD 即失败。
    /// 这是 PROV-1 的穷举回归：逐 chunk lossy 解码在多字节字符被切开时必现乱码。
    #[test]
    fn utf8_multibyte_survives_every_chunk_boundary() {
        let lines = [
            "data: {\"delta\":{\"content\":\"你好，世界\"}}",
            "data: {\"delta\":{\"content\":\"😀 表情与中文混排\"}}",
            "",
            "data: [DONE]",
        ];
        let stream = lines.join("\n") + "\n";

        // Baseline: whole stream in one chunk.
        let mut whole = SseLineBuffer::new(4 * 1024 * 1024);
        whole.push(stream.as_bytes()).unwrap();
        let baseline: Vec<String> = std::iter::from_fn(|| whole.next_line()).collect();

        // Every possible two-chunk split of the stream.
        let bytes = stream.as_bytes();
        for split in 0..bytes.len() {
            let mut buf = SseLineBuffer::new(4 * 1024 * 1024);
            buf.push(&bytes[..split]).unwrap();
            buf.push(&bytes[split..]).unwrap();
            let got: Vec<String> = std::iter::from_fn(|| buf.next_line()).collect();
            assert_eq!(got, baseline, "切分点 {split} 重组行不一致");
        }

        // Extreme: one byte per "chunk" — every multi-byte sequence is cut repeatedly.
        let mut byte_by_byte = SseLineBuffer::new(4 * 1024 * 1024);
        for b in bytes {
            byte_by_byte.push(&[*b]).unwrap();
        }
        let got: Vec<String> = std::iter::from_fn(|| byte_by_byte.next_line()).collect();
        assert_eq!(got, baseline, "逐字节喂入不应产生 U+FFFD");
    }

    /// LOOP-9 回归：无换行的超长行触发上限错误；恰好在上限内的数据不报错。
    #[test]
    fn overlong_line_without_newline_hits_cap() {
        let mut buf = SseLineBuffer::new(16);
        assert!(buf.push(b"short").is_ok());
        // 17 bytes total, no newline -> over the 16-byte cap
        assert!(buf.push(b"0123456789abcdef0").is_err());
        // Reset with a fresh buffer: exactly at the cap (16 bytes) is fine, and the
        // newline (17th byte) still fits under a 17-byte cap, flushing the line.
        let mut ok_buf = SseLineBuffer::new(17);
        assert!(ok_buf.push(b"0123456789abcdef").is_ok());
        // A newline then flushes the buffered bytes.
        assert!(ok_buf.push(b"\n").is_ok());
        assert_eq!(ok_buf.next_line().as_deref(), Some("0123456789abcdef"));
        assert!(ok_buf.next_line().is_none());
    }

    /// Lines are trimmed on extraction (leading/trailing whitespace incl. \r dropped).
    #[test]
    fn lines_are_trimmed_and_crlf_tolerated() {
        let mut buf = SseLineBuffer::new(1024);
        buf.push(b"  data: x  \r\n\r\n").unwrap();
        assert_eq!(buf.next_line().as_deref(), Some("data: x"));
        // The blank line between blocks extracts as an empty (trimmed) line.
        assert_eq!(buf.next_line().as_deref(), Some(""));
        assert!(buf.next_line().is_none());
    }
}

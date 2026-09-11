//! LLM HTTP 客户端与聊天补全辅助 —— 实现已下沉至 `plugins::llm`。
//!
//! 本文件仅作 `commands::llm` 内部的再导出通道（保持既有 `client::*` 引用路径）。

pub(crate) use crate::plugins::llm::*;

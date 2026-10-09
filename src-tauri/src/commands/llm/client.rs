//! LLM HTTP client and chat-completion helpers — implementation lives in `plugins::llm`.
//!
//! This file is only a re-export channel inside `commands::llm` (preserves existing `client::*` import paths).

pub(crate) use crate::plugins::llm::*;

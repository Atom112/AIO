//! Utilities layer (stateless helpers and Agent tool implementations).
//!
//! Modules here are organized into four concerns; confirm where a new module belongs before adding it, to avoid a catch-all dumping ground:
//!
//! - **Agent tool implementations**: `file_tools`, `git_tools`, `shell_tools`, `web_tools`,
//!   `lsp_tools`, `lsp_agent_tools`, `project_map`, `project_index`, `think`, `worktree`
//!   (subagent worktree isolation)
//! - **Security infrastructure**: `sandbox` (subprocess sandbox), `url_validation` (SSRF protection),
//!   `exec_resolve` (PATH-only executable resolution — every bare-name spawn must go through it)
//! - **File and attachment handling**: `file_parser` (PDF/Office/text extraction)
//! - **General utilities**: `tool_result`, `truncate`, `time`, `diff`, `token_counter`,
//!   `generated_images`
//!
//! Conventions: tool results always use `tool_result`'s constructors; truncation goes through `truncate`; time/date uses `time`.
//! `knowledge` is a read-only legacy module (superseded by project-level RAG); no new dependencies on it.

pub mod diff;
pub mod exec_resolve;
pub mod file_parser;
pub mod file_tools;
pub mod generated_images;
pub mod git_tools;
pub mod jsonrpc;
pub mod knowledge;
pub mod lsp_agent_tools;
pub mod lsp_tools;
pub mod project_index;
pub mod project_map;
pub mod sandbox;
pub mod shell_tools;
pub mod think;
pub mod time;
pub mod token_counter;
pub mod tool_result;
pub mod truncate;
pub mod url_validation;
pub mod web_tools;
pub mod worktree;

//! Harness workbench domain — the CLI-harness GUI compatibility layer.
//!
//! AIO drives third-party CLI coding agents (Claude Code, Codex, OpenCode,
//! Oh My Pi, Gemini CLI, Qwen Code, goose, DSH, ...) as headless engines and
//! renders their message stream / chain of thought / tool calls natively in the
//! harness page. Boundary: in this mode AIO never runs its own agent tools —
//! the CLI harness keeps its own tools, sandbox, and permissions; AIO only
//! displays its events (normalized here) and forwards user actions.
//!
//! Layout:
//! - [`model`]    — the normalized event + approval wire model
//! - [`adapter`]  — driver context, command channel, shared process plumbing
//! - [`registry`] — built-in tool entries + availability detection
//! - [`acp`]      — the universal ACP driver (JSON-RPC over stdio)
//! - [`store`]    — SQLite persistence (workspaces / topics / messages)
//! - [`manager`]  — live-session runtime management + event emission

pub mod acp;
pub mod adapter;
pub mod manager;
pub mod model;
pub mod registry;
pub mod store;

pub use manager::HarnessManager;

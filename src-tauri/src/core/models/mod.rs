//! Shared data models, split by domain.
//!
//! Compatibility: everything is re-exported here via pub use, so external paths like crate::core::models::X stay unchanged.

pub mod agent;
pub mod chat;
pub mod config;
pub mod engine;
pub mod mcp;
pub mod project;
pub mod skill;

pub use agent::*;
pub use chat::*;
pub use config::*;
pub use engine::*;
pub use mcp::*;
pub use project::*;
pub use skill::*;

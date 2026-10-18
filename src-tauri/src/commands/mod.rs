//! Command layer entry point (Tauri commands are split into per-domain submodules; commands must stay thin, with business logic pushed down to plugins/services/utils).
//! Adding a command: implement it in the matching domain file, declare it here, and register it in lib.rs (the check:gates cmd-dispatch gate verifies consistency).

pub mod attachment;
pub mod catalog;
pub mod config;
pub mod embedding;
pub mod engine;
pub mod export;
pub mod feedback;
pub mod git;
pub mod harness;
pub mod image_gen;
pub mod llm;
pub mod local_model_download;
pub mod local_model_market;
pub mod lsp;
pub mod mcp;
pub mod mcp_catalog;
pub mod memory;
pub mod model_download;
pub mod project;
pub mod provider_config;
pub mod rerank;
pub mod skill;
pub mod update;

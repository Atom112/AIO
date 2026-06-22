// 鉴权相关命令已迁移到 `crate::cloud_backend::auth`
// （统一管理预留云端后端的 HTTP 调用）
pub mod attachment;
pub mod catalog;
pub mod config;
pub mod engine;
pub mod llm;
pub mod mcp;
pub mod mcp_catalog;
pub mod provider_config;
pub mod skill;
pub mod update;

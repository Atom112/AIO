//! 命令层入口（Tauri commands 按域拆分为各子模块；命令必须薄，业务下沉到 plugins/services/utils）。
//! 新增命令：在对应域文件实现 + 在此声明 + 在 lib.rs 注册（check:gates 的 cmd-dispatch 门禁校验一致性）。

pub mod attachment;
pub mod catalog;
pub mod config;
pub mod embedding;
pub mod engine;
pub mod export;
pub mod git;
pub mod image_gen;
pub mod llm;
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

//! 工具层（无状态辅助与 Agent 工具实现）。
//!
//! 本模块按四类关注点组织；新增模块前请先确认归属，避免继续演变成"什么都能放"的杂货铺：
//!
//! - **Agent 工具实现**：`file_tools`、`git_tools`、`shell_tools`、`web_tools`、
//!   `lsp_tools`、`lsp_agent_tools`、`project_map`、`project_index`、`think`
//! - **安全基础设施**：`sandbox`（子进程沙箱）、`url_validation`（SSRF 防护）
//! - **文件与附件处理**：`file_parser`（PDF/Office/文本提取）
//! - **通用小工具**：`tool_result`、`truncate`、`time`、`diff`、`token_counter`、
//!   `generated_images`
//!
//! 约定：工具结果一律用 `tool_result` 的构造器；截断用 `truncate`；时间/日期用 `time`。
//! `knowledge` 为只读遗留模块（已被项目级 RAG 取代），不得新增依赖。

pub mod diff;
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

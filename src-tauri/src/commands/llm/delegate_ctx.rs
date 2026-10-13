//! Delegate 参数打包（阶段4 第1项子目标 3/5：delegate 参数打包工厂）。
//!
//! `handle_delegate_task` / `handle_delegate_tasks` 此前各持 17 个独立参数，其中
//! 15 个是主循环逐个克隆再按位传的"继承执行上下文"——主循环三处调用点（后台
//! 委派 / 单任务委派 / 批量委派）各自维护一份 17 行克隆瀑布 + 参数瀑布，每加
//! 一个上下文字段要同步改 2 处函数签名 + 3 处调用点。
//! 本结构把继承上下文打包为单一值：主循环构造一次 `DelegateContext`，调用
//! 签名收敛为 `(ctx, arguments)`；新增上下文字段只改一个结构体定义。

use crate::core::models::{AgentMode, CustomSubagentProfile, ProfileModelOverride};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

/// 继承执行上下文（owned：主循环构造一次，可整体 move 进后台任务；各字段句柄
/// 均为廉价 Arc 克隆）。
#[derive(Clone)]
pub(crate) struct DelegateContext {
    pub window: tauri::Window,
    pub app: tauri::AppHandle,
    pub client: reqwest::Client,
    pub api_url: String,
    pub api_key: String,
    pub model: String,
    pub parent_assistant_id: String,
    pub parent_topic_id: String,
    pub project_id: Option<String>,
    pub agent_mode: AgentMode,
    pub token: CancellationToken,
    pub profile_model_overrides: Vec<ProfileModelOverride>,
    pub custom_profiles: Vec<CustomSubagentProfile>,
    pub mcp_server_ids: Vec<String>,
    pub semaphore: Arc<Semaphore>,
    pub locale: String,
}

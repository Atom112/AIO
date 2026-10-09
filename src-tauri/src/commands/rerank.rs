//! 记忆重排命令（rerank_*）：连接测试。
//!
//! 仅本地 ONNX 重排模型（manifest 中 kind=rerank 条目）。
//! 自 v0.8.1 起移除在线 rerank 端点支持，rerank_save_api_key 命令随之删除。

use crate::core::models::MemoryRerankConfig;
use tauri::AppHandle;

/// 重排连接测试（本地模型需已下载且全局启用；返回延迟与错误信息）。
#[tauri::command]
pub async fn rerank_test(
    app: AppHandle,
    config: MemoryRerankConfig,
) -> crate::plugins::rerank::RerankTestResult {
    crate::plugins::rerank::test(&app, &config).await
}

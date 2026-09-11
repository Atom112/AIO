//! 记忆重排命令（rerank_*）：连接测试与在线 API Key 管理。
//!
//! 本地内置重排（bge-reranker-base）按需下载后即可用，无需命令；
//! 本模块仅服务在线 OpenAI 兼容 rerank 端点（Cohere /rerank 形状）。

use crate::core::models::MemoryRerankConfig;
use tauri::AppHandle;

/// 为在线 provider 从 secure_store 注入 API Key。
fn inject_key(app: &AppHandle, cfg: &mut MemoryRerankConfig) {
    if cfg.provider == "online" && cfg.api_key.is_empty() {
        if let Ok(Some(key)) = crate::core::secure_store::get(app, "rerank_api_key") {
            cfg.api_key = key;
        }
    }
}

/// 保存在线重排 API Key（secure_store，不落盘；空值删除）。
#[tauri::command]
pub fn rerank_save_api_key(app: AppHandle, key: String) -> Result<(), String> {
    if key.trim().is_empty() {
        crate::core::secure_store::delete(&app, "rerank_api_key").map_err(|e| e.to_string())
    } else {
        crate::core::secure_store::set(&app, "rerank_api_key", key.trim())
            .map_err(|e| e.to_string())
    }
}

/// 重排连接测试（本地 provider 需模型已下载；在线 provider 发真实请求）。
#[tauri::command]
pub async fn rerank_test(
    app: AppHandle,
    config: MemoryRerankConfig,
) -> crate::plugins::rerank::RerankTestResult {
    let mut cfg = config;
    inject_key(&app, &mut cfg);
    crate::plugins::rerank::test(&app, &cfg).await
}

//! Memory rerank commands (rerank_*): connection testing.
//!
//! Only local ONNX rerank models (manifest entries with kind=rerank).
//! Online rerank endpoint support was removed as of v0.8.1, along with the rerank_save_api_key command.

use crate::core::models::MemoryRerankConfig;
use tauri::AppHandle;

/// Rerank connection test (local model must be downloaded and globally enabled; returns latency and error info).
#[tauri::command]
pub async fn rerank_test(
    app: AppHandle,
    config: MemoryRerankConfig,
) -> crate::plugins::rerank::RerankTestResult {
    crate::plugins::rerank::test(&app, &config).await
}

//! 嵌入供给命令（embedding_*）：连接测试。
//!
//! 自 v0.8 起仅支持本地嵌入模型（内置或按需下载），不再支持在线 OpenAI 兼容端点，
//! 相关 API Key 管理命令（embedding_save_api_key）已移除。

use crate::core::models::MemoryEmbeddingConfig;
use tauri::AppHandle;

/// 嵌入连接测试（返回维度与延迟；本地模型无需联网）。
#[tauri::command]
pub async fn embedding_test(
    app: AppHandle,
    config: MemoryEmbeddingConfig,
) -> crate::plugins::embed::EmbeddingTestResult {
    crate::plugins::embed::test(&app, &config).await
}

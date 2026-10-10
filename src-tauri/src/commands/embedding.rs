//! Embedding provider commands (embedding_*): connection testing.
//!
//! As of v0.8 only local embedding models are supported (built-in or downloaded on demand);
//! online OpenAI-compatible endpoints are no longer supported, and the related API key management command (embedding_save_api_key) has been removed.

use crate::core::models::MemoryEmbeddingConfig;
use tauri::AppHandle;

/// Embedding connection test (returns dimensions and latency; local models need no network).
#[tauri::command]
pub async fn embedding_test(
    app: AppHandle,
    config: MemoryEmbeddingConfig,
) -> crate::plugins::embed::EmbeddingTestResult {
    crate::plugins::embed::test(&app, &config).await
}

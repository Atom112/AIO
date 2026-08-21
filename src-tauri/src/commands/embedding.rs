//! 嵌入供给命令（embedding_*）：状态、连接测试、在线 API Key 管理。
//!
//! 本地内置嵌入模型开箱即用，无需任何命令；本模块仅服务在线 OpenAI 兼容端点。

use crate::core::models::MemoryEmbeddingConfig;
use tauri::AppHandle;

/// 为在线 provider 从 secure_store 注入 API Key。
fn inject_key(app: &AppHandle, cfg: &mut MemoryEmbeddingConfig) {
    if cfg.provider == "online" && cfg.api_key.is_empty() {
        if let Ok(Some(key)) = crate::core::secure_store::get(app, "embedding_api_key") {
            cfg.api_key = key;
        }
    }
}

/// 保存在线嵌入 API Key（secure_store，不落盘；空值删除）。
#[tauri::command]
pub fn embedding_save_api_key(app: AppHandle, key: String) -> Result<(), String> {
    if key.trim().is_empty() {
        crate::core::secure_store::delete(&app, "embedding_api_key").map_err(|e| e.to_string())
    } else {
        crate::core::secure_store::set(&app, "embedding_api_key", key.trim())
            .map_err(|e| e.to_string())
    }
}

/// 嵌入连接测试（返回维度与延迟；本地内置 provider 无需联网）。
#[tauri::command]
pub async fn embedding_test(
    app: AppHandle,
    config: MemoryEmbeddingConfig,
) -> crate::plugins::embed::EmbeddingTestResult {
    let mut cfg = config;
    inject_key(&app, &mut cfg);
    crate::plugins::embed::test(&cfg).await
}

/// 当前嵌入配置与可用状态。
#[tauri::command]
pub fn embedding_status(app: AppHandle) -> Result<serde_json::Value, String> {
    let cfg = crate::commands::config::load_app_config(app.clone()).map_err(|e| e.to_string())?;
    let mut cfg2 = cfg.memory_embedding.clone();
    inject_key(&app, &mut cfg2);
    let (configured, available, dims) = match crate::plugins::embed::resolve(&cfg2) {
        Ok(e) => (true, e.is_available(), e.dimensions()),
        Err(_) => (cfg2.enabled, false, cfg2.dimensions),
    };
    Ok(serde_json::json!({
        "provider": cfg2.provider,
        "model": cfg2.model,
        "configured": configured,
        "available": available,
        "dimensions": dims,
        "enabled": cfg2.enabled,
    }))
}

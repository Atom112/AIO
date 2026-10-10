//! Memory rerank (second-stage refine) plugin layer: local ONNX cross-encoder (kind=rerank models in the manifest).
//!
//! Retrieval flow: hybrid retrieval RRF-fused top-k -> rerank refine -> token-budget truncation before injection.
//! When disabled, resolution fails, or the model is not downloaded, the caller falls back to RRF-only (fail-safe, same behavior as before rerank was introduced).
//! Since v0.8.1, online rerank endpoint support is removed; online/openai_compat in legacy configs is normalized to local.

pub mod local_onnx;

use crate::core::models::MemoryRerankConfig;
use crate::ierr;
use std::sync::Arc;
use tauri::AppHandle;

/// Rerank plugin trait.
#[async_trait::async_trait]
pub trait Reranker: Send + Sync {
    /// Plugin identifier: local | online
    fn id(&self) -> &str;
    /// Current rerank model name.
    fn model_key(&self) -> String;
    /// Reranks (id, text) candidates by query; returns (id, score) sorted descending.
    async fn rerank(
        &self,
        query: &str,
        docs: &[(String, String)],
    ) -> Result<Vec<(String, f32)>, String>;
    /// Whether prerequisites hold (local model downloaded / online URL configured); real availability is probed by rerank.
    fn is_available(&self) -> bool;
}

/// Rerank connection test result (returned by the rerank_test command).
#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RerankTestResult {
    pub ok: bool,
    pub provider: String,
    pub model: String,
    /// Whether prerequisites hold (local model downloaded / online URL configured)
    pub available: bool,
    pub latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Resolves a reranker from config (not cached here; model loading is lazily cached per model id inside the plugin).
/// Local models only; online / openai_compat providers in legacy configs are normalized to local here.
pub fn resolve(app: &AppHandle, cfg: &MemoryRerankConfig) -> Result<Arc<dyn Reranker>, String> {
    if !cfg.enabled {
        return Err(ierr!("重排未启用", "Rerank is not enabled"));
    }
    let model = cfg.model.trim();
    if model.is_empty() {
        return Err(ierr!("未选择重排模型", "No rerank model selected"));
    }
    let entry = crate::core::model_registry::find_entry(model).ok_or_else(|| {
        ierr!(
            "未知重排模型: {}，请在「Embedding 模型管理」中下载支持的模型",
            "Unknown rerank model: {}, download a supported model in Settings → Embedding Models",
            model
        )
    })?;
    if entry.kind != "rerank" {
        return Err(ierr!("非重排模型: {}", "Not a rerank model: {}", model));
    }
    if !crate::core::model_registry::is_model_enabled(app, model) {
        return Err(ierr!(
            "重排模型 {} 已全局停用，请在「Embedding 模型管理」中启用",
            "Rerank model {} is globally disabled, enable it in Settings → Embedding Models",
            model
        ));
    }
    Ok(Arc::new(local_onnx::LocalReranker::new(app, cfg)))
}

/// Connectivity test (used by the rerank_test command).
pub async fn test(app: &AppHandle, cfg: &MemoryRerankConfig) -> RerankTestResult {
    let started = std::time::Instant::now();
    match resolve(app, cfg) {
        Ok(reranker) => {
            let docs = vec![(
                "a".to_string(),
                "项目使用 rusqlite 存储聊天记录与配置".to_string(),
            )];
            match reranker.rerank("数据库选型", &docs).await {
                Ok(_) => RerankTestResult {
                    ok: true,
                    provider: reranker.id().to_string(),
                    model: reranker.model_key(),
                    available: reranker.is_available(),
                    latency_ms: started.elapsed().as_millis() as u64,
                    error: None,
                },
                Err(err) => RerankTestResult {
                    ok: false,
                    provider: cfg.provider.clone(),
                    model: cfg.model.clone(),
                    available: reranker.is_available(),
                    latency_ms: started.elapsed().as_millis() as u64,
                    error: Some(err),
                },
            }
        }
        Err(err) => RerankTestResult {
            ok: false,
            provider: cfg.provider.clone(),
            model: cfg.model.clone(),
            available: false,
            latency_ms: 0,
            error: Some(err),
        },
    }
}

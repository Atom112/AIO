//! Embedding (vectorization) plugin layer: local ONNX models (bundled
//! all-MiniLM-L6-v2 works out of the box, or mainstream open-source models
//! downloaded on demand from Settings). Online embedding (OpenAI-compatible
//! endpoints) is unsupported since v0.8.
//!
//! Design follows the plugin convention of extensions.md: trait + standalone implementation + registry.

pub mod cache;
pub mod local_onnx;

use crate::core::models::MemoryEmbeddingConfig;
use crate::ierr;
use std::sync::Arc;
use tauri::AppHandle;

/// Vectorization plugin trait.
#[async_trait::async_trait]
pub trait Embedder: Send + Sync {
    /// Plugin identifier: local
    fn id(&self) -> &str;
    /// Current embedding model id (manifest entry id).
    fn model_key(&self) -> String;
    /// Configured dimensions (the embed response is authoritative).
    fn dimensions(&self) -> usize;
    /// Batch embedding; returns a vector list of the same length as the input.
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String>;
    /// Whether prerequisites hold (model downloaded/bundled); real availability is probed by embed.
    fn is_available(&self) -> bool;
}

/// Embedding connection test result (returned by the embedding_test command).
#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingTestResult {
    pub ok: bool,
    pub provider: String,
    pub model: String,
    pub dimensions: usize,
    pub latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Resolves an embedder from config (not cached).
///
/// provider is always normalized to local: legacy online / openai_compat / ollama
/// configs are all treated as local; errors on invalid model names or models not
/// downloaded / globally disabled (retrieval then degrades to keyword search).
pub fn resolve(app: &AppHandle, cfg: &MemoryEmbeddingConfig) -> Result<Arc<dyn Embedder>, String> {
    if !cfg.enabled {
        return Err(ierr!("嵌入未启用", "Embedding is not enabled"));
    }
    let id = if cfg.model.trim().is_empty() {
        local_onnx::LOCAL_MODEL_NAME
    } else {
        cfg.model.trim()
    };
    // Global enable/disable check (similar to provider settings)
    if let Some(entry) = crate::core::model_registry::find_entry(id) {
        if entry.kind == "embed" && !crate::core::model_registry::is_model_enabled(app, id) {
            return Err(ierr!(
                "嵌入模型 {} 已全局停用，请在「Embedding 模型管理」中启用",
                "Embedding model {} is globally disabled, enable it in Settings → Embedding Models",
                id
            ));
        }
    } else {
        return Err(ierr!(
            "未知嵌入模型: {}，请在「Embedding 模型管理」中选择已支持的模型",
            "Unknown embedding model: {}, select a supported model in Settings → Embedding Models",
            id
        ));
    }
    Ok(Arc::new(local_onnx::LocalEmbedder::new(app, cfg)))
}

/// Matryoshka dimension trimming: only for models that support dimension trimming
/// (bge-m3 / gte- series) when "configured target dim < model's actual dim"; unknown
/// models are not trimmed (truncation would corrupt non-Matryoshka vectors).
/// Trimming happens on the embedder's output, so the write and query sides stay consistent.
pub fn trim_to_dim(cfg: &MemoryEmbeddingConfig, v: Vec<f32>) -> Vec<f32> {
    let target = cfg.dimensions;
    if target == 0 || target >= v.len() {
        return v;
    }
    let model = cfg.model.to_lowercase();
    let capable =
        model.contains("bge-m3") || model.contains("qwen3-embedding") || model.contains("gte-");
    if !capable {
        return v;
    }
    v.into_iter().take(target).collect()
}

/// Connectivity test (used by the embedding_test command; local models need no network).
pub async fn test(app: &AppHandle, cfg: &MemoryEmbeddingConfig) -> EmbeddingTestResult {
    let started = std::time::Instant::now();
    match resolve(app, cfg) {
        Ok(embedder) => {
            let sample = vec!["连接测试".to_string()];
            match embedder.embed(&sample).await {
                Ok(v) => EmbeddingTestResult {
                    ok: true,
                    provider: embedder.id().to_string(),
                    model: embedder.model_key(),
                    dimensions: v.first().map(|x| x.len()).unwrap_or(0),
                    latency_ms: started.elapsed().as_millis() as u64,
                    error: None,
                },
                Err(err) => EmbeddingTestResult {
                    ok: false,
                    provider: cfg.provider.clone(),
                    model: cfg.model.clone(),
                    dimensions: 0,
                    latency_ms: started.elapsed().as_millis() as u64,
                    error: Some(err),
                },
            }
        }
        Err(err) => EmbeddingTestResult {
            ok: false,
            provider: cfg.provider.clone(),
            model: cfg.model.clone(),
            dimensions: 0,
            latency_ms: 0,
            error: Some(err),
        },
    }
}

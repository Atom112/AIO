//! 嵌入（向量化）插件层：本地内置（开箱即用）与在线 OpenAI 兼容 /v1/embeddings。
//!
//! 设计遵循 extensions.md 的插件规范：trait + 独立实现 + 注册表（EmbedderManager）。

pub mod local_onnx;
pub mod openai_compat;

use crate::core::models::MemoryEmbeddingConfig;
use std::sync::Arc;

/// 向量化插件 trait。
#[async_trait::async_trait]
pub trait Embedder: Send + Sync {
    /// 插件标识：local | openai_compat
    fn id(&self) -> &str;
    /// 当前嵌入模型名。
    fn model_key(&self) -> String;
    /// 配置维度（实际以嵌入响应为准）。
    fn dimensions(&self) -> usize;
    /// 批量嵌入，返回与输入等长的向量列表。
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String>;
    /// 是否具备可用前提（如地址已配置）；真实连通性由 embed 探测。
    fn is_available(&self) -> bool;
}

/// 嵌入连接测试结果（embedding_test 命令返回）。
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

/// 按配置解析嵌入器（不缓存）。
pub fn resolve(cfg: &MemoryEmbeddingConfig) -> Result<Arc<dyn Embedder>, String> {
    if !cfg.enabled {
        return Err("嵌入未启用".into());
    }
    // 兼容旧配置：ollama（已移除本地自选）归入内置；openai_compat 即在线。
    match cfg.provider.as_str() {
        "local" | "ollama" => Ok(Arc::new(local_onnx::LocalEmbedder::new(cfg))),
        "online" | "openai_compat" => Ok(Arc::new(openai_compat::OpenAiCompatEmbedder::new(cfg)?)),
        other => Err(format!("不支持的嵌入 provider: {other}")),
    }
}

/// 连通性测试（embedding_test 命令用）。
pub async fn test(cfg: &MemoryEmbeddingConfig) -> EmbeddingTestResult {
    let started = std::time::Instant::now();
    match resolve(cfg) {
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

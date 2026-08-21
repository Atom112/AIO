//! Ollama 嵌入实现：POST {base}/api/embeddings（支持批量 input）。

use super::Embedder;
use crate::core::models::MemoryEmbeddingConfig;
use serde::Deserialize;

#[derive(Deserialize)]
struct OllamaEmbeddingResponse {
    embeddings: Vec<Vec<f32>>,
}

/// Ollama 嵌入器。
pub struct OllamaEmbedder {
    base_url: String,
    model: String,
    dims: usize,
    client: reqwest::Client,
}

impl OllamaEmbedder {
    /// 从配置构建；api_url 缺省时使用本机默认地址。
    pub fn new(cfg: &MemoryEmbeddingConfig) -> Self {
        let base_url = if cfg.api_url.trim().is_empty() {
            "http://127.0.0.1:11434".to_string()
        } else {
            cfg.api_url.trim_end_matches('/').to_string()
        };
        let model = if cfg.model.trim().is_empty() {
            "bge-m3:latest".to_string()
        } else {
            cfg.model.clone()
        };
        Self {
            base_url,
            model,
            dims: cfg.dimensions.max(1),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
        }
    }
}

#[async_trait::async_trait]
impl Embedder for OllamaEmbedder {
    fn id(&self) -> &str {
        "ollama"
    }

    fn model_key(&self) -> String {
        self.model.clone()
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let url = format!("{}/api/embeddings", self.base_url);
        let body = serde_json::json!({ "model": self.model, "input": texts });
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("嵌入请求失败: {e}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!(
                "Ollama 嵌入失败（{status}）: {}",
                text.chars().take(200).collect::<String>()
            ));
        }
        let parsed: OllamaEmbeddingResponse = resp
            .json()
            .await
            .map_err(|e| format!("解析嵌入响应失败: {e}"))?;
        if parsed.embeddings.len() != texts.len() {
            return Err(format!(
                "嵌入数量不匹配：请求 {} 条，返回 {} 条",
                texts.len(),
                parsed.embeddings.len()
            ));
        }
        Ok(parsed.embeddings)
    }

    fn is_available(&self) -> bool {
        true
    }
}

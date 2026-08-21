//! OpenAI 兼容 /v1/embeddings 实现。

use super::Embedder;
use crate::core::models::MemoryEmbeddingConfig;
use serde::Deserialize;

#[derive(Deserialize)]
struct OpenAiEmbeddingData {
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct OpenAiEmbeddingResponse {
    data: Vec<OpenAiEmbeddingData>,
}

/// OpenAI 兼容嵌入器。
pub struct OpenAiCompatEmbedder {
    api_url: String,
    api_key: String,
    model: String,
    dims: usize,
    client: reqwest::Client,
}

impl OpenAiCompatEmbedder {
    /// 从配置构建；api_key 由调用方从 secure_store 解析后注入 config。
    pub fn new(cfg: &MemoryEmbeddingConfig) -> Result<Self, String> {
        if cfg.api_url.trim().is_empty() {
            return Err("OpenAI 兼容嵌入需要 apiUrl".into());
        }
        let model = if cfg.model.trim().is_empty() {
            "text-embedding-3-small".to_string()
        } else {
            cfg.model.clone()
        };
        Ok(Self {
            api_url: cfg.api_url.trim_end_matches('/').to_string(),
            api_key: cfg.api_key.clone(),
            model,
            dims: cfg.dimensions.max(1),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
        })
    }
}

#[async_trait::async_trait]
impl Embedder for OpenAiCompatEmbedder {
    fn id(&self) -> &str {
        "openai_compat"
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
        let url = format!("{}/embeddings", self.api_url);
        let body = serde_json::json!({ "model": self.model, "input": texts });
        let mut req = self.client.post(&url).json(&body);
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }
        let resp = req.send().await.map_err(|e| format!("嵌入请求失败: {e}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!(
                "嵌入失败（{status}）: {}",
                text.chars().take(200).collect::<String>()
            ));
        }
        let parsed: OpenAiEmbeddingResponse = resp
            .json()
            .await
            .map_err(|e| format!("解析嵌入响应失败: {e}"))?;
        if parsed.data.len() != texts.len() {
            return Err(format!(
                "嵌入数量不匹配：请求 {} 条，返回 {} 条",
                texts.len(),
                parsed.data.len()
            ));
        }
        Ok(parsed.data.into_iter().map(|d| d.embedding).collect())
    }

    fn is_available(&self) -> bool {
        !self.api_url.trim().is_empty()
    }
}

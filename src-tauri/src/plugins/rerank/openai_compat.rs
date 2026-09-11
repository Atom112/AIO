//! OpenAI 兼容 rerank 端点实现（Cohere `/rerank` 形状，兼容各类网关）。
//!
//! 请求：POST {apiUrl}/rerank  body { model, query, documents, top_n }
//! 响应：{ "results": [ { "index": 0, "relevance_score": 0.97 }, ... ] }
//! 兼容弱解析：index 缺失时按 results 出现顺序对齐文档。

use super::Reranker;
use crate::core::models::MemoryRerankConfig;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
struct RerankItem {
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    relevance_score: Option<f32>,
    #[serde(default)]
    score: Option<f32>,
}

#[derive(Deserialize)]
struct RerankResponse {
    #[serde(default)]
    results: Vec<RerankItem>,
}

/// 在线重排器。
pub struct OpenAiCompatReranker {
    api_url: String,
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl OpenAiCompatReranker {
    /// 从配置构建；api_key 由调用方从 secure_store 解析后注入 config。
    pub fn new(cfg: &MemoryRerankConfig) -> Result<Self, String> {
        if cfg.api_url.trim().is_empty() {
            return Err("在线重排需要 apiUrl".into());
        }
        let model = if cfg.model.trim().is_empty() {
            "rerank-multilingual-v3".to_string()
        } else {
            cfg.model.clone()
        };
        Ok(Self {
            api_url: cfg.api_url.trim_end_matches('/').to_string(),
            api_key: cfg.api_key.clone(),
            model,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        })
    }
}

#[async_trait::async_trait]
impl Reranker for OpenAiCompatReranker {
    fn id(&self) -> &str {
        "online"
    }

    fn model_key(&self) -> String {
        self.model.clone()
    }

    async fn rerank(
        &self,
        query: &str,
        docs: &[(String, String)],
    ) -> Result<Vec<(String, f32)>, String> {
        if docs.is_empty() {
            return Ok(Vec::new());
        }
        let url = format!("{}/rerank", self.api_url);
        let body = serde_json::json!({
            "model": self.model,
            "query": query,
            "documents": docs.iter().map(|(_, t)| t).collect::<Vec<_>>(),
            "top_n": docs.len(),
        });
        let mut req = self.client.post(&url).json(&body);
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }
        let resp = req.send().await.map_err(|e| format!("重排请求失败: {e}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!(
                "重排失败（{status}）: {}",
                text.chars().take(200).collect::<String>()
            ));
        }
        let parsed: RerankResponse = resp
            .json()
            .await
            .map_err(|e| format!("解析重排响应失败: {e}"))?;
        let mut scores: HashMap<String, f32> = HashMap::new();
        for (i, item) in parsed.results.iter().enumerate() {
            let idx = item.index.unwrap_or(i);
            if let Some((id, _)) = docs.get(idx) {
                let score = item.relevance_score.or(item.score).unwrap_or(0.0);
                scores.insert(id.clone(), score);
            }
        }
        let mut out: Vec<(String, f32)> = docs
            .iter()
            .filter_map(|(id, _)| scores.get(id).map(|s| (id.clone(), *s)))
            .collect();
        out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(out)
    }

    fn is_available(&self) -> bool {
        !self.api_url.trim().is_empty()
    }
}

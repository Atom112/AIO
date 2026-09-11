//! 记忆重排（二阶段精排）插件层：本地 ONNX 交叉编码器与在线 OpenAI 兼容 rerank 端点。
//!
//! 检索流程：混合检索 RRF 融合 top-k → 重排精排 → token 预算截断注入。
//! 未启用/解析失败/端点超时时调用方回退 RRF-only（fail-safe，行为同未引入重排前）。
//! 与 `plugins/embed` 同构：trait + 独立实现 + `resolve` 按配置分发。

pub mod local_onnx;
pub mod openai_compat;

use crate::core::models::MemoryRerankConfig;
use std::sync::Arc;
use tauri::AppHandle;

/// 重排插件 trait。
#[async_trait::async_trait]
pub trait Reranker: Send + Sync {
    /// 插件标识：local | online
    fn id(&self) -> &str;
    /// 当前重排模型名。
    fn model_key(&self) -> String;
    /// 对 (id, text) 候选按查询精排，返回 (id, score) 降序。
    async fn rerank(
        &self,
        query: &str,
        docs: &[(String, String)],
    ) -> Result<Vec<(String, f32)>, String>;
    /// 是否具备可用前提（本地模型已下载 / 在线地址已配置）；真实可用性由 rerank 探测。
    fn is_available(&self) -> bool;
}

/// 重排连接测试结果（rerank_test 命令返回）。
#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RerankTestResult {
    pub ok: bool,
    pub provider: String,
    pub model: String,
    /// 是否具备可用前提（本地模型已下载 / 在线地址已配置）
    pub available: bool,
    pub latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 按配置解析重排器（不缓存；本地模型加载在插件内部惰性缓存）。
pub fn resolve(app: &AppHandle, cfg: &MemoryRerankConfig) -> Result<Arc<dyn Reranker>, String> {
    if !cfg.enabled {
        return Err("重排未启用".into());
    }
    match cfg.provider.as_str() {
        "local" => Ok(Arc::new(local_onnx::LocalReranker::new(app, cfg))),
        "online" | "openai_compat" => Ok(Arc::new(openai_compat::OpenAiCompatReranker::new(cfg)?)),
        other => Err(format!("不支持的重排 provider: {other}")),
    }
}

/// 连通性测试（rerank_test 命令用）。
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

//! 记忆重排（二阶段精排）插件层：本地 ONNX 交叉编码器（manifest 中的 kind=rerank 模型）。
//!
//! 检索流程：混合检索 RRF 融合 top-k → 重排精排 → token 预算截断注入。
//! 未启用/解析失败/模型未下载时调用方回退 RRF-only（fail-safe，行为同未引入重排前）。
//! 自 v0.8.1 起移除在线 rerank 端点支持，旧配置中的 online/openai_compat 自动归一化为 local。

pub mod local_onnx;

use crate::core::models::MemoryRerankConfig;
use crate::ierr;
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

/// 按配置解析重排器（不缓存；模型加载在插件内部按模型 id 惰性缓存）。
/// 仅支持本地模型；旧配置中的 online / openai_compat provider 在此归一化为 local。
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

//! 嵌入（向量化）插件层：本地 ONNX 模型（内置 all-MiniLM-L6-v2 开箱即用，
//! 或设置页按需下载的主流开源模型）。在线嵌入（OpenAI 兼容端点）自 v0.8 起不再支持。
//!
//! 设计遵循 extensions.md 的插件规范：trait + 独立实现 + 注册表（EmbedderManager）。

pub mod cache;
pub mod local_onnx;

use crate::core::models::MemoryEmbeddingConfig;
use std::sync::Arc;
use tauri::AppHandle;

/// 向量化插件 trait。
#[async_trait::async_trait]
pub trait Embedder: Send + Sync {
    /// 插件标识：local
    fn id(&self) -> &str;
    /// 当前嵌入模型 id（manifest 条目 id）。
    fn model_key(&self) -> String;
    /// 配置维度（实际以嵌入响应为准）。
    fn dimensions(&self) -> usize;
    /// 批量嵌入，返回与输入等长的向量列表。
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String>;
    /// 是否具备可用前提（模型已下载/内置）；真实可用性由 embed 探测。
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
///
/// provider 统一归一化为 local：旧配置中的 online / openai_compat / ollama 均按本地处理，
/// 模型名无效或模型未下载/全局停用时报错（检索侧退化为关键词）。
pub fn resolve(app: &AppHandle, cfg: &MemoryEmbeddingConfig) -> Result<Arc<dyn Embedder>, String> {
    if !cfg.enabled {
        return Err("嵌入未启用".into());
    }
    let id = if cfg.model.trim().is_empty() {
        local_onnx::LOCAL_MODEL_NAME
    } else {
        cfg.model.trim()
    };
    // 全局启用/停用校验（与供应商设置类似）
    if let Some(entry) = crate::core::model_registry::find_entry(id) {
        if entry.kind == "embed" && !crate::core::model_registry::is_model_enabled(app, id) {
            return Err(format!(
                "嵌入模型 {id} 已全局停用，请在「Embedding 模型管理」中启用"
            ));
        }
    } else {
        return Err(format!(
            "未知嵌入模型: {id}，请在「Embedding 模型管理」中选择已支持的模型"
        ));
    }
    Ok(Arc::new(local_onnx::LocalEmbedder::new(app, cfg)))
}

/// Matryoshka 维度裁剪：仅对支持维度裁剪的模型（bge-m3 / gte- 系列）在
/// “配置目标维度 < 模型实际维度”时截断向量；未知模型不裁剪（截断会破坏非 Matryoshka 向量的语义）。
/// 裁剪发生在嵌入器内部输出，写入与查询两侧自动一致。
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

/// 连通性测试（embedding_test 命令用；本地模型无需联网）。
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

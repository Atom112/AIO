//! 本地内置小型嵌入模型（all-MiniLM-L6-v2，ONNX，随应用打包，开箱即用）。
//!
//! 模型文件通过 `include_bytes!` 直接编译进二进制，无需联网、无需安装或选择
//! 本地模型；离线也稳定可用。更强的语义检索可切换在线 OpenAI 兼容 API。

use super::Embedder;
use crate::core::models::MemoryEmbeddingConfig;
use fastembed::{
    InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
};
use std::sync::OnceLock;

pub const LOCAL_MODEL_NAME: &str = "all-MiniLM-L6-v2";
pub const LOCAL_DIMENSIONS: usize = 384;

/// 打包进二进制的模型文件（随安装分发，构成"开箱即用"）。
struct BundledModel;

impl BundledModel {
    const ONNX: &'static [u8] =
        include_bytes!("../../../resources/models/all-MiniLM-L6-v2/model.onnx");
    const TOKENIZER: &'static [u8] =
        include_bytes!("../../../resources/models/all-MiniLM-L6-v2/tokenizer.json");
    const CONFIG: &'static [u8] =
        include_bytes!("../../../resources/models/all-MiniLM-L6-v2/config.json");
    const SPECIAL: &'static [u8] =
        include_bytes!("../../../resources/models/all-MiniLM-L6-v2/special_tokens_map.json");
    const TOKENIZER_CONFIG: &'static [u8] =
        include_bytes!("../../../resources/models/all-MiniLM-L6-v2/tokenizer_config.json");
}

/// 惰性初始化全局模型实例（进程内仅加载一次，供所有调用共享）。
fn model() -> Result<&'static TextEmbedding, String> {
    static INSTANCE: OnceLock<Result<TextEmbedding, String>> = OnceLock::new();
    INSTANCE
        .get_or_init(|| {
            // 说明：UserDefinedEmbeddingModel 为 #[non_exhaustive]，须用构造器。
            let user_model = UserDefinedEmbeddingModel::new(
                BundledModel::ONNX.to_vec(),
                TokenizerFiles {
                    tokenizer_file: BundledModel::TOKENIZER.to_vec(),
                    config_file: BundledModel::CONFIG.to_vec(),
                    special_tokens_map_file: BundledModel::SPECIAL.to_vec(),
                    tokenizer_config_file: BundledModel::TOKENIZER_CONFIG.to_vec(),
                },
            )
            .with_pooling(Pooling::Mean);
            TextEmbedding::try_new_from_user_defined(
                user_model,
                InitOptionsUserDefined::default().with_max_length(256),
            )
            .map_err(|e| format!("本地嵌入模型加载失败: {e}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// 本地内置嵌入器。
pub struct LocalEmbedder;

impl LocalEmbedder {
    pub fn new(_cfg: &MemoryEmbeddingConfig) -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Embedder for LocalEmbedder {
    fn id(&self) -> &str {
        "local"
    }

    fn model_key(&self) -> String {
        LOCAL_MODEL_NAME.into()
    }

    fn dimensions(&self) -> usize {
        LOCAL_DIMENSIONS
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        let m = model()?;
        let texts = texts.to_vec();
        let out = tokio::task::spawn_blocking(move || m.embed(texts, None))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        Ok(out)
    }

    fn is_available(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 内置模型开箱即用：应能独立产出 384 维向量（无需联网/Ollama）。
    #[tokio::test]
    async fn local_embedder_produces_384_dim() {
        let cfg = MemoryEmbeddingConfig::default();
        let e = LocalEmbedder::new(&cfg);
        assert_eq!(e.dimensions(), 384);
        let v = e
            .embed(&[
                "hello world".to_string(),
                "the cat sits on the mat".to_string(),
            ])
            .await
            .unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].len(), 384);
        assert!(v[0].iter().any(|&x| x != 0.0));
    }
}

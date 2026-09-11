//! 本地嵌入模型（ONNX）：内置 all-MiniLM-L6-v2（随应用打包，开箱即用）与
//! 设置页按需下载的主流开源模型（安装目录可自定义）。
//!
//! 模型文件加载约定：内置模型走 `include_bytes!`；下载模型读取
//! `<embed_dir>/<id>/`（onnx + tokenizer 文件，与 commands/model_download.rs manifest 一致），
//! 通过 fastembed `UserDefinedEmbeddingModel` 加载，进程内按模型 id 惰性缓存一次。
//! 池化方式（mean/cls）取自 manifest（BGE/GTE/MXBAI 为 CLS，MiniLM/E5/paraphrase 为 Mean）。

use super::Embedder;
use crate::core::models::MemoryEmbeddingConfig;
use fastembed::{
    InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::AppHandle;

pub const LOCAL_MODEL_NAME: &str = "all-MiniLM-L6-v2";
pub const LOCAL_DIMENSIONS: usize = 384;

/// 打包进二进制的内置模型文件（随安装分发，构成"开箱即用"）。
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

type ModelCache = HashMap<String, Result<Arc<TextEmbedding>, String>>;

/// 进程内模型实例缓存（按模型 id；加载结果缓存，失败不重复尝试直至重启）。
static INSTANCES: OnceLock<Mutex<ModelCache>> = OnceLock::new();

fn instances() -> &'static Mutex<ModelCache> {
    INSTANCES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn pooling_of(id: &str) -> Pooling {
    match crate::core::model_registry::find_entry(id) {
        Some(e) => match e.pooling {
            "mean" => Pooling::Mean,
            _ => Pooling::Cls,
        },
        None => Pooling::Mean,
    }
}

/// 加载（或取缓存）模型实例。内置模型走 include_bytes；下载模型读安装目录。
fn load_model(embed_dir: &Path, id: &str) -> Result<Arc<TextEmbedding>, String> {
    {
        let map = instances().lock().map_err(|e| e.to_string())?;
        if let Some(hit) = map.get(id) {
            return hit.clone();
        }
    }
    let entry = crate::core::model_registry::find_entry(id).ok_or_else(|| {
        format!("未知嵌入模型: {id}，请在「Embedding 模型管理」中选择已支持的模型")
    })?;
    let (onnx, tokenizer) = if entry.bundled {
        (
            BundledModel::ONNX.to_vec(),
            TokenizerFiles {
                tokenizer_file: BundledModel::TOKENIZER.to_vec(),
                config_file: BundledModel::CONFIG.to_vec(),
                special_tokens_map_file: BundledModel::SPECIAL.to_vec(),
                tokenizer_config_file: BundledModel::TOKENIZER_CONFIG.to_vec(),
            },
        )
    } else {
        let dir = embed_dir.join(id);
        let onnx_path = dir.join(entry.files.first().map(|f| f.name).unwrap_or("model.onnx"));
        if !onnx_path.exists() {
            return Err(format!(
                "嵌入模型未下载（{}），请在「Embedding 模型管理」中下载",
                dir.display()
            ));
        }
        let read = |name: &str| -> Result<Vec<u8>, String> {
            std::fs::read(dir.join(name))
                .map_err(|e| format!("读取 {} 失败: {e}", dir.join(name).display()))
        };
        let onnx = std::fs::read(&onnx_path)
            .map_err(|e| format!("读取 ONNX 权重失败（{}）: {e}", onnx_path.display()))?;
        (
            onnx,
            TokenizerFiles {
                tokenizer_file: read("tokenizer.json")?,
                config_file: read("config.json")?,
                special_tokens_map_file: read("special_tokens_map.json").unwrap_or_default(),
                tokenizer_config_file: read("tokenizer_config.json").unwrap_or_default(),
            },
        )
    };
    let user_model = UserDefinedEmbeddingModel::new(onnx, tokenizer).with_pooling(pooling_of(id));
    let result = TextEmbedding::try_new_from_user_defined(
        user_model,
        InitOptionsUserDefined::default().with_max_length(256),
    )
    .map(Arc::new)
    .map_err(|e| format!("本地嵌入模型加载失败（{id}）: {e}"));
    let mut map = instances().lock().map_err(|e| e.to_string())?;
    map.insert(id.to_string(), result.clone());
    result
}

/// 本地嵌入器。
pub struct LocalEmbedder {
    /// 嵌入模型安装根目录（<embed_dir>/<id>/ 存放模型文件）。
    embed_dir: PathBuf,
    cfg: MemoryEmbeddingConfig,
    id: String,
    dims: usize,
}

impl LocalEmbedder {
    pub fn new(app: &AppHandle, cfg: &MemoryEmbeddingConfig) -> Self {
        let dir = crate::core::model_registry::embed_models_dir(app);
        Self::with_dir(dir, cfg)
    }

    /// 指定安装目录构造（测试与自定义目录共用）。
    pub fn with_dir(embed_dir: PathBuf, cfg: &MemoryEmbeddingConfig) -> Self {
        let id = if cfg.model.trim().is_empty() {
            LOCAL_MODEL_NAME.to_string()
        } else {
            cfg.model.clone()
        };
        let dims = crate::core::model_registry::find_entry(&id)
            .map(|e| e.dims)
            .filter(|d| *d > 0)
            .unwrap_or(LOCAL_DIMENSIONS);
        Self {
            embed_dir,
            cfg: cfg.clone(),
            id,
            dims,
        }
    }
}

#[async_trait::async_trait]
impl Embedder for LocalEmbedder {
    fn id(&self) -> &str {
        "local"
    }

    fn model_key(&self) -> String {
        self.id.clone()
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        let mut result: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
        let mut pending: Vec<(usize, String)> = Vec::new();
        for (i, t) in texts.iter().enumerate() {
            match super::cache::get(&self.id, self.cfg.dimensions, t) {
                Some(v) => result.push(v),
                None => {
                    pending.push((i, t.clone()));
                    result.push(Vec::new());
                }
            }
        }
        if !pending.is_empty() {
            let m = load_model(&self.embed_dir, &self.id)?;
            let cfg = self.cfg.clone();
            let pending_texts: Vec<String> = pending.iter().map(|(_, t)| t.clone()).collect();
            let out = tokio::task::spawn_blocking(move || m.embed(pending_texts, None))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
            for (slot, (i, t)) in pending.iter().enumerate() {
                let v = super::trim_to_dim(&cfg, out.get(slot).cloned().unwrap_or_default());
                super::cache::put(&self.id, cfg.dimensions, t, v.clone());
                result[*i] = v;
            }
        }
        Ok(result)
    }

    fn is_available(&self) -> bool {
        if let Some(entry) = crate::core::model_registry::find_entry(&self.id) {
            if entry.bundled {
                return true;
            }
            let dir = self.embed_dir.join(&self.id);
            return entry.files.iter().all(|f| dir.join(f.name).exists());
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 内置模型开箱即用：应能独立产出 384 维向量（无需联网/下载）。
    #[tokio::test]
    async fn bundled_embedder_produces_384_dim() {
        let cfg = MemoryEmbeddingConfig::default();
        // bundled 路径不访问安装目录，测试传入空目录即可
        let e = LocalEmbedder::with_dir(PathBuf::new(), &cfg);
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

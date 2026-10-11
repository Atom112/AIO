//! Local embedding models (ONNX): bundled all-MiniLM-L6-v2 (shipped with the app,
//! works out of the box) and mainstream open-source models downloaded on demand
//! from Settings (install directory is customizable).
//!
//! Model file loading convention: bundled models use `include_bytes!`; downloaded
//! models read from `<embed_dir>/<id>/` (onnx + tokenizer files, matching the
//! commands/model_download.rs manifest), loaded via fastembed
//! `UserDefinedEmbeddingModel`, cached lazily once per model id in-process.
//! Pooling (mean/cls) comes from the manifest (BGE/GTE/MXBAI use CLS; MiniLM/E5/paraphrase use Mean).

use super::Embedder;
use crate::core::models::MemoryEmbeddingConfig;
use crate::ierr;
use fastembed::{
    InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::AppHandle;

pub const LOCAL_MODEL_NAME: &str = "all-MiniLM-L6-v2";
pub const LOCAL_DIMENSIONS: usize = 384;

/// Built-in model files embedded into the binary (shipped with installs, enabling out-of-the-box use).
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

type ModelCache = HashMap<String, Arc<TextEmbedding>>;

/// In-process model instance cache (keyed by model id).
/// PROV-8: only successful loads are cached — a failure (model not downloaded yet,
/// corrupt file, …) must be retried on the next call, never frozen until restart
/// (the historic error-caching made a freshly downloaded model unusable until the
/// app restarted; re-checking a missing file is a few cheap stats).
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

/// Loads (or fetches from cache) a model instance. Bundled models use include_bytes; downloaded models read from the install directory.
/// PROV-9: reads and graph-builds a multi-hundred-MB ONNX — callers must run this on
/// the blocking pool, never on an async worker thread.
fn load_model(embed_dir: &Path, id: &str) -> Result<Arc<TextEmbedding>, String> {
    {
        let map = instances().lock().map_err(|e| e.to_string())?;
        if let Some(hit) = map.get(id) {
            return Ok(hit.clone());
        }
    }
    let entry = crate::core::model_registry::find_entry(id).ok_or_else(|| {
        ierr!(
            "未知嵌入模型: {}，请在「Embedding 模型管理」中选择已支持的模型",
            "Unknown embedding model: {}, select a supported model in Settings → Embedding Models",
            id
        )
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
            return Err(ierr!(
                "嵌入模型未下载（{}），请在「Embedding 模型管理」中下载",
                "Embedding model not downloaded ({}), download it in Settings → Embedding Models",
                dir.display()
            ));
        }
        let read = |name: &str| -> Result<Vec<u8>, String> {
            std::fs::read(dir.join(name)).map_err(|e| {
                ierr!(
                    "读取 {} 失败: {}",
                    "Failed to read {}: {}",
                    dir.join(name).display(),
                    e
                )
            })
        };
        let onnx = std::fs::read(&onnx_path).map_err(|e| {
            ierr!(
                "读取 ONNX 权重失败（{}）: {}",
                "Failed to read ONNX weights ({}): {}",
                onnx_path.display(),
                e
            )
        })?;
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
    let instance = Arc::new(
        TextEmbedding::try_new_from_user_defined(
            user_model,
            InitOptionsUserDefined::default().with_max_length(256),
        )
        .map_err(|e| {
            ierr!(
                "本地嵌入模型加载失败（{}）: {}",
                "Failed to load local embedding model ({}): {}",
                id,
                e
            )
        })?,
    );
    // PROV-8: cache the success only (see INSTANCES doc)
    let mut map = instances().lock().map_err(|e| e.to_string())?;
    map.insert(id.to_string(), instance.clone());
    Ok(instance)
}

/// Removes a model from the in-process instance cache (called after model files are deleted, so stale instances do not remain usable).
pub fn evict_model(id: &str) {
    if let Ok(mut map) = instances().lock() {
        map.remove(id);
    }
}

/// Local embedder.
pub struct LocalEmbedder {
    /// Root install directory for embedding models (<embed_dir>/<id>/ holds the model files).
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

    /// Constructs with an explicit install directory (shared by tests and custom directories).
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
            // PROV-9: reading + graph-building a multi-hundred-MB ONNX must not run
            // on an async worker thread.
            let dir = self.embed_dir.clone();
            let id = self.id.clone();
            let m = tokio::task::spawn_blocking(move || load_model(&dir, &id))
                .await
                .map_err(|e| e.to_string())??;
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

    /// The bundled model works out of the box: must produce 384-dim vectors standalone (no network/download needed).
    #[tokio::test]
    async fn bundled_embedder_produces_384_dim() {
        let cfg = MemoryEmbeddingConfig::default();
        // The bundled path never touches the install directory, so an empty dir suffices for the test
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

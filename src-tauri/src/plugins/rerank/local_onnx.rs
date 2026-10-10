//! Local ONNX cross-encoder rerank: models are downloaded on demand to `<app_data_dir>/models/rerank/<id>/`,
//! loaded via fastembed's `TextRerank` (user-defined) and lazily cached per model id.
//! Supports every kind=rerank model in the manifest (bge-reranker series + mxbai-rerank).

use super::Reranker;
use crate::core::models::MemoryRerankConfig;
use crate::ierr;
use fastembed::{
    OnnxSource, RerankInitOptionsUserDefined, TextRerank, TokenizerFiles, UserDefinedRerankingModel,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use tauri::AppHandle;

/// Default local rerank model id in the manifest (matches the default in core/models/config.rs).
pub const LOCAL_MODEL_ID: &str = "bge-reranker-base";
const MAX_LENGTH: usize = 512;

/// App data directory (injected at startup by lib.rs; local model loading does not depend on the AppHandle lifecycle).
static APP_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// In-process cache of loaded rerank models (slotted by model id: each model loads only once;
/// deleting model files unloads the slot via evict_model).
static MODEL_CACHE: OnceLock<Mutex<HashMap<String, Arc<TextRerank>>>> = OnceLock::new();

fn model_cache() -> &'static Mutex<HashMap<String, Arc<TextRerank>>> {
    MODEL_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Unload a loaded rerank model instance (called when deleting model files; no-op if not loaded).
pub fn evict_model(id: &str) {
    if let Ok(mut map) = model_cache().lock() {
        map.remove(id);
    }
}

/// Inject the app data directory at startup (called from lib.rs setup).
pub fn init_app_data_dir(dir: PathBuf) {
    let _ = APP_DATA_DIR.set(dir);
}

fn app_data_dir() -> Option<&'static PathBuf> {
    APP_DATA_DIR.get()
}

/// Local rerank model directory: <app_data_dir>/models/rerank/<id>/
fn model_dir(model_id: &str) -> Result<PathBuf, String> {
    let base = app_data_dir().ok_or_else(|| {
        ierr!(
            "应用数据目录未初始化",
            "App data directory is not initialized"
        )
    })?;
    Ok(base.join("models").join("rerank").join(model_id))
}

/// Relative path of the model's ONNX weights file (quantized variants differ per model: model_int8 / model_quantized),
/// taken as the unique .onnx entry from the manifest entry's file list.
fn onnx_rel_path(model_id: &str) -> Result<&'static str, String> {
    let entry = crate::core::model_registry::find_entry(model_id)
        .ok_or_else(|| ierr!("未知重排模型: {}", "Unknown rerank model: {}", model_id))?;
    if entry.kind != "rerank" {
        return Err(ierr!("非重排模型: {}", "Not a rerank model: {}", model_id));
    }
    entry
        .files
        .iter()
        .find(|f| f.name.ends_with(".onnx"))
        .map(|f| f.name)
        .ok_or_else(|| {
            ierr!(
                "重排模型 {} 缺少 ONNX 文件清单",
                "Rerank model {} has no ONNX file in its manifest",
                model_id
            )
        })
}

/// Whether the local rerank model is downloaded (onnx + tokenizer files all present). When missing, is_available()=false,
/// retrieval falls back to RRF-only, and the UI shows "pending download" and guides the user to download it.
fn files_present(model_id: &str) -> bool {
    let Ok(dir) = model_dir(model_id) else {
        return false;
    };
    let Ok(onnx) = onnx_rel_path(model_id) else {
        return false;
    };
    dir.join(onnx).exists()
        && dir.join("tokenizer.json").exists()
        && dir.join("config.json").exists()
}

/// Local reranker: lazily loads the ONNX model for the configured model id (once per model in-process).
pub struct LocalReranker {
    model_id: String,
}

impl LocalReranker {
    pub fn new(_app: &AppHandle, cfg: &MemoryRerankConfig) -> Self {
        let id = if cfg.model.trim().is_empty() {
            LOCAL_MODEL_ID.to_string()
        } else {
            cfg.model.trim().to_string()
        };
        Self { model_id: id }
    }

    fn model(&self) -> Result<Arc<TextRerank>, String> {
        if let Ok(map) = model_cache().lock() {
            if let Some(m) = map.get(&self.model_id) {
                return Ok(m.clone());
            }
        }
        let dir = model_dir(&self.model_id)?;
        let onnx_rel = onnx_rel_path(&self.model_id)?;
        let onnx = dir.join(onnx_rel);
        if !onnx.exists() {
            return Err(ierr!(
                "本地重排模型未下载（{}），请在「设置 → Embedding 模型管理」中下载后再启用重排",
                "Local rerank model not downloaded ({}), download it in Settings → Embedding Models before enabling rerank",
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
        let tokenizer_files = TokenizerFiles {
            tokenizer_file: read("tokenizer.json")?,
            config_file: read("config.json")?,
            special_tokens_map_file: read("special_tokens_map.json").unwrap_or_default(),
            tokenizer_config_file: read("tokenizer_config.json").unwrap_or_default(),
        };
        let user_model = UserDefinedRerankingModel::new(OnnxSource::File(onnx), tokenizer_files);
        // RerankInitOptionsUserDefined is #[non_exhaustive]; must be built via Default + mutable fields
        let mut init = RerankInitOptionsUserDefined::default();
        init.max_length = MAX_LENGTH;
        let m = Arc::new(
            TextRerank::try_new_from_user_defined(user_model, init).map_err(|e| {
                ierr!(
                    "本地重排模型加载失败: {}",
                    "Failed to load local rerank model: {}",
                    e
                )
            })?,
        );
        if let Ok(mut map) = model_cache().lock() {
            map.insert(self.model_id.clone(), m.clone());
        }
        Ok(m)
    }
}

#[async_trait::async_trait]
impl Reranker for LocalReranker {
    fn id(&self) -> &str {
        "local"
    }

    fn model_key(&self) -> String {
        self.model_id.clone()
    }

    async fn rerank(
        &self,
        query: &str,
        docs: &[(String, String)],
    ) -> Result<Vec<(String, f32)>, String> {
        if docs.is_empty() {
            return Ok(Vec::new());
        }
        let m = self.model()?;
        let texts: Vec<String> = docs.iter().map(|(_, t)| t.clone()).collect();
        let query = query.to_string();
        let result = tokio::task::spawn_blocking(move || m.rerank(query, texts, false, Some(16)))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        // RerankResult.index is the document index; map back to (id, score) and sort descending
        let mut out: Vec<(String, f32)> = Vec::with_capacity(result.len());
        for r in result {
            if let Some(doc) = docs.get(r.index) {
                out.push((doc.0.clone(), r.score));
            }
        }
        out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(out)
    }

    fn is_available(&self) -> bool {
        files_present(&self.model_id)
    }
}

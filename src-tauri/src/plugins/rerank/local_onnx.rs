//! 本地 ONNX 交叉编码器重排：模型按需下载到 `<app_data_dir>/models/rerank/<id>/`，
//! 通过 fastembed 的 `TextRerank`（user-defined）加载，按模型 id 分槽惰性缓存。
//! 支持 manifest 中全部 kind=rerank 模型（bge-reranker 系列 + mxbai-rerank）。

use super::Reranker;
use crate::core::models::MemoryRerankConfig;
use fastembed::{
    OnnxSource, RerankInitOptionsUserDefined, TextRerank, TokenizerFiles, UserDefinedRerankingModel,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use tauri::AppHandle;

/// manifest 中的默认本地重排模型 id（与 core/models/config.rs 的默认配置一致）。
pub const LOCAL_MODEL_ID: &str = "bge-reranker-base";
const MAX_LENGTH: usize = 512;

/// 应用数据目录（由 lib.rs 启动时注入；本地模型加载不依赖 AppHandle 生命周期）。
static APP_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 已加载重排模型的进程内缓存（按模型 id 分槽：每个模型仅加载一次；
/// 删除模型文件时经 evict_model 卸载对应槽位）。
static MODEL_CACHE: OnceLock<Mutex<HashMap<String, Arc<TextRerank>>>> = OnceLock::new();

fn model_cache() -> &'static Mutex<HashMap<String, Arc<TextRerank>>> {
    MODEL_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 卸载已加载的重排模型实例（删除模型文件时调用；未加载为 no-op）。
pub fn evict_model(id: &str) {
    if let Ok(mut map) = model_cache().lock() {
        map.remove(id);
    }
}

/// 启动时注入应用数据目录（lib.rs setup 调用）。
pub fn init_app_data_dir(dir: PathBuf) {
    let _ = APP_DATA_DIR.set(dir);
}

fn app_data_dir() -> Option<&'static PathBuf> {
    APP_DATA_DIR.get()
}

/// 本地重排模型目录：<app_data_dir>/models/rerank/<id>/
fn model_dir(model_id: &str) -> Result<PathBuf, String> {
    let base = app_data_dir().ok_or_else(|| "应用数据目录未初始化".to_string())?;
    Ok(base.join("models").join("rerank").join(model_id))
}

/// 模型的 ONNX 权重文件相对路径（各模型量化版命名不同：model_int8 / model_quantized），
/// 从 manifest 条目的文件清单中取唯一的 .onnx 条目。
fn onnx_rel_path(model_id: &str) -> Result<&'static str, String> {
    let entry = crate::core::model_registry::find_entry(model_id)
        .ok_or_else(|| format!("未知重排模型: {model_id}"))?;
    if entry.kind != "rerank" {
        return Err(format!("非重排模型: {model_id}"));
    }
    entry
        .files
        .iter()
        .find(|f| f.name.ends_with(".onnx"))
        .map(|f| f.name)
        .ok_or_else(|| format!("重排模型 {model_id} 缺少 ONNX 文件清单"))
}

/// 本地重排模型是否已下载（onnx + tokenizer 文件齐备）。未下载时 is_available()=false，
/// 检索回退 RRF-only，UI 显示"待下载"并引导下载。
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

/// 本地重排器：按配置的模型 id 惰性加载对应 ONNX 模型（进程内每模型仅一次）。
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
            return Err(format!(
                "本地重排模型未下载（{}），请在「设置 → Embedding 模型管理」中下载后再启用重排",
                dir.display()
            ));
        }
        let read = |name: &str| -> Result<Vec<u8>, String> {
            std::fs::read(dir.join(name))
                .map_err(|e| format!("读取 {} 失败: {e}", dir.join(name).display()))
        };
        let tokenizer_files = TokenizerFiles {
            tokenizer_file: read("tokenizer.json")?,
            config_file: read("config.json")?,
            special_tokens_map_file: read("special_tokens_map.json").unwrap_or_default(),
            tokenizer_config_file: read("tokenizer_config.json").unwrap_or_default(),
        };
        let user_model = UserDefinedRerankingModel::new(OnnxSource::File(onnx), tokenizer_files);
        // RerankInitOptionsUserDefined 为 #[non_exhaustive]，须用 Default + 可变字段
        let mut init = RerankInitOptionsUserDefined::default();
        init.max_length = MAX_LENGTH;
        let m = Arc::new(
            TextRerank::try_new_from_user_defined(user_model, init)
                .map_err(|e| format!("本地重排模型加载失败: {e}"))?,
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
        // RerankResult.index 即文档下标，映射回 (id, score) 并降序
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

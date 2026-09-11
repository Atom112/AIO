//! 本地 ONNX 交叉编码器重排：模型按需下载到 `<app_data_dir>/models/rerank/<id>/`，
//! 通过 fastembed 的 `TextRerank`（user-defined）加载，进程内惰性缓存一次。
//! 默认 bge-reranker-base（中英双语，约 110MB，单 onnx 文件）。

use super::Reranker;
use crate::core::models::MemoryRerankConfig;
use fastembed::{
    OnnxSource, RerankInitOptionsUserDefined, TextRerank, TokenizerFiles, UserDefinedRerankingModel,
};
use std::path::PathBuf;
use std::sync::OnceLock;
use tauri::{AppHandle, Manager};

/// manifest 中的本地重排模型条目 id（与 commands/model_download.rs 保持一致）。
pub const LOCAL_MODEL_ID: &str = "bge-reranker-base";
/// ONNX 权重文件（int8 量化版；与 model_download.rs manifest 保持一致）。
const MODEL_ONNX: &str = "onnx/model_int8.onnx";
const MAX_LENGTH: usize = 512;

/// 应用数据目录（由 lib.rs 启动时注入；本地模型加载不依赖 AppHandle 生命周期）。
static APP_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 启动时注入应用数据目录（lib.rs setup 调用）。
pub fn init_app_data_dir(dir: PathBuf) {
    let _ = APP_DATA_DIR.set(dir);
}

fn app_data_dir() -> Option<&'static PathBuf> {
    APP_DATA_DIR.get()
}

/// 本地重排模型目录：<app_data_dir>/models/rerank/<id>/
pub fn model_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("models")
        .join("rerank")
        .join(LOCAL_MODEL_ID)
}

/// 本地模型是否已下载（onnx + tokenizer 文件齐备）。未下载时 is_available()=false，
/// 检索回退 RRF-only，UI 显示"待下载"并引导下载。
fn files_present(app: &AppHandle) -> bool {
    let dir = model_dir(app);
    dir.join(MODEL_ONNX).exists()
        && dir.join("tokenizer.json").exists()
        && dir.join("config.json").exists()
}

/// 本地重排器：惰性加载下载的 ONNX 模型（进程内仅一次，线程安全共享）。
pub struct LocalReranker {
    app: AppHandle,
}

impl LocalReranker {
    pub fn new(app: &AppHandle, _cfg: &MemoryRerankConfig) -> Self {
        Self { app: app.clone() }
    }

    fn model() -> Result<&'static TextRerank, String> {
        static INSTANCE: OnceLock<Result<TextRerank, String>> = OnceLock::new();
        INSTANCE
            .get_or_init(|| {
                let base = app_data_dir().ok_or_else(|| "应用数据目录未初始化".to_string())?;
                let dir = base.join("models").join("rerank").join(LOCAL_MODEL_ID);
                let onnx = dir.join(MODEL_ONNX);
                if !onnx.exists() {
                    return Err(format!(
                        "本地重排模型未下载（{}），请在记忆面板下载后再启用重排",
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
                let user_model =
                    UserDefinedRerankingModel::new(OnnxSource::File(onnx), tokenizer_files);
                // RerankInitOptionsUserDefined 为 #[non_exhaustive]，须用 Default + 可变字段
                let mut init = RerankInitOptionsUserDefined::default();
                init.max_length = MAX_LENGTH;
                TextRerank::try_new_from_user_defined(user_model, init)
                    .map_err(|e| format!("本地重排模型加载失败: {e}"))
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

#[async_trait::async_trait]
impl Reranker for LocalReranker {
    fn id(&self) -> &str {
        "local"
    }

    fn model_key(&self) -> String {
        LOCAL_MODEL_ID.into()
    }

    async fn rerank(
        &self,
        query: &str,
        docs: &[(String, String)],
    ) -> Result<Vec<(String, f32)>, String> {
        if docs.is_empty() {
            return Ok(Vec::new());
        }
        let m = Self::model()?;
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
        files_present(&self.app)
    }
}

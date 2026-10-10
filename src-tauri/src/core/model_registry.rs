//! Embedding/rerank model manifest registry and install-directory resolution.
//!
//! Moved down from commands/model_download.rs: plugins/embed needs the manifest lookup and
//! directory resolution, and plugins must not depend back on the command layer. Download/delete IPC logic stays in the command layer.

use std::path::PathBuf;
use tauri::{AppHandle, Manager};
/// A single remote file.
pub struct ManifestFile {
    pub url: &'static str,
    pub name: &'static str,
}

/// A manifest entry.
pub struct ManifestEntry {
    pub id: &'static str,
    /// embed | rerank
    pub kind: &'static str,
    pub label: &'static str,
    /// Vector dimensions (meaningless for rerank, always 0)
    pub dims: usize,
    /// Pooling mode: mean | cls (meaningless for rerank, always "none")
    pub pooling: &'static str,
    /// Bundled model (shipped with the app, no download needed)
    pub bundled: bool,
    pub files: &'static [ManifestFile],
}

/// Bundled embedding model id (shipped with the app, works out of the box).
pub const BUNDLED_EMBED_ID: &str = "all-MiniLM-L6-v2";

/// Build the onnx + tokenizer file list for an HF repo (works for export repos like Xenova/Qdrant/Alibaba-NLP).
macro_rules! hf_files {
    ($repo:expr, $onnx:expr) => {
        &[
            ManifestFile {
                url: concat!("https://huggingface.co/", $repo, "/resolve/main/", $onnx),
                name: $onnx,
            },
            ManifestFile {
                url: concat!(
                    "https://huggingface.co/",
                    $repo,
                    "/resolve/main/tokenizer.json"
                ),
                name: "tokenizer.json",
            },
            ManifestFile {
                url: concat!(
                    "https://huggingface.co/",
                    $repo,
                    "/resolve/main/config.json"
                ),
                name: "config.json",
            },
            ManifestFile {
                url: concat!(
                    "https://huggingface.co/",
                    $repo,
                    "/resolve/main/special_tokens_map.json"
                ),
                name: "special_tokens_map.json",
            },
            ManifestFile {
                url: concat!(
                    "https://huggingface.co/",
                    $repo,
                    "/resolve/main/tokenizer_config.json"
                ),
                name: "tokenizer_config.json",
            },
        ]
    };
}

/// Model manifest.
pub const MANIFEST: &[ManifestEntry] = &[
    // ---- Local rerank models (download dir fixed at <app_data_dir>/models/rerank/<id>/) ----
    ManifestEntry {
        id: "bge-reranker-base",
        kind: "rerank",
        label: "bge-reranker-base（中英双语交叉编码器重排，int8 约 280MB）",
        dims: 0,
        pooling: "none",
        bundled: false,
        files: &[
            ManifestFile {
                url: "https://huggingface.co/Xenova/bge-reranker-base/resolve/main/onnx/model_int8.onnx",
                name: "onnx/model_int8.onnx",
            },
            ManifestFile {
                url: "https://huggingface.co/Xenova/bge-reranker-base/resolve/main/tokenizer.json",
                name: "tokenizer.json",
            },
            ManifestFile {
                url: "https://huggingface.co/Xenova/bge-reranker-base/resolve/main/config.json",
                name: "config.json",
            },
            ManifestFile {
                url: "https://huggingface.co/Xenova/bge-reranker-base/resolve/main/special_tokens_map.json",
                name: "special_tokens_map.json",
            },
            ManifestFile {
                url: "https://huggingface.co/Xenova/bge-reranker-base/resolve/main/tokenizer_config.json",
                name: "tokenizer_config.json",
            },
        ],
    },
    ManifestEntry {
        id: "bge-reranker-large",
        kind: "rerank",
        label: "bge-reranker-large（同家族加大版，精度更高、延迟约 2-4 倍，int8 约 560MB）",
        dims: 0,
        pooling: "none",
        bundled: false,
        files: hf_files!("Xenova/bge-reranker-large", "onnx/model_int8.onnx"),
    },
    ManifestEntry {
        id: "bge-reranker-v2-m3",
        kind: "rerank",
        label: "bge-reranker-v2-m3（BGE-M3 底座多语言旗舰，中文效果最佳，int8 约 570MB）",
        dims: 0,
        pooling: "none",
        bundled: false,
        files: hf_files!("onnx-community/bge-reranker-v2-m3-ONNX", "onnx/model_int8.onnx"),
    },
    ManifestEntry {
        id: "mxbai-rerank-base-v1",
        kind: "rerank",
        label: "mxbai-rerank-base-v1（bge-reranker-base 再训练版，与 base 同级，量化约 240MB）",
        dims: 0,
        pooling: "none",
        bundled: false,
        files: hf_files!("mixedbread-ai/mxbai-rerank-base-v1", "onnx/model_quantized.onnx"),
    },
    // ---- Local embedding models (install dir customizable in settings, default <app_data_dir>/models/embed/<id>/) ----
    ManifestEntry {
        id: BUNDLED_EMBED_ID,
        kind: "embed",
        label: "all-MiniLM-L6-v2（内置开箱即用，384 维，英文/多语言通用）",
        dims: 384,
        pooling: "mean",
        bundled: true,
        files: &[],
    },
    ManifestEntry {
        id: "bge-small-en-v1.5",
        kind: "embed",
        label: "bge-small-en-v1.5（英文小模型，384 维，量化约 34MB）",
        dims: 384,
        pooling: "cls",
        bundled: false,
        files: hf_files!("Xenova/bge-small-en-v1.5", "onnx/model_quantized.onnx"),
    },
    ManifestEntry {
        id: "bge-base-en-v1.5",
        kind: "embed",
        label: "bge-base-en-v1.5（英文，768 维，量化约 110MB）",
        dims: 768,
        pooling: "cls",
        bundled: false,
        files: hf_files!("Xenova/bge-base-en-v1.5", "onnx/model_quantized.onnx"),
    },
    ManifestEntry {
        id: "bge-large-en-v1.5",
        kind: "embed",
        label: "bge-large-en-v1.5（英文，1024 维，量化约 337MB）",
        dims: 1024,
        pooling: "cls",
        bundled: false,
        files: hf_files!("Xenova/bge-large-en-v1.5", "onnx/model_quantized.onnx"),
    },
    ManifestEntry {
        id: "bge-small-zh-v1.5",
        kind: "embed",
        label: "bge-small-zh-v1.5（中文小模型，512 维，量化约 24MB）",
        dims: 512,
        pooling: "cls",
        bundled: false,
        files: hf_files!("Xenova/bge-small-zh-v1.5", "onnx/model_quantized.onnx"),
    },
    ManifestEntry {
        id: "bge-large-zh-v1.5",
        kind: "embed",
        label: "bge-large-zh-v1.5（中文，1024 维，量化约 327MB）",
        dims: 1024,
        pooling: "cls",
        bundled: false,
        files: hf_files!("Xenova/bge-large-zh-v1.5", "onnx/model_quantized.onnx"),
    },
    ManifestEntry {
        id: "bge-m3",
        kind: "embed",
        label: "bge-m3（中英多语言旗舰，1024 维，Matryoshka 可裁剪，量化约 569MB）",
        dims: 1024,
        pooling: "cls",
        bundled: false,
        files: hf_files!("Xenova/bge-m3", "onnx/model_quantized.onnx"),
    },
    ManifestEntry {
        id: "multilingual-e5-small",
        kind: "embed",
        label: "multilingual-e5-small（多语言，384 维，量化约 118MB）",
        dims: 384,
        pooling: "mean",
        bundled: false,
        files: hf_files!("Xenova/multilingual-e5-small", "onnx/model_quantized.onnx"),
    },
    ManifestEntry {
        id: "multilingual-e5-base",
        kind: "embed",
        label: "multilingual-e5-base（多语言，768 维，量化约 278MB）",
        dims: 768,
        pooling: "mean",
        bundled: false,
        files: hf_files!("Xenova/multilingual-e5-base", "onnx/model_quantized.onnx"),
    },
    ManifestEntry {
        id: "paraphrase-multilingual-MiniLM-L12-v2",
        kind: "embed",
        label: "paraphrase-multilingual-MiniLM-L12-v2（多语言，384 维，量化约 118MB）",
        dims: 384,
        pooling: "mean",
        bundled: false,
        files: hf_files!("Xenova/paraphrase-multilingual-MiniLM-L12-v2", "onnx/model_quantized.onnx"),
    },
    ManifestEntry {
        id: "gte-base-en-v1.5",
        kind: "embed",
        label: "gte-base-en-v1.5（英文，768 维，量化约 146MB）",
        dims: 768,
        pooling: "cls",
        bundled: false,
        files: hf_files!("Alibaba-NLP/gte-base-en-v1.5", "onnx/model_quantized.onnx"),
    },
    ManifestEntry {
        id: "gte-large-en-v1.5",
        kind: "embed",
        label: "gte-large-en-v1.5（英文，1024 维，量化约 445MB）",
        dims: 1024,
        pooling: "cls",
        bundled: false,
        files: hf_files!("Alibaba-NLP/gte-large-en-v1.5", "onnx/model_quantized.onnx"),
    },
    ManifestEntry {
        id: "mxbai-embed-large-v1",
        kind: "embed",
        label: "mxbai-embed-large-v1（英文，1024 维，量化约 337MB）",
        dims: 1024,
        pooling: "cls",
        bundled: false,
        files: hf_files!("mixedbread-ai/mxbai-embed-large-v1", "onnx/model_quantized.onnx"),
    },
];

/// Look up a manifest entry by id.
pub fn find_entry(id: &str) -> Option<&'static ManifestEntry> {
    MANIFEST.iter().find(|e| e.id == id)
}

/// Root directory for rerank/general models (used by non-embed kinds).
fn models_root(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("models")
}

/// Default install directory for embed models: <app_data_dir>/models/embed.
pub fn default_embed_models_dir(app: &AppHandle) -> PathBuf {
    models_root(app).join("embed")
}

/// Actual install directory for embed models (customizable in settings; empty config falls back to the default).
pub fn embed_models_dir(app: &AppHandle) -> PathBuf {
    let cfg = crate::core::config::load_app_config(app).unwrap_or_default();
    let dir = cfg.embedding_models_dir.trim();
    if dir.is_empty() {
        default_embed_models_dir(app)
    } else {
        PathBuf::from(dir)
    }
}

pub(crate) fn entry_dir(app: &AppHandle, entry: &ManifestEntry) -> PathBuf {
    if entry.kind == "embed" {
        embed_models_dir(app).join(entry.id)
    } else {
        models_root(app).join(entry.kind).join(entry.id)
    }
}

/// Whether the model is globally enabled (applies to embed and rerank alike; absent entries count as enabled).
pub fn is_model_enabled(app: &AppHandle, id: &str) -> bool {
    let cfg = crate::core::config::load_app_config(app).unwrap_or_default();
    match cfg.embedding_models.iter().find(|s| s.id == id) {
        Some(s) => s.enabled,
        None => true,
    }
}

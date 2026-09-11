//! 嵌入/重排模型 manifest 注册表与安装目录解析。
//!
//! 从 commands/model_download.rs 下沉：plugins/embed 需要查表与目录解析，
//! 而插件不应反向依赖命令层。下载/删除等 IPC 逻辑仍留在命令层。

use std::path::PathBuf;
use tauri::{AppHandle, Manager};
/// 单个远端文件。
pub struct ManifestFile {
    pub url: &'static str,
    pub name: &'static str,
}

/// manifest 条目。
pub struct ManifestEntry {
    pub id: &'static str,
    /// embed | rerank
    pub kind: &'static str,
    pub label: &'static str,
    /// 向量维度（rerank 无意义，恒 0）
    pub dims: usize,
    /// 池化方式：mean | cls（rerank 无意义，恒 "none"）
    pub pooling: &'static str,
    /// 内置模型（随应用打包，无需下载）
    pub bundled: bool,
    pub files: &'static [ManifestFile],
}

/// 内置嵌入模型 id（随应用打包，开箱即用）。
pub const BUNDLED_EMBED_ID: &str = "all-MiniLM-L6-v2";

/// 生成 HF 仓库的 onnx + tokenizer 文件清单（Xenova/Qdrant/Alibaba-NLP 等导出仓库通用）。
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

/// 模型清单。
pub const MANIFEST: &[ManifestEntry] = &[
    // ---- 本地重排（下载目录固定 <app_data_dir>/models/rerank/<id>/）----
    ManifestEntry {
        id: "bge-reranker-base",
        kind: "rerank",
        label: "bge-reranker-base（中英双语交叉编码器重排，int8 约 110MB）",
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
    // ---- 本地嵌入模型（安装目录可在设置页自定义，默认 <app_data_dir>/models/embed/<id>/）----
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

/// 按 id 查 manifest 条目。
pub fn find_entry(id: &str) -> Option<&'static ManifestEntry> {
    MANIFEST.iter().find(|e| e.id == id)
}

/// 重排/通用模型根目录（非 embed kind 使用）。
fn models_root(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("models")
}

/// embed 模型默认安装目录：<app_data_dir>/models/embed。
pub fn default_embed_models_dir(app: &AppHandle) -> PathBuf {
    models_root(app).join("embed")
}

/// embed 模型实际安装目录（设置页可自定义；空配置回退默认目录）。
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

/// 全局是否启用该嵌入模型（缺省视为启用；只对 kind=embed 有意义）。
pub fn is_embed_model_enabled(app: &AppHandle, id: &str) -> bool {
    let cfg = crate::core::config::load_app_config(app).unwrap_or_default();
    match cfg.embedding_models.iter().find(|s| s.id == id) {
        Some(s) => s.enabled,
        None => true,
    }
}

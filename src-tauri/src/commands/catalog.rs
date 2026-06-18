//! # 模型目录加载模块
//!
//! 从应用打包资源中读取由 `@aio/models-data` 包生成的 `models.json`，
//! 并返回原始 JSON 字符串供前端使用。
//!
//! ## 工作机制
//! 1. `aio-models-data` 仓库每周日 04:00 UTC 自动从 models.dev 同步数据。
//! 2. AIO 升级时通过 npm 依赖拉取新版数据，CI 打包时把 `dist/data/models.json`
//!    复制到应用 resource 目录。
//! 3. 启动时前端 invoke 本命令，Rust 通过 `app.path().resolve_resource` 定位文件
//!    并返回内容。

use std::fs;
use std::path::PathBuf;
use tauri::Manager;

/// 加载应用打包的模型目录 JSON（原始字符串）
///
/// ## 行为
/// - 尝试从打包资源目录 (`app.path().resource_dir()`) 读取 `models.json`。
/// - 找不到（开发模式没打包资源）时回退到 `node_modules/@aio/models-data/dist/data/models.json`。
/// - 两次都失败时返回空 catalog JSON，前端不报错。
#[tauri::command]
pub fn load_models_catalog(app: tauri::AppHandle) -> Result<String, String> {
    if let Ok(resource_dir) = app.path().resource_dir() {
        let candidate = resource_dir.join("models.json");
        if candidate.exists() {
            return fs::read_to_string(&candidate).map_err(|e| {
                format!(
                    "读取打包资源失败 ({}): {}",
                    candidate.display(),
                    e
                )
            });
        }
    }

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let candidates: [PathBuf; 2] = [
        PathBuf::from(manifest_dir).join("../node_modules/@aio/models-data/dist/data/models.json"),
        PathBuf::from(manifest_dir).join("node_modules/@aio/models-data/dist/data/models.json"),
    ];
    for p in &candidates {
        if let Ok(content) = fs::read_to_string(p) {
            return Ok(content);
        }
    }

    Ok(r#"{
        "version": "0.0.0",
        "generatedAt": "1970-01-01T00:00:00.000Z",
        "source": "",
        "providerCount": 0,
        "modelCount": 0,
        "providers": [],
        "models": []
    }"#
    .to_string())
}

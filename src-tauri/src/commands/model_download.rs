//! 按需模型下载器（通用）：manifest 描述远端文件，下载到对应安装目录，
//! 经 `model-download-progress` 事件推送进度。
//!
//! 当前条目：
//! - kind=embed：主流开源本地嵌入模型（ONNX，来自 HF 的 Xenova / Qdrant / Alibaba-NLP 等
//!   导出仓库，统一取 `onnx/model_quantized.onnx` 自包含量化权重，CPU 推理快、体积小）；
//!   安装目录可由用户在设置页自定义（默认 `<app_data_dir>/models/embed/<id>/`）。
//! - kind=rerank：本地重排模型（bge-reranker-base，目录固定 `<app_data_dir>/models/rerank/<id>/`）。
//!
//! 内置模型 all-MiniLM-L6-v2（kind=embed，bundled=true）随应用打包（include_bytes），
//! 无需下载；列表中恒为已安装，删除为 no-op。

use crate::core::events as ev;
use crate::core::models::EmbeddingModelSetting;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Window};
use tokio::io::AsyncWriteExt;

// manifest 注册表与目录解析已下沉到 core::model_registry（插件层需要，命令层仅再导出）。
pub(crate) use crate::core::model_registry::entry_dir;
pub use crate::core::model_registry::*;

/// 模型信息（前端展示：id/kind/名称/维度/是否内置/是否启用/文件/是否已下载/占用空间）。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub dims: usize,
    pub pooled: String,
    pub bundled: bool,
    pub enabled: bool,
    pub files: Vec<ModelFileInfo>,
    pub downloaded: bool,
    pub size_bytes: u64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ModelFileInfo {
    pub name: String,
    pub downloaded: bool,
}

fn model_info(app: &AppHandle, entry: &'static ManifestEntry) -> ModelInfo {
    let dir = entry_dir(app, entry);
    let files = entry
        .files
        .iter()
        .map(|f| ModelFileInfo {
            name: f.name.to_string(),
            downloaded: dir.join(f.name).exists(),
        })
        .collect::<Vec<_>>();
    let size_bytes = files
        .iter()
        .filter(|f| f.downloaded)
        .map(|f| {
            std::fs::metadata(dir.join(&f.name))
                .map(|m| m.len())
                .unwrap_or(0)
        })
        .sum::<u64>();
    ModelInfo {
        id: entry.id.to_string(),
        kind: entry.kind.to_string(),
        label: entry.label.to_string(),
        dims: entry.dims,
        pooled: entry.pooling.to_string(),
        bundled: entry.bundled,
        enabled: if entry.kind == "embed" {
            is_embed_model_enabled(app, entry.id)
        } else {
            true
        },
        downloaded: entry.bundled || files.iter().all(|f| f.downloaded),
        files,
        size_bytes,
    }
}

/// 列出可下载/已安装模型及其状态（本地重排 + 本地嵌入模型）。
#[tauri::command]
pub fn embedding_model_list(app: AppHandle) -> Result<Vec<ModelInfo>, String> {
    Ok(MANIFEST.iter().map(|e| model_info(&app, e)).collect())
}

/// 查询单个模型下载/启用状态（含文件级明细）。
#[tauri::command]
pub fn embedding_model_status(app: AppHandle, id: String) -> Result<Option<ModelInfo>, String> {
    Ok(find_entry(&id).map(|e| model_info(&app, e)))
}

/// 当前 embed 模型安装目录（设置页展示用）。
#[tauri::command]
pub fn embedding_models_get_dir(app: AppHandle) -> Result<String, String> {
    Ok(embed_models_dir(&app).to_string_lossy().to_string())
}

/// 设置 embed 模型安装目录（持久化到应用配置；空串恢复默认目录）。
#[tauri::command]
pub fn embedding_models_set_dir(app: AppHandle, dir: String) -> Result<String, String> {
    let mut cfg =
        crate::core::config::load_app_config(&app).map_err(|e| format!("读取配置失败: {e}"))?;
    cfg.embedding_models_dir = dir.trim().to_string();
    crate::commands::config::save_app_config(app.clone(), cfg)
        .map_err(|e| format!("保存配置失败: {e}"))?;
    Ok(embed_models_dir(&app).to_string_lossy().to_string())
}

/// 全局启用/停用某个嵌入模型（与供应商设置类似；停用后所有项目不可用该模型）。
#[tauri::command]
pub fn embedding_model_set_enabled(
    app: AppHandle,
    id: String,
    enabled: bool,
) -> Result<ModelInfo, String> {
    let entry = find_entry(&id).ok_or_else(|| format!("未知模型: {id}"))?;
    if entry.kind != "embed" {
        return Err("仅嵌入模型支持全局启停".into());
    }
    let mut cfg =
        crate::core::config::load_app_config(&app).map_err(|e| format!("读取配置失败: {e}"))?;
    let mut list: Vec<EmbeddingModelSetting> = cfg
        .embedding_models
        .iter()
        .filter(|s| s.id != id)
        .cloned()
        .collect();
    list.push(EmbeddingModelSetting {
        id: id.clone(),
        enabled,
    });
    cfg.embedding_models = list;
    crate::commands::config::save_app_config(app.clone(), cfg)
        .map_err(|e| format!("保存配置失败: {e}"))?;
    Ok(model_info(&app, entry))
}

/// 下载模型（流式写盘 + model-download-progress 事件；已存在则跳过）。
/// 断点续传：同名 `.part` 残留会被覆盖重下（本轮不做字节级续传）。
#[tauri::command]
pub async fn embedding_model_download(
    window: Window,
    app: AppHandle,
    id: String,
) -> Result<ModelInfo, String> {
    let entry = find_entry(&id).ok_or_else(|| format!("未知模型: {id}"))?;
    if entry.bundled {
        return Ok(model_info(&app, entry));
    }
    let dir = entry_dir(&app, entry);
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建模型目录失败: {e}"))?;
    let client = reqwest::Client::new();
    for file in entry.files {
        let dest = dir.join(file.name);
        if dest.exists() {
            continue;
        }
        let resp = client
            .get(file.url)
            .send()
            .await
            .map_err(|e| format!("下载 {} 失败: {e}", file.name))?;
        if !resp.status().is_success() {
            let status = resp.status();
            return Err(format!("下载 {} 失败（{status}）", file.name));
        }
        let total = resp.content_length().unwrap_or(0);
        let mut stream = resp.bytes_stream();
        let tmp = dest.with_extension("part");
        let mut out = tokio::fs::File::create(&tmp)
            .await
            .map_err(|e| format!("创建临时文件失败: {e}"))?;
        let mut downloaded: u64 = 0;
        use futures_util::StreamExt as _;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("下载中断: {e}"))?;
            out.write_all(&chunk)
                .await
                .map_err(|e| format!("写入失败: {e}"))?;
            downloaded += chunk.len() as u64;
            let _ = window.emit(
                ev::MODEL_DOWNLOAD_PROGRESS,
                serde_json::json!({
                    "id": entry.id,
                    "file": file.name,
                    "downloaded": downloaded,
                    "total": total,
                }),
            );
        }
        out.flush().await.map_err(|e| format!("落盘失败: {e}"))?;
        drop(out);
        std::fs::rename(&tmp, &dest).map_err(|e| format!("重命名失败: {e}"))?;
    }
    let _ = window.emit(
        ev::MODEL_DOWNLOAD_PROGRESS,
        serde_json::json!({ "id": entry.id, "done": true }),
    );
    Ok(model_info(&app, entry))
}

/// 删除已下载模型（释放磁盘；内置模型为 no-op）。
#[tauri::command]
pub fn embedding_model_delete(app: AppHandle, id: String) -> Result<(), String> {
    let entry = find_entry(&id).ok_or_else(|| format!("未知模型: {id}"))?;
    if entry.bundled {
        return Ok(());
    }
    let dir = entry_dir(&app, entry);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("删除模型失败: {e}"))?;
    }
    Ok(())
}

//! 按需模型下载器（通用）：manifest 描述远端文件，下载到对应安装目录，
//! 经 `model-download-progress` 事件推送进度。
//!
//! 当前条目：
//! - kind=embed：主流开源本地嵌入模型（ONNX，来自 HF 的 Xenova / Qdrant / Alibaba-NLP 等
//!   导出仓库，统一取 `onnx/model_quantized.onnx` 自包含量化权重，CPU 推理快、体积小）；
//!   安装目录可由用户在设置页自定义（默认 `<app_data_dir>/models/embed/<id>/`）。
//! - kind=rerank：本地重排模型（bge 系列交叉编码器 + mxbai，目录固定 `<app_data_dir>/models/rerank/<id>/`）。
//!
//! 内置模型 all-MiniLM-L6-v2（kind=embed，bundled=true）随应用打包（include_bytes），
//! 无需下载；列表中恒为已安装，删除为 no-op。
//!
//! 下载控制：暂停（保留 `.part` 断点，续传走 HTTP Range）/ 取消（删除断点）。
//! 标志按模型 id 注册在进程级控制表，由独立命令置位，下载循环逐块检查。

use crate::core::events as ev;
use crate::core::models::EmbeddingModelSetting;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Window};
use tokio::io::AsyncWriteExt;

use crate::ierr;

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

/// 下载结果：status = ok（完成）| paused（已暂停，断点保留）| cancelled（已取消，断点已删）。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DownloadOutcome {
    pub status: String,
    pub info: ModelInfo,
}

/// 进行中下载的控制标志（按模型 id 注册）。
#[derive(Clone)]
struct DownloadControl {
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
}

fn download_controls() -> &'static Mutex<HashMap<String, DownloadControl>> {
    static CONTROLS: OnceLock<Mutex<HashMap<String, DownloadControl>>> = OnceLock::new();
    CONTROLS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 置位指定模型的下载标志；返回是否存在进行中的下载。
fn signal_download(id: &str, set: impl Fn(&DownloadControl)) -> bool {
    match download_controls().lock() {
        Ok(map) => match map.get(id) {
            Some(c) => {
                set(c);
                true
            }
            None => false,
        },
        Err(_) => false,
    }
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
        enabled: is_model_enabled(app, entry.id),
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

/// 查询单个模型下载/启用状态（含文件级明细）。`id`：manifest 条目 id。
#[tauri::command]
pub fn embedding_model_status(app: AppHandle, id: String) -> Result<Option<ModelInfo>, String> {
    Ok(find_entry(&id).map(|e| model_info(&app, e)))
}

/// 当前 embed 模型安装目录（设置页展示用）。
#[tauri::command]
pub fn embedding_models_get_dir(app: AppHandle) -> Result<String, String> {
    Ok(embed_models_dir(&app).to_string_lossy().to_string())
}

/// 设置 embed 模型安装目录（持久化到应用配置；`dir` 空串恢复默认目录）。
/// 修改目录后，原目录下已下载的 embed 模型会自动迁移到新目录（释放原目录空间；
/// 迁移为尽力而为，单项失败仅记录日志不阻塞目录切换）。
#[tauri::command]
pub fn embedding_models_set_dir(app: AppHandle, dir: String) -> Result<String, String> {
    let old = embed_models_dir(&app);
    let mut cfg = crate::core::config::load_app_config(&app)
        .map_err(|e| ierr!("读取配置失败: {}", "Failed to read config: {}", e))?;
    cfg.embedding_models_dir = dir.trim().to_string();
    crate::commands::config::save_app_config(app.clone(), cfg)
        .map_err(|e| ierr!("保存配置失败: {}", "Failed to save config: {}", e))?;
    let new = embed_models_dir(&app);
    if !same_path(&old, &new) {
        migrate_embed_models(&old, &new);
    }
    Ok(new.to_string_lossy().to_string())
}

/// 全局启用/停用某个模型（嵌入/重排通用；停用后所有项目不可用该模型）。
#[tauri::command]
pub fn embedding_model_set_enabled(
    app: AppHandle,
    id: String,
    enabled: bool,
) -> Result<ModelInfo, String> {
    let entry = find_entry(&id).ok_or_else(|| ierr!("未知模型: {}", "Unknown model: {}", id))?;
    let mut cfg = crate::core::config::load_app_config(&app)
        .map_err(|e| ierr!("读取配置失败: {}", "Failed to read config: {}", e))?;
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
        .map_err(|e| ierr!("保存配置失败: {}", "Failed to save config: {}", e))?;
    Ok(model_info(&app, entry))
}

/// 下载模型（流式写盘 + model-download-progress 事件；已存在的文件跳过）。
/// `id`：manifest 条目 id。返回 ok / paused / cancelled 三种结局：
/// - 暂停：保留 `.part` 断点，再次调用时带 Range 续传；
/// - 取消：删除 `.part` 断点；
/// - 网络/写盘错误同样保留断点（下次续传）。
#[tauri::command]
pub async fn embedding_model_download(
    window: Window,
    app: AppHandle,
    id: String,
) -> Result<DownloadOutcome, String> {
    let entry = find_entry(&id).ok_or_else(|| ierr!("未知模型: {}", "Unknown model: {}", id))?;
    if entry.bundled {
        return Ok(DownloadOutcome {
            status: "ok".into(),
            info: model_info(&app, entry),
        });
    }
    // 注册控制标志（同 id 重复下载时覆盖旧标志）
    let control = DownloadControl {
        cancel: Arc::new(AtomicBool::new(false)),
        pause: Arc::new(AtomicBool::new(false)),
    };
    if let Ok(mut map) = download_controls().lock() {
        map.insert(id.clone(), control.clone());
    }
    // 命令结束（含被前端放弃）时注销控制标志
    struct UnregisterGuard(String);
    impl Drop for UnregisterGuard {
        fn drop(&mut self) {
            if let Ok(mut map) = download_controls().lock() {
                map.remove(&self.0);
            }
        }
    }
    let _guard = UnregisterGuard(id.clone());

    let dir = entry_dir(&app, entry);
    std::fs::create_dir_all(&dir).map_err(|e| {
        ierr!(
            "创建模型目录失败: {}",
            "Failed to create model directory: {}",
            e
        )
    })?;
    let client = reqwest::Client::new();
    for file in entry.files {
        let dest = dir.join(file.name);
        if dest.exists() {
            continue;
        }
        // manifest 文件名可含子目录（onnx/model_quantized.onnx），先建父目录
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                ierr!(
                    "创建模型目录失败: {}",
                    "Failed to create model directory: {}",
                    e
                )
            })?;
        }
        let tmp = dest.with_extension("part");
        // 断点续传：.part 残留时带 Range 从断点继续
        let mut resume_from = tokio::fs::metadata(&tmp)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        let mut resp = if resume_from > 0 {
            client
                .get(file.url)
                .header(reqwest::header::RANGE, format!("bytes={resume_from}-"))
                .send()
                .await
        } else {
            client.get(file.url).send().await
        }
        .map_err(|e| {
            ierr!(
                "下载 {} 失败: {}",
                "Failed to download {}: {}",
                file.name,
                e
            )
        })?;
        // .part 大于远端文件（损坏）→ 删除断点整档重下
        if resp.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            let _ = tokio::fs::remove_file(&tmp).await;
            resume_from = 0;
            resp = client.get(file.url).send().await.map_err(|e| {
                ierr!(
                    "下载 {} 失败: {}",
                    "Failed to download {}: {}",
                    file.name,
                    e
                )
            })?;
        }
        let partial = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        if !resp.status().is_success() {
            let status = resp.status();
            return Err(ierr!(
                "下载 {} 失败（{}）",
                "Failed to download {} ({})",
                file.name,
                status
            ));
        }
        if !partial {
            resume_from = 0; // 服务器忽略 Range → 整档重下
        }
        let total = resume_from + resp.content_length().unwrap_or(0);
        let mut stream = resp.bytes_stream();
        let mut out = if partial {
            tokio::fs::OpenOptions::new().append(true).open(&tmp).await
        } else {
            tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp)
                .await
        }
        .map_err(|e| {
            ierr!(
                "创建临时文件失败: {}",
                "Failed to create temporary file: {}",
                e
            )
        })?;
        let mut downloaded: u64 = resume_from;
        let mut interrupted: Option<&'static str> = None;
        let mut last_emit: Option<Instant> = None;
        use futures_util::StreamExt as _;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| ierr!("下载中断: {}", "Download interrupted: {}", e))?;
            out.write_all(&chunk)
                .await
                .map_err(|e| ierr!("写入失败: {}", "Write failed: {}", e))?;
            downloaded += chunk.len() as u64;
            // 进度事件节流（首个分块立即发一次）
            if last_emit.is_none_or(|t| t.elapsed() >= Duration::from_millis(200)) {
                last_emit = Some(Instant::now());
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
            if control.cancel.load(Ordering::Relaxed) {
                interrupted = Some("cancelled");
                break;
            }
            if control.pause.load(Ordering::Relaxed) {
                interrupted = Some("paused");
                break;
            }
        }
        out.flush()
            .await
            .map_err(|e| ierr!("落盘失败: {}", "Failed to flush to disk: {}", e))?;
        drop(out);
        match interrupted {
            Some("cancelled") => {
                let _ = tokio::fs::remove_file(&tmp).await;
                return Ok(DownloadOutcome {
                    status: "cancelled".into(),
                    info: model_info(&app, entry),
                });
            }
            Some("paused") => {
                return Ok(DownloadOutcome {
                    status: "paused".into(),
                    info: model_info(&app, entry),
                });
            }
            _ => {
                std::fs::rename(&tmp, &dest)
                    .map_err(|e| ierr!("重命名失败: {}", "Failed to rename: {}", e))?;
            }
        }
    }
    let _ = window.emit(
        ev::MODEL_DOWNLOAD_PROGRESS,
        serde_json::json!({ "id": entry.id, "done": true }),
    );
    Ok(DownloadOutcome {
        status: "ok".into(),
        info: model_info(&app, entry),
    })
}

/// 暂停正在进行的模型下载。`id`：manifest 条目 id。返回是否有进行中的下载。
#[tauri::command]
pub fn embedding_model_pause_download(id: String) -> bool {
    signal_download(&id, |c| c.pause.store(true, Ordering::Relaxed))
}

/// 取消模型下载。`id`：manifest 条目 id。返回是否有进行中的下载；
/// 无进行中的下载（如已暂停）时直接清理该模型的所有 `.part` 断点文件。
#[tauri::command]
pub fn embedding_model_cancel_download(app: AppHandle, id: String) -> Result<bool, String> {
    let active = signal_download(&id, |c| c.cancel.store(true, Ordering::Relaxed));
    if !active {
        if let Some(entry) = find_entry(&id) {
            let dir = entry_dir(&app, entry);
            if dir.is_dir() {
                for f in entry.files {
                    let _ = std::fs::remove_file(dir.join(f.name).with_extension("part"));
                }
            }
        }
    }
    Ok(active)
}

/// 删除已下载模型（释放磁盘 + 清理进程内已加载的实例缓存；内置模型为 no-op）。
#[tauri::command]
pub fn embedding_model_delete(app: AppHandle, id: String) -> Result<(), String> {
    let entry = find_entry(&id).ok_or_else(|| ierr!("未知模型: {}", "Unknown model: {}", id))?;
    if entry.bundled {
        return Ok(());
    }
    if entry.kind == "embed" {
        crate::plugins::embed::local_onnx::evict_model(&id);
    } else if entry.kind == "rerank" {
        crate::plugins::rerank::local_onnx::evict_model(&id);
    }
    let dir = entry_dir(&app, entry);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .map_err(|e| ierr!("删除模型失败: {}", "Failed to delete model: {}", e))?;
    }
    Ok(())
}

/// 规范化比较两个目录路径（存在时走 canonicalize，否则按字面比较）。
fn same_path(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(a) == canon(b)
}

/// 把 old 目录下已下载的 embed 模型迁移到 new 目录（rename 优先；跨盘回退递归复制）。
/// 尽力而为：目标已存在或单项失败仅记录日志并继续。
fn migrate_embed_models(old: &Path, new: &Path) {
    if !old.is_dir() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(new) {
        tracing::warn!("[models] 迁移目标目录创建失败（{}）: {e}", new.display());
        return;
    }
    for entry in MANIFEST.iter().filter(|e| e.kind == "embed" && !e.bundled) {
        let src = old.join(entry.id);
        if !src.is_dir() {
            continue;
        }
        let dst = new.join(entry.id);
        if dst.exists() {
            tracing::warn!(
                "[models] 迁移跳过 {}: 目标已存在 {}",
                entry.id,
                dst.display()
            );
            continue;
        }
        if std::fs::rename(&src, &dst).is_err() {
            // 跨盘 rename 失败 → 递归复制后删除源目录
            if let Err(e) = copy_dir_recursive(&src, &dst) {
                tracing::warn!("[models] 迁移 {} 失败: {e}", entry.id);
                continue;
            }
            let _ = std::fs::remove_dir_all(&src);
        }
        tracing::info!(
            "[models] 已迁移 embed 模型 {} → {}",
            entry.id,
            dst.display()
        );
    }
}

/// 递归复制目录（跨盘迁移回退路径）。
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for item in std::fs::read_dir(src)? {
        let item = item?;
        let from = item.path();
        let to = dst.join(item.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

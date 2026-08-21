//! 嵌入供给命令（embedding_*）：连接测试、状态、Ollama 模型列表 / 拉取 / 删除。

use crate::core::models::MemoryEmbeddingConfig;
use serde::Deserialize;
use tauri::{AppHandle, Emitter, Window};

#[derive(Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaTagsModel>,
}

#[derive(Deserialize)]
struct OllamaTagsModel {
    name: String,
    size: u64,
    digest: Option<String>,
}

/// Ollama 模型信息（embedding_list_ollama_models 返回）。
#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OllamaModelInfo {
    pub name: String,
    pub size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

#[derive(Deserialize)]
struct OllamaPullLine {
    status: String,
    #[serde(default)]
    digest: Option<String>,
    #[serde(default)]
    total: Option<u64>,
    #[serde(default)]
    completed: Option<u64>,
}

fn ollama_base(api_url: Option<String>) -> String {
    let base = api_url.unwrap_or_else(|| "http://127.0.0.1:11434".into());
    base.trim_end_matches('/').to_string()
}

/// 为 openai_compat 从 secure_store 注入 API Key。
fn inject_key(app: &AppHandle, cfg: &mut MemoryEmbeddingConfig) {
    if cfg.provider == "openai_compat" && cfg.api_key.is_empty() {
        if let Ok(Some(key)) = crate::core::secure_store::get(app, "embedding_api_key") {
            cfg.api_key = key;
        }
    }
}

/// 保存嵌入 API Key（secure_store，不落盘；空值删除）。
#[tauri::command]
pub fn embedding_save_api_key(app: AppHandle, key: String) -> Result<(), String> {
    if key.trim().is_empty() {
        crate::core::secure_store::delete(&app, "embedding_api_key").map_err(|e| e.to_string())
    } else {
        crate::core::secure_store::set(&app, "embedding_api_key", key.trim())
            .map_err(|e| e.to_string())
    }
}

/// 嵌入连接测试（返回维度与延迟）。
#[tauri::command]
pub async fn embedding_test(
    app: AppHandle,
    config: MemoryEmbeddingConfig,
) -> crate::plugins::embed::EmbeddingTestResult {
    let mut cfg = config;
    inject_key(&app, &mut cfg);
    crate::plugins::embed::test(&cfg).await
}

/// 当前嵌入配置与可用状态。
#[tauri::command]
pub fn embedding_status(app: AppHandle) -> Result<serde_json::Value, String> {
    let cfg = crate::commands::config::load_app_config(app.clone()).map_err(|e| e.to_string())?;
    let mut cfg2 = cfg.memory_embedding.clone();
    inject_key(&app, &mut cfg2);
    let (configured, available, dims) = match crate::plugins::embed::resolve(&cfg2) {
        Ok(e) => (true, e.is_available(), e.dimensions()),
        Err(_) => (cfg2.enabled, false, cfg2.dimensions),
    };
    Ok(serde_json::json!({
        "provider": cfg2.provider,
        "model": cfg2.model,
        "configured": configured,
        "available": available,
        "dimensions": dims,
        "enabled": cfg2.enabled,
    }))
}

/// 列出 Ollama 已安装模型。
#[tauri::command]
pub async fn embedding_list_ollama_models(
    api_url: Option<String>,
) -> Result<Vec<OllamaModelInfo>, String> {
    let base = ollama_base(api_url);
    let url = format!("{base}/api/tags");
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("无法连接 Ollama: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Ollama 返回异常状态: {}", resp.status()));
    }
    let parsed: OllamaTagsResponse = resp.json().await.map_err(|e| format!("解析失败: {e}"))?;
    Ok(parsed
        .models
        .into_iter()
        .map(|m| OllamaModelInfo {
            name: m.name,
            size_bytes: m.size,
            digest: m.digest,
        })
        .collect())
}

/// 后台拉取 Ollama 模型（流式进度经 embedding-download-progress 事件推送）。
#[tauri::command]
pub fn embedding_pull_ollama_model(
    window: Window,
    model: String,
    api_url: Option<String>,
) -> Result<String, String> {
    let base = ollama_base(api_url);
    tauri::async_runtime::spawn(async move {
        let url = format!("{base}/api/pull");
        let client = reqwest::Client::new();
        let body = serde_json::json!({ "model": model, "stream": true });
        let resp = match client.post(&url).json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                let _ = window.emit(
                    "embedding-download-progress",
                    serde_json::json!({ "provider": "ollama", "model": model, "status": "error", "error": format!("无法连接 Ollama: {e}") }),
                );
                return;
            }
        };
        use futures_util::StreamExt;
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let mut last_status = String::new();
        let mut last_completed: i64 = -1;
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    let _ = window.emit(
                        "embedding-download-progress",
                        serde_json::json!({ "provider": "ollama", "model": model, "status": "error", "error": format!("流式读取失败: {e}") }),
                    );
                    return;
                }
            };
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim().to_string();
                buf = buf[pos + 1..].to_string();
                if line.is_empty() {
                    continue;
                }
                if let Ok(pl) = serde_json::from_str::<OllamaPullLine>(&line) {
                    let completed = pl.completed.unwrap_or(0) as i64;
                    let total = pl.total.unwrap_or(0) as i64;
                    let progress_changed =
                        completed - last_completed >= 1_048_576 || pl.status != last_status;
                    if progress_changed || pl.status == "success" || pl.status == "error" {
                        let _ = window.emit(
                            "embedding-download-progress",
                            serde_json::json!({ "provider": "ollama", "model": model, "status": pl.status, "completed": completed, "total": total, "digest": pl.digest }),
                        );
                        last_status = pl.status.clone();
                        last_completed = completed;
                    }
                }
            }
        }
    });
    Ok("started".into())
}

/// 删除 Ollama 模型（释放磁盘）。
#[tauri::command]
pub async fn embedding_delete_ollama_model(
    model: String,
    api_url: Option<String>,
) -> Result<(), String> {
    let base = ollama_base(api_url);
    let url = format!("{base}/api/delete");
    let client = reqwest::Client::new();
    let resp = client
        .delete(&url)
        .json(&serde_json::json!({ "model": model }))
        .send()
        .await
        .map_err(|e| format!("无法连接 Ollama: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("删除失败: {}", resp.status()));
    }
    Ok(())
}

//! # 模型目录加载与更新模块
//!
//! 从应用打包资源中读取由 `@aio/models-data` 包生成的 `models.json`，
//! 并支持一键从远端拉取最新数据保存到 AppData 目录。
//!
//! ## 数据优先级
//! 1. **AppData 缓存**（`$APPDATA/com.loch.aio/models-catalog.json`，用户主动拉的最新版本）
//! 2. **打包资源**（`app.path().resource_dir()/models.json`，应用内置快照）
//! 3. **dev fallback**（`node_modules/@aio/models-data/dist/data/models.json`）
//! 4. **空 catalog**（兜底，前端不报错）
//!
//! ## 更新机制
//! `update_models_catalog` 从 GitHub raw URL 拉取最新 JSON，
//! 校验大小/可解析性后写入 AppData 目录。重启后自动生效。

use std::fs;
use std::path::PathBuf;
use serde::Serialize;
use tauri::{Manager, Emitter};

const DEFAULT_CATALOG_URL: &str =
    "https://raw.githubusercontent.com/Atom112/aio-models-data/main/dist/data/models.json";

const APPDATA_FILENAME: &str = "models-catalog.json";
const BUNDLE_FILENAME: &str = "models.json";
const NODE_MODULES_REL: &str = "node_modules/@aio/models-data/dist/data/models.json";

const EMPTY_CATALOG: &str = r#"{
    "version": "0.0.0",
    "generatedAt": "1970-01-01T00:00:00.000Z",
    "source": "",
    "providerCount": 0,
    "modelCount": 0,
    "providers": [],
    "models": []
}"#;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum CatalogSource {
    Appdata,
    Bundled,
    DevFallback,
    Empty,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CatalogResponse {
    pub source: CatalogSource,
    pub json: String,
    pub path: Option<String>,
    pub version: Option<String>,
    pub generated_at: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UpdateResult {
    pub success: bool,
    pub model_count: usize,
    pub provider_count: usize,
    pub version: String,
    pub cached_path: String,
    pub error: Option<String>,
    pub bytes: usize,
    pub elapsed_ms: u128,
}

fn parse_catalog_meta(json: &str) -> (Option<String>, Option<String>, usize, usize) {
    let v: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return (None, None, 0, 0),
    };
    let version = v.get("version").and_then(|x| x.as_str()).map(String::from);
    let generated_at = v.get("generatedAt").and_then(|x| x.as_str()).map(String::from);
    let model_count = v.get("modelCount").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
    let provider_count = v.get("providerCount").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
    (version, generated_at, model_count, provider_count)
}

fn appdata_catalog_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_data_dir().ok()?;
    if !dir.exists() {
        let _ = fs::create_dir_all(&dir);
    }
    Some(dir.join(APPDATA_FILENAME))
}

fn try_read(path: &PathBuf) -> Option<String> {
    if path.exists() {
        fs::read_to_string(path).ok()
    } else {
        None
    }
}

/// 加载当前生效的 catalog JSON（按 AppData > Bundled > Dev > Empty 优先级）
#[tauri::command]
pub fn load_models_catalog(app: tauri::AppHandle) -> Result<String, String> {
    let resp = load_models_catalog_full(app)?;
    Ok(resp.json)
}

/// 加载当前生效的 catalog，附带来源元信息
#[tauri::command]
pub fn load_models_catalog_full(app: tauri::AppHandle) -> Result<CatalogResponse, String> {
    if let Some(p) = appdata_catalog_path(&app) {
        if let Some(content) = try_read(&p) {
            let (v, g, _, _) = parse_catalog_meta(&content);
            return Ok(CatalogResponse {
                source: CatalogSource::Appdata,
                json: content,
                path: Some(p.display().to_string()),
                version: v,
                generated_at: g,
            });
        }
    }

    if let Ok(resource_dir) = app.path().resource_dir() {
        let p = resource_dir.join(BUNDLE_FILENAME);
        if let Some(content) = try_read(&p) {
            let (v, g, _, _) = parse_catalog_meta(&content);
            return Ok(CatalogResponse {
                source: CatalogSource::Bundled,
                json: content,
                path: Some(p.display().to_string()),
                version: v,
                generated_at: g,
            });
        }
    }

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    for rel in &["../".to_string() + NODE_MODULES_REL, NODE_MODULES_REL.to_string()] {
        let p = PathBuf::from(manifest_dir).join(rel);
        if let Some(content) = try_read(&p) {
            let (v, g, _, _) = parse_catalog_meta(&content);
            return Ok(CatalogResponse {
                source: CatalogSource::DevFallback,
                json: content,
                path: Some(p.display().to_string()),
                version: v,
                generated_at: g,
            });
        }
    }

    Ok(CatalogResponse {
        source: CatalogSource::Empty,
        json: EMPTY_CATALOG.to_string(),
        path: None,
        version: None,
        generated_at: None,
    })
}

/// 拉取最新 catalog 并保存到 AppData
#[tauri::command]
pub async fn update_models_catalog(
    app: tauri::AppHandle,
    url: Option<String>,
) -> Result<UpdateResult, String> {
    let target_url = url.unwrap_or_else(|| DEFAULT_CATALOG_URL.to_string());
    let started = std::time::Instant::now();

    let client = reqwest::Client::builder()
        .user_agent("AIO-Desktop/0.4 (aio-models-data-updater)")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("构造 HTTP 客户端失败: {}", e))?;

    let resp = client
        .get(&target_url)
        .send()
        .await
        .map_err(|e| format!("拉取失败: {}", e))?;

    if !resp.status().is_success() {
        return Ok(UpdateResult {
            success: false,
            model_count: 0,
            provider_count: 0,
            version: String::new(),
            cached_path: String::new(),
            error: Some(format!("HTTP {} {}", resp.status().as_u16(), resp.status().canonical_reason().unwrap_or(""))),
            bytes: 0,
            elapsed_ms: started.elapsed().as_millis(),
        });
    }

    let body = resp
        .text()
        .await
        .map_err(|e| format!("读取响应体失败: {}", e))?;

    if body.len() < 1024 {
        return Ok(UpdateResult {
            success: false,
            model_count: 0,
            provider_count: 0,
            version: String::new(),
            cached_path: String::new(),
            error: Some(format!("响应体过小 ({} 字节)，可能不是有效 catalog", body.len())),
            bytes: body.len(),
            elapsed_ms: started.elapsed().as_millis(),
        });
    }

    let (version, _generated_at, model_count, provider_count) = parse_catalog_meta(&body);
    if model_count == 0 {
        return Ok(UpdateResult {
            success: false,
            model_count: 0,
            provider_count: 0,
            version: version.clone().unwrap_or_default(),
            cached_path: String::new(),
            error: Some("响应体不包含 modelCount 字段，可能不是 catalog JSON".to_string()),
            bytes: body.len(),
            elapsed_ms: started.elapsed().as_millis(),
        });
    }

    let cached = appdata_catalog_path(&app)
        .ok_or_else(|| "无法获取 AppData 目录".to_string())?;

    fs::write(&cached, &body).map_err(|e| {
        format!(
            "写入 AppData 失败 ({}): {}",
            cached.display(),
            e
        )
    })?;

    let _ = app.emit(
        "models-catalog-updated",
        serde_json::json!({
            "version": version,
            "modelCount": model_count,
            "providerCount": provider_count,
            "path": cached.display().to_string(),
        }),
    );

    Ok(UpdateResult {
        success: true,
        model_count,
        provider_count,
        version: version.unwrap_or_else(|| "unknown".to_string()),
        cached_path: cached.display().to_string(),
        error: None,
        bytes: body.len(),
        elapsed_ms: started.elapsed().as_millis(),
    })
}

/// 获取当前 catalog URL 配置（供 UI 展示）
#[tauri::command]
pub fn get_catalog_url() -> String {
    DEFAULT_CATALOG_URL.to_string()
}

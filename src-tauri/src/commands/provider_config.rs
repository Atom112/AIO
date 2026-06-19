//! # Provider 配置模块 (lobehub v2 形态)
//!
//! 每个 provider 独立配置：开关 / API URL / Key / 启用的模型列表。
//! 持久化到 `$APPDATA/com.loch.aio/provider-configs.json`。
//!
//! ## 设计 (v2)
//! - 完整的 provider 列表来自前端 catalog (`@aio/models-data`)，后端不持有预设。
//! - `version=2` 配置文件与旧版本不兼容：加载时若 version 不匹配或解析失败，直接清空返回。
//! - 旧 `activated_models.json` / `config.json` 不再迁移（用户需重新配置）。
//! - 模型元数据（displayName / contextWindow / pricing / capabilities）由 catalog 提供，
//!   后端 `fetch_provider_models` 仅作为可选的"实时校验"通道。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use crate::core::models::LiveModel;
use crate::plugins::provider::{
    classify_reqwest_error, ProviderManager, TEST_TIMEOUT_SECS, DEFAULT_TIMEOUT_SECS,
};

const APPDATA_DIRNAME: &str = "com.loch.aio";
const PROVIDER_FILE: &str = "provider-configs.json";
const CURRENT_VERSION: u32 = 2;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    pub id: String,
    pub enabled: bool,
    pub display_name: String,
    pub api_url: String,
    pub api_key: String,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub is_custom: bool,
    #[serde(default)]
    pub custom_model_ids: Vec<String>,
    /// per-provider HTTP/HTTPS 代理 URL (例如 `http://127.0.0.1:7890`)。
    /// 旧配置无此字段时反序列化为 None，逻辑上视为"不代理"。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,
    /// 从 API 持久化拉取的模型列表（含 displayName/releasedAt）。
    /// 旧配置无此字段时反序列化为空数组。
    #[serde(default)]
    pub fetched_models: Vec<LiveModel>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfigFile {
    pub version: u32,
    pub updated_at: String,
    pub providers: BTreeMap<String, ProviderConfig>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TestConnectionResult {
    pub success: bool,
    pub model_count: usize,
    pub sample_model_ids: Vec<String>,
    pub error: Option<String>,
    pub elapsed_ms: u128,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FetchLiveModelsResult {
    pub success: bool,
    pub models: Vec<LiveModel>,
    pub error: Option<String>,
    pub elapsed_ms: u128,
}

fn config_dir() -> Option<PathBuf> {
    let dir = dirs::config_dir()?.join(APPDATA_DIRNAME);
    if !dir.exists() {
        let _ = fs::create_dir_all(&dir);
    }
    Some(dir)
}

fn provider_path() -> Option<PathBuf> {
    Some(config_dir()?.join(PROVIDER_FILE))
}

fn now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 简化：仅输出秒级时间戳
    format!("{}", secs)
}

/// 加载 provider 配置
/// - 文件存在且 version=CURRENT_VERSION: 正常返回
/// - 文件不存在 / 解析失败 / version 不匹配: 视为首次安装, 返回空 map
/// - 不会从旧 activated_models.json / config.json 迁移 (v2 设计)
#[tauri::command]
pub fn load_provider_configs() -> Result<ProviderConfigFile, String> {
    if let Some(p) = provider_path() {
        if let Ok(raw) = fs::read_to_string(&p) {
            match serde_json::from_str::<ProviderConfigFile>(&raw) {
                Ok(parsed) if parsed.version == CURRENT_VERSION => return Ok(parsed),
                Ok(_) => {
                    // 旧 version (1) 不兼容，直接删除
                    let _ = fs::remove_file(&p);
                }
                Err(_) => {
                    // 解析失败（旧 snake_case 格式等），删除
                    let _ = fs::remove_file(&p);
                }
            }
        }
    }

    // 首次：返回空 map，立即写回 v2 文件
    let file = ProviderConfigFile {
        version: CURRENT_VERSION,
        updated_at: now_iso(),
        providers: BTreeMap::new(),
    };
    let _ = save_provider_configs_internal(&file);
    Ok(file)
}

fn save_provider_configs_internal(file: &ProviderConfigFile) -> Result<(), String> {
    let p = provider_path().ok_or_else(|| "无法获取 config 目录".to_string())?;
    let json = serde_json::to_string_pretty(file).map_err(|e| e.to_string())?;
    fs::write(&p, json).map_err(|e| format!("写入失败: {}", e))?;
    Ok(())
}

/// 保存 provider 配置
#[tauri::command]
pub fn save_provider_configs(file: ProviderConfigFile) -> Result<(), String> {
    let mut f = file;
    f.updated_at = now_iso();
    save_provider_configs_internal(&f)
}

/// 测试 provider 连接（按 host 派发到对应 provider 插件）
#[tauri::command]
pub async fn test_provider_connection(
    api_url: String,
    api_key: String,
    proxy_url: Option<String>,
) -> Result<TestConnectionResult, String> {
    let started = std::time::Instant::now();
    let mgr = ProviderManager::new();
    let plugin = mgr.for_url(&api_url);

    let client = plugin.build_client(proxy_url.as_deref(), TEST_TIMEOUT_SECS)?;
    let url = plugin.models_url(&api_url);
    let req = plugin.apply_auth(client.get(&url), &api_key);

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            return Ok(TestConnectionResult {
                success: false,
                model_count: 0,
                sample_model_ids: vec![],
                error: Some(classify_reqwest_error(&e, &api_url)),
                elapsed_ms: started.elapsed().as_millis(),
            });
        }
    };

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let msg = if status == 401 || status == 403 {
            format!("HTTP {}: 认证失败，请检查 API Key 是否正确", status)
        } else if status == 404 {
            format!("HTTP 404: 端点不存在 ({})", plugin.identifier())
        } else {
            format!("HTTP {}", status)
        };
        return Ok(TestConnectionResult {
            success: false,
            model_count: 0,
            sample_model_ids: vec![],
            error: Some(msg),
            elapsed_ms: started.elapsed().as_millis(),
        });
    }

    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => {
            return Ok(TestConnectionResult {
                success: false,
                model_count: 0,
                sample_model_ids: vec![],
                error: Some(format!("读取响应失败: {}", e)),
                elapsed_ms: started.elapsed().as_millis(),
            });
        }
    };

    let v: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            return Ok(TestConnectionResult {
                success: false,
                model_count: 0,
                sample_model_ids: vec![],
                error: Some(format!("响应非 JSON: {}", e)),
                elapsed_ms: started.elapsed().as_millis(),
            });
        }
    };

    let models = plugin.parse_models(&v);
    let count = models.len();
    let samples: Vec<String> = models.iter().take(5).map(|m| m.id.clone()).collect();

    Ok(TestConnectionResult {
        success: true,
        model_count: count,
        sample_model_ids: samples,
        error: None,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

/// 从 provider 的 API 拉取模型列表（按 host 派发到对应 provider 插件）
#[tauri::command]
pub async fn fetch_provider_models(
    api_url: String,
    api_key: String,
    proxy_url: Option<String>,
) -> Result<FetchLiveModelsResult, String> {
    let started = std::time::Instant::now();
    let mgr = ProviderManager::new();
    let plugin = mgr.for_url(&api_url);

    let client = plugin.build_client(proxy_url.as_deref(), DEFAULT_TIMEOUT_SECS)?;
    let url = plugin.models_url(&api_url);
    let req = plugin.apply_auth(client.get(&url), &api_key);

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            return Ok(FetchLiveModelsResult {
                success: false,
                models: vec![],
                error: Some(classify_reqwest_error(&e, &api_url)),
                elapsed_ms: started.elapsed().as_millis(),
            });
        }
    };

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let msg = if status == 401 || status == 403 {
            format!("HTTP {}: 认证失败，请检查 API Key", status)
        } else if status == 404 {
            format!("HTTP 404: 端点不存在 ({})", plugin.identifier())
        } else {
            format!("HTTP {}", status)
        };
        return Ok(FetchLiveModelsResult {
            success: false,
            models: vec![],
            error: Some(msg),
            elapsed_ms: started.elapsed().as_millis(),
        });
    }

    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => {
            return Ok(FetchLiveModelsResult {
                success: false,
                models: vec![],
                error: Some(format!("读取响应失败: {}", e)),
                elapsed_ms: started.elapsed().as_millis(),
            });
        }
    };

    let v: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            return Ok(FetchLiveModelsResult {
                success: false,
                models: vec![],
                error: Some(format!("响应非 JSON: {}", e)),
                elapsed_ms: started.elapsed().as_millis(),
            });
        }
    };

    let models = plugin.parse_models(&v);

    Ok(FetchLiveModelsResult {
        success: true,
        models,
        error: None,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

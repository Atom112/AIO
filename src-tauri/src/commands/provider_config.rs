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
use tauri::AppHandle;

use crate::core::models::LiveModel;
use crate::core::secure_store;
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
    /// 兼容字段：旧配置可能含明文 api_key；新代码不再写盘，只从 keyring 读
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
    /// 标记 key 是否存放在 keyring（前端展示用）
    #[serde(default)]
    pub has_stored_key: bool,
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

/// 拉取 provider 的实际 api_key（从 keyring）
/// 给前端 chat 调用时拼 header 用，不直接暴露在配置对象中
#[tauri::command]
pub fn read_provider_api_key(app: AppHandle, provider_id: String) -> Result<String, String> {
    let key_name = secure_store::accounts::provider_key(&provider_id);
    secure_store::get(&app, &key_name)
        .map(|opt| opt.unwrap_or_default())
        .map_err(|e| e.to_string())
}

/// 显式删除某 provider 的 key
#[tauri::command]
pub fn delete_provider_api_key(app: AppHandle, provider_id: String) -> Result<(), String> {
    let key_name = secure_store::accounts::provider_key(&provider_id);
    secure_store::delete(&app, &key_name).map_err(|e| e.to_string())
}

/// 校验 API URL 协议合法（仅允许 http/https；M5 防护）
pub fn validate_api_url(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("URL 不能为空".into());
    }
    let parsed = url::Url::parse(trimmed).map_err(|e| format!("URL 解析失败: {}", e))?;
    match parsed.scheme() {
        "http" | "https" => {}
        s => return Err(format!("不支持的协议: {}（仅允许 http/https）", s)),
    }
    if parsed.host_str().is_none() {
        return Err("URL 必须包含 host".into());
    }
    Ok(parsed.to_string())
}

/// 加载 provider 配置
/// - 文件存在且 version=CURRENT_VERSION: 正常返回
/// - 文件不存在 / 解析失败 / version 不匹配: 视为首次安装, 返回空 map
/// - 不会从旧 activated_models.json / config.json 迁移 (v2 设计)
#[tauri::command]
pub fn load_provider_configs(app: AppHandle) -> Result<ProviderConfigFile, String> {
    if let Some(p) = provider_path() {
        if let Ok(raw) = fs::read_to_string(&p) {
            match serde_json::from_str::<ProviderConfigFile>(&raw) {
                Ok(mut parsed) if parsed.version == CURRENT_VERSION => {
                    // 还原每个 provider 的 api_key：优先 keyring，缺失时退回明文（旧配置）
                    for (_, cfg) in parsed.providers.iter_mut() {
                        let key_name = secure_store::accounts::provider_key(&cfg.id);
                        match secure_store::get(&app, &key_name) {
                            Ok(Some(v)) => {
                                cfg.api_key = v;
                                cfg.has_stored_key = true;
                            }
                            _ => {
                                if !cfg.api_key.is_empty() {
                                    // 旧配置含明文 → 迁出到 keyring 后清空
                                    let _ = secure_store::set(&app, &key_name, &cfg.api_key);
                                    cfg.has_stored_key = true;
                                }
                            }
                        }
                    }
                    return Ok(parsed);
                }
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
    let _ = save_provider_configs_internal(&app, &file);
    Ok(file)
}

fn save_provider_configs_internal(_app: &AppHandle, file: &ProviderConfigFile) -> Result<(), String> {
    let p = provider_path().ok_or_else(|| "无法获取 config 目录".to_string())?;
    // 在落盘前剥离 api_key（明文 key 一律只存 keyring）
    let mut sanitized = file.clone();
    for (id, cfg) in sanitized.providers.iter_mut() {
        if !cfg.api_key.is_empty() {
            // 同步到 keyring
            let key_name = secure_store::accounts::provider_key(id);
            let _ = secure_store::set(_app, &key_name, &cfg.api_key);
            cfg.api_key.clear();
            cfg.has_stored_key = true;
        }
    }
    let json = serde_json::to_string_pretty(&sanitized).map_err(|e| e.to_string())?;
    fs::write(&p, json).map_err(|e| format!("写入失败: {}", e))?;
    Ok(())
}

/// 保存 provider 配置
#[tauri::command]
pub fn save_provider_configs(app: AppHandle, file: ProviderConfigFile) -> Result<(), String> {
    let mut f = file;
    // M5 校验：所有 provider 的 api_url 必须是合法 http/https URL
    for (id, cfg) in f.providers.iter() {
        if !cfg.api_url.is_empty() {
            if let Err(e) = validate_api_url(&cfg.api_url) {
                return Err(format!("provider[{}].apiUrl 非法: {}", id, e));
            }
        }
        if let Some(proxy) = &cfg.proxy_url {
            if !proxy.is_empty() {
                if let Err(e) = validate_api_url(proxy) {
                    return Err(format!("provider[{}].proxyUrl 非法: {}", id, e));
                }
            }
        }
    }
    f.updated_at = now_iso();
    save_provider_configs_internal(&app, &f)
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

//! # Provider 配置模块 (lobehub 形态)
//!
//! 每个 provider 独立配置：开关 / API URL / Key / 启用的模型列表。
//! 持久化到 `$APPDATA/com.loch.aio/provider-configs.json`。
//!
//! ## 数据迁移
//! 首次启动时如果新文件不存在 + 旧 `activatedModels.json` 存在：
//! - 按 `api_url` 分组（相同 URL 视为同一 provider）
//! - 用 host 反查推断 provider id（OpenAI/Anthropic 等）
//! - 生成初始 ProviderConfig 列表
//!
//! ## 测试连接
//! 调 `fetch_models` 内部使用的同一套逻辑（GET {api_url}/models），
//! 验证返回 200 + JSON 数组。

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
const ACTIVATED_MODELS_FILE: &str = "activated_models.json";
const APP_CONFIG_FILE: &str = "config.json";

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
    #[serde(default)]
    pub legacy_activated_models: Option<Vec<LegacyActivatedModel>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LegacyActivatedModel {
    pub model_id: String,
    pub owned_by: String,
    pub api_url: String,
    pub api_key: String,
    #[serde(default)]
    pub local_path: Option<String>,
    #[serde(default)]
    pub engine_type: Option<String>,
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

fn activated_models_path() -> Option<PathBuf> {
    Some(config_dir()?.join(ACTIVATED_MODELS_FILE))
}

fn app_config_path() -> Option<PathBuf> {
    Some(config_dir()?.join(APP_CONFIG_FILE))
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

fn detect_provider_from_url(url: &str) -> Option<(&'static str, &'static str)> {
    let u = url.to_lowercase();
    let known: &[(&str, &str, &str)] = &[
        ("api.openai.com", "openai", "https://api.openai.com/v1"),
        ("api.anthropic.com", "anthropic", "https://api.anthropic.com"),
        ("generativelanguage.googleapis.com", "google", "https://generativelanguage.googleapis.com/v1beta"),
        ("api.deepseek.com", "deepseek", "https://api.deepseek.com/v1"),
        ("api.groq.com", "groq", "https://api.groq.com/openai/v1"),
        ("api.mistral.ai", "mistral", "https://api.mistral.ai/v1"),
        ("api.x.ai", "xai", "https://api.x.ai/v1"),
        ("api.cohere.ai", "cohere", "https://api.cohere.ai/v1"),
        ("openrouter.ai", "openrouter", "https://openrouter.ai/api/v1"),
    ];
    for (host, id, default_url) in known {
        if u.contains(host) {
            return Some((id, default_url));
        }
    }
    None
}

fn default_providers() -> BTreeMap<String, ProviderConfig> {
    let presets: Vec<(&str, &str, &str, Vec<&str>)> = vec![
        ("openai", "OpenAI", "https://api.openai.com/v1",
         vec!["gpt-4o", "gpt-4o-mini", "o3", "o4-mini"]),
        ("anthropic", "Anthropic", "https://api.anthropic.com",
         vec!["claude-sonnet-4-5", "claude-opus-4-1", "claude-haiku-4-5"]),
        ("google", "Google", "https://generativelanguage.googleapis.com/v1beta",
         vec!["gemini-2.5-pro", "gemini-2.5-flash"]),
        ("deepseek", "DeepSeek", "https://api.deepseek.com/v1",
         vec!["deepseek-v4-pro", "deepseek-v4-flash"]),
        ("groq", "Groq", "https://api.groq.com/openai/v1",
         vec!["llama-3.3-70b-versatile", "llama-3.1-8b-instant"]),
        ("mistral", "Mistral", "https://api.mistral.ai/v1",
         vec!["mistral-large-latest", "codestral-latest"]),
        ("xai", "xAI", "https://api.x.ai/v1",
         vec!["grok-4", "grok-4-fast"]),
        ("cohere", "Cohere", "https://api.cohere.ai/v1",
         vec!["command-a", "command-r-plus"]),
    ];
    let mut map = BTreeMap::new();
    for (id, name, url, models) in presets {
        map.insert(
            id.to_string(),
            ProviderConfig {
                id: id.to_string(),
                enabled: true,  // 默认启用：让用户立刻看到所有 preset 模型，再决定是否禁用/填 key
                display_name: name.to_string(),
                api_url: url.to_string(),
                api_key: String::new(),
                enabled_models: models.into_iter().map(String::from).collect(),
                is_custom: false,
                custom_model_ids: vec![],
                proxy_url: None,
                fetched_models: vec![],
            },
        );
    }
    map
}

fn migrate_from_activated_models(
    activated: &[LegacyActivatedModel],
    base: &mut BTreeMap<String, ProviderConfig>,
) {
    use std::collections::HashMap;
    let mut by_url: HashMap<String, Vec<&LegacyActivatedModel>> = HashMap::new();
    for m in activated {
        by_url.entry(m.api_url.clone()).or_default().push(m);
    }

    for (url, group) in by_url {
        if url.contains("127.0.0.1") || url.contains("localhost") {
            continue;
        }
        let (id, default_url) = match detect_provider_from_url(&url) {
            Some(v) => v,
            None => continue,
        };

        let entry = base.entry(id.to_string()).or_insert_with(|| ProviderConfig {
            id: id.to_string(),
            enabled: true,
            display_name: id.to_string(),
            api_url: default_url.to_string(),
            api_key: String::new(),
            enabled_models: vec![],
            is_custom: false,
            custom_model_ids: vec![],
            proxy_url: None,
            fetched_models: vec![],
        });
        entry.enabled = true;
        entry.api_url = if url.starts_with("http") { url.clone() } else { entry.api_url.clone() };
        // 取第一条非空 key
        if entry.api_key.is_empty() {
            if let Some(m) = group.iter().find(|m| !m.api_key.is_empty()) {
                entry.api_key = m.api_key.clone();
            }
        }
        // 合并模型 id
        let mut ids: Vec<String> = entry.enabled_models.clone();
        for m in &group {
            if !ids.contains(&m.model_id) {
                ids.push(m.model_id.clone());
            }
        }
        entry.enabled_models = ids;
    }
}

/// 加载 provider 配置（首次自动迁移）
#[tauri::command]
pub fn load_provider_configs() -> Result<ProviderConfigFile, String> {
    if let Some(p) = provider_path() {
        if let Ok(raw) = fs::read_to_string(&p) {
            if let Ok(parsed) = serde_json::from_str::<ProviderConfigFile>(&raw) {
                return Ok(parsed);
            }
            // 解析失败：可能是旧 snake_case 格式 (rename_all 之前版本)，直接删除重来
            let _ = fs::remove_file(&p);
        }
    }

    // 首次：尝试从旧 activated_models.json + config.json 迁移
    let mut providers = default_providers();

    if let Some(p) = app_config_path() {
        if let Ok(raw) = fs::read_to_string(&p) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(url) = v.get("apiUrl").and_then(|x| x.as_str()) {
                    if let Some((id, default_url)) = detect_provider_from_url(url) {
                        let entry = providers.entry(id.to_string()).or_insert_with(|| ProviderConfig {
                            id: id.to_string(),
                            enabled: false,
                            display_name: id.to_string(),
                            api_url: default_url.to_string(),
                            api_key: String::new(),
                            enabled_models: vec![],
                            is_custom: false,
                            custom_model_ids: vec![],
                            proxy_url: None,
                            fetched_models: vec![],
                        });
                        entry.enabled = true;
                        entry.api_url = url.to_string();
                    }
                }
                if let Some(key) = v.get("apiKey").and_then(|x| x.as_str()) {
                    if let Some((id, _)) = detect_provider_from_url(
                        v.get("apiUrl").and_then(|x| x.as_str()).unwrap_or(""),
                    ) {
                        if let Some(entry) = providers.get_mut(id) {
                            entry.api_key = key.to_string();
                        }
                    }
                }
            }
        }
    }

    let legacy: Option<Vec<LegacyActivatedModel>> = activated_models_path()
        .as_ref()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok());

    let legacy_clone = legacy.clone();
    if let Some(ref l) = legacy {
        migrate_from_activated_models(l, &mut providers);
    }

    let file = ProviderConfigFile {
        version: 1,
        updated_at: now_iso(),
        providers,
        legacy_activated_models: legacy_clone,
    };

    // 立即写回
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

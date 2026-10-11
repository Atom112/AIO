//! # Provider configuration module (LobeHub v2 shape)
//!
//! Each provider is configured independently: toggle / API URL / key / enabled model list.
//! Persisted to `$APPDATA/com.loch.aio/provider-configs.json`.
//!
//! ## Design (v2)
//! - The full provider list comes from the frontend catalog (`@aio/models-data`); the backend holds no presets.
//! - `version=2` config files are incompatible with older versions: on load, a version mismatch or parse failure clears the file and returns empty.
//! - Legacy `activated_models.json` / `config.json` are not migrated (users must reconfigure).
//! - Model metadata (displayName / contextWindow / pricing / capabilities) comes from the catalog;
//!   the backend `fetch_provider_models` is only an optional "live validation" channel.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use tauri::AppHandle;

use crate::core::models::LiveModel;
use crate::core::secure_store;
use crate::plugins::provider::{
    classify_reqwest_error, ProviderManager, DEFAULT_TIMEOUT_SECS, TEST_TIMEOUT_SECS,
};

use crate::ierr;

/// Compat constant: app config directory name (canonical definition lives in core::paths::BUNDLE_ID).
const APPDATA_DIRNAME: &str = crate::core::paths::BUNDLE_ID;
const PROVIDER_FILE: &str = "provider-configs.json";
const CURRENT_VERSION: u32 = 2;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    pub id: String,
    pub enabled: bool,
    pub display_name: String,
    pub api_url: String,
    /// Compatibility field: legacy configs may contain a plaintext api_key; new code never writes it to disk, reading only from the keyring
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
    /// Flags whether the key is stored in the keyring (for frontend display)
    #[serde(default)]
    pub has_stored_key: bool,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub is_custom: bool,
    #[serde(default)]
    pub custom_model_ids: Vec<String>,
    /// per-provider HTTP/HTTPS proxy URL (e.g. `http://127.0.0.1:7890`).
    /// Legacy configs without this field deserialize to None, treated as "no proxy".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,
    /// Model list persisted from the API fetch (includes displayName/releasedAt).
    /// Legacy configs without this field deserialize to an empty array.
    #[serde(default)]
    pub fetched_models: Vec<LiveModel>,
    /// Local-engine only: model file path mapping { modelId → localPath }.
    /// Legacy configs without this field deserialize to None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_model_paths: Option<std::collections::HashMap<String, String>>,
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

/// Current UNIX second timestamp as a string (legacy format: provider config updated_at stores second-level timestamps).
/// Naming note: this is not an ISO string; the implementation lives in utils::time.
fn now_unix_secs_str() -> String {
    crate::utils::time::now_unix_secs_string()
}

/// Fetch a provider's actual api_key (from the keyring)
/// Used by the frontend chat call to build the auth header; not exposed directly in the config object
#[tauri::command]
pub fn read_provider_api_key(app: AppHandle, provider_id: String) -> Result<String, String> {
    let key_name = secure_store::accounts::provider_key(&provider_id);
    secure_store::get(&app, &key_name)
        .map(|opt| opt.unwrap_or_default())
        .map_err(|e| e.to_string())
}

/// Explicitly delete a provider's key
#[tauri::command]
pub fn delete_provider_api_key(app: AppHandle, provider_id: String) -> Result<(), String> {
    let key_name = secure_store::accounts::provider_key(&provider_id);
    secure_store::delete(&app, &key_name).map_err(|e| e.to_string())
}

/// Validate API URL safety (SSRF protection).
///
/// Uses the shared URL validation module; intranet addresses and cloud metadata endpoints are rejected.
pub fn validate_api_url(input: &str) -> Result<String, String> {
    crate::utils::url_validation::validate_http_url(
        input,
        &crate::utils::url_validation::HttpUrlOptions::default(),
    )
}

/// Load provider configs
/// - File exists with version=CURRENT_VERSION: returned as-is
/// - File missing / parse failure / version mismatch: treated as first install, returns an empty map
/// - No migration from legacy activated_models.json / config.json (v2 design)
#[tauri::command]
pub fn load_provider_configs(app: AppHandle) -> Result<ProviderConfigFile, String> {
    if let Some(p) = provider_path() {
        if let Ok(raw) = fs::read_to_string(&p) {
            match serde_json::from_str::<ProviderConfigFile>(&raw) {
                Ok(mut parsed) if parsed.version == CURRENT_VERSION => {
                    // Restore each provider's api_key: keyring first, falling back to plaintext (legacy configs)
                    for cfg in parsed.providers.values_mut() {
                        let key_name = secure_store::accounts::provider_key(&cfg.id);
                        match secure_store::get(&app, &key_name) {
                            Ok(Some(v)) => {
                                cfg.api_key = v;
                                cfg.has_stored_key = true;
                            }
                            _ => {
                                if !cfg.api_key.is_empty() {
                                    // Legacy plaintext present → migrate to secure storage; mark has_stored_key only on success
                                    // (persisted plaintext is stripped conditionally by save_provider_configs_internal;
                                    //  on failure the plaintext is kept and migration retries on next load)
                                    if secure_store::set_reported(&app, &key_name, &cfg.api_key) {
                                        cfg.has_stored_key = true;
                                    }
                                }
                            }
                        }
                    }
                    return Ok(parsed);
                }
                Ok(_) => {
                    // Old version (1) is incompatible; delete outright (v1 content
                    // is never recoverable into v2 by renaming)
                    let _ = fs::remove_file(&p);
                }
                Err(_) => {
                    // Parse failure: quarantine as .corrupt.bak instead of deleting —
                    // a crash mid-write (pre-atomic-write files) must not turn into
                    // "all provider configs gone" (PROV-7).
                    let bak = p.with_extension("corrupt.bak");
                    match fs::rename(&p, &bak) {
                        Ok(()) => tracing::warn!(
                            "provider-configs.json 解析失败，已备份为 {}",
                            bak.display()
                        ),
                        Err(e) => tracing::warn!("provider-configs.json 解析失败，备份失败: {e}"),
                    }
                }
            }
        }
    }

    // First run: return an empty map and immediately write back a v2 file
    let file = ProviderConfigFile {
        version: CURRENT_VERSION,
        updated_at: now_unix_secs_str(),
        providers: BTreeMap::new(),
    };
    let _ = save_provider_configs_internal(&app, &file);
    Ok(file)
}

fn save_provider_configs_internal(
    _app: &AppHandle,
    file: &ProviderConfigFile,
) -> Result<(), String> {
    let p = provider_path()
        .ok_or_else(|| ierr!("无法获取 config 目录", "Failed to get config directory"))?;
    // Strip api_key before writing to disk (plaintext keys only ever live in keyring/the fallback encrypted file).
    // Clear the plaintext only when the secure store write succeeds; on failure keep it and record the error (never lose a key silently).
    let mut sanitized = file.clone();
    for (id, cfg) in sanitized.providers.iter_mut() {
        if !cfg.api_key.is_empty() {
            // Sync to secure storage
            let key_name = secure_store::accounts::provider_key(id);
            if secure_store::set_reported(_app, &key_name, &cfg.api_key) {
                cfg.api_key.clear();
                cfg.has_stored_key = true;
            }
        }
    }
    let json = serde_json::to_string_pretty(&sanitized).map_err(|e| e.to_string())?;
    // HOR-1：原子写（崩溃/掉电不再产生半写文件叠加 PROV-7 的删除路径丢全部供应商配置）
    crate::core::fs_util::atomic_write(&p, json.as_bytes())
        .map_err(|e| ierr!("写入失败: {}", "Write failed: {}", e))?;
    Ok(())
}

/// Save provider configs
#[tauri::command]
pub fn save_provider_configs(app: AppHandle, file: ProviderConfigFile) -> Result<(), String> {
    let mut f = file;
    // Known local-engine provider IDs
    const LOCAL_ENGINE_IDS: &[&str] = &["local-llamacpp", "local-vllm", "ollama"];

    // Validation: every provider's api_url must be a valid http/https URL
    for (id, cfg) in f.providers.iter() {
        let opts = if LOCAL_ENGINE_IDS.contains(&id.as_str()) {
            crate::utils::url_validation::HttpUrlOptions::local_engine()
        } else {
            crate::utils::url_validation::HttpUrlOptions::default()
        };
        if !cfg.api_url.is_empty() {
            if let Err(e) = crate::utils::url_validation::validate_http_url(&cfg.api_url, &opts) {
                return Err(ierr!(
                    "provider[{}].apiUrl 非法: {}",
                    "provider[{}].apiUrl is invalid: {}",
                    id,
                    e
                ));
            }
        }
        if let Some(proxy) = &cfg.proxy_url {
            if !proxy.is_empty() {
                if let Err(e) = validate_api_url(proxy) {
                    return Err(ierr!(
                        "provider[{}].proxyUrl 非法: {}",
                        "provider[{}].proxyUrl is invalid: {}",
                        id,
                        e
                    ));
                }
            }
        }
    }
    f.updated_at = now_unix_secs_str();
    save_provider_configs_internal(&app, &f)
}

/// Test a provider connection (dispatches to the matching provider plugin by host; local engines use the OpenAI-compat endpoint)
#[tauri::command]
pub async fn test_provider_connection(
    api_url: String,
    api_key: String,
    proxy_url: Option<String>,
    engine_type: Option<String>,
) -> Result<TestConnectionResult, String> {
    let started = std::time::Instant::now();

    // local engine (llama_cpp / vllm) → bypass host matching and go straight to OpenAI-compat
    let use_openai_compat =
        engine_type.as_deref() == Some("llama_cpp") || engine_type.as_deref() == Some("vllm");

    // Plugin selection is centralized: ProviderManager::for_url owns it (new plugins need no command-layer changes).
    // 阶段4 第5项: Arc registry — the plugin outlives the manager without the historic boxed_for_url rebuild list.
    let plugin: std::sync::Arc<dyn crate::plugins::provider::ProviderPlugin> = if use_openai_compat
    {
        std::sync::Arc::new(crate::plugins::provider::openai_compat::OpenAICompatProvider)
    } else {
        ProviderManager::new().for_url(&api_url)
    };

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
            ierr!(
                "HTTP {}: 认证失败，请检查 API Key 是否正确",
                "HTTP {}: authentication failed, please check that the API key is correct",
                status
            )
        } else if status == 404 {
            ierr!(
                "HTTP 404: 端点不存在 ({})",
                "HTTP 404: endpoint not found ({})",
                plugin.identifier()
            )
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
                error: Some(ierr!("读取响应失败: {}", "Failed to read response: {}", e)),
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
                error: Some(ierr!("响应非 JSON: {}", "Response is not JSON: {}", e)),
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

/// Fetch the model list from a provider's API (dispatches to the matching provider plugin by host; local engines use the OpenAI-compat endpoint)
#[tauri::command]
pub async fn fetch_provider_models(
    api_url: String,
    api_key: String,
    proxy_url: Option<String>,
    engine_type: Option<String>,
) -> Result<FetchLiveModelsResult, String> {
    let started = std::time::Instant::now();

    let use_openai_compat =
        engine_type.as_deref() == Some("llama_cpp") || engine_type.as_deref() == Some("vllm");

    // Plugin selection is centralized: ProviderManager::for_url owns it (new plugins need no command-layer changes).
    // 阶段4 第5项: Arc registry — the plugin outlives the manager without the historic boxed_for_url rebuild list.
    let plugin: std::sync::Arc<dyn crate::plugins::provider::ProviderPlugin> = if use_openai_compat
    {
        std::sync::Arc::new(crate::plugins::provider::openai_compat::OpenAICompatProvider)
    } else {
        ProviderManager::new().for_url(&api_url)
    };

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
            ierr!(
                "HTTP {}: 认证失败，请检查 API Key",
                "HTTP {}: authentication failed, please check the API key",
                status
            )
        } else if status == 404 {
            ierr!(
                "HTTP 404: 端点不存在 ({})",
                "HTTP 404: endpoint not found ({})",
                plugin.identifier()
            )
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
                error: Some(ierr!("读取响应失败: {}", "Failed to read response: {}", e)),
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
                error: Some(ierr!("响应非 JSON: {}", "Response is not JSON: {}", e)),
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

/// Probe local engine health (llama.cpp / vLLM / Ollama).
///
/// - llama.cpp / vLLM: use the `/health` endpoint (lighter than `/v1/models`).
/// - Ollama: use the `/api/version` endpoint (Ollama has no /health).
/// - Request timeouts: 3s connect + 5s full response; failures uniformly return false, no error thrown.
#[tauri::command]
pub async fn probe_engine_health(
    api_url: String,
    engine_type: Option<String>,
) -> Result<bool, String> {
    let base = api_url.trim_end_matches('/');

    let health_url = if engine_type.as_deref() == Some("ollama") {
        format!("{}/api/version", base)
    } else {
        // Strip /v1, /v1beta, /models suffixes, then append /health
        let stripped = base
            .trim_end_matches("/v1beta")
            .trim_end_matches("/v1")
            .trim_end_matches("/models");
        format!("{}/health", stripped)
    };

    let client = reqwest::Client::builder()
        .user_agent(format!("AIO-Desktop/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(3))
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    match client.get(&health_url).send().await {
        Ok(r) => Ok(r.status().is_success()),
        Err(_) => Ok(false),
    }
}

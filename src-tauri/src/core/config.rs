//! App config persistence - authoritative implementation of AppConfigDisk (disk schema) and load/save.
//!
//! Moved down from commands/config.rs so services/plugins depend on core only, never on the command layer.
//!
//! Compatibility surface: config.json disk field names (mixed snake/camel) and the lazy
//! migration semantics of the legacy plaintext key stay unchanged; the Tauri commands save_app_config/load_app_config are thin wrappers.

use crate::core::models::*;
use crate::core::secure_store;
use std::fs;
use tauri::AppHandle;
/// Persisted app config file schema: api_key is never written to disk, it always goes to the OS keychain
#[derive(serde::Serialize, serde::Deserialize)]
struct AppConfigDisk {
    api_url: String,
    default_model: String,
    local_model_path: String,
    #[serde(default, rename = "autoStartEnabled")]
    auto_start_enabled: bool,
    #[serde(default = "default_max_tool_rounds", rename = "maxToolRounds")]
    max_tool_rounds: i32,
    #[serde(
        default = "default_max_concurrent_subagents",
        rename = "maxConcurrentSubagents"
    )]
    max_concurrent_subagents: i32,
    /// Whether read-only tools within the same response block run concurrently
    #[serde(
        default = "crate::core::models::default_true",
        rename = "parallelToolExecution"
    )]
    parallel_tool_execution: bool,
    /// Retry count for the pre-flight stage of the main model call
    #[serde(
        default = "crate::core::models::default_model_call_retries",
        rename = "modelCallRetries"
    )]
    model_call_retries: u32,
    /// Whether autonomous-mode command execution is refused when the sandbox is unavailable (fail-closed)
    #[serde(
        default = "crate::core::models::default_true",
        rename = "sandboxRequired"
    )]
    sandbox_required: bool,
    /// Windows command sandbox mode (appcontainer default / off)
    #[serde(
        default = "crate::core::models::default_sandbox_windows",
        rename = "sandboxWindows"
    )]
    sandbox_windows: String,
    /// Windows AppContainer toolchain R+X directories
    #[serde(default, rename = "sandboxWindowsToolchainDirs")]
    sandbox_windows_toolchain_dirs: Vec<String>,
    /// Sandbox-internal `<root>/.git` deny-write (Codex-style history protection, default true)
    #[serde(
        default = "crate::core::models::default_true",
        rename = "sandboxProtectGit"
    )]
    sandbox_protect_git: bool,
    /// Full-permission sandbox self-heal may fall back to one unsandboxed retry
    /// (SEC 审查修复：默认 false——安全默认，与 AppConfig::default、serde 缺字段、
    /// 字段文档承诺四源一致；磁盘文件缺该字段时同样落 false，不再 default_true)
    #[serde(default, rename = "sandboxUnsandboxedFallback")]
    sandbox_unsandboxed_fallback: bool,
    /// Consecutive env-class failures before a tool is disabled for the turn (0 = default 6)
    /// Windows shell selection (powershell default / cmd)
    #[serde(
        default = "crate::core::models::default_shell_windows",
        rename = "shellWindows"
    )]
    shell_windows: String,
    #[serde(default, rename = "toolBreakerThreshold")]
    tool_breaker_threshold: u32,
    #[serde(default, rename = "memoryEnabled")]
    memory_enabled: bool,
    #[serde(
        default = "crate::core::models::default_true",
        rename = "memoryAutoInject"
    )]
    memory_auto_inject: bool,
    #[serde(
        default = "crate::core::models::default_true",
        rename = "memoryAutoExtract"
    )]
    memory_auto_extract: bool,
    #[serde(
        default = "crate::core::models::default_max_facts",
        rename = "memoryMaxFacts"
    )]
    memory_max_facts: u32,
    #[serde(
        default = "crate::core::models::default_inject_budget",
        rename = "memoryInjectionBudgetTokens"
    )]
    memory_injection_budget_tokens: u32,
    #[serde(
        default = "crate::core::models::default_extract_debounce",
        rename = "memoryExtractDebounceSecs"
    )]
    memory_extract_debounce_secs: u64,
    #[serde(default, rename = "memoryEmbedding")]
    memory_embedding: MemoryEmbeddingConfig,
    #[serde(default, rename = "memoryRerank")]
    memory_rerank: MemoryRerankConfig,
    #[serde(default, rename = "embeddingModelsDir")]
    embedding_models_dir: String,
    #[serde(default, rename = "localModelsDir")]
    local_models_dir: String,
    #[serde(default, rename = "embeddingModels")]
    embedding_models: Vec<crate::core::models::EmbeddingModelSetting>,
    #[serde(
        default = "crate::core::models::default_dedup_threshold",
        rename = "memoryDedupThreshold"
    )]
    memory_dedup_threshold: f32,
    #[serde(default, rename = "memoryQueryRewrite")]
    memory_query_rewrite: bool,
    #[serde(
        default = "crate::core::models::default_consolidate_budget",
        rename = "memoryConsolidateBudget"
    )]
    memory_consolidate_budget: u32,
    /// Whether failed tool calls are retried automatically (persisted so it survives the save/load round-trip)
    #[serde(
        default = "crate::core::models::default_auto_retry_enabled",
        rename = "autoRetryEnabled"
    )]
    auto_retry_enabled: bool,
    #[serde(
        default = "crate::core::models::default_auto_retry_count",
        rename = "autoRetryCount"
    )]
    auto_retry_count: u32,
    #[serde(
        default = "crate::core::models::default_auto_retry_delay_ms",
        rename = "autoRetryDelayMs"
    )]
    auto_retry_delay_ms: u64,
}

impl AppConfigDisk {
    /// Build the on-disk struct from the runtime config.
    /// api_key is never written to disk (the global key goes to the keyring); inline api_keys inside the
    /// embed/rerank configs are cleared as well (commands/embedding.rs and commands/rerank.rs store them in the keyring separately).
    fn from_config(cfg: &AppConfig) -> Self {
        Self {
            api_url: cfg.api_url.clone(),
            default_model: cfg.default_model.clone(),
            local_model_path: cfg.local_model_path.clone(),
            auto_start_enabled: cfg.auto_start_enabled,
            max_tool_rounds: cfg
                .max_tool_rounds
                .unwrap_or(crate::core::models::DEFAULT_MAX_TOOL_ROUNDS as i32),
            max_concurrent_subagents: cfg.max_concurrent_subagents.unwrap_or(5),
            parallel_tool_execution: cfg.parallel_tool_execution,
            model_call_retries: cfg.model_call_retries,
            sandbox_required: cfg.sandbox_required,
            sandbox_windows: cfg.sandbox_windows.clone(),
            sandbox_windows_toolchain_dirs: cfg.sandbox_windows_toolchain_dirs.clone(),
            shell_windows: cfg.shell_windows.clone(),
            sandbox_protect_git: cfg.sandbox_protect_git,
            sandbox_unsandboxed_fallback: cfg.sandbox_unsandboxed_fallback,
            tool_breaker_threshold: cfg.tool_breaker_threshold,
            memory_enabled: cfg.memory_enabled,
            memory_auto_inject: cfg.memory_auto_inject,
            memory_auto_extract: cfg.memory_auto_extract,
            memory_max_facts: cfg.memory_max_facts,
            memory_injection_budget_tokens: cfg.memory_injection_budget_tokens,
            memory_extract_debounce_secs: cfg.memory_extract_debounce_secs,
            memory_embedding: {
                let mut e = cfg.memory_embedding.clone();
                e.api_key.clear();
                e
            },
            memory_rerank: cfg.memory_rerank.clone(),
            embedding_models_dir: cfg.embedding_models_dir.clone(),
            local_models_dir: cfg.local_models_dir.clone(),
            embedding_models: cfg.embedding_models.clone(),
            memory_dedup_threshold: cfg.memory_dedup_threshold,
            memory_query_rewrite: cfg.memory_query_rewrite,
            memory_consolidate_budget: cfg.memory_consolidate_budget,
            auto_retry_enabled: cfg.auto_retry_enabled,
            auto_retry_count: cfg.auto_retry_count,
            auto_retry_delay_ms: cfg.auto_retry_delay_ms,
        }
    }

    /// Restore the runtime config (the caller injects api_key from the keyring).
    fn into_config(self, api_key: String) -> AppConfig {
        AppConfig {
            api_url: self.api_url,
            api_key,
            default_model: self.default_model,
            local_model_path: self.local_model_path,
            auto_retry_enabled: self.auto_retry_enabled,
            auto_retry_count: self.auto_retry_count,
            auto_retry_delay_ms: self.auto_retry_delay_ms,
            auto_start_enabled: self.auto_start_enabled,
            // Normalize hand-edited values into the valid domain: -1 (unlimited) is the
            // floor, 0 is invalid -> minimum 1 (the consumers re-clamp defensively too)
            max_concurrent_subagents: Some(match self.max_concurrent_subagents {
                n if n < -1 => -1,
                0 => 1,
                n => n,
            }),
            max_tool_rounds: Some(match self.max_tool_rounds {
                n if n < -1 => -1,
                0 => 1,
                n => n,
            }),
            parallel_tool_execution: self.parallel_tool_execution,
            model_call_retries: self.model_call_retries,
            sandbox_required: self.sandbox_required,
            sandbox_windows: self.sandbox_windows.clone(),
            sandbox_windows_toolchain_dirs: self.sandbox_windows_toolchain_dirs.clone(),
            shell_windows: self.shell_windows.clone(),
            sandbox_protect_git: self.sandbox_protect_git,
            sandbox_unsandboxed_fallback: self.sandbox_unsandboxed_fallback,
            tool_breaker_threshold: self.tool_breaker_threshold,
            memory_enabled: self.memory_enabled,
            memory_auto_inject: self.memory_auto_inject,
            memory_auto_extract: self.memory_auto_extract,
            memory_max_facts: self.memory_max_facts,
            memory_injection_budget_tokens: self.memory_injection_budget_tokens,
            memory_extract_debounce_secs: self.memory_extract_debounce_secs,
            memory_embedding: self.memory_embedding,
            memory_rerank: self.memory_rerank,
            embedding_models_dir: self.embedding_models_dir,
            local_models_dir: self.local_models_dir,
            embedding_models: self.embedding_models,
            memory_dedup_threshold: self.memory_dedup_threshold,
            memory_query_rewrite: self.memory_query_rewrite,
            memory_consolidate_budget: self.memory_consolidate_budget,
        }
    }
}

impl Default for AppConfigDisk {
    fn default() -> Self {
        Self {
            api_url: String::new(),
            default_model: String::new(),
            local_model_path: String::new(),
            auto_start_enabled: false,
            max_tool_rounds: crate::core::models::DEFAULT_MAX_TOOL_ROUNDS as i32,
            max_concurrent_subagents: 5,
            parallel_tool_execution: crate::core::models::default_true(),
            model_call_retries: crate::core::models::default_model_call_retries(),
            sandbox_required: crate::core::models::default_true(),
            sandbox_windows: crate::core::models::default_sandbox_windows(),
            sandbox_windows_toolchain_dirs: Vec::new(),
            shell_windows: crate::core::models::default_shell_windows(),
            sandbox_protect_git: crate::core::models::default_true(),
            sandbox_unsandboxed_fallback: false,
            tool_breaker_threshold: 0,
            memory_enabled: false,
            memory_auto_inject: crate::core::models::default_true(),
            memory_auto_extract: crate::core::models::default_true(),
            memory_max_facts: crate::core::models::default_max_facts(),
            memory_injection_budget_tokens: crate::core::models::default_inject_budget(),
            memory_extract_debounce_secs: crate::core::models::default_extract_debounce(),
            memory_embedding: MemoryEmbeddingConfig::default(),
            memory_rerank: MemoryRerankConfig::default(),
            embedding_models_dir: String::new(),
            local_models_dir: String::new(),
            embedding_models: Vec::new(),
            memory_dedup_threshold: crate::core::models::default_dedup_threshold(),
            memory_query_rewrite: false,
            memory_consolidate_budget: crate::core::models::default_consolidate_budget(),
            auto_retry_enabled: crate::core::models::default_auto_retry_enabled(),
            auto_retry_count: crate::core::models::default_auto_retry_count(),
            auto_retry_delay_ms: crate::core::models::default_auto_retry_delay_ms(),
        }
    }
}

fn default_max_tool_rounds() -> i32 {
    crate::core::models::DEFAULT_MAX_TOOL_ROUNDS as i32
}

fn default_max_concurrent_subagents() -> i32 {
    5
}

/// Save the general app config (api_key goes to the OS keychain, only the other fields are written to disk; atomic write).
pub fn save_app_config(app: &AppHandle, config: &AppConfig) -> Result<(), String> {
    // api_key goes to the OS keychain (keyring); only the other fields are written to disk
    if !config.api_key.is_empty() {
        secure_store::set(app, secure_store::accounts::APP_API_KEY, &config.api_key)
            .map_err(|e| e.to_string())?;
    } else {
        let _ = secure_store::delete(app, secure_store::accounts::APP_API_KEY);
    }

    // Config directory and file name come from core::paths (directory creation included)
    let path = crate::core::paths::app_config_file("config.json")?;

    // Field mapping is centralized in AppConfigDisk::from_config (including api_key stripping and the auto_retry fields)
    let disk = AppConfigDisk::from_config(config);
    let json = serde_json::to_string_pretty(&disk).map_err(|e| e.to_string())?;
    // Atomic write (fsync before rename; a crash can never leave a truncated half-written config.json)
    crate::core::fs_util::atomic_write(&path, json.as_bytes()).map_err(|e| e.to_string())?;
    // H-3: explicit invalidation — the keychain half of a save (api_key) does not
    // touch the disk file, so the mtime fingerprint alone would miss it.
    *config_cache().lock() = None;
    Ok(())
}

// ===== H-3: process-level config cache =====
//
// load_app_config is a tool hot path (the agent loop, memory pipeline and command
// layer each read it — several loads per execute_command round, previously = several
// disk reads + JSON parses + keychain IPC round-trips). The cache holds the loaded
// AppConfig keyed by the config.json fingerprint (mtime, len); a save invalidates
// it explicitly (the keychain half of a save does not change the file). Worst case
// on a fingerprint alias (same mtime+len, different content — sub-mtime-granularity
// external edits) is one stale read cycle, same class as any stat-based cache.

struct ConfigCacheEntry {
    config: AppConfig,
    /// (modified, len) fingerprint of config.json at load time; None = no file.
    fingerprint: Option<(std::time::SystemTime, u64)>,
}

static CONFIG_CACHE: std::sync::OnceLock<parking_lot::Mutex<Option<ConfigCacheEntry>>> =
    std::sync::OnceLock::new();

fn config_cache() -> &'static parking_lot::Mutex<Option<ConfigCacheEntry>> {
    CONFIG_CACHE.get_or_init(|| parking_lot::Mutex::new(None))
}

/// Load the general app config (lazily migrates the legacy-schema plaintext api_key; returns defaults when no file exists).
///
/// H-3: served from the process-level cache while config.json's fingerprint holds;
/// see the cache block above for the invalidation contract.
pub fn load_app_config(app: &AppHandle) -> Result<AppConfig, String> {
    let path = crate::core::paths::app_config_dir()?.join("config.json");
    let fingerprint = fs::metadata(&path).ok().map(|m| {
        (
            m.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH),
            m.len(),
        )
    });

    {
        let cache = config_cache().lock();
        if let Some(entry) = cache.as_ref() {
            if entry.fingerprint == fingerprint {
                return Ok(entry.config.clone());
            }
        }
    }

    let config = load_app_config_uncached(app, &path)?;
    *config_cache().lock() = Some(ConfigCacheEntry {
        config: config.clone(),
        fingerprint,
    });
    Ok(config)
}

/// The uncached loader — the historic load_app_config body (file read + schema
/// migration + keychain api_key resolution).
fn load_app_config_uncached(app: &AppHandle, path: &std::path::Path) -> Result<AppConfig, String> {
    // Try the v2 schema first (no api_key field)
    if path.exists() {
        if let Ok(content) = fs::read_to_string(path) {
            if let Ok(disk) = serde_json::from_str::<AppConfigDisk>(&content) {
                let api_key = secure_store::get(app, secure_store::accounts::APP_API_KEY)
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default();
                // Field mapping is centralized in AppConfigDisk::into_config (auto_retry_* reads back from here)
                return Ok(disk.into_config(api_key));
            }
            // Legacy schema compat (with plaintext api_key): migrate it to secure storage after reading.
            // The disk file is rewritten to strip the plaintext only when the key migration succeeded; on failure the original file is kept
            // (migration retries on next startup), so the key is never silently lost.
            if let Ok(legacy) = serde_json::from_str::<AppConfig>(&content) {
                let key_migrated = if !legacy.api_key.is_empty() {
                    secure_store::set_reported(
                        app,
                        secure_store::accounts::APP_API_KEY,
                        &legacy.api_key,
                    )
                } else {
                    true
                };
                let disk = AppConfigDisk {
                    api_url: legacy.api_url.clone(),
                    default_model: legacy.default_model.clone(),
                    local_model_path: legacy.local_model_path.clone(),
                    ..AppConfigDisk::default()
                };
                if key_migrated {
                    if let Err(e) = crate::core::fs_util::atomic_write(
                        path,
                        serde_json::to_string_pretty(&disk)
                            .unwrap_or_default()
                            .as_bytes(),
                    ) {
                        tracing::error!("迁移写回 config.json 失败: {e}");
                    }
                }
                // Field mapping is centralized in AppConfigDisk::into_config (auto_retry_* reads back from here)
                return Ok(disk.into_config(legacy.api_key));
            }
        }
    }

    // No config file: return the default config directly (defaults are defined solely in AppConfig::default())
    Ok(AppConfig::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: auto_retry_* settings must survive the save -> load round-trip.
    #[test]
    fn app_config_disk_roundtrip_preserves_auto_retry() {
        let cfg = AppConfig {
            auto_retry_enabled: false,
            auto_retry_count: 9,
            auto_retry_delay_ms: 1234,
            ..AppConfig::default()
        };

        let disk = AppConfigDisk::from_config(&cfg);
        let back = disk.into_config(String::new());
        assert!(!back.auto_retry_enabled);
        assert_eq!(back.auto_retry_count, 9);
        assert_eq!(back.auto_retry_delay_ms, 1234);
    }

    /// The on-disk struct must never contain a plaintext api_key (neither the global key nor the embed inline key is persisted;
    /// the rerank config has contained only enabled/provider/model since v0.8.1, with no key field at all).
    #[test]
    fn app_config_disk_never_contains_api_keys() {
        let cfg = AppConfig {
            api_key: "secret-global".into(),
            memory_embedding: MemoryEmbeddingConfig {
                api_key: "secret-embed".into(),
                ..MemoryEmbeddingConfig::default()
            },
            ..AppConfig::default()
        };

        let json = serde_json::to_string(&AppConfigDisk::from_config(&cfg)).unwrap();
        assert!(!json.contains("secret-global"));
        assert!(!json.contains("secret-embed"));
        assert!(!json.contains("apiKey"), "不应序列化 api_key 字段");
    }

    /// Old-format config.json (without auto_retry fields) must deserialize to the defaults.
    #[test]
    fn app_config_disk_defaults_when_fields_missing() {
        // Note: the disk format uses mixed naming (fields like api_url keep snake_case without rename,
        // renamed fields like memoryEnabled are camelCase) - existing-file compatibility, do not unify
        let json = r#"{"api_url":"http://x","default_model":"m","local_model_path":""}"#;
        let disk: AppConfigDisk = serde_json::from_str(json).unwrap();
        let back = disk.into_config(String::new());
        assert!(back.auto_retry_enabled);
        assert_eq!(
            back.auto_retry_count,
            crate::core::models::default_auto_retry_count()
        );
        assert_eq!(back.api_url, "http://x");
    }

    /// 安全默认一致性（SEC 审查修复的回归防线）：`sandbox_unsandboxed_fallback`
    /// （完全权限档下"会话授权无法落地时回退一次无沙箱重试"的唯一沙箱逃逸通道）
    /// 在全部默认来源必须为 false——任何一处为 true 都等于默认开启沙箱逃逸：
    /// 1. `AppConfig::default()`（全新安装 / unwrap_or_default 兜底）
    /// 2. `AppConfigDisk::default()`（磁盘结构默认）
    /// 3. serde 缺字段反序列化（磁盘文件没有该字段时）
    ///
    /// 与字段文档承诺的 "Default **false**" 对齐。
    #[test]
    fn sandbox_unsandboxed_fallback_defaults_to_false_in_all_sources() {
        assert!(
            !AppConfig::default().sandbox_unsandboxed_fallback,
            "AppConfig::default() 必须默认关闭无沙箱回退"
        );
        assert!(
            !AppConfigDisk::default().sandbox_unsandboxed_fallback,
            "AppConfigDisk::default() 必须默认关闭无沙箱回退"
        );
        let disk: AppConfigDisk =
            serde_json::from_str(r#"{"api_url":"","default_model":"","local_model_path":""}"#)
                .unwrap();
        assert!(
            !disk.sandbox_unsandboxed_fallback,
            "serde 缺字段默认必须关闭无沙箱回退"
        );
    }
}

//! 应用配置持久化 —— AppConfigDisk（磁盘 schema）与 load/save 的权威实现。
//!
//! 从 commands/config.rs 下沉：services/plugins 只应依赖 core，不应反向依赖命令层。
//!
//! 兼容面：config.json 的磁盘字段名（snake/camel 混合）与旧 schema 明文 key 的
//! 惰性迁移语义保持不变；Tauri 命令 save_app_config/load_app_config 为薄包装。

use crate::core::models::*;
use crate::core::secure_store;
use std::fs;
use tauri::AppHandle;
/// 应用配置文件持久化结构：api_key 不入库，统一存到系统钥匙串
#[derive(serde::Serialize, serde::Deserialize)]
struct AppConfigDisk {
    api_url: String,
    default_model: String,
    local_model_path: String,
    #[serde(default, rename = "autoStartEnabled")]
    auto_start_enabled: bool,
    #[serde(default = "default_max_tool_rounds", rename = "maxToolRounds")]
    max_tool_rounds: u32,
    #[serde(
        default = "default_max_concurrent_subagents",
        rename = "maxConcurrentSubagents"
    )]
    max_concurrent_subagents: u32,
    /// 工作流步骤权限模式（"inherit" 默认 / "auto" 旧行为）
    #[serde(
        default = "crate::core::models::default_workflow_step_mode",
        rename = "workflowStepMode"
    )]
    workflow_step_mode: String,
    /// 同一响应块内的只读工具是否并发执行
    #[serde(
        default = "crate::core::models::default_true",
        rename = "parallelToolExecution"
    )]
    parallel_tool_execution: bool,
    /// 主 Agent 内置工具集（hybrid 默认 / orchestrator / full）
    #[serde(
        default = "crate::core::models::default_main_agent_tool_access",
        rename = "mainAgentToolAccess"
    )]
    main_agent_tool_access: String,
    /// 主模型调用前置阶段重试次数
    #[serde(
        default = "crate::core::models::default_model_call_retries",
        rename = "modelCallRetries"
    )]
    model_call_retries: u32,
    /// 沙箱不可用时是否拒绝自主模式执行命令（fail-closed）
    #[serde(
        default = "crate::core::models::default_true",
        rename = "sandboxRequired"
    )]
    sandbox_required: bool,
    /// Windows 命令沙箱模式（appcontainer 默认 / off）
    #[serde(
        default = "crate::core::models::default_sandbox_windows",
        rename = "sandboxWindows"
    )]
    sandbox_windows: String,
    /// Windows AppContainer 工具链 R+X 目录
    #[serde(default, rename = "sandboxWindowsToolchainDirs")]
    sandbox_windows_toolchain_dirs: Vec<String>,
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
    /// 工具调用失败时是否自动重试（修复：此前落盘不含此字段，每次加载被常量重置）
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
    /// 从运行期配置构造落盘结构。
    /// api_key 不入盘（全局 key 走 keyring）；嵌入/重排配置里内联的 api_key 同样清空
    /// （它们由 commands/embedding.rs 与 commands/rerank.rs 单独入 keyring）。
    fn from_config(cfg: &AppConfig) -> Self {
        Self {
            api_url: cfg.api_url.clone(),
            default_model: cfg.default_model.clone(),
            local_model_path: cfg.local_model_path.clone(),
            auto_start_enabled: cfg.auto_start_enabled,
            max_tool_rounds: cfg
                .max_tool_rounds
                .unwrap_or(crate::core::models::DEFAULT_MAX_TOOL_ROUNDS),
            max_concurrent_subagents: cfg.max_concurrent_subagents.unwrap_or(5),
            workflow_step_mode: cfg.workflow_step_mode.clone(),
            parallel_tool_execution: cfg.parallel_tool_execution,
            main_agent_tool_access: cfg.main_agent_tool_access.clone(),
            model_call_retries: cfg.model_call_retries,
            sandbox_required: cfg.sandbox_required,
            sandbox_windows: cfg.sandbox_windows.clone(),
            sandbox_windows_toolchain_dirs: cfg.sandbox_windows_toolchain_dirs.clone(),
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
            memory_rerank: {
                let mut r = cfg.memory_rerank.clone();
                r.api_key.clear();
                r
            },
            embedding_models_dir: cfg.embedding_models_dir.clone(),
            embedding_models: cfg.embedding_models.clone(),
            memory_dedup_threshold: cfg.memory_dedup_threshold,
            memory_query_rewrite: cfg.memory_query_rewrite,
            memory_consolidate_budget: cfg.memory_consolidate_budget,
            auto_retry_enabled: cfg.auto_retry_enabled,
            auto_retry_count: cfg.auto_retry_count,
            auto_retry_delay_ms: cfg.auto_retry_delay_ms,
        }
    }

    /// 还原为运行期配置（api_key 由调用方从 keyring 注入）。
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
            max_concurrent_subagents: Some(self.max_concurrent_subagents),
            max_tool_rounds: Some(self.max_tool_rounds),
            workflow_step_mode: self.workflow_step_mode,
            parallel_tool_execution: self.parallel_tool_execution,
            main_agent_tool_access: self.main_agent_tool_access,
            model_call_retries: self.model_call_retries,
            sandbox_required: self.sandbox_required,
            sandbox_windows: self.sandbox_windows.clone(),
            sandbox_windows_toolchain_dirs: self.sandbox_windows_toolchain_dirs.clone(),
            memory_enabled: self.memory_enabled,
            memory_auto_inject: self.memory_auto_inject,
            memory_auto_extract: self.memory_auto_extract,
            memory_max_facts: self.memory_max_facts,
            memory_injection_budget_tokens: self.memory_injection_budget_tokens,
            memory_extract_debounce_secs: self.memory_extract_debounce_secs,
            memory_embedding: self.memory_embedding,
            memory_rerank: self.memory_rerank,
            embedding_models_dir: self.embedding_models_dir,
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
            max_tool_rounds: crate::core::models::DEFAULT_MAX_TOOL_ROUNDS,
            max_concurrent_subagents: 5,
            workflow_step_mode: crate::core::models::default_workflow_step_mode(),
            parallel_tool_execution: crate::core::models::default_true(),
            main_agent_tool_access: crate::core::models::default_main_agent_tool_access(),
            model_call_retries: crate::core::models::default_model_call_retries(),
            sandbox_required: crate::core::models::default_true(),
            sandbox_windows: crate::core::models::default_sandbox_windows(),
            sandbox_windows_toolchain_dirs: Vec::new(),
            memory_enabled: false,
            memory_auto_inject: crate::core::models::default_true(),
            memory_auto_extract: crate::core::models::default_true(),
            memory_max_facts: crate::core::models::default_max_facts(),
            memory_injection_budget_tokens: crate::core::models::default_inject_budget(),
            memory_extract_debounce_secs: crate::core::models::default_extract_debounce(),
            memory_embedding: MemoryEmbeddingConfig::default(),
            memory_rerank: MemoryRerankConfig::default(),
            embedding_models_dir: String::new(),
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

fn default_max_tool_rounds() -> u32 {
    crate::core::models::DEFAULT_MAX_TOOL_ROUNDS
}

fn default_max_concurrent_subagents() -> u32 {
    5
}

/// 保存应用程序通用配置（api_key 走系统钥匙串，落盘仅写其他字段；原子写入）。
pub fn save_app_config(app: &AppHandle, config: &AppConfig) -> Result<(), String> {
    // api_key 走系统钥匙串（keyring），落盘仅写其他字段
    if !config.api_key.is_empty() {
        secure_store::set(app, secure_store::accounts::APP_API_KEY, &config.api_key)
            .map_err(|e| e.to_string())?;
    } else {
        let _ = secure_store::delete(app, secure_store::accounts::APP_API_KEY);
    }

    // 配置目录与文件名统一由 core::paths 提供（含目录创建）
    let path = crate::core::paths::app_config_file("config.json")?;

    // 字段映射集中在 AppConfigDisk::from_config（含 api_key 剥离与 auto_retry 字段）
    let disk = AppConfigDisk::from_config(config);
    let json = serde_json::to_string_pretty(&disk).map_err(|e| e.to_string())?;
    // 原子写入
    let tmp_path = path.with_extension("tmp");
    use std::io::Write;
    let mut f = std::fs::File::create(&tmp_path).map_err(|e| e.to_string())?;
    f.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    std::fs::rename(&tmp_path, &path).map_err(|e| e.to_string())?;
    Ok(())
}

/// 读取应用程序通用配置（含旧 schema 明文 api_key 的惰性迁移；无文件时返回默认值）。
pub fn load_app_config(app: &AppHandle) -> Result<AppConfig, String> {
    let path = crate::core::paths::app_config_dir()?.join("config.json");

    // 优先尝试 v2 schema（不含 api_key 字段）
    if path.exists() {
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(disk) = serde_json::from_str::<AppConfigDisk>(&content) {
                let api_key = secure_store::get(app, secure_store::accounts::APP_API_KEY)
                    .map_err(|e| e.to_string())?
                    .unwrap_or_default();
                // 字段映射集中在 AppConfigDisk::into_config（auto_retry_* 从此可读回）
                return Ok(disk.into_config(api_key));
            }
            // 兼容旧 schema（含明文 api_key）：读出后迁出到安全存储。
            // 仅当密钥迁移成功时才改写磁盘文件去除明文；失败则保留原文件
            // （下次启动会重试迁移），绝不静默丢失密钥。
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
                    if let Err(e) = fs::write(
                        &path,
                        serde_json::to_string_pretty(&disk).unwrap_or_default(),
                    ) {
                        tracing::error!("迁移写回 config.json 失败: {e}");
                    }
                }
                // 字段映射集中在 AppConfigDisk::into_config（auto_retry_* 从此可读回）
                return Ok(disk.into_config(legacy.api_key));
            }
        }
    }

    // 无配置文件：直接返回默认配置（默认值唯一定义在 AppConfig::default()）
    Ok(AppConfig::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归：auto_retry_* 配置必须在 落盘 → 读取 往返中保持
    /// （历史 bug：落盘结构无此字段，load 硬编码回常量，用户设置被静默重置）。
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

    /// 落盘结构不得包含明文 api_key（全局 key 与嵌入/重排内联 key 都不入盘）。
    #[test]
    fn app_config_disk_never_contains_api_keys() {
        let cfg = AppConfig {
            api_key: "secret-global".into(),
            memory_embedding: MemoryEmbeddingConfig {
                api_key: "secret-embed".into(),
                ..MemoryEmbeddingConfig::default()
            },
            memory_rerank: MemoryRerankConfig {
                api_key: "secret-rerank".into(),
                ..MemoryRerankConfig::default()
            },
            ..AppConfig::default()
        };

        let json = serde_json::to_string(&AppConfigDisk::from_config(&cfg)).unwrap();
        assert!(!json.contains("secret-global"));
        assert!(!json.contains("secret-embed"));
        assert!(!json.contains("secret-rerank"));
        assert!(!json.contains("apiKey"), "不应序列化 api_key 字段");
    }

    /// 旧格式 config.json（无 auto_retry 字段）反序列化应落到默认值。
    #[test]
    fn app_config_disk_defaults_when_fields_missing() {
        // 注意：磁盘格式是混合命名（api_url 等字段无 rename 保持 snake_case，
        // memoryEnabled 等带 rename 是 camelCase）——兼容既有文件，勿统一改名
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
}

//! App configuration domain: AppConfig, its default value functions, and memory embedding/rerank configs.

use serde::{Deserialize, Serialize};

/// Memory embedding (vectorization) config.
///
/// Since v0.8 online embedding is no longer supported: provider is always local (built-in or downloaded-on-demand local ONNX models);
/// legacy config values of online / openai_compat / ollama are normalized to the local default model at parse time.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryEmbeddingConfig {
    /// Embedding provider: local (local model, the only supported provider)
    pub provider: String,
    /// Local embedding model id (manifest entry id, e.g. bge-small-en-v1.5; built-in model all-MiniLM-L6-v2)
    pub model: String,
    /// Online service URL (deprecated, always empty; kept only for legacy config deserialization compat)
    pub api_url: String,
    /// Vector dimensions (configured value; the first embedding response is authoritative)
    pub dimensions: usize,
    /// Whether embedding is enabled (when off, retrieval degrades to keyword search)
    pub enabled: bool,
    /// Runtime-injected API key (not persisted in config; stored in secure_store)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
}

impl Default for MemoryEmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: "local".into(),
            model: "all-MiniLM-L6-v2".into(),
            api_url: String::new(),
            dimensions: 384,
            enabled: true,
            api_key: String::new(),
        }
    }
}

/// Global embedding model management setting (Settings page "Embedding model management").
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct EmbeddingModelSetting {
    /// Manifest entry id (e.g. bge-small-en-v1.5 / all-MiniLM-L6-v2)
    pub id: String,
    /// Whether globally enabled (when off, the model is unavailable to all projects and retrieval degrades to keyword search)
    pub enabled: bool,
}

impl Default for EmbeddingModelSetting {
    fn default() -> Self {
        Self {
            id: String::new(),
            enabled: true,
        }
    }
}

/// Memory rerank config: second-stage re-ranking of candidates after RRF fusion.
///
/// Structurally mirrors the embedding config (global + per-project tracks):
/// provider = local (local ONNX cross-encoder with kind=rerank in the manifest, default bge-reranker-base).
/// Since v0.8.1 online rerank endpoint support was removed: online provider / apiUrl / apiKey
/// fields in legacy configs are ignored at deserialization (serde ignores unknown fields); the read layer normalizes to local.
/// When disabled or on parse failure, retrieval falls back to RRF-only (fail-safe).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryRerankConfig {
    /// Whether rerank is enabled (default off)
    pub enabled: bool,
    /// Provider: always local (kept only for legacy config normalization compat)
    pub provider: String,
    /// Local model id (kind=rerank manifest entry, e.g. bge-reranker-base / bge-reranker-v2-m3)
    pub model: String,
}

impl Default for MemoryRerankConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "local".into(),
            model: "bge-reranker-base".into(),
        }
    }
}

/// Global application configuration.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AppConfig {
    #[serde(rename = "apiUrl")]
    pub api_url: String,
    #[serde(rename = "apiKey")]
    pub api_key: String,
    #[serde(rename = "defaultModel")]
    pub default_model: String,
    #[serde(rename = "localModelPath", default)]
    pub local_model_path: String,
    /// Whether failed tool calls are auto-retried (default true)
    #[serde(default = "default_auto_retry_enabled")]
    pub auto_retry_enabled: bool,
    /// Auto-retry attempt cap (default 2)
    #[serde(default = "default_auto_retry_count")]
    pub auto_retry_count: u32,
    /// Delay between retries, in milliseconds (default 500)
    #[serde(default = "default_auto_retry_delay_ms")]
    pub auto_retry_delay_ms: u64,
    /// Master switch for per-project memory (semantic RAG). Default off; users enable it in project settings.
    #[serde(default, rename = "memoryEnabled")]
    pub memory_enabled: bool,
    /// Whether retrieved memories are auto-injected into the system prompt (default true)
    #[serde(default = "default_true", rename = "memoryAutoInject")]
    pub memory_auto_inject: bool,
    /// Whether facts are auto-extracted in the background (default true)
    #[serde(default = "default_true", rename = "memoryAutoExtract")]
    pub memory_auto_extract: bool,
    /// Active fact cap per project (default 2000)
    #[serde(default = "default_max_facts", rename = "memoryMaxFacts")]
    pub memory_max_facts: u32,
    /// Token budget for injecting memories into the system prompt (default 3000)
    #[serde(
        default = "default_inject_budget",
        rename = "memoryInjectionBudgetTokens"
    )]
    pub memory_injection_budget_tokens: u32,
    /// Debounce, in seconds, for background auto-extraction (default 60)
    #[serde(
        default = "default_extract_debounce",
        rename = "memoryExtractDebounceSecs"
    )]
    pub memory_extract_debounce_secs: u64,
    /// Memory embedding config (legacy global migration source for per-project embeddings; local only)
    #[serde(default, rename = "memoryEmbedding")]
    pub memory_embedding: MemoryEmbeddingConfig,
    /// Global embedding model install directory (empty = default app_data_dir/models/embed)
    #[serde(default, rename = "embeddingModelsDir")]
    pub embedding_models_dir: String,
    /// Local GGUF model install directory (empty = default app_data_dir/models/local)
    #[serde(default, rename = "localModelsDir")]
    pub local_models_dir: String,
    /// Global embedding model enable/disable registry (missing entries are treated as enabled)
    #[serde(default, rename = "embeddingModels")]
    pub embedding_models: Vec<EmbeddingModelSetting>,
    /// Memory rerank config (second-stage re-ranking after RRF; default off)
    #[serde(default, rename = "memoryRerank")]
    pub memory_rerank: MemoryRerankConfig,
    /// Cosine similarity threshold that triggers dedup/arbitration (shared by remember and auto-extraction, default 0.92)
    #[serde(default = "default_dedup_threshold", rename = "memoryDedupThreshold")]
    pub memory_dedup_threshold: f32,
    /// Whether retrieval rewrites queries via LLM (generates 1-2 variants for RRF fusion; default off, adds extra LLM calls)
    #[serde(default, rename = "memoryQueryRewrite")]
    pub memory_query_rewrite: bool,
    /// Cap on LLM arbitration calls for daily background consolidation (sleep-time consolidation) (default 20)
    #[serde(
        default = "default_consolidate_budget",
        rename = "memoryConsolidateBudget"
    )]
    pub memory_consolidate_budget: u32,
    /// Auto-start the app at system login. Default off.
    #[serde(default, rename = "autoStartEnabled")]
    pub auto_start_enabled: bool,
    /// Maximum concurrent subagents (None = use default 5). Caps concurrency of delegate_tasks and multiple delegate_task calls.
    #[serde(default, rename = "maxConcurrentSubagents")]
    pub max_concurrent_subagents: Option<u32>,
    /// Maximum tool-call rounds per agent run (None = use the default). On hitting the cap the run wraps up gracefully instead of being hard-interrupted, to bound API cost.
    #[serde(default, rename = "maxToolRounds")]
    pub max_tool_rounds: Option<u32>,
    /// Parallel write isolation for subagents: "auto" (default; awaited write-capable tasks get a
    /// per-task git worktree when the project is a git repo, falling back to the shared root),
    /// "off" (always shared root; per-file locks only), "worktree" (require worktrees; fall
    /// back to shared with a notice when unavailable, e.g. non-git projects).
    #[serde(
        default = "default_subagent_isolation_mode",
        rename = "subagentIsolationMode"
    )]
    pub subagent_isolation_mode: String,
    /// Permission mode for workflow steps: "inherit" inherits the parent turn's mode (default; write actions
    /// still require per-action approval under Normal), "auto" forces Auto mode (legacy behavior: step tools are auto-approved).
    #[serde(default = "default_workflow_step_mode", rename = "workflowStepMode")]
    pub workflow_step_mode: String,
    /// Whether read-only tools in the same response block execute concurrently (default true). Off falls back to fully serial execution.
    #[serde(default = "default_true", rename = "parallelToolExecution")]
    pub parallel_tool_execution: bool,
    /// Main agent built-in tool set: "hybrid" (default: read-only tools direct, mutating tools via subagents),
    /// "orchestrator" (strict orchestrator, legacy behavior), "full" (all built-in tools, Claude Code style).
    #[serde(
        default = "default_main_agent_tool_access",
        rename = "mainAgentToolAccess"
    )]
    pub main_agent_tool_access: String,
    /// Retries for the pre-flight phase of main model calls (POST + status check) (default 3, backoff 1s/2s/4s).
    /// Only covers failures before any content is emitted; mid-stream errors are not retried.
    #[serde(default = "default_model_call_retries", rename = "modelCallRetries")]
    pub model_call_retries: u32,
    /// Whether autonomous-mode command execution is refused when no sandbox is available (no bwrap/sandbox-exec)
    /// (fail-closed, default true). false = fall back to unsandboxed execution (legacy behavior, not recommended).
    #[serde(default = "default_true", rename = "sandboxRequired")]
    pub sandbox_required: bool,
    /// Windows command sandbox mode: "appcontainer" (default: OS-level file-write control + network blocked by default)
    /// or "off" (Job Object lifetime limits only, legacy behavior).
    #[serde(default = "default_sandbox_windows", rename = "sandboxWindows")]
    pub sandbox_windows: String,
    /// Toolchain directories granted extra R+X under the Windows AppContainer (e.g. cargo/scoop install dirs;
    /// system dirs rely on the built-in ALL APPLICATION PACKAGES ACE and need no configuration).
    #[serde(default, rename = "sandboxWindowsToolchainDirs")]
    pub sandbox_windows_toolchain_dirs: Vec<String>,
    /// Consecutive ENVIRONMENT-class failures (missing command / permission denied /
    /// missing dependency) before a tool is disabled for the rest of the turn
    /// (harness round 3, P1-1: configurable). Business failures (command ran, nonzero
    /// exit) never trip the breaker. 0 = default (6).
    #[serde(default, rename = "toolBreakerThreshold")]
    pub tool_breaker_threshold: u32,
    /// Auto/Workflow modes only: when a sandboxed command hits an access-denied /
    /// forwarded-interpreter-not-found signature, auto-apply the recommended expansion
    /// (session grant for the derived directory when available, otherwise one
    /// unsandboxed retry) instead of popping the remediation dialog — full-autonomy
    /// opt-in, risk acknowledged by the user. Custom Deny rules and the
    /// network-declaration confirmation are NOT affected. Default false.
    #[serde(default, rename = "autoExpandSandbox")]
    pub auto_expand_sandbox: bool,
}

/// Default value of the Windows command sandbox mode.
pub(crate) fn default_sandbox_windows() -> String {
    "appcontainer".into()
}

/// Default value of main model call retries.
pub(crate) fn default_model_call_retries() -> u32 {
    3
}

/// Default value of the main agent built-in tool set.
pub(crate) fn default_main_agent_tool_access() -> String {
    "hybrid".into()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            api_url: String::new(),
            api_key: String::new(),
            default_model: String::new(),
            local_model_path: String::new(),
            auto_retry_enabled: default_auto_retry_enabled(),
            auto_retry_count: default_auto_retry_count(),
            auto_retry_delay_ms: default_auto_retry_delay_ms(),
            auto_start_enabled: false,
            max_concurrent_subagents: None,
            max_tool_rounds: None,
            subagent_isolation_mode: default_subagent_isolation_mode(),
            workflow_step_mode: default_workflow_step_mode(),
            parallel_tool_execution: default_true(),
            main_agent_tool_access: default_main_agent_tool_access(),
            model_call_retries: default_model_call_retries(),
            sandbox_required: default_true(),
            sandbox_windows: default_sandbox_windows(),
            sandbox_windows_toolchain_dirs: Vec::new(),
            tool_breaker_threshold: 0,
            auto_expand_sandbox: false,
            memory_enabled: false,
            memory_auto_inject: default_true(),
            memory_auto_extract: default_true(),
            memory_max_facts: default_max_facts(),
            memory_injection_budget_tokens: default_inject_budget(),
            memory_extract_debounce_secs: default_extract_debounce(),
            memory_embedding: MemoryEmbeddingConfig::default(),
            memory_rerank: MemoryRerankConfig::default(),
            embedding_models_dir: String::new(),
            local_models_dir: String::new(),
            embedding_models: Vec::new(),
            memory_dedup_threshold: default_dedup_threshold(),
            memory_query_rewrite: false,
            memory_consolidate_budget: default_consolidate_budget(),
        }
    }
}

/// Default maximum tool-call rounds per agent run.
pub const DEFAULT_MAX_TOOL_ROUNDS: u32 = 50;

/// Default value of the workflow step permission mode: inherit the parent turn's mode.
pub(crate) fn default_workflow_step_mode() -> String {
    "inherit".into()
}

/// Default value of the subagent parallel write isolation mode: auto (worktree when git repo).
pub fn default_subagent_isolation_mode() -> String {
    "auto".into()
}

pub(crate) fn default_auto_retry_enabled() -> bool {
    true
}

pub(crate) fn default_auto_retry_count() -> u32 {
    2
}

pub(crate) fn default_auto_retry_delay_ms() -> u64 {
    500
}

pub(crate) fn default_true() -> bool {
    true
}

pub(crate) fn default_max_facts() -> u32 {
    2000
}

pub(crate) fn default_inject_budget() -> u32 {
    3000
}

pub(crate) fn default_extract_debounce() -> u64 {
    60
}

/// Cosine similarity threshold for dedup/arbitration (default 0.92, matches the design document).
pub(crate) fn default_dedup_threshold() -> f32 {
    0.92
}

/// Cap on arbitration calls for the daily consolidation.
pub(crate) fn default_consolidate_budget() -> u32 {
    20
}

// ====== MCP server configuration ======

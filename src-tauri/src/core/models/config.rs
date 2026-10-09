//! 应用配置域：AppConfig 及其默认值函数、记忆嵌入/重排配置。

use serde::{Deserialize, Serialize};

/// 记忆嵌入（向量化）配置。
///
/// 自 v0.8 起不再支持在线嵌入：provider 恒为 local（内置或按需下载的本地 ONNX 模型），
/// 旧配置中的 online / openai_compat / ollama 在解析层自动归一化为本地默认模型。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryEmbeddingConfig {
    /// 嵌入 provider：local（本地模型，唯一受支持的 provider）
    pub provider: String,
    /// 本地嵌入模型 id（manifest 条目 id，如 bge-small-en-v1.5；内置模型 all-MiniLM-L6-v2）
    pub model: String,
    /// 在线服务地址（已废弃，恒为空；保留字段仅为旧配置反序列化兼容）
    pub api_url: String,
    /// 向量维度（配置值；实际以首次嵌入返回为准）
    pub dimensions: usize,
    /// 是否启用嵌入（关闭时检索退化为关键词）
    pub enabled: bool,
    /// 运行时注入的 API Key（不随配置持久化，存 secure_store）
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

/// 全局嵌入模型管理配置项（设置页「Embedding 模型管理」）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct EmbeddingModelSetting {
    /// manifest 条目 id（如 bge-small-en-v1.5 / all-MiniLM-L6-v2）
    pub id: String,
    /// 是否全局启用（停用后所有项目不可用该模型，检索退化为关键词）
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

/// 记忆重排（rerank）配置：RRF 融合后对候选做二阶段精排。
///
/// 与嵌入配置同构（全局 + 项目级双轨）：
/// provider = local（manifest 中 kind=rerank 的本地 ONNX 交叉编码器，默认 bge-reranker-base）。
/// 自 v0.8.1 起移除在线 rerank 端点支持：旧配置中的 online provider / apiUrl / apiKey
/// 字段在反序列化时被忽略（serde 忽略未知字段），读取层归一化为 local。
/// 未启用或解析失败时检索回退为 RRF-only（fail-safe）。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryRerankConfig {
    /// 是否启用重排（默认关）
    pub enabled: bool,
    /// provider：恒为 local（保留字段仅为旧配置归一化兼容）
    pub provider: String,
    /// 本地模型 id（manifest 中的 kind=rerank 条目，如 bge-reranker-base / bge-reranker-v2-m3）
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

/// 应用程序全局配置。
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
    /// 工具调用失败时是否自动重试（默认 true）
    #[serde(default = "default_auto_retry_enabled")]
    pub auto_retry_enabled: bool,
    /// 自动重试次数上限（默认 2）
    #[serde(default = "default_auto_retry_count")]
    pub auto_retry_count: u32,
    /// 每次重试间隔（毫秒，默认 500）
    #[serde(default = "default_auto_retry_delay_ms")]
    pub auto_retry_delay_ms: u64,
    /// 项目级记忆（语义 RAG）总开关。默认关闭，用户在项目设置中开启。
    #[serde(default, rename = "memoryEnabled")]
    pub memory_enabled: bool,
    /// 是否自动注入检索到的记忆到系统提示词（默认 true）
    #[serde(default = "default_true", rename = "memoryAutoInject")]
    pub memory_auto_inject: bool,
    /// 是否后台自动提取事实（默认 true；提取管线随 P2 生效）
    #[serde(default = "default_true", rename = "memoryAutoExtract")]
    pub memory_auto_extract: bool,
    /// 每项目活跃事实上限（默认 2000）
    #[serde(default = "default_max_facts", rename = "memoryMaxFacts")]
    pub memory_max_facts: u32,
    /// 记忆注入系统提示词的 token 预算（默认 3000）
    #[serde(
        default = "default_inject_budget",
        rename = "memoryInjectionBudgetTokens"
    )]
    pub memory_injection_budget_tokens: u32,
    /// 后台自动提取去抖秒数（默认 60）
    #[serde(
        default = "default_extract_debounce",
        rename = "memoryExtractDebounceSecs"
    )]
    pub memory_extract_debounce_secs: u64,
    /// 记忆嵌入配置（项目级嵌入的旧全局迁移源；仅 local）
    #[serde(default, rename = "memoryEmbedding")]
    pub memory_embedding: MemoryEmbeddingConfig,
    /// 全局嵌入模型安装目录（空 = 默认 app_data_dir/models/embed）
    #[serde(default, rename = "embeddingModelsDir")]
    pub embedding_models_dir: String,
    /// 全局嵌入模型启用/停用注册表（缺省条目视为启用）
    #[serde(default, rename = "embeddingModels")]
    pub embedding_models: Vec<EmbeddingModelSetting>,
    /// 记忆重排配置（RRF 后二阶段精排；默认关）
    #[serde(default, rename = "memoryRerank")]
    pub memory_rerank: MemoryRerankConfig,
    /// 判重/仲裁触发的余弦相似度阈值（remember 与自动提取共用，默认 0.92）
    #[serde(default = "default_dedup_threshold", rename = "memoryDedupThreshold")]
    pub memory_dedup_threshold: f32,
    /// 检索时是否用 LLM 改写查询（生成 1-2 个变体做 RRF 融合；默认关，涉及额外 LLM 调用）
    #[serde(default, rename = "memoryQueryRewrite")]
    pub memory_query_rewrite: bool,
    /// 每日后台合并整理（sleep-time consolidation）的 LLM 仲裁调用预算上限（默认 20）
    #[serde(
        default = "default_consolidate_budget",
        rename = "memoryConsolidateBudget"
    )]
    pub memory_consolidate_budget: u32,
    /// 系统自启。默认关闭。
    #[serde(default, rename = "autoStartEnabled")]
    pub auto_start_enabled: bool,
    /// 最大并发子智能体数量（None = 使用默认值 5）。用于限制 delegate_tasks 和多个 delegate_task 的并发数。
    #[serde(default, rename = "maxConcurrentSubagents")]
    pub max_concurrent_subagents: Option<u32>,
    /// 单次 Agent 运行最大工具调用轮数（None = 使用默认值）。命中时优雅收尾而非硬中断，防无界 API 成本。
    #[serde(default, rename = "maxToolRounds")]
    pub max_tool_rounds: Option<u32>,
    /// 工作流步骤的权限模式："inherit" 继承父回合模式（默认；Normal 下写操作逐次审批），
    /// "auto" 强制 Auto 模式（旧行为，步骤内工具自动放行）。
    #[serde(default = "default_workflow_step_mode", rename = "workflowStepMode")]
    pub workflow_step_mode: String,
    /// 同一响应块内的只读工具是否并发执行（默认 true）。关闭后回退全串行。
    #[serde(default = "default_true", rename = "parallelToolExecution")]
    pub parallel_tool_execution: bool,
    /// 主 Agent 内置工具集："hybrid"（默认，只读工具直达 + 变更走子代理）、
    /// "orchestrator"（严格编排者，旧行为）、"full"（全部内置工具，Claude Code 形态）。
    #[serde(
        default = "default_main_agent_tool_access",
        rename = "mainAgentToolAccess"
    )]
    pub main_agent_tool_access: String,
    /// 主模型调用前置阶段（POST + 状态检查）的重试次数（默认 3，退避 1s/2s/4s）。
    /// 仅覆盖未发射任何内容前的失败；流中段错误不重试。
    #[serde(default = "default_model_call_retries", rename = "modelCallRetries")]
    pub model_call_retries: u32,
    /// 沙箱不可用时（无 bwrap/sandbox-exec）是否拒绝自主模式执行命令（fail-closed，
    /// 默认 true）。false = 回退无沙箱执行（旧行为，不推荐）。
    #[serde(default = "default_true", rename = "sandboxRequired")]
    pub sandbox_required: bool,
    /// Windows 命令沙箱模式："appcontainer"（默认，OS 级文件写管控 + 默认断网）
    /// 或 "off"（仅 Job Object 生命周期限制，旧行为）。
    #[serde(default = "default_sandbox_windows", rename = "sandboxWindows")]
    pub sandbox_windows: String,
    /// Windows AppContainer 下额外授予 R+X 的工具链目录（如 cargo/scoop 安装目录；
    /// 系统目录依赖内置 ALL APPLICATION PACKAGES ACE 无需配置）。
    #[serde(default, rename = "sandboxWindowsToolchainDirs")]
    pub sandbox_windows_toolchain_dirs: Vec<String>,
}

/// Windows 命令沙箱模式默认值。
pub(crate) fn default_sandbox_windows() -> String {
    "appcontainer".into()
}

/// 主模型调用重试次数默认值。
pub(crate) fn default_model_call_retries() -> u32 {
    3
}

/// 主 Agent 内置工具集默认值。
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
            workflow_step_mode: default_workflow_step_mode(),
            parallel_tool_execution: default_true(),
            main_agent_tool_access: default_main_agent_tool_access(),
            model_call_retries: default_model_call_retries(),
            sandbox_required: default_true(),
            sandbox_windows: default_sandbox_windows(),
            sandbox_windows_toolchain_dirs: Vec::new(),
            memory_enabled: false,
            memory_auto_inject: default_true(),
            memory_auto_extract: default_true(),
            memory_max_facts: default_max_facts(),
            memory_injection_budget_tokens: default_inject_budget(),
            memory_extract_debounce_secs: default_extract_debounce(),
            memory_embedding: MemoryEmbeddingConfig::default(),
            memory_rerank: MemoryRerankConfig::default(),
            embedding_models_dir: String::new(),
            embedding_models: Vec::new(),
            memory_dedup_threshold: default_dedup_threshold(),
            memory_query_rewrite: false,
            memory_consolidate_budget: default_consolidate_budget(),
        }
    }
}

/// 单次 Agent 运行默认最大工具调用轮数。
pub const DEFAULT_MAX_TOOL_ROUNDS: u32 = 50;

/// 工作流步骤权限模式默认值：继承父回合模式。
pub(crate) fn default_workflow_step_mode() -> String {
    "inherit".into()
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

/// 判重/仲裁余弦阈值（默认 0.92，与设计文档一致）。
pub(crate) fn default_dedup_threshold() -> f32 {
    0.92
}

/// 每日合并整理的仲裁调用预算上限。
pub(crate) fn default_consolidate_budget() -> u32 {
    20
}

// ====== MCP 服务器配置 ======

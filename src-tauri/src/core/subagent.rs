//! Sub-agent profiles and tool definitions.
//!
//! Provides ten built-in sub-agent profiles, each restricting a different toolset and behavior:
//! - `explorer`: read-only code explorer for large-scale file retrieval and architecture analysis
//! - `coder`: code implementer focused on writing and modifying files
//! - `general`: general-purpose sub-agent with the full toolset
//! - `architect`: architecture designer, read-only analysis of dependencies and design decisions
//! - `debugger`: problem diagnostician, can run commands to find root causes but cannot modify files
//! - `reviewer`: code reviewer, read-only security audit and code quality assessment
//! - `writer`: documentation writer, can read and write files to produce documentation
//! - `tester`: test engineer, can run tests and write test files
//! - `requirements`: requirements analyst, read-only analysis that decomposes user requests into structured workflow plans
//! - `vision`: vision analyst, toolless; auto-invoked to describe user images when the main model lacks vision
//!
//! Sub-agents are created via the main agent's `delegate_task` tool; each sub-agent
//! has its own LLM context window and reports progress to the frontend via Tauri events.

use serde::{Deserialize, Serialize};

use super::models::ToolSpec;
use crate::core::tool_names;
/// Sub-agent profile.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SubagentProfile {
    /// Unique profile identifier ("explorer" | "coder" | "general" | "architect" | "debugger" | "reviewer" | "writer" | "tester")
    pub id: String,
    /// Display name
    pub name: String,
    /// Purpose description
    pub description: String,
    /// Allowed tools (glob patterns); empty array = allow all
    pub allowed_tools: Vec<String>,
    /// Denied tools (glob patterns); empty array = nothing extra denied
    pub denied_tools: Vec<String>,
    /// Extra system prompt appended after the sub-agent base system prompt
    pub system_prompt_extension: String,
    /// Optional model override (run the subtask on a cheaper model); None = follow the main agent's model
    pub model_override: Option<String>,
    /// Whether MCP tools (servers enabled by the main agent) are injected. Only `general` (full
    /// capability) enables it by default; other built-ins stay least-privileged. Custom profiles are user-configured.
    #[serde(default)]
    pub allow_mcp_tools: bool,
}

impl SubagentProfile {
    /// Check whether a tool is explicitly excluded by denied_tools (allowed list not consulted).
    /// Used for MCP tools: their gate is the allow_mcp_tools flag, but explicit exclusion still applies.
    pub fn is_tool_denied(&self, tool_name: &str) -> bool {
        self.denied_tools
            .iter()
            .any(|pattern| simple_glob_match(pattern, tool_name))
    }

    /// Check whether a tool is allowed by this profile
    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        // denied takes precedence
        if self
            .denied_tools
            .iter()
            .any(|pattern| simple_glob_match(pattern, tool_name))
        {
            return false;
        }
        // empty allowed = allow all
        if self.allowed_tools.is_empty() {
            return true;
        }
        self.allowed_tools
            .iter()
            .any(|pattern| simple_glob_match(pattern, tool_name))
    }
}

/// Simple glob matching (supports a `*` suffix wildcard, e.g. `git_*` matches all git tools)
fn simple_glob_match(pattern: &str, target: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return target.starts_with(prefix);
    }
    pattern == target
}

/// Return all built-in sub-agent profiles
pub fn builtin_profiles() -> Vec<SubagentProfile> {
    vec![
        SubagentProfile {
            id: "explorer".into(),
            allow_mcp_tools: false,
            name: "代码探索者".into(),
            description: "只读搜索和分析，用于大规模代码探索、多文件检索、架构分析。不能修改任何文件。".into(),
            allowed_tools: vec![
                tool_names::READ_FILE.into(),
                tool_names::LIST_DIRECTORY.into(),
                "search_files".into(),
                tool_names::SEARCH_CONTENT.into(),
                "read_lints".into(),
                tool_names::WEB_SEARCH.into(),
                tool_names::WEB_FETCH.into(),
                tool_names::RECALL.into(),
                "search_code".into(),
            ],
            denied_tools: vec![
                tool_names::WRITE_FILE.into(),
                tool_names::REPLACE_IN_FILE.into(),
                tool_names::DELETE_FILE.into(),
                tool_names::MAKE_DIRECTORY.into(),
                tool_names::EXECUTE_COMMAND.into(),
                "git_*".into(),
                tool_names::DELEGATE_TASK.into(),
            ],
            system_prompt_extension: concat!(
                "你是代码探索者，只能搜索和分析代码，不能修改任何文件。\n",
                "你的任务是：理解代码结构、查找相关文件、分析代码逻辑，并返回详细的发现总结。\n",
                "在回复中清晰地组织你的发现，包括：文件路径、关键代码片段、相关关系。\n",
                "不要尝试修改文件、执行命令或进行写操作。"
            )
            .into(),
            model_override: None,
        },
        SubagentProfile {
            id: "coder".into(),
            allow_mcp_tools: false,
            name: "代码实现者".into(),
            description: "代码编写和修改，用于实现具体功能模块。禁止执行 shell 命令。".into(),
            allowed_tools: vec![], // empty = allow all
            denied_tools: vec![tool_names::EXECUTE_COMMAND.into(), tool_names::DELEGATE_TASK.into()],
            system_prompt_extension: concat!(
                "你是代码实现者，专注编写和修改代码文件。\n",
                "修改前务必先阅读相关文件以充分理解上下文。\n",
                "完成后总结你所做的所有修改，包括文件路径和变更内容。\n",
                "不要执行 shell 命令。"
            )
            .into(),
            model_override: None,
        },
        SubagentProfile {
            id: "general".into(),
            allow_mcp_tools: true,
            name: "通用子智能体".into(),
            description: "全能力子智能体，拥有和主 Agent 完全相同的工具集。".into(),
            allowed_tools: vec![],
            denied_tools: vec![tool_names::DELEGATE_TASK.into()], // forbid recursive sub-agent creation
            system_prompt_extension: concat!(
                "你是通用子智能体，可以执行任意任务。\n",
                "充分利用所有可用工具来完成目标。\n",
                "完成后总结你的工作成果。"
            )
            .into(),
            model_override: None,
        },
        SubagentProfile {
            id: "architect".into(),
            allow_mcp_tools: false,
            name: "架构设计师".into(),
            description: "只读架构分析、依赖映射、设计决策评估与技术选型建议。不能修改任何文件。".into(),
            allowed_tools: vec![
                tool_names::READ_FILE.into(),
                tool_names::LIST_DIRECTORY.into(),
                "search_files".into(),
                tool_names::SEARCH_CONTENT.into(),
                "read_lints".into(),
                tool_names::WEB_SEARCH.into(),
                tool_names::WEB_FETCH.into(),
                tool_names::RECALL.into(),
                "search_code".into(),
            ],
            denied_tools: vec![
                tool_names::WRITE_FILE.into(),
                tool_names::REPLACE_IN_FILE.into(),
                tool_names::DELETE_FILE.into(),
                tool_names::MAKE_DIRECTORY.into(),
                tool_names::EXECUTE_COMMAND.into(),
                "git_*".into(),
                tool_names::DELEGATE_TASK.into(),
            ],
            system_prompt_extension: concat!(
                "你是架构设计师，只能分析代码架构和设计，不能修改任何文件。\n",
                "你的任务是：分析项目结构、识别设计模式、评估技术决策、绘制依赖关系。\n",
                "输出应包含：关键文件路径、模块依赖图、设计建议与潜在风险。\n",
                "不要尝试修改文件、执行命令或进行写操作。"
            )
            .into(),
            model_override: None,
        },
        SubagentProfile {
            id: "debugger".into(),
            allow_mcp_tools: false,
            name: "问题诊断师".into(),
            description: "错误调查与根因分析，可运行命令复现问题但不能修改任何文件。".into(),
            allowed_tools: vec![
                tool_names::READ_FILE.into(),
                tool_names::LIST_DIRECTORY.into(),
                "search_files".into(),
                tool_names::SEARCH_CONTENT.into(),
                "read_lints".into(),
                tool_names::EXECUTE_COMMAND.into(),
                tool_names::WEB_SEARCH.into(),
                tool_names::WEB_FETCH.into(),
            ],
            denied_tools: vec![
                tool_names::WRITE_FILE.into(),
                tool_names::REPLACE_IN_FILE.into(),
                tool_names::DELETE_FILE.into(),
                tool_names::MAKE_DIRECTORY.into(),
                "git_*".into(),
                tool_names::DELEGATE_TASK.into(),
            ],
            system_prompt_extension: concat!(
                "你是问题诊断师，可以运行命令来复现和分析错误，但不能修改任何文件。\n",
                "你的任务是：定位错误根因、分析日志输出、复现问题、提出修复建议。\n",
                "输出应包含：根因分析、复现步骤、建议的修复方案（仅建议，不执行修改）。\n",
                "不要尝试修改文件、提交代码或进行写操作。"
            )
            .into(),
            model_override: None,
        },
        SubagentProfile {
            id: "reviewer".into(),
            allow_mcp_tools: false,
            name: "代码审查员".into(),
            description: "只读代码质量评估、安全审计与最佳实践检查。不能修改任何文件。".into(),
            allowed_tools: vec![
                tool_names::READ_FILE.into(),
                tool_names::LIST_DIRECTORY.into(),
                "search_files".into(),
                tool_names::SEARCH_CONTENT.into(),
                "read_lints".into(),
                tool_names::WEB_SEARCH.into(),
                tool_names::WEB_FETCH.into(),
                tool_names::RECALL.into(),
                "search_code".into(),
            ],
            denied_tools: vec![
                tool_names::WRITE_FILE.into(),
                tool_names::REPLACE_IN_FILE.into(),
                tool_names::DELETE_FILE.into(),
                tool_names::MAKE_DIRECTORY.into(),
                tool_names::EXECUTE_COMMAND.into(),
                "git_*".into(),
                tool_names::DELEGATE_TASK.into(),
            ],
            system_prompt_extension: concat!(
                "你是代码审查员，只能审查代码质量和安全性，不能修改任何文件。\n",
                "你的任务是：评估代码质量、识别安全漏洞、检查最佳实践遵从度。\n",
                "输出应包含：按严重性分级（严重/警告/建议）的审查意见，附文件路径和行号引用。\n",
                "不要尝试修改文件、执行命令或进行写操作。"
            )
            .into(),
            model_override: None,
        },
        SubagentProfile {
            id: "writer".into(),
            allow_mcp_tools: false,
            name: "文档撰写员".into(),
            description: "编写文档、注释、README、变更日志与技术规范。可读写文件。".into(),
            allowed_tools: vec![], // empty = allow all
            denied_tools: vec![
                tool_names::DELETE_FILE.into(),
                tool_names::MAKE_DIRECTORY.into(),
                tool_names::EXECUTE_COMMAND.into(),
                "git_*".into(),
                tool_names::DELEGATE_TASK.into(),
            ],
            system_prompt_extension: concat!(
                "你是文档撰写员，专注于编写高质量的文档和注释。\n",
                "你的任务是：生成 README、API 文档、变更日志、技术规范和内联注释。\n",
                "输出应使用清晰的 Markdown 格式，结构良好，层次分明。\n",
                "可以读取和写入文件，但不要执行 shell 命令或删除文件。"
            )
            .into(),
            model_override: None,
        },
        SubagentProfile {
            id: "tester".into(),
            allow_mcp_tools: false,
            name: "测试工程师".into(),
            description: "测试用例生成、覆盖率分析与测试执行。可运行测试并编写测试文件。".into(),
            allowed_tools: vec![], // empty = allow all
            denied_tools: vec![
                tool_names::DELETE_FILE.into(),
                tool_names::MAKE_DIRECTORY.into(),
                "git_*".into(),
                tool_names::DELEGATE_TASK.into(),
            ],
            system_prompt_extension: concat!(
                "你是测试工程师，专注于测试的编写、执行和分析。\n",
                "你的任务是：生成测试用例、执行测试、分析覆盖率、报告测试结果。\n",
                "输出应包含：测试执行总结、覆盖率报告、失败的测试及其根因。\n",
                "可以执行命令和写入测试文件，但不要提交代码或删除文件。"
            )
            .into(),
            model_override: None,
        },
        SubagentProfile {
            id: "requirements".into(),
            allow_mcp_tools: false,
            name: "需求分析员".into(),
            description: "用户需求分析，将用户输入拆解为结构化工作流方案。不能修改文件。".into(),
            allowed_tools: vec![
                tool_names::READ_FILE.into(),
                tool_names::LIST_DIRECTORY.into(),
                "search_files".into(),
                tool_names::SEARCH_CONTENT.into(),
                "read_lints".into(),
            ],
            denied_tools: vec![
                tool_names::WRITE_FILE.into(),
                tool_names::REPLACE_IN_FILE.into(),
                tool_names::DELETE_FILE.into(),
                tool_names::MAKE_DIRECTORY.into(),
                tool_names::EXECUTE_COMMAND.into(),
                "git_*".into(),
                tool_names::DELEGATE_TASK.into(),
                tool_names::WEB_SEARCH.into(),
                tool_names::WEB_FETCH.into(),
            ],
            system_prompt_extension: concat!(
                "你是需求分析员，只能分析用户需求，不能修改任何文件。\n",
                "你的任务是：理解用户的请求、识别核心需求、将复杂任务拆解为可独立执行的子任务。\n",
                "如果用户的请求涉及多个步骤，建议以下工作流角色序列：\n",
                "  - explorer：代码探索，搜索和分析现有代码\n",
                "  - architect：架构设计，分析依赖和设计决策\n",
                "  - coder：代码实现，编写或修改文件\n",
                "  - reviewer：代码审查，检查质量和安全\n",
                "  - tester：测试执行，验证功能\n",
                "输出格式：清晰地列出分析结果和建议的子任务划分，每个子任务包含目标、推荐角色和预期产出。\n",
                "不要调用 create_workflow 工具，你只负责分析需求。主 Agent 会根据你的分析结果决定是否创建工 作流。"
            )
            .into(),
            model_override: None,
        },
        SubagentProfile {
            id: "vision".into(),
            allow_mcp_tools: false,
            name: "视觉分析师".into(),
            description: "图像理解与视觉分析。主模型不支持图片输入时自动代为看图，输出结构化的图像描述。不使用任何工具。".into(),
            // Toolless agent: denied_tools ["*"] blocks every tool (empty
            // allowed_tools would mean allow-all).
            allowed_tools: vec![],
            denied_tools: vec!["*".into()],
            system_prompt_extension: concat!(
                "你是 AIO 的视觉分析智能体。主对话模型不支持图像输入时，由你代为查看用户发送的图片。\n",
                "你的任务是：结合用户的消息，详细描述图片中的关键内容——场景与主体、可见文字（逐字转录）、",
                "图表与其中的数据、界面元素、以及与用户问题直接相关的细节。\n",
                "输出一份结构化描述，供主模型作为图像内容的依据。不要臆造图中不存在的信息。"
            )
            .into(),
            model_override: None,
        },
    ]
}

/// Look up a profile by ID (built-ins first, then custom profiles)
pub fn find_profile(
    profile_id: &str,
    custom_profiles: &[crate::core::models::CustomSubagentProfile],
) -> Option<SubagentProfile> {
    // Check built-ins first
    if let Some(p) = builtin_profiles().into_iter().find(|p| p.id == profile_id) {
        return Some(p);
    }
    // Then custom profiles
    for cp in custom_profiles {
        if cp.id == profile_id {
            return Some(SubagentProfile {
                id: cp.id.clone(),
                name: cp.name.clone(),
                description: cp.description.clone(),
                allowed_tools: cp.allowed_tools.clone(),
                denied_tools: cp.denied_tools.clone(),
                system_prompt_extension: cp.system_prompt_extension.clone(),
                model_override: None,
                allow_mcp_tools: cp.allow_mcp_tools,
            });
        }
    }
    None
}

/// Build the ToolSpec for the `delegate_task` tool.
pub fn delegate_task_tool_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".into(),
        function: super::models::ToolFunctionSpec {
            name: tool_names::DELEGATE_TASK.into(),
            description: concat!(
                "创建一个子智能体执行独立子任务，复杂/多步任务应优先委托。可用角色：",
                "explorer(只读探索)、coder(实现)、general(全能力)、architect(架构)、debugger(诊断)、",
                "reviewer(审查)、writer(文档)、tester(测试)、requirements(需求分析)、vision(看图分析，",
                "无需模型自行具备视觉能力)或自定义角色 ID。",
                "子智能体独立执行并在完成后返回工作总结；同一轮可并行创建多个子智能体。"
            )
            .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "profile": {
                        "description": "子智能体类型：explorer（只读探索）、coder（代码编写）、general（通用全能力）、architect（架构设计）、debugger（问题诊断）、reviewer（代码审查）、writer（文档撰写）、tester（测试执行）、requirements（需求分析）、vision（看图分析），或任意自定义角色 ID"
                    },
                    "task": {
                        "type": "string",
                        "description": "分配给子智能体的完整任务描述。应包含具体目标、预期产出和任何必要的上下文。子智能体没有完整聊天历史，只有此任务描述。"
                    },
                    "context_files": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "子任务相关的初始文件路径列表（相对于项目根目录）。子智能体会先读取这些文件再开始工作。"
                    },
                    "wait": {
                        "type": "boolean",
                        "description": "是否等待子智能体完成（默认 true）。设为 false 时子智能体在后台运行，主 Agent 立即继续；其完成后结果会以系统通知自动注入对话，也可随时用 get_subagent_result 工具按 subagent_id 主动查询或等待结果。后台子智能体始终在共享项目目录工作（不适用 worktree 隔离）。"
                    },
                    "isolation": {
                        "type": "string",
                        "enum": ["worktree", "shared"],
                        "description": "写入类任务的文件隔离方式（仅 wait=true 时生效）。git 项目默认 worktree：子智能体在独立 git worktree+分支上工作，完成后自动合并回当前分支（冲突会报告冲突文件）；shared 表示在共享项目目录工作（有文件锁保护）。按应用配置 subagentIsolationMode 覆盖此默认。注意：worktree 基于主检出当前 HEAD，不含主目录未提交的改动；需要基于未提交改动工作时请传 shared。"
                    }
                },
                "required": ["profile", "task"]
            }),
        },
    }
}

/// Build the ToolSpec for the `delegate_tasks` batch tool.
///
/// Lets the main agent create multiple sub-agents in parallel within a single call.
/// Unlike `delegate_task`, all subtasks start at once and run concurrently.
/// The shared context parameter is injected into every sub-agent's system prompt.
pub fn delegate_tasks_tool_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".into(),
        function: super::models::ToolFunctionSpec {
            name: tool_names::DELEGATE_TASKS.into(),
            description: concat!(
                "批量创建多个子智能体并行执行。所有子任务同时启动，总耗时约等于最慢的子任务；",
                "context 参数注入公共背景，避免每个子任务重复描述；单次调用启动全部子智能体。",
                "适用场景：同时探索多个独立目录、并行实现+测试、多个独立分析任务等彼此独立的子任务组合。",
                "git 项目上写入类任务默认使用独立 worktree 隔离（各自分支，完成后自动合并回当前分支，",
                "冲突会报告冲突文件清单），任务间不会互相覆盖文件；并行写入任务仍应尽量按文件集分工。"
            )
            .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "context": {
                        "type": "string",
                        "description": "所有子任务共享的上下文信息（项目背景、约束、契约等），会注入到每个子智能体的系统提示词中。可选。"
                    },
                    "tasks": {
                        "type": "array",
                        "description": "要并行执行的子任务列表。每个任务独立分配一个子智能体，任务间互不影响。",
                        "items": {
                            "type": "object",
                            "properties": {
                                "profile": {
                                    "type": "string",
                                    "description": "子智能体类型：explorer（只读探索）、coder（代码编写）、general（通用）、architect（架构设计）、debugger（问题诊断）、reviewer（代码审查）、writer（文档撰写）、tester（测试）、requirements（需求分析）、vision（看图分析）或自定义角色 ID"
                                },
                                "task": {
                                    "type": "string",
                                    "description": "分配给该子智能体的完整任务描述"
                                },
                                "context_files": {
                                    "type": "array",
                                    "items": {"type": "string"},
                                    "description": "该子任务相关的初始文件路径列表（相对于项目根目录）"
                                },
                                "isolation": {
                                    "type": "string",
                                    "enum": ["worktree", "shared"],
                                    "description": "该任务的文件隔离方式。git 项目上写入类任务默认 worktree（独立分支，完成后自动合并，冲突会报告）；shared 在共享项目目录工作（有文件锁保护）。按应用配置 subagentIsolationMode 覆盖此默认。注意：worktree 不含主目录未提交的改动；需要基于未提交改动工作时请传 shared。"
                                }
                            },
                            "required": ["profile", "task"]
                        },
                        "minItems": 1,
                        "maxItems": 10
                    }
                },
                "required": ["tasks"]
            }),
        },
    }
}

/// Build the ToolSpec for the `get_subagent_result` tool.
///
/// Lets the main agent query / await the outcome of a background (fire-and-forget)
/// subagent launched via `delegate_task(wait=false)`.
pub fn get_subagent_result_tool_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".into(),
        function: super::models::ToolFunctionSpec {
            name: tool_names::GET_SUBAGENT_RESULT.into(),
            description: concat!(
                "查询/等待后台子智能体的结果（配合 delegate_task(wait=false) 使用）。",
                "wait=true（默认）时阻塞等待其完成并返回结果（超时返回当前状态，可再次调用）；",
                "wait=false 立即返回当前状态。结果在缓存中保留，可多次查询。",
                "注意：后台子智能体完成后其结果也会以系统通知自动注入对话；本工具用于主动等待或在通知被截断时获取完整结果。"
            )
            .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "subagent_id": {
                        "type": "string",
                        "description": "后台子智能体 ID（delegate_task(wait=false) 返回的 ID）"
                    },
                    "wait": {
                        "type": "boolean",
                        "description": "是否阻塞等待完成（默认 true）"
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "等待超时秒数（默认 120，最大 300）"
                    }
                },
                "required": ["subagent_id"]
            }),
        },
    }
}

/// Build the ToolSpec for the `create_workflow` tool.
/// Lets the LLM create a sub-agent workflow that executes sequentially.
/// Use case: complex tasks requiring multiple sub-agents to cooperate in order.
pub fn create_workflow_tool_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".into(),
        function: super::models::ToolFunctionSpec {
            name: tool_names::CREATE_WORKFLOW.into(),
            description: concat!(
                "创建一个按顺序执行的子智能体工作流。\n\n",
                "适用于复杂任务，需要多个子智能体分工协作、按序完成的场景。\n",
                "调用此工具后，系统会按步骤顺序依次执行，每个步骤的输出会自动传递到下一步作为上下文。\n\n",
                "典型工作流序列示例：\n",
                "- requirements → architect → coder → reviewer → tester（全流程开发）\n",
                "- explorer → coder（探索 + 实现）\n",
                "- debugger → coder（诊断 + 修复）\n",
                "- explorer → writer（探索 + 文档化）\n\n",
                "注意：每个步骤是**顺序执行**的（非并行），前一步完成后下一步才开始；",
                "步骤数上限 20，超出会被拒绝，复杂任务请拆分为多个工作流。",
            )
            .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "工作流的标题/目的简述"
                    },
                    "steps": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "profile": {
                                    "type": "string",
                                    "description": "子智能体类型：explorer、coder、general、architect、debugger、reviewer、writer、tester、requirements、vision，或任意自定义角色 ID"
                                },
                                "name": {
                                    "type": "string",
                                    "description": "步骤的简要名称（如\"代码探索\"、\"实现登录模块\"）"
                                },
                                "task": {
                                    "type": "string",
                                    "description": "分配给该子智能体执行的任务描述。应包含具体目标、预期产出和必要上下文。"
                                }
                            },
                            "required": ["profile", "name", "task"]
                        },
                        "description": "工作流步骤列表（按顺序执行）",
                        "minItems": 2
                    }
                },
                "required": ["title", "steps"]
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_explorer_profile_tool_restrictions() {
        let explorer = find_profile("explorer", &[]).unwrap();
        assert!(explorer.is_tool_allowed(tool_names::READ_FILE));
        assert!(explorer.is_tool_allowed("search_files"));
        assert!(explorer.is_tool_allowed(tool_names::WEB_SEARCH));
        assert!(!explorer.is_tool_allowed(tool_names::WRITE_FILE));
        assert!(!explorer.is_tool_allowed(tool_names::DELETE_FILE));
        assert!(!explorer.is_tool_allowed(tool_names::EXECUTE_COMMAND));
        assert!(!explorer.is_tool_allowed("git_commit"));
        assert!(!explorer.is_tool_allowed(tool_names::DELEGATE_TASK));
    }

    #[test]
    fn test_coder_profile_tool_restrictions() {
        let coder = find_profile("coder", &[]).unwrap();
        assert!(coder.is_tool_allowed(tool_names::READ_FILE));
        assert!(coder.is_tool_allowed(tool_names::WRITE_FILE));
        assert!(!coder.is_tool_allowed(tool_names::EXECUTE_COMMAND));
        assert!(!coder.is_tool_allowed(tool_names::DELEGATE_TASK));
    }

    #[test]
    fn test_general_profile_no_recursion() {
        let general = find_profile("general", &[]).unwrap();
        assert!(general.is_tool_allowed(tool_names::READ_FILE));
        assert!(general.is_tool_allowed(tool_names::EXECUTE_COMMAND));
        assert!(!general.is_tool_allowed(tool_names::DELEGATE_TASK));
    }

    #[test]
    fn test_architect_profile_read_only() {
        let architect = find_profile("architect", &[]).unwrap();
        assert!(architect.is_tool_allowed(tool_names::READ_FILE));
        assert!(architect.is_tool_allowed(tool_names::WEB_SEARCH));
        assert!(!architect.is_tool_allowed(tool_names::WRITE_FILE));
        assert!(!architect.is_tool_allowed(tool_names::EXECUTE_COMMAND));
        assert!(!architect.is_tool_allowed(tool_names::DELEGATE_TASK));
    }

    #[test]
    fn test_debugger_profile_can_execute() {
        let debugger = find_profile("debugger", &[]).unwrap();
        assert!(debugger.is_tool_allowed(tool_names::READ_FILE));
        assert!(debugger.is_tool_allowed(tool_names::EXECUTE_COMMAND));
        assert!(!debugger.is_tool_allowed(tool_names::WRITE_FILE));
        assert!(!debugger.is_tool_allowed(tool_names::DELEGATE_TASK));
    }

    #[test]
    fn test_reviewer_profile_read_only() {
        let reviewer = find_profile("reviewer", &[]).unwrap();
        assert!(reviewer.is_tool_allowed(tool_names::READ_FILE));
        assert!(reviewer.is_tool_allowed("read_lints"));
        assert!(!reviewer.is_tool_allowed(tool_names::WRITE_FILE));
        assert!(!reviewer.is_tool_allowed(tool_names::EXECUTE_COMMAND));
        assert!(!reviewer.is_tool_allowed(tool_names::DELEGATE_TASK));
    }

    #[test]
    fn test_writer_profile_can_write() {
        let writer = find_profile("writer", &[]).unwrap();
        assert!(writer.is_tool_allowed(tool_names::READ_FILE));
        assert!(writer.is_tool_allowed(tool_names::WRITE_FILE));
        assert!(writer.is_tool_allowed(tool_names::REPLACE_IN_FILE));
        assert!(!writer.is_tool_allowed(tool_names::EXECUTE_COMMAND));
        assert!(!writer.is_tool_allowed(tool_names::DELETE_FILE));
        assert!(!writer.is_tool_allowed(tool_names::DELEGATE_TASK));
    }

    #[test]
    fn test_tester_profile_can_execute() {
        let tester = find_profile("tester", &[]).unwrap();
        assert!(tester.is_tool_allowed(tool_names::READ_FILE));
        assert!(tester.is_tool_allowed(tool_names::WRITE_FILE));
        assert!(tester.is_tool_allowed(tool_names::EXECUTE_COMMAND));
        assert!(!tester.is_tool_allowed(tool_names::DELETE_FILE));
        assert!(!tester.is_tool_allowed(tool_names::DELEGATE_TASK));
    }

    #[test]
    fn test_requirements_profile_read_only() {
        let requirements = find_profile("requirements", &[]).unwrap();
        assert!(requirements.is_tool_allowed(tool_names::READ_FILE));
        assert!(requirements.is_tool_allowed("search_files"));
        assert!(requirements.is_tool_allowed(tool_names::SEARCH_CONTENT));
        assert!(requirements.is_tool_allowed("read_lints"));
        assert!(!requirements.is_tool_allowed(tool_names::WRITE_FILE));
        assert!(!requirements.is_tool_allowed(tool_names::REPLACE_IN_FILE));
        assert!(!requirements.is_tool_allowed(tool_names::DELETE_FILE));
        assert!(!requirements.is_tool_allowed(tool_names::EXECUTE_COMMAND));
        assert!(!requirements.is_tool_allowed(tool_names::WEB_SEARCH));
        assert!(!requirements.is_tool_allowed(tool_names::WEB_FETCH));
        assert!(!requirements.is_tool_allowed(tool_names::DELEGATE_TASK));
    }
    #[test]
    fn test_glob_match() {
        assert!(simple_glob_match("git_*", "git_status"));
        assert!(simple_glob_match("git_*", "git_commit"));
        assert!(!simple_glob_match("git_*", tool_names::READ_FILE));
        assert!(simple_glob_match("*", "anything"));
        assert!(simple_glob_match("exact", "exact"));
        assert!(!simple_glob_match("exact", "other"));
    }
}

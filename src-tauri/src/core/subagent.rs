//! 子智能体（Sub-agent）配置文件与工具定义。
//!
//! 提供九种内置子智能体配置文件，每种限定不同的工具集和行为模式：
//! - `explorer`：只读代码探索者，用于大规模文件检索和架构分析
//! - `coder`：代码实现者，专注文件编写和修改
//! - `general`：通用子智能体，拥有全部工具能力
//! - `architect`：架构设计师，只读分析依赖与设计决策
//! - `debugger`：问题诊断师，可运行命令查找根因但不能修改文件
//! - `reviewer`：代码审查员，只读安全审计和代码质量评估
//! - `writer`：文档撰写员，可读写文件产出文档
//! - `tester`：测试工程师，可运行测试并编写测试文件
//! - `requirements`：需求分析员，只读分析用户请求并拆解为结构化工作流方案
//!
//! 子智能体通过主 Agent 的 `delegate_task` 工具调用创建，每个子智能体
//! 拥有独立的 LLM 上下文窗口，通过 Tauri 事件向前端报告进度。

use serde::{Deserialize, Serialize};

use super::models::ToolSpec;
/// 子智能体配置文件
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SubagentProfile {
    /// 配置文件唯一标识（"explorer" | "coder" | "general" | "architect" | "debugger" | "reviewer" | "writer" | "tester"）
    pub id: String,
    /// 显示名称
    pub name: String,
    /// 用途说明
    pub description: String,
    /// 允许的工具列表（glob 模式），空数组 = 全部允许
    pub allowed_tools: Vec<String>,
    /// 禁止的工具列表（glob 模式），空数组 = 无额外禁止
    pub denied_tools: Vec<String>,
    /// 附加的系统提示词（追加到子 Agent 基础系统提示词之后）
    pub system_prompt_extension: String,
    /// 可选的模型覆盖（使用更便宜的模型执行子任务），None = 跟随主 Agent 模型
    pub model_override: Option<String>,
}

impl SubagentProfile {
    /// 检查指定工具是否被该配置文件允许
    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        // denied 优先
        if self
            .denied_tools
            .iter()
            .any(|pattern| simple_glob_match(pattern, tool_name))
        {
            return false;
        }
        // allowed 为空 = 全部允许
        if self.allowed_tools.is_empty() {
            return true;
        }
        self.allowed_tools
            .iter()
            .any(|pattern| simple_glob_match(pattern, tool_name))
    }
}

/// 简易 glob 匹配（支持 `*` 后缀通配，如 `git_*` 匹配所有 git 工具）
fn simple_glob_match(pattern: &str, target: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return target.starts_with(prefix);
    }
    pattern == target
}

/// 返回所有内置子智能体配置文件
pub fn builtin_profiles() -> Vec<SubagentProfile> {
    vec![
        SubagentProfile {
            id: "explorer".into(),
            name: "代码探索者".into(),
            description: "只读搜索和分析，用于大规模代码探索、多文件检索、架构分析。不能修改任何文件。".into(),
            allowed_tools: vec![
                "read_file".into(),
                "list_directory".into(),
                "search_files".into(),
                "search_content".into(),
                "read_lints".into(),
                "web_search".into(),
                "web_fetch".into(),
            ],
            denied_tools: vec![
                "write_file".into(),
                "replace_in_file".into(),
                "delete_file".into(),
                "make_directory".into(),
                "execute_command".into(),
                "git_*".into(),
                "delegate_task".into(),
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
            name: "代码实现者".into(),
            description: "代码编写和修改，用于实现具体功能模块。禁止执行 shell 命令。".into(),
            allowed_tools: vec![], // 空 = 全部允许
            denied_tools: vec!["execute_command".into(), "delegate_task".into()],
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
            name: "通用子智能体".into(),
            description: "全能力子智能体，拥有和主 Agent 完全相同的工具集。".into(),
            allowed_tools: vec![],
            denied_tools: vec!["delegate_task".into()], // 禁止递归创建子智能体
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
            name: "架构设计师".into(),
            description: "只读架构分析、依赖映射、设计决策评估与技术选型建议。不能修改任何文件。".into(),
            allowed_tools: vec![
                "read_file".into(),
                "list_directory".into(),
                "search_files".into(),
                "search_content".into(),
                "read_lints".into(),
                "web_search".into(),
                "web_fetch".into(),
            ],
            denied_tools: vec![
                "write_file".into(),
                "replace_in_file".into(),
                "delete_file".into(),
                "make_directory".into(),
                "execute_command".into(),
                "git_*".into(),
                "delegate_task".into(),
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
            name: "问题诊断师".into(),
            description: "错误调查与根因分析，可运行命令复现问题但不能修改任何文件。".into(),
            allowed_tools: vec![
                "read_file".into(),
                "list_directory".into(),
                "search_files".into(),
                "search_content".into(),
                "read_lints".into(),
                "execute_command".into(),
                "web_search".into(),
                "web_fetch".into(),
            ],
            denied_tools: vec![
                "write_file".into(),
                "replace_in_file".into(),
                "delete_file".into(),
                "make_directory".into(),
                "git_*".into(),
                "delegate_task".into(),
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
            name: "代码审查员".into(),
            description: "只读代码质量评估、安全审计与最佳实践检查。不能修改任何文件。".into(),
            allowed_tools: vec![
                "read_file".into(),
                "list_directory".into(),
                "search_files".into(),
                "search_content".into(),
                "read_lints".into(),
                "web_search".into(),
                "web_fetch".into(),
            ],
            denied_tools: vec![
                "write_file".into(),
                "replace_in_file".into(),
                "delete_file".into(),
                "make_directory".into(),
                "execute_command".into(),
                "git_*".into(),
                "delegate_task".into(),
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
            name: "文档撰写员".into(),
            description: "编写文档、注释、README、变更日志与技术规范。可读写文件。".into(),
            allowed_tools: vec![], // 空 = 全部允许
            denied_tools: vec![
                "delete_file".into(),
                "make_directory".into(),
                "execute_command".into(),
                "git_*".into(),
                "delegate_task".into(),
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
            name: "测试工程师".into(),
            description: "测试用例生成、覆盖率分析与测试执行。可运行测试并编写测试文件。".into(),
            allowed_tools: vec![], // 空 = 全部允许
            denied_tools: vec![
                "delete_file".into(),
                "make_directory".into(),
                "git_*".into(),
                "delegate_task".into(),
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
            name: "需求分析员".into(),
            description: "用户需求分析，将用户输入拆解为结构化工作流方案。不能修改文件。".into(),
            allowed_tools: vec![
                "read_file".into(),
                "list_directory".into(),
                "search_files".into(),
                "search_content".into(),
                "read_lints".into(),
            ],
            denied_tools: vec![
                "write_file".into(),
                "replace_in_file".into(),
                "delete_file".into(),
                "make_directory".into(),
                "execute_command".into(),
                "git_*".into(),
                "delegate_task".into(),
                "web_search".into(),
                "web_fetch".into(),
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
    ]
}

/// 按 ID 查找配置文件（先查内置，再查自定义）
pub fn find_profile(profile_id: &str, custom_profiles: &[crate::core::models::CustomSubagentProfile]) -> Option<SubagentProfile> {
    // 先查内置
    if let Some(p) = builtin_profiles().into_iter().find(|p| p.id == profile_id) {
        return Some(p);
    }
    // 再查自定义
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
            });
        }
    }
    None
}

/// 列出所有可用配置文件的摘要信息（供前端使用）
pub fn list_profiles_summary() -> Vec<SubagentProfileSummary> {
    builtin_profiles()
        .into_iter()
        .map(|p| SubagentProfileSummary {
            id: p.id,
            name: p.name,
            description: p.description,
        })
        .collect()
}

/// 配置文件摘要（轻量版，供前端展示）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SubagentProfileSummary {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// 构造 `delegate_task` 工具的 ToolSpec
pub fn delegate_task_tool_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".into(),
        function: super::models::ToolFunctionSpec {
            name: "delegate_task".into(),
            description: concat!(
                "【推荐方式】创建一个子智能体来执行独立的子任务。收到复杂任务时应优先使用此工具。\n\n",
                "当你面对复杂任务时，可以将任务拆分为子任务并委托给子智能体：\n",
                "- 使用 explorer 子智能体搜索和探索代码库\n",
                "- 使用 coder 子智能体编写或修改具体的代码文件\n",
                "- 使用 general 子智能体处理需要完整能力的子任务\n",
                "- 使用 architect 子智能体进行架构分析和设计评估\n",
                "- 使用 debugger 子智能体进行错误调查和根因分析\n",
                "- 使用 reviewer 子智能体进行代码审查和质量评估\n",
                "- 使用 writer 子智能体撰写文档和注释\n",
                "- 使用 tester 子智能体生成和执行测试\n",
                "- 使用 requirements 子智能体分析用户需求并拆解任务\n\n",
                "也可以使用用户自定义的子智能体角色 ID。\n",
                "子智能体会独立执行并在完成后返回工作总结。你可以在同一轮中并行创建多个子智能体。"
            )
            .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "profile": {
                        "description": "子智能体类型：explorer（只读探索）、coder（代码编写）、general（通用全能力）、architect（架构设计）、debugger（问题诊断）、reviewer（代码审查）、writer（文档撰写）、tester（测试执行）、requirements（需求分析），或任意自定义角色 ID"
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
                        "description": "是否等待子智能体完成（默认 true）。设为 false 时子智能体在后台运行，主 Agent 立即继续。"
                    }
                },
                "required": ["profile", "task"]
            }),
        },
    }
}

/// 构造 `create_workflow` 工具的 ToolSpec。
/// 此工具允许 LLM 创建一个按顺序执行的子智能体工作流。
/// 适用场景：复杂任务需要多个子智能体分工协作、按序完成。
pub fn create_workflow_tool_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".into(),
        function: super::models::ToolFunctionSpec {
            name: "create_workflow".into(),
            description: concat!(
                "创建一个按顺序执行的子智能体工作流。\n\n",
                "适用于复杂任务，需要多个子智能体分工协作、按序完成的场景。\n",
                "调用此工具后，系统会按步骤顺序依次执行，每个步骤的输出会自动传递到下一步作为上下文。\n\n",
                "典型工作流序列示例：\n",
                "- requirements → architect → coder → reviewer → tester（全流程开发）\n",
                "- explorer → coder（探索 + 实现）\n",
                "- debugger → coder（诊断 + 修复）\n",
                "- explorer → writer（探索 + 文档化）\n\n",
                "注意：每个步骤是**顺序执行**的（非并行），前一步完成后下一步才开始。",
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
                                    "description": "子智能体类型：explorer、coder、general、architect、debugger、reviewer、writer、tester、requirements，或任意自定义角色 ID"
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
                        "min_items": 2
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
        assert!(explorer.is_tool_allowed("read_file"));
        assert!(explorer.is_tool_allowed("search_files"));
        assert!(explorer.is_tool_allowed("web_search"));
        assert!(!explorer.is_tool_allowed("write_file"));
        assert!(!explorer.is_tool_allowed("delete_file"));
        assert!(!explorer.is_tool_allowed("execute_command"));
        assert!(!explorer.is_tool_allowed("git_commit"));
        assert!(!explorer.is_tool_allowed("delegate_task"));
    }

    #[test]
    fn test_coder_profile_tool_restrictions() {
        let coder = find_profile("coder", &[]).unwrap();
        assert!(coder.is_tool_allowed("read_file"));
        assert!(coder.is_tool_allowed("write_file"));
        assert!(!coder.is_tool_allowed("execute_command"));
        assert!(!coder.is_tool_allowed("delegate_task"));
    }

    #[test]
    fn test_general_profile_no_recursion() {
        let general = find_profile("general", &[]).unwrap();
        assert!(general.is_tool_allowed("read_file"));
        assert!(general.is_tool_allowed("execute_command"));
        assert!(!general.is_tool_allowed("delegate_task"));
    }

    #[test]
    fn test_architect_profile_read_only() {
        let architect = find_profile("architect", &[]).unwrap();
        assert!(architect.is_tool_allowed("read_file"));
        assert!(architect.is_tool_allowed("web_search"));
        assert!(!architect.is_tool_allowed("write_file"));
        assert!(!architect.is_tool_allowed("execute_command"));
        assert!(!architect.is_tool_allowed("delegate_task"));
    }

    #[test]
    fn test_debugger_profile_can_execute() {
        let debugger = find_profile("debugger", &[]).unwrap();
        assert!(debugger.is_tool_allowed("read_file"));
        assert!(debugger.is_tool_allowed("execute_command"));
        assert!(!debugger.is_tool_allowed("write_file"));
        assert!(!debugger.is_tool_allowed("delegate_task"));
    }

    #[test]
    fn test_reviewer_profile_read_only() {
        let reviewer = find_profile("reviewer", &[]).unwrap();
        assert!(reviewer.is_tool_allowed("read_file"));
        assert!(reviewer.is_tool_allowed("read_lints"));
        assert!(!reviewer.is_tool_allowed("write_file"));
        assert!(!reviewer.is_tool_allowed("execute_command"));
        assert!(!reviewer.is_tool_allowed("delegate_task"));
    }

    #[test]
    fn test_writer_profile_can_write() {
        let writer = find_profile("writer", &[]).unwrap();
        assert!(writer.is_tool_allowed("read_file"));
        assert!(writer.is_tool_allowed("write_file"));
        assert!(writer.is_tool_allowed("replace_in_file"));
        assert!(!writer.is_tool_allowed("execute_command"));
        assert!(!writer.is_tool_allowed("delete_file"));
        assert!(!writer.is_tool_allowed("delegate_task"));
    }

    #[test]
    fn test_tester_profile_can_execute() {
        let tester = find_profile("tester", &[]).unwrap();
        assert!(tester.is_tool_allowed("read_file"));
        assert!(tester.is_tool_allowed("write_file"));
        assert!(tester.is_tool_allowed("execute_command"));
        assert!(!tester.is_tool_allowed("delete_file"));
        assert!(!tester.is_tool_allowed("delegate_task"));
    }


    #[test]
    fn test_requirements_profile_read_only() {
        let requirements = find_profile("requirements", &[]).unwrap();
        assert!(requirements.is_tool_allowed("read_file"));
        assert!(requirements.is_tool_allowed("search_files"));
        assert!(requirements.is_tool_allowed("search_content"));
        assert!(requirements.is_tool_allowed("read_lints"));
        assert!(!requirements.is_tool_allowed("write_file"));
        assert!(!requirements.is_tool_allowed("replace_in_file"));
        assert!(!requirements.is_tool_allowed("delete_file"));
        assert!(!requirements.is_tool_allowed("execute_command"));
        assert!(!requirements.is_tool_allowed("web_search"));
        assert!(!requirements.is_tool_allowed("web_fetch"));
        assert!(!requirements.is_tool_allowed("delegate_task"));
    }
    #[test]
    fn test_glob_match() {
        assert!(simple_glob_match("git_*", "git_status"));
        assert!(simple_glob_match("git_*", "git_commit"));
        assert!(!simple_glob_match("git_*", "read_file"));
        assert!(simple_glob_match("*", "anything"));
        assert!(simple_glob_match("exact", "exact"));
        assert!(!simple_glob_match("exact", "other"));
    }
}

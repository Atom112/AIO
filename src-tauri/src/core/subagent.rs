//! 子智能体（Sub-agent）配置文件与工具定义。
//!
//! 提供三种内置子智能体配置文件，每种限定不同的工具集和行为模式：
//! - `explorer`：只读代码探索者，用于大规模文件检索和架构分析
//! - `coder`：代码实现者，专注文件编写和修改
//! - `general`：通用子智能体，拥有全部工具能力
//!
//! 子智能体通过主 Agent 的 `delegate_task` 工具调用创建，每个子智能体
//! 拥有独立的 LLM 上下文窗口，通过 Tauri 事件向前端报告进度。

use serde::{Deserialize, Serialize};

use super::models::ToolSpec;

/// 子智能体配置文件
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SubagentProfile {
    /// 配置文件唯一标识（"explorer" | "coder" | "general"）
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
    ]
}

/// 按 ID 查找配置文件
pub fn find_profile(profile_id: &str) -> Option<SubagentProfile> {
    builtin_profiles().into_iter().find(|p| p.id == profile_id)
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
                "创建一个子智能体来执行独立的子任务。\n\n",
                "当你面对复杂任务时，可以将任务拆分为子任务并委托给子智能体：\n",
                "- 使用 explorer 子智能体搜索和探索代码库\n",
                "- 使用 coder 子智能体编写或修改具体的代码文件\n",
                "- 使用 general 子智能体处理需要完整能力的子任务\n\n",
                "子智能体会独立执行并在完成后返回工作总结。你可以在同一轮中并行创建多个子智能体。"
            )
            .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "profile": {
                        "type": "string",
                        "enum": ["explorer", "coder", "general"],
                        "description": "子智能体类型：explorer（只读探索）、coder（代码编写）、general（通用全能力）"
                    },
                    "task": {
                        "type": "string",
                        "description": "分配给子智能体的完整任务描述。应包含具体目标、预期产出和任何必要的上下文。子智能体没有完整聊天历史，只有此任务描述。"
                    },
                    "context_files": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "子任务相关的初始文件路径列表（相对于项目根目录）。子智能体会先读取这些文件再开始工作。"
                    }
                },
                "required": ["profile", "task"]
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_explorer_profile_tool_restrictions() {
        let explorer = find_profile("explorer").unwrap();
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
        let coder = find_profile("coder").unwrap();
        assert!(coder.is_tool_allowed("read_file"));
        assert!(coder.is_tool_allowed("write_file"));
        assert!(!coder.is_tool_allowed("execute_command"));
        assert!(!coder.is_tool_allowed("delegate_task"));
    }

    #[test]
    fn test_general_profile_no_recursion() {
        let general = find_profile("general").unwrap();
        assert!(general.is_tool_allowed("read_file"));
        assert!(general.is_tool_allowed("execute_command"));
        assert!(!general.is_tool_allowed("delegate_task"));
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

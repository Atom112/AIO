//! Agent 工具调用权限规则引擎。
//!
//! 实现 allow / ask / deny 三层权限模型，参考 Claude Code 的权限设计。
//! 规则按优先级评估，Deny 总是优先于 Allow，无匹配规则时默认 Ask（fail-closed）。
//!
//! 规则可自定义存储在项目 `.aio/permissions.json` 中，覆盖内置默认配置。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::models::AgentMode;

// ====== 权限动作 ======

/// 权限决策结果
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PermissionAction {
    /// 自动允许，无需确认
    Allow,
    /// 需要用户确认
    Ask,
    /// 直接拒绝
    Deny,
}

// ====== 规则定义 ======

/// 一条权限规则
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRule {
    /// 规则唯一 ID
    pub id: String,
    /// 工具名称或 glob 模式，如 `"write_file"`, `"delete_*"`, `"*"`（全部匹配）
    pub tool_pattern: String,
    /// 限制到特定 MCP server ID，`None` = 应用于所有 server
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
    /// 该规则适用的 agent 模式列表，空列表 = 所有模式
    #[serde(default)]
    pub modes: Vec<AgentMode>,
    /// 决策
    pub action: PermissionAction,
    /// 可选的路径 glob 匹配（针对 filesystem 类工具的 `path` 参数值）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_pattern: Option<String>,
    /// 优先级，越大越优先。Deny 类型的规则即使优先级低也优先于 Allow
    #[serde(default)]
    pub priority: i32,
}

// ====== 持久化文件 ======

/// 权限配置持久化文件（`.aio/permissions.json`）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PermissionsFile {
    pub version: u32,
    pub updated_at: String,
    /// 自定义规则列表（覆盖内置默认规则）
    #[serde(default)]
    pub rules: Vec<PermissionRule>,
}

impl Default for PermissionsFile {
    fn default() -> Self {
        Self {
            version: 1,
            updated_at: String::new(),
            rules: Vec::new(),
        }
    }
}

/// 工具分类（用于内置默认规则）
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolCategory {
    /// 读操作：read_file, list_directory, search_files, search_content
    Read,
    /// 写操作：write_file, make_directory
    Write,
    /// 删除操作：delete_file
    Delete,
    /// 其他 MCP 工具
    Other,
}

#[allow(dead_code)]
fn categorize_tool(name: &str) -> ToolCategory {
    match name {
        "read_file" | "list_directory" | "search_files" | "search_content" | "web_fetch" | "web_search" | "git_status" | "git_diff" | "git_log" => ToolCategory::Read,
        "write_file" | "make_directory" | "replace_in_file" | "git_add" | "git_commit" => ToolCategory::Write,
        "delete_file" => ToolCategory::Delete,
        _ => ToolCategory::Other,
    }
}

// ====== 内置默认规则 ======

/// 根据 mode 返回内置默认规则集。
/// 自定义规则（来自 `.aio/permissions.json`）会叠加/覆盖这些默认规则。
pub fn default_rules_for_mode(mode: &AgentMode) -> Vec<PermissionRule> {
    match mode {
        AgentMode::Off => vec![
            // Off 模式不应有工具调用，全部拒绝
            PermissionRule {
                id: "builtin-off-deny-all".into(),
                tool_pattern: "*".into(),
                server_id: None,
                modes: vec![AgentMode::Off],
                action: PermissionAction::Deny,
                path_pattern: None,
                priority: 100,
            },
        ],

        AgentMode::Normal => vec![
            // 读操作：自动允许
            PermissionRule {
                id: "builtin-normal-allow-read".into(),
                tool_pattern: "read_file".into(),
                server_id: None,
                modes: vec![AgentMode::Normal],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            PermissionRule {
                id: "builtin-normal-allow-list-dir".into(),
                tool_pattern: "list_directory".into(),
                server_id: None,
                modes: vec![AgentMode::Normal],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            PermissionRule {
                id: "builtin-normal-allow-search".into(),
                tool_pattern: "search_*".into(),
                server_id: None,
                modes: vec![AgentMode::Normal],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            // 写操作：需要确认
            PermissionRule {
                id: "builtin-normal-ask-write".into(),
                tool_pattern: "write_file".into(),
                server_id: None,
                modes: vec![AgentMode::Normal],
                action: PermissionAction::Ask,
                path_pattern: None,
                priority: 10,
            },
            PermissionRule {
                id: "builtin-normal-ask-replace".into(),
                tool_pattern: "replace_in_file".into(),
                server_id: None,
                modes: vec![AgentMode::Normal],
                action: PermissionAction::Ask,
                path_pattern: None,
                priority: 10,
            },
            // Git 读操作：自动允许
            PermissionRule {
                id: "builtin-normal-allow-git-read".into(),
                tool_pattern: "git_status".into(),
                server_id: None,
                modes: vec![AgentMode::Normal],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            PermissionRule {
                id: "builtin-normal-allow-git-diff".into(),
                tool_pattern: "git_diff".into(),
                server_id: None,
                modes: vec![AgentMode::Normal],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            PermissionRule {
                id: "builtin-normal-allow-git-log".into(),
                tool_pattern: "git_log".into(),
                server_id: None,
                modes: vec![AgentMode::Normal],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            // Git 写操作：需要确认
            PermissionRule {
                id: "builtin-normal-ask-git-add".into(),
                tool_pattern: "git_add".into(),
                server_id: None,
                modes: vec![AgentMode::Normal],
                action: PermissionAction::Ask,
                path_pattern: None,
                priority: 10,
            },
            PermissionRule {
                id: "builtin-normal-ask-git-commit".into(),
                tool_pattern: "git_commit".into(),
                server_id: None,
                modes: vec![AgentMode::Normal],
                action: PermissionAction::Ask,
                path_pattern: None,
                priority: 10,
            },
            PermissionRule {
                id: "builtin-normal-ask-mkdir".into(),
                tool_pattern: "make_directory".into(),
                server_id: None,
                modes: vec![AgentMode::Normal],
                action: PermissionAction::Ask,
                path_pattern: None,
                priority: 10,
            },
            // 删除操作：需要确认
            PermissionRule {
                id: "builtin-normal-ask-delete".into(),
                tool_pattern: "delete_file".into(),
                server_id: None,
                modes: vec![AgentMode::Normal],
                action: PermissionAction::Ask,
                path_pattern: None,
                priority: 10,
            },
            // 其他 MCP 工具（非内置 filesystem）：需要确认
            PermissionRule {
                id: "builtin-normal-ask-other-tools".into(),
                tool_pattern: "*".into(),
                server_id: None,
                modes: vec![AgentMode::Normal],
                action: PermissionAction::Ask,
                path_pattern: None,
                priority: 0,
            },
        ],

        AgentMode::Auto => vec![
            // 所有内置 filesystem 操作：自动允许
            PermissionRule {
                id: "builtin-auto-allow-fs-read".into(),
                tool_pattern: "read_file".into(),
                server_id: None,
                modes: vec![AgentMode::Auto],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            PermissionRule {
                id: "builtin-auto-allow-fs-list".into(),
                tool_pattern: "list_directory".into(),
                server_id: None,
                modes: vec![AgentMode::Auto],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            PermissionRule {
                id: "builtin-auto-allow-fs-search".into(),
                tool_pattern: "search_*".into(),
                server_id: None,
                modes: vec![AgentMode::Auto],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            PermissionRule {
                id: "builtin-auto-allow-fs-write".into(),
                tool_pattern: "write_file".into(),
                server_id: None,
                modes: vec![AgentMode::Auto],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            PermissionRule {
                id: "builtin-auto-allow-fs-replace".into(),
                tool_pattern: "replace_in_file".into(),
                server_id: None,
                modes: vec![AgentMode::Auto],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            // Git 工具：全部自动允许（Auto 模式的前提是用户已信任所有操作）
            PermissionRule {
                id: "builtin-auto-allow-git-status".into(),
                tool_pattern: "git_status".into(),
                server_id: None,
                modes: vec![AgentMode::Auto],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            PermissionRule {
                id: "builtin-auto-allow-git-diff".into(),
                tool_pattern: "git_diff".into(),
                server_id: None,
                modes: vec![AgentMode::Auto],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            PermissionRule {
                id: "builtin-auto-allow-git-log".into(),
                tool_pattern: "git_log".into(),
                server_id: None,
                modes: vec![AgentMode::Auto],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            PermissionRule {
                id: "builtin-auto-allow-git-add".into(),
                tool_pattern: "git_add".into(),
                server_id: None,
                modes: vec![AgentMode::Auto],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            PermissionRule {
                id: "builtin-auto-allow-git-commit".into(),
                tool_pattern: "git_commit".into(),
                server_id: None,
                modes: vec![AgentMode::Auto],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            PermissionRule {
                id: "builtin-auto-allow-fs-mkdir".into(),
                tool_pattern: "make_directory".into(),
                server_id: None,
                modes: vec![AgentMode::Auto],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            // 删除操作：需要确认（危险操作）
            PermissionRule {
                id: "builtin-auto-ask-delete".into(),
                tool_pattern: "delete_file".into(),
                server_id: None,
                modes: vec![AgentMode::Auto],
                action: PermissionAction::Ask,
                path_pattern: None,
                priority: 10,
            },
            // 命令执行：自动允许（Auto 模式下无需逐条确认，进入前已有风险提醒）
            PermissionRule {
                id: "builtin-auto-allow-command".into(),
                tool_pattern: "execute_command".into(),
                server_id: None,
                modes: vec![AgentMode::Auto],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            // 其他 MCP 工具：自动允许
            PermissionRule {
                id: "builtin-auto-allow-other-tools".into(),
                tool_pattern: "*".into(),
                server_id: None,
                modes: vec![AgentMode::Auto],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 0,
            },
        ],

        AgentMode::Plan => vec![
            // Plan 模式：只读，所有写/删操作拒绝
            PermissionRule {
                id: "builtin-plan-allow-read".into(),
                tool_pattern: "read_file".into(),
                server_id: None,
                modes: vec![AgentMode::Plan],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            PermissionRule {
                id: "builtin-plan-allow-list-dir".into(),
                tool_pattern: "list_directory".into(),
                server_id: None,
                modes: vec![AgentMode::Plan],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            PermissionRule {
                id: "builtin-plan-allow-search".into(),
                tool_pattern: "search_*".into(),
                server_id: None,
                modes: vec![AgentMode::Plan],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            // 写/删操作：拒绝
            PermissionRule {
                id: "builtin-plan-deny-write".into(),
                tool_pattern: "write_file".into(),
                server_id: None,
                modes: vec![AgentMode::Plan],
                action: PermissionAction::Deny,
                path_pattern: None,
                priority: 20,
            },
            PermissionRule {
                id: "builtin-plan-deny-replace".into(),
                tool_pattern: "replace_in_file".into(),
                server_id: None,
                modes: vec![AgentMode::Plan],
                action: PermissionAction::Deny,
                path_pattern: None,
                priority: 20,
            },
            PermissionRule {
                id: "builtin-plan-deny-mkdir".into(),
                tool_pattern: "make_directory".into(),
                server_id: None,
                modes: vec![AgentMode::Plan],
                action: PermissionAction::Deny,
                path_pattern: None,
                priority: 20,
            },
            PermissionRule {
                id: "builtin-plan-deny-delete".into(),
                tool_pattern: "delete_file".into(),
                server_id: None,
                modes: vec![AgentMode::Plan],
                action: PermissionAction::Deny,
                path_pattern: None,
                priority: 20,
            },
            // 禁止 Web 访问
            PermissionRule {
                id: "builtin-plan-deny-web-fetch".into(),
                tool_pattern: "web_fetch".into(),
                server_id: None,
                modes: vec![AgentMode::Plan],
                action: PermissionAction::Deny,
                path_pattern: None,
                priority: 20,
            },
            PermissionRule {
                id: "builtin-plan-deny-web-search".into(),
                tool_pattern: "web_search".into(),
                server_id: None,
                modes: vec![AgentMode::Plan],
                action: PermissionAction::Deny,
                path_pattern: None,
                priority: 20,
            },
            // Git 读操作：允许（Plan 阶段可以查看历史和状态）
            PermissionRule {
                id: "builtin-plan-allow-git-status".into(),
                tool_pattern: "git_status".into(),
                server_id: None,
                modes: vec![AgentMode::Plan],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            PermissionRule {
                id: "builtin-plan-allow-git-diff".into(),
                tool_pattern: "git_diff".into(),
                server_id: None,
                modes: vec![AgentMode::Plan],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            PermissionRule {
                id: "builtin-plan-allow-git-log".into(),
                tool_pattern: "git_log".into(),
                server_id: None,
                modes: vec![AgentMode::Plan],
                action: PermissionAction::Allow,
                path_pattern: None,
                priority: 10,
            },
            // Git 写操作：拒绝（Plan 模式不修改文件）
            PermissionRule {
                id: "builtin-plan-deny-git-add".into(),
                tool_pattern: "git_add".into(),
                server_id: None,
                modes: vec![AgentMode::Plan],
                action: PermissionAction::Deny,
                path_pattern: None,
                priority: 20,
            },
            PermissionRule {
                id: "builtin-plan-deny-git-commit".into(),
                tool_pattern: "git_commit".into(),
                server_id: None,
                modes: vec![AgentMode::Plan],
                action: PermissionAction::Deny,
                path_pattern: None,
                priority: 20,
            },
            // 其他 MCP 工具需要确认
            PermissionRule {
                id: "builtin-plan-ask-other-tools".into(),
                tool_pattern: "*".into(),
                server_id: None,
                modes: vec![AgentMode::Plan],
                action: PermissionAction::Ask,
                path_pattern: None,
                priority: 0,
            },
        ],
    }
}

// ====== 简单 Glob 匹配 ======

/// 简单的 glob 匹配，仅支持 `*`（匹配任意字符序列）。
/// 不处理 `?` 或 `[...]`。
fn glob_match(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix('*') {
        // 模式以 * 开头：如 `*_file` 匹配任何以 `_file` 结尾的
        return name.ends_with(suffix);
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        // 模式以 * 结尾：如 `search_*` 匹配任何以 `search_` 开头的
        return name.starts_with(prefix);
    }
    // 无通配符：精确匹配
    pattern == name
}

// ====== 路径匹配 ======

/// 检查工具参数中的 path 是否匹配路径模式。
/// 如果规则没有 `path_pattern`，则忽略路径检查。
fn path_matches(rule_path: &str, arguments: &Value) -> bool {
    let arg_path = match arguments.get("path") {
        Some(Value::String(p)) => p,
        _ => return true, // 无 path 参数，忽略路径检查
    };
    glob_match(rule_path, arg_path)
}

// ====== 规则评估 ======

/// 检查一条规则是否匹配当前工具调用。
fn rule_matches(
    rule: &PermissionRule,
    tool_name: &str,
    server_id: &str,
    arguments: &Value,
    agent_mode: &AgentMode,
) -> bool {
    // 1. 检查模式匹配
    if !rule.modes.is_empty() && !rule.modes.contains(agent_mode) {
        return false;
    }
    // 2. 检查 server_id
    if let Some(ref sid) = rule.server_id {
        if sid != server_id {
            return false;
        }
    }
    // 3. 检查工具名称 glob
    if !glob_match(&rule.tool_pattern, tool_name) {
        return false;
    }
    // 4. 检查路径模式
    if let Some(ref pat) = rule.path_pattern {
        if !path_matches(pat, arguments) {
            return false;
        }
    }
    true
}

/// 评估工具调用权限。
///
/// # 参数
/// - `tool_name` — MCP 工具名称
/// - `server_id` — MCP server ID
/// - `arguments` — 工具调用参数（JSON）
/// - `agent_mode` — 当前 agent 模式
/// - `custom_rules` — 用户自定义规则（来自 `.aio/permissions.json`）
///
/// # 返回
/// `PermissionAction::Allow` — 可执行
/// `PermissionAction::Ask` — 需要用户确认
/// `PermissionAction::Deny` — 已拒绝
pub fn check_permission(
    tool_name: &str,
    server_id: &str,
    arguments: &Value,
    agent_mode: &AgentMode,
    custom_rules: &[PermissionRule],
) -> PermissionAction {
    // 1. 收集所有匹配规则（内置 + 自定义）
    let builtin_rules = default_rules_for_mode(agent_mode);
    let mut all_candidates: Vec<&PermissionRule> = Vec::new();

    for rule in &builtin_rules {
        if rule_matches(rule, tool_name, server_id, arguments, agent_mode) {
            all_candidates.push(rule);
        }
    }
    for rule in custom_rules {
        if rule_matches(rule, tool_name, server_id, arguments, agent_mode) {
            all_candidates.push(rule);
        }
    }

    // 2. 按优先级排序（高 → 低）
    all_candidates.sort_by(|a, b| b.priority.cmp(&a.priority));

    // 3. 检查是否有 Deny（Deny 总是优先于 Allow 和 Ask）
    for rule in &all_candidates {
        if rule.action == PermissionAction::Deny {
            return PermissionAction::Deny;
        }
    }

    // 4. 取最高优先级的非 Deny 动作
    if let Some(highest) = all_candidates.first() {
        return highest.action.clone();
    }

    // 5. 无匹配规则：fail-closed → Ask
    PermissionAction::Ask
}

/// 检查工具调用权限（简化版，直接使用默认规则）。
/// 供 `call_mcp_tool` 后端防御调用。
#[allow(dead_code)]
pub fn check_permission_defaults(
    tool_name: &str,
    server_id: &str,
    arguments: &Value,
    agent_mode: &AgentMode,
) -> PermissionAction {
    let custom_rules: Vec<PermissionRule> = Vec::new();
    check_permission(tool_name, server_id, arguments, agent_mode, &custom_rules)
}

// ====== 路径工具 ======

/// 加载项目级权限配置。
/// 如果文件不存在，返回默认空配置。
pub fn load_permissions(project_path: Option<&str>) -> PermissionsFile {
    let path = match project_path {
        Some(p) => std::path::Path::new(p).join(".aio").join("permissions.json"),
        None => return PermissionsFile::default(),
    };
    if !path.exists() {
        return PermissionsFile::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<PermissionsFile>(&content) {
            Ok(f) => f,
            // 解析失败时警告（旧实现静默 unwrap_or_default，用户自定义规则会无声消失）
            Err(e) => {
                tracing::warn!("[permissions] 解析 {} 失败，已回退到空配置：{}", path.display(), e);
                PermissionsFile::default()
            }
        },
        Err(e) => {
            tracing::warn!("[permissions] 读取 {} 失败，已回退到空配置：{}", path.display(), e);
            PermissionsFile::default()
        }
    }
}

/// 保存项目级权限配置。
pub fn save_permissions(project_path: &str, file: &PermissionsFile) -> Result<(), String> {
    let dir = std::path::Path::new(project_path).join(".aio");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建 .aio 目录失败: {}", e))?;
    let path = dir.join("permissions.json");
    let content =
        serde_json::to_string_pretty(file).map_err(|e| format!("序列化权限配置失败: {}", e))?;
    std::fs::write(&path, content).map_err(|e| format!("写入权限配置失败: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_match() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("write_file", "write_file"));
        assert!(!glob_match("write_file", "read_file"));
        assert!(glob_match("search_*", "search_files"));
        assert!(glob_match("search_*", "search_content"));
        assert!(!glob_match("search_*", "read_file"));
        assert!(glob_match("*_file", "write_file"));
        assert!(glob_match("*_file", "delete_file"));
        assert!(!glob_match("*_file", "list_directory"));
    }

    #[test]
    fn test_normal_mode_read_allowed() {
        let args = serde_json::json!({"path": "src/main.rs"});
        let result = check_permission_defaults("read_file", "__aio-filesystem__", &args, &AgentMode::Normal);
        assert_eq!(result, PermissionAction::Allow);
    }

    #[test]
    fn test_normal_mode_write_asks() {
        let args = serde_json::json!({"path": "src/main.rs", "content": "fn main() {}"});
        let result = check_permission_defaults("write_file", "__aio-filesystem__", &args, &AgentMode::Normal);
        assert_eq!(result, PermissionAction::Ask);
    }

    #[test]
    fn test_normal_mode_delete_asks() {
        let args = serde_json::json!({"path": "src/main.rs"});
        let result = check_permission_defaults("delete_file", "__aio-filesystem__", &args, &AgentMode::Normal);
        assert_eq!(result, PermissionAction::Ask);
    }

    #[test]
    fn test_plan_mode_write_denied() {
        let args = serde_json::json!({"path": "src/main.rs", "content": "fn main() {}"});
        let result = check_permission_defaults("write_file", "__aio-filesystem__", &args, &AgentMode::Plan);
        assert_eq!(result, PermissionAction::Deny);
    }

    #[test]
    fn test_auto_mode_write_allowed() {
        let args = serde_json::json!({"path": "src/main.rs", "content": "fn main() {}"});
        let result = check_permission_defaults("write_file", "__aio-filesystem__", &args, &AgentMode::Auto);
        assert_eq!(result, PermissionAction::Allow);
    }

    #[test]
    fn test_auto_mode_delete_asks() {
        let args = serde_json::json!({"path": "src/main.rs"});
        let result = check_permission_defaults("delete_file", "__aio-filesystem__", &args, &AgentMode::Auto);
        assert_eq!(result, PermissionAction::Ask);
    }

    #[test]
    fn test_off_mode_denies_all() {
        let args = serde_json::json!({"path": "src/main.rs"});
        let result = check_permission_defaults("read_file", "__aio-filesystem__", &args, &AgentMode::Off);
        assert_eq!(result, PermissionAction::Deny);
    }

    #[test]
    fn test_custom_rule_overrides_default() {
        let custom = vec![PermissionRule {
            id: "custom-allow-delete".into(),
            tool_pattern: "delete_file".into(),
            server_id: None,
            modes: vec![AgentMode::Normal],
            action: PermissionAction::Allow,
            path_pattern: None,
            priority: 100, // 高于内置（默认 10）
        }];
        let args = serde_json::json!({"path": "src/old.rs"});
        let result = check_permission("delete_file", "__aio-filesystem__", &args, &AgentMode::Normal, &custom);
        assert_eq!(result, PermissionAction::Allow);
    }

    #[test]
    fn test_deny_overrides_allow() {
        let custom = vec![PermissionRule {
            id: "custom-deny-read-dotenv".into(),
            tool_pattern: "read_file".into(),
            server_id: None,
            modes: vec![AgentMode::Auto],
            action: PermissionAction::Deny,
            path_pattern: Some(".env*".into()),
            priority: 5, // 低于内置 allow（10），但 Deny 总是优先
        }];
        let args = serde_json::json!({"path": ".env"});
        let result = check_permission("read_file", "__aio-filesystem__", &args, &AgentMode::Auto, &custom);
        assert_eq!(result, PermissionAction::Deny);
    }

    #[test]
    fn test_no_match_fallbacks_to_ask() {
        // 自定义工具不在内置规则中
        let args = serde_json::json!({"query": "SELECT * FROM users"});
        let result = check_permission_defaults("custom_db_query", "my-db-server", &args, &AgentMode::Normal);
        assert_eq!(result, PermissionAction::Ask);
    }
}

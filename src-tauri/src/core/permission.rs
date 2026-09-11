//! Agent 工具调用权限规则引擎。
//!
//! 实现 allow / ask / deny 三层权限模型，参考 Claude Code 的权限设计。
//! 规则按优先级评估，Deny 总是优先于 Allow，无匹配规则时默认 Ask（fail-closed）。
//!
//! 规则可自定义存储在项目 `.aio/permissions.json` 中，覆盖内置默认配置。

use regex::Regex;
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
    ///
    /// 生效语义：Deny 全局优先（不受 priority 影响）；其余动作按 priority 高→低，
    /// 同优先级时自定义规则胜出。路径规则（path_pattern）支持 path / basePath /
    /// file_path / filePath / url 与 files / paths 数组参数（详见 path_matches）。
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

// （已删除：ToolCategory / categorize_tool —— 零调用的第二张工具分类表，
//   与下方 default_rules_for_mode 的规则表语义重叠且已漂移）

// ====== 内置默认规则 ======

/// 根据 mode 返回内置默认规则集。
/// 自定义规则（来自 `.aio/permissions.json`）会叠加/覆盖这些默认规则。
/// 内置规则构造器：默认 server=None / path_pattern=None / priority=10。
/// 把 8 字段结构体字面量收敛为一行，让模式矩阵可审计。
fn r(
    id: &str,
    tool_pattern: &str,
    modes: &[AgentMode],
    action: PermissionAction,
) -> PermissionRule {
    rf(id, tool_pattern, modes, action, None, None, 10)
}

/// 完整构造器（server / path_pattern / priority 可覆盖）。
#[allow(
    clippy::too_many_arguments,
    reason = "
声明式规则表的单一构造入口
"
)]
fn rf(
    id: &str,
    tool_pattern: &str,
    modes: &[AgentMode],
    action: PermissionAction,
    server_id: Option<&str>,
    path_pattern: Option<&str>,
    priority: i32,
) -> PermissionRule {
    PermissionRule {
        id: id.into(),
        tool_pattern: tool_pattern.into(),
        server_id: server_id.map(Into::into),
        modes: modes.to_vec(),
        action,
        path_pattern: path_pattern.map(Into::into),
        priority,
    }
}

pub fn default_rules_for_mode(mode: &AgentMode) -> Vec<PermissionRule> {
    match mode {
        AgentMode::Off => vec![
            // Off 模式不应有工具调用，全部拒绝
            rf(
                "builtin-off-deny-all",
                "*",
                &[AgentMode::Off],
                PermissionAction::Deny,
                None,
                None,
                100,
            ),
        ],

        AgentMode::Normal => vec![
            // 读操作：自动允许
            r(
                "builtin-normal-allow-read",
                "read_file",
                &[AgentMode::Normal],
                PermissionAction::Allow,
            ),
            r(
                "builtin-normal-allow-list-dir",
                "list_directory",
                &[AgentMode::Normal],
                PermissionAction::Allow,
            ),
            r(
                "builtin-normal-allow-search",
                "search_*",
                &[AgentMode::Normal],
                PermissionAction::Allow,
            ),
            // 写操作：需要确认
            r(
                "builtin-normal-ask-write",
                "write_file",
                &[AgentMode::Normal],
                PermissionAction::Ask,
            ),
            r(
                "builtin-normal-ask-replace",
                "replace_in_file",
                &[AgentMode::Normal],
                PermissionAction::Ask,
            ),
            // Git 读操作：自动允许
            r(
                "builtin-normal-allow-git-read",
                "git_status",
                &[AgentMode::Normal],
                PermissionAction::Allow,
            ),
            r(
                "builtin-normal-allow-git-diff",
                "git_diff",
                &[AgentMode::Normal],
                PermissionAction::Allow,
            ),
            r(
                "builtin-normal-allow-git-log",
                "git_log",
                &[AgentMode::Normal],
                PermissionAction::Allow,
            ),
            // Git 写操作：需要确认
            r(
                "builtin-normal-ask-git-add",
                "git_add",
                &[AgentMode::Normal],
                PermissionAction::Ask,
            ),
            r(
                "builtin-normal-ask-git-commit",
                "git_commit",
                &[AgentMode::Normal],
                PermissionAction::Ask,
            ),
            r(
                "builtin-normal-ask-mkdir",
                "make_directory",
                &[AgentMode::Normal],
                PermissionAction::Ask,
            ),
            // 删除操作：需要确认
            r(
                "builtin-normal-ask-delete",
                "delete_file",
                &[AgentMode::Normal],
                PermissionAction::Ask,
            ),
            // 记忆工具：读放行、写放行（项目内低风险元数据，用户可经 permissions.json 覆盖）
            r(
                "builtin-normal-allow-memory-read",
                "recall",
                &[AgentMode::Normal],
                PermissionAction::Allow,
            ),
            r(
                "builtin-normal-allow-memory-search",
                "search_memory",
                &[AgentMode::Normal],
                PermissionAction::Allow,
            ),
            r(
                "builtin-normal-allow-memory-write",
                "remember",
                &[AgentMode::Normal],
                PermissionAction::Allow,
            ),
            r(
                "builtin-normal-allow-memory-update",
                "update_memory",
                &[AgentMode::Normal],
                PermissionAction::Allow,
            ),
            r(
                "builtin-normal-allow-memory-forget",
                "forget_memory",
                &[AgentMode::Normal],
                PermissionAction::Allow,
            ),
            // 其他 MCP 工具（非内置 filesystem）：需要确认
            rf(
                "builtin-normal-ask-other-tools",
                "*",
                &[AgentMode::Normal],
                PermissionAction::Ask,
                None,
                None,
                0,
            ),
        ],

        AgentMode::Auto => vec![
            // 所有内置 filesystem 操作：自动允许
            r(
                "builtin-auto-allow-fs-read",
                "read_file",
                &[AgentMode::Auto],
                PermissionAction::Allow,
            ),
            r(
                "builtin-auto-allow-fs-list",
                "list_directory",
                &[AgentMode::Auto],
                PermissionAction::Allow,
            ),
            r(
                "builtin-auto-allow-fs-search",
                "search_*",
                &[AgentMode::Auto],
                PermissionAction::Allow,
            ),
            r(
                "builtin-auto-allow-fs-write",
                "write_file",
                &[AgentMode::Auto],
                PermissionAction::Allow,
            ),
            r(
                "builtin-auto-allow-fs-replace",
                "replace_in_file",
                &[AgentMode::Auto],
                PermissionAction::Allow,
            ),
            // Git 工具：全部自动允许（Auto 模式的前提是用户已信任所有操作）
            r(
                "builtin-auto-allow-git-status",
                "git_status",
                &[AgentMode::Auto],
                PermissionAction::Allow,
            ),
            r(
                "builtin-auto-allow-git-diff",
                "git_diff",
                &[AgentMode::Auto],
                PermissionAction::Allow,
            ),
            r(
                "builtin-auto-allow-git-log",
                "git_log",
                &[AgentMode::Auto],
                PermissionAction::Allow,
            ),
            r(
                "builtin-auto-allow-git-add",
                "git_add",
                &[AgentMode::Auto],
                PermissionAction::Allow,
            ),
            r(
                "builtin-auto-allow-git-commit",
                "git_commit",
                &[AgentMode::Auto],
                PermissionAction::Allow,
            ),
            r(
                "builtin-auto-allow-fs-mkdir",
                "make_directory",
                &[AgentMode::Auto],
                PermissionAction::Allow,
            ),
            // 删除操作：需要确认（危险操作）
            r(
                "builtin-auto-ask-delete",
                "delete_file",
                &[AgentMode::Auto],
                PermissionAction::Ask,
            ),
            // 命令执行：自动允许（Auto 模式下无需逐条确认，进入前已有风险提醒）
            r(
                "builtin-auto-allow-command",
                "execute_command",
                &[AgentMode::Auto],
                PermissionAction::Allow,
            ),
            // 其他 MCP 工具：自动允许
            rf(
                "builtin-auto-allow-other-tools",
                "*",
                &[AgentMode::Auto],
                PermissionAction::Allow,
                None,
                None,
                0,
            ),
        ],

        AgentMode::Workflow => vec![
            // 完全等同于 Auto 模式的权限规则：全自动允许，文件删除需确认
            r(
                "builtin-workflow-allow-fs-read",
                "read_file",
                &[AgentMode::Workflow],
                PermissionAction::Allow,
            ),
            r(
                "builtin-workflow-allow-fs-list",
                "list_directory",
                &[AgentMode::Workflow],
                PermissionAction::Allow,
            ),
            r(
                "builtin-workflow-allow-fs-write",
                "write_file",
                &[AgentMode::Workflow],
                PermissionAction::Allow,
            ),
            r(
                "builtin-workflow-allow-fs-edit",
                "replace_in_file",
                &[AgentMode::Workflow],
                PermissionAction::Allow,
            ),
            r(
                "builtin-workflow-allow-fs-glob",
                "glob",
                &[AgentMode::Workflow],
                PermissionAction::Allow,
            ),
            r(
                "builtin-workflow-allow-fs-search",
                "search_*",
                &[AgentMode::Workflow],
                PermissionAction::Allow,
            ),
            r(
                "builtin-workflow-allow-fs-mkdir",
                "make_directory",
                &[AgentMode::Workflow],
                PermissionAction::Allow,
            ),
            // 删除操作：需要确认（危险操作）
            r(
                "builtin-workflow-ask-delete",
                "delete_file",
                &[AgentMode::Workflow],
                PermissionAction::Ask,
            ),
            // 命令执行：自动允许
            r(
                "builtin-workflow-allow-command",
                "execute_command",
                &[AgentMode::Workflow],
                PermissionAction::Allow,
            ),
            // 其他 MCP 工具：自动允许
            rf(
                "builtin-workflow-allow-other-tools",
                "*",
                &[AgentMode::Workflow],
                PermissionAction::Allow,
                None,
                None,
                0,
            ),
        ],
        AgentMode::Plan => vec![
            // Plan 模式：只读，所有写/删操作拒绝
            r(
                "builtin-plan-allow-read",
                "read_file",
                &[AgentMode::Plan],
                PermissionAction::Allow,
            ),
            r(
                "builtin-plan-allow-list-dir",
                "list_directory",
                &[AgentMode::Plan],
                PermissionAction::Allow,
            ),
            r(
                "builtin-plan-allow-search",
                "search_*",
                &[AgentMode::Plan],
                PermissionAction::Allow,
            ),
            // 写/删操作：拒绝
            rf(
                "builtin-plan-deny-write",
                "write_file",
                &[AgentMode::Plan],
                PermissionAction::Deny,
                None,
                None,
                20,
            ),
            rf(
                "builtin-plan-deny-replace",
                "replace_in_file",
                &[AgentMode::Plan],
                PermissionAction::Deny,
                None,
                None,
                20,
            ),
            rf(
                "builtin-plan-deny-mkdir",
                "make_directory",
                &[AgentMode::Plan],
                PermissionAction::Deny,
                None,
                None,
                20,
            ),
            rf(
                "builtin-plan-deny-delete",
                "delete_file",
                &[AgentMode::Plan],
                PermissionAction::Deny,
                None,
                None,
                20,
            ),
            // 禁止 Web 访问
            rf(
                "builtin-plan-deny-web-fetch",
                "web_fetch",
                &[AgentMode::Plan],
                PermissionAction::Deny,
                None,
                None,
                20,
            ),
            rf(
                "builtin-plan-deny-web-search",
                "web_search",
                &[AgentMode::Plan],
                PermissionAction::Deny,
                None,
                None,
                20,
            ),
            // Git 读操作：允许（Plan 阶段可以查看历史和状态）
            r(
                "builtin-plan-allow-git-status",
                "git_status",
                &[AgentMode::Plan],
                PermissionAction::Allow,
            ),
            r(
                "builtin-plan-allow-git-diff",
                "git_diff",
                &[AgentMode::Plan],
                PermissionAction::Allow,
            ),
            r(
                "builtin-plan-allow-git-log",
                "git_log",
                &[AgentMode::Plan],
                PermissionAction::Allow,
            ),
            // Git 写操作：拒绝（Plan 模式不修改文件）
            rf(
                "builtin-plan-deny-git-add",
                "git_add",
                &[AgentMode::Plan],
                PermissionAction::Deny,
                None,
                None,
                20,
            ),
            rf(
                "builtin-plan-deny-git-commit",
                "git_commit",
                &[AgentMode::Plan],
                PermissionAction::Deny,
                None,
                None,
                20,
            ),
            // 记忆工具：读放行、写拒绝
            r(
                "builtin-plan-allow-memory-read",
                "recall",
                &[AgentMode::Plan],
                PermissionAction::Allow,
            ),
            r(
                "builtin-plan-allow-memory-search",
                "search_memory",
                &[AgentMode::Plan],
                PermissionAction::Allow,
            ),
            rf(
                "builtin-plan-deny-memory-write",
                "remember",
                &[AgentMode::Plan],
                PermissionAction::Deny,
                None,
                None,
                20,
            ),
            rf(
                "builtin-plan-deny-memory-update",
                "update_memory",
                &[AgentMode::Plan],
                PermissionAction::Deny,
                None,
                None,
                20,
            ),
            rf(
                "builtin-plan-deny-memory-forget",
                "forget_memory",
                &[AgentMode::Plan],
                PermissionAction::Deny,
                None,
                None,
                20,
            ),
            // 其他 MCP 工具需要确认
            rf(
                "builtin-plan-ask-other-tools",
                "*",
                &[AgentMode::Plan],
                PermissionAction::Ask,
                None,
                None,
                0,
            ),
        ],
    }
}

// ====== 简单 Glob 匹配 ======

/// 简单 glob → 正则。
/// 支持 `*`（任意字符序列，不跨路径分隔符）与 `**`（任意层级，含路径分隔符）。
/// 其余字符（含 `?`、`[`、`]`）按字面转义，保持与旧实现一致。
fn glob_to_regex(pattern: &str) -> String {
    let mut out = String::from("^");
    let mut it = pattern.chars().peekable();
    while let Some(c) = it.next() {
        if c == '*' {
            if it.peek() == Some(&'*') {
                it.next();
                out.push_str(".*");
            } else {
                out.push_str("[^/]*");
            }
        } else {
            out.push_str(&regex::escape(&c.to_string()));
        }
    }
    out.push('$');
    out
}

/// 简单的 glob 匹配，支持 `*` 与 `**`。
/// `*` 匹配任意字符序列（不跨 `/`），`**` 匹配任意层级（可跨 `/`）。
fn glob_match(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    Regex::new(&glob_to_regex(pattern))
        .map(|re| re.is_match(name))
        .unwrap_or(false)
}

// ====== 路径匹配 ======

/// 检查工具参数中的路径类取值是否匹配路径模式。
/// 如果规则没有 path_pattern，则忽略路径检查。
///
/// 支持的路径类参数键：path、basePath、file_path、filePath、url（单值），
/// 以及 files / paths（数组，逐元素）——此前只认 path，导致 search_files(basePath)、
/// git_add(files[]) 等工具静默绕过路径规则。
///
/// 语义按规则动作区分（宁紧勿松）：
/// - Deny 规则：任一路径命中即匹配（数组参数里混入一个受限路径也要触发 Deny）；
/// - Allow/Ask 规则：所有路径都必须命中（授权范围不允许"部分越界"）。
///
/// 参数完全没有路径类取值时保持历史行为：规则照常匹配（退化为纯 tool_pattern 规则）。
fn path_matches(rule: &PermissionRule, arguments: &Value) -> bool {
    let pattern = match &rule.path_pattern {
        Some(p) => p,
        None => return true,
    };
    let mut paths: Vec<&str> = Vec::new();
    for key in ["path", "basePath", "file_path", "filePath", "url"] {
        if let Some(v) = arguments.get(key).and_then(|v| v.as_str()) {
            paths.push(v);
        }
    }
    for key in ["files", "paths"] {
        if let Some(arr) = arguments.get(key).and_then(|v| v.as_array()) {
            paths.extend(arr.iter().filter_map(|v| v.as_str()));
        }
    }
    if paths.is_empty() {
        return true;
    }
    match rule.action {
        PermissionAction::Deny => paths.iter().any(|p| glob_match(pattern, p)),
        _ => paths.iter().all(|p| glob_match(pattern, p)),
    }
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
    // 4. 检查路径模式（支持多路径键，语义见 path_matches）
    if !path_matches(rule, arguments) {
        return false;
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
    // 1. 收集所有匹配规则（内置 + 自定义），并标记是否自定义
    let builtin_rules = default_rules_for_mode(agent_mode);
    let mut all_candidates: Vec<(&PermissionRule, bool)> = Vec::new();

    for rule in &builtin_rules {
        if rule_matches(rule, tool_name, server_id, arguments, agent_mode) {
            all_candidates.push((rule, false));
        }
    }
    for rule in custom_rules {
        if rule_matches(rule, tool_name, server_id, arguments, agent_mode) {
            all_candidates.push((rule, true));
        }
    }

    // 2. 按优先级排序（高 → 低）；同优先级时自定义规则优先于内置规则——
    //    对齐 PermissionsFile.rules 的文档承诺（"覆盖内置默认规则"）。
    //    （修复前：内置先入列 + 稳定排序，使同优先级自定义规则永远不会赢。）
    all_candidates.sort_by_key(|(rule, is_custom)| {
        (
            std::cmp::Reverse(rule.priority),
            std::cmp::Reverse(*is_custom),
        )
    });

    // 3. 检查是否有 Deny（Deny 总是优先于 Allow 和 Ask，不受优先级影响）
    for (rule, _) in &all_candidates {
        if rule.action == PermissionAction::Deny {
            return PermissionAction::Deny;
        }
    }

    // 4. 取最高优先级的非 Deny 动作（同优先级自定义优先）
    if let Some((highest, _)) = all_candidates.first() {
        return highest.action.clone();
    }

    // 5. 无匹配规则：fail-closed → Ask
    PermissionAction::Ask
}

/// 检查工具调用权限（简化版，仅使用内置默认规则，不读取自定义规则）。
/// 仅供测试与需要"纯内置语义"的内部路径使用；真实工具调用一律走
/// [`check_permission`]（含 .aio/permissions.json 自定义规则）。
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
        Some(p) => std::path::Path::new(p)
            .join(".aio")
            .join("permissions.json"),
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
                tracing::warn!(
                    "[permissions] 解析 {} 失败，已回退到空配置：{}",
                    path.display(),
                    e
                );
                PermissionsFile::default()
            }
        },
        Err(e) => {
            tracing::warn!(
                "[permissions] 读取 {} 失败，已回退到空配置：{}",
                path.display(),
                e
            );
            PermissionsFile::default()
        }
    }
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

        // ** 递归匹配路径：dir/** 应匹配深层路径
        assert!(glob_match("dir/**", "dir/a/b"));
        assert!(glob_match("dir/**", "dir/file.txt"));
        assert!(!glob_match("dir/**", "other/a"));
        assert!(!glob_match("dir/**", "beta/x"));
        // * 不跨 `/`：dir/* 只匹配单层
        assert!(glob_match("dir/*", "dir/a.rs"));
        assert!(!glob_match("dir/*", "dir/a/b.rs"));
        // .env* 保持现有语义
        assert!(glob_match(".env*", ".env"));
        assert!(glob_match(".env*", ".env.local"));
        assert!(!glob_match(".env*", "x.env"));
        // ? / [ ] 按字面处理
        assert!(glob_match("a?b", "a?b"));
        assert!(!glob_match("a?b", "axb"));
    }

    #[test]
    fn test_normal_mode_read_allowed() {
        let args = serde_json::json!({"path": "src/main.rs"});
        let result = check_permission_defaults(
            "read_file",
            crate::core::ids::BUILTIN_FS_SERVER_ID,
            &args,
            &AgentMode::Normal,
        );
        assert_eq!(result, PermissionAction::Allow);
    }

    #[test]
    fn test_normal_mode_write_asks() {
        let args = serde_json::json!({"path": "src/main.rs", "content": "fn main() {}"});
        let result = check_permission_defaults(
            "write_file",
            crate::core::ids::BUILTIN_FS_SERVER_ID,
            &args,
            &AgentMode::Normal,
        );
        assert_eq!(result, PermissionAction::Ask);
    }

    #[test]
    fn test_normal_mode_delete_asks() {
        let args = serde_json::json!({"path": "src/main.rs"});
        let result = check_permission_defaults(
            "delete_file",
            crate::core::ids::BUILTIN_FS_SERVER_ID,
            &args,
            &AgentMode::Normal,
        );
        assert_eq!(result, PermissionAction::Ask);
    }

    #[test]
    fn test_plan_mode_write_denied() {
        let args = serde_json::json!({"path": "src/main.rs", "content": "fn main() {}"});
        let result = check_permission_defaults(
            "write_file",
            crate::core::ids::BUILTIN_FS_SERVER_ID,
            &args,
            &AgentMode::Plan,
        );
        assert_eq!(result, PermissionAction::Deny);
    }

    #[test]
    fn test_auto_mode_write_allowed() {
        let args = serde_json::json!({"path": "src/main.rs", "content": "fn main() {}"});
        let result = check_permission_defaults(
            "write_file",
            crate::core::ids::BUILTIN_FS_SERVER_ID,
            &args,
            &AgentMode::Auto,
        );
        assert_eq!(result, PermissionAction::Allow);
    }

    #[test]
    fn test_auto_mode_delete_asks() {
        let args = serde_json::json!({"path": "src/main.rs"});
        let result = check_permission_defaults(
            "delete_file",
            crate::core::ids::BUILTIN_FS_SERVER_ID,
            &args,
            &AgentMode::Auto,
        );
        assert_eq!(result, PermissionAction::Ask);
    }

    #[test]
    fn test_off_mode_denies_all() {
        let args = serde_json::json!({"path": "src/main.rs"});
        let result = check_permission_defaults(
            "read_file",
            crate::core::ids::BUILTIN_FS_SERVER_ID,
            &args,
            &AgentMode::Off,
        );
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
        let result = check_permission(
            "delete_file",
            crate::core::ids::BUILTIN_FS_SERVER_ID,
            &args,
            &AgentMode::Normal,
            &custom,
        );
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
        let result = check_permission(
            "read_file",
            crate::core::ids::BUILTIN_FS_SERVER_ID,
            &args,
            &AgentMode::Auto,
            &custom,
        );
        assert_eq!(result, PermissionAction::Deny);
    }

    #[test]
    fn test_normal_memory_tools_allowed() {
        let args = serde_json::json!({});
        for tool in [
            "remember",
            "recall",
            "search_memory",
            "update_memory",
            "forget_memory",
        ] {
            let result = check_permission_defaults(
                tool,
                crate::core::ids::BUILTIN_FS_SERVER_ID,
                &args,
                &AgentMode::Normal,
            );
            assert_eq!(
                result,
                PermissionAction::Allow,
                "tool {tool} should be allowed in Normal"
            );
        }
    }

    #[test]
    fn test_plan_memory_read_allowed_write_denied() {
        let args = serde_json::json!({});
        assert_eq!(
            check_permission_defaults(
                "recall",
                crate::core::ids::BUILTIN_FS_SERVER_ID,
                &args,
                &AgentMode::Plan
            ),
            PermissionAction::Allow,
        );
        assert_eq!(
            check_permission_defaults(
                "remember",
                crate::core::ids::BUILTIN_FS_SERVER_ID,
                &args,
                &AgentMode::Plan
            ),
            PermissionAction::Deny,
        );
    }

    #[test]
    fn test_no_match_fallbacks_to_ask() {
        // 自定义工具不在内置规则中
        let args = serde_json::json!({"query": "SELECT * FROM users"});
        let result =
            check_permission_defaults("custom_db_query", "my-db-server", &args, &AgentMode::Normal);
        assert_eq!(result, PermissionAction::Ask);
    }

    /// 回归：同优先级自定义规则必须覆盖内置规则（对齐文档"覆盖内置默认规则"）。
    /// 修复前：内置先入列 + 稳定排序，使同优先级自定义永远不会赢。
    #[test]
    fn test_custom_rule_same_priority_overrides_builtin() {
        // Normal 模式 read_file 内置为 Allow（priority 10）；自定义同优先级收紧为 Ask
        let custom = vec![PermissionRule {
            id: "tighten-read".into(),
            tool_pattern: "read_file".into(),
            server_id: None,
            modes: vec![],
            action: PermissionAction::Ask,
            path_pattern: None,
            priority: 10,
        }];
        let args = serde_json::json!({"path": "src/main.rs"});
        let result = check_permission(
            "read_file",
            crate::core::ids::BUILTIN_FS_SERVER_ID,
            &args,
            &AgentMode::Normal,
            &custom,
        );
        assert_eq!(result, PermissionAction::Ask, "同优先级自定义规则必须胜出");
    }

    /// Deny 全局优先：低优先级自定义 Deny 依然生效。
    #[test]
    fn test_custom_low_priority_deny_still_wins() {
        let custom = vec![PermissionRule {
            id: "deny-write-src".into(),
            tool_pattern: "write_file".into(),
            server_id: None,
            modes: vec![AgentMode::Auto],
            action: PermissionAction::Deny,
            path_pattern: Some("src/**".into()),
            priority: 1,
        }];
        let args = serde_json::json!({"path": "src/main.rs", "content": "x"});
        let result = check_permission(
            "write_file",
            crate::core::ids::BUILTIN_FS_SERVER_ID,
            &args,
            &AgentMode::Auto,
            &custom,
        );
        assert_eq!(result, PermissionAction::Deny);
    }

    /// 回归：path_pattern 必须对 basePath / files[] 参数生效（此前只认 path）。
    #[test]
    fn test_path_pattern_matches_basepath_and_files_array() {
        let deny_env = vec![PermissionRule {
            id: "deny-env".into(),
            tool_pattern: "*".into(),
            server_id: None,
            modes: vec![],
            action: PermissionAction::Deny,
            path_pattern: Some(".env*".into()),
            priority: 100,
        }];
        // search_files 用 basePath
        let r = check_permission(
            "search_files",
            crate::core::ids::BUILTIN_FS_SERVER_ID,
            &serde_json::json!({"basePath": ".env", "query": "x"}),
            &AgentMode::Auto,
            &deny_env,
        );
        assert_eq!(
            r,
            PermissionAction::Deny,
            "basePath 命中 .env* 必须触发 Deny"
        );
        // git_add 用 files 数组：Deny 语义为任一命中即匹配
        let r = check_permission(
            "git_add",
            crate::core::ids::BUILTIN_FS_SERVER_ID,
            &serde_json::json!({"files": [".env", "src/a.rs"]}),
            &AgentMode::Auto,
            &deny_env,
        );
        assert_eq!(
            r,
            PermissionAction::Deny,
            "files 数组混入 .env 必须触发 Deny"
        );
    }

    /// Allow/Ask 路径规则采用全命中语义（不允许部分越界授权）。
    #[test]
    fn test_allow_path_rule_requires_all_paths_to_match() {
        let allow_src = vec![PermissionRule {
            id: "allow-src".into(),
            tool_pattern: "git_add".into(),
            server_id: None,
            modes: vec![AgentMode::Normal],
            action: PermissionAction::Allow,
            path_pattern: Some("src/**".into()),
            priority: 100,
        }];
        // 全部在 src/** 内 → 匹配该 Allow
        let r = check_permission(
            "git_add",
            crate::core::ids::BUILTIN_FS_SERVER_ID,
            &serde_json::json!({"files": ["src/a.rs", "src/b.rs"]}),
            &AgentMode::Normal,
            &allow_src,
        );
        assert_eq!(r, PermissionAction::Allow);
        // 混入了 src/** 之外的路径 → 该 Allow 规则不匹配，回落到内置/Ask
        let r2 = check_permission(
            "git_add",
            crate::core::ids::BUILTIN_FS_SERVER_ID,
            &serde_json::json!({"files": ["src/a.rs", "README.md"]}),
            &AgentMode::Normal,
            &allow_src,
        );
        assert_ne!(
            r2,
            PermissionAction::Allow,
            "部分路径越界时不得命中 Allow 规则"
        );
    }
}

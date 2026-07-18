//! 内置 Git 工具 — 结构化版本控制操作。
//!
//! 通过系统 `git` CLI 提供结构化的版本控制操作，替代在 `execute_command` 中
//! 手动拼接 git 命令。所有调用使用参数数组（非 shell 字符串），杜绝注入风险。
//!
//! 参考：Claude Code / Cursor 的 Git 工具设计。

use crate::core::models::{ToolResult, ToolResultContent, ToolSpec, ToolFunctionSpec};
use serde_json::{json, Value};
use std::process::Command;

// ====== 常量 ======

const MAX_OUTPUT_BYTES: usize = 80_000;
const MAX_COMMIT_MSG_BYTES: usize = 10_000;

// ====== 辅助函数 ======

pub fn tool_ok(text: String) -> ToolResult {
    ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({"text": text}),
        }],
        is_error: false,
    }
}

pub fn tool_err(msg: &str) -> ToolResult {
    ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({"text": format!("[Error] {msg}")}),
        }],
        is_error: true,
    }
}

fn truncate_output(s: &str, limit: usize) -> String {
    if s.len() <= limit {
        s.to_string()
    } else {
        let safe_end = s.floor_char_boundary(limit);
        format!("{}\n\n⚠ 输出过长已截断 ({}B / {}B)", &s[..safe_end], limit, s.len())
    }
}

/// 执行 git 命令并返回 stdout（stderr 在失败时作为错误信息返回）。
/// 使用参数数组，永不拼接 shell 字符串。
fn run_git(project_root: &str, args: &[&str]) -> Result<String, String> {
    // 仅允许 git 二进制
    let output = Command::new("git")
        .args(args)
        .current_dir(project_root)
        .output()
        .map_err(|e| format!("无法执行 git: {e}（请确认 git 已安装并在 PATH 中）"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() { stdout } else { stderr };
        return Err(format!("git {} 失败: {}", args.join(" "), detail.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// 检查目录是否为 git 仓库
fn is_git_repo(project_root: &str) -> bool {
    Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(project_root)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ====== 工具定义 ======

pub fn get_git_tool_specs() -> Vec<ToolSpec> {
    vec![
        // ---- 只读工具 ----
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "git_status".into(),
                description: "查看 git 工作区和暂存区状态（机器可读格式）。显示所有已修改、已暂存、未跟踪的文件。".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "git_diff".into(),
                description: "查看 git 差异对比。默认显示工作区未暂存的变更；设置 staged=true 查看已暂存的变更。".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "staged": { "type": "boolean", "description": "是否查看已暂存的变更（默认 false，即工作区变更）" },
                        "path": { "type": "string", "description": "限定到指定文件或目录" }
                    }
                }),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "git_log".into(),
                description: "查看 git 提交历史。默认显示最近 10 条。".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "count": { "type": "integer", "description": "显示条数（默认 10，最大 50）" },
                        "path": { "type": "string", "description": "限定到指定文件或目录" },
                        "oneline": { "type": "boolean", "description": "简洁单行模式（默认 true）" }
                    }
                }),
            },
        },
        // ---- 写入工具 ----
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "git_add".into(),
                description: "将文件添加到 git 暂存区。可以指定文件列表或使用 all=true 暂存全部变更。".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "files": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "要暂存的文件路径列表"
                        },
                        "all": { "type": "boolean", "description": "暂存所有变更（git add -A），默认 false" }
                    }
                }),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "git_commit".into(),
                description: "创建 git 提交。message 参数必填，请编写清晰的提交信息。".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "message": { "type": "string", "description": "提交信息（必填，最大 10KB）" }
                    },
                    "required": ["message"]
                }),
            },
        },
    ]
}

// ====== 工具执行 ======

pub fn execute_git_tool(
    name: &str,
    arguments: &Value,
    project_root: &str,
) -> ToolResult {
    // 前置：检查是否为 git 仓库
    if !is_git_repo(project_root) {
        return tool_err("当前目录不是 git 仓库（或 git 未安装）");
    }

    match name {
        "git_status" => {
            match run_git(project_root, &["status", "--porcelain=v2", "--branch"]) {
                Ok(out) => {
                    let result = if out.trim().is_empty() {
                        "✓ 工作区干净，没有待提交的变更。".to_string()
                    } else {
                        format!("git status:\n\n{}", out.trim())
                    };
                    tool_ok(truncate_output(&result, MAX_OUTPUT_BYTES))
                }
                Err(e) => tool_err(&e),
            }
        }

        "git_diff" => {
            let staged = arguments["staged"].as_bool().unwrap_or(false);
            let path = arguments["path"].as_str().unwrap_or("");
            let mut args = vec!["diff", "--no-color"];
            if staged {
                args.push("--staged");
            }
            if !path.is_empty() {
                args.push("--");
                args.push(path);
            }
            match run_git(project_root, &args) {
                Ok(out) => {
                    if out.trim().is_empty() {
                        tool_ok("✓ 没有差异。".into())
                    } else {
                        tool_ok(truncate_output(&out, MAX_OUTPUT_BYTES))
                    }
                }
                Err(e) => tool_err(&e),
            }
        }

        "git_log" => {
            let count = arguments["count"].as_u64().unwrap_or(10).min(50);
            let oneline = arguments["oneline"].as_bool().unwrap_or(true);
            let path = arguments["path"].as_str().unwrap_or("");
            let mut args = vec!["log"];
            if oneline {
                args.push("--oneline");
            }
            args.push("-n");
            let count_str = count.to_string();
            args.push(&count_str);
            if !path.is_empty() {
                args.push("--");
                args.push(path);
            }
            match run_git(project_root, &args) {
                Ok(out) => {
                    if out.trim().is_empty() {
                        tool_ok("（暂无提交记录）".into())
                    } else {
                        tool_ok(truncate_output(&out, MAX_OUTPUT_BYTES))
                    }
                }
                Err(e) => tool_err(&e),
            }
        }

        "git_add" => {
            let all = arguments["all"].as_bool().unwrap_or(false);
            if all {
                match run_git(project_root, &["add", "--all"]) {
                    Ok(out) => tool_ok(format!("✓ git add --all\n{}", if out.trim().is_empty() { "(无输出)" } else { out.trim() })),
                    Err(e) => tool_err(&e),
                }
            } else {
                let files = arguments["files"].as_array();
                match files {
                    Some(arr) if !arr.is_empty() => {
                        let mut args: Vec<String> = vec!["add".into()];
                        for f in arr {
                            if let Some(s) = f.as_str() {
                                if !s.is_empty() {
                                    args.push(s.to_string());
                                }
                            }
                        }
                        if args.len() == 1 {
                            return tool_err("files 列表为空");
                        }
                        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                        match run_git(project_root, &arg_refs) {
                            Ok(_) => {
                                let names: Vec<&str> = args[1..].iter().map(|s| s.as_str()).collect();
                                tool_ok(format!("✓ git add {}", names.join(" ")))
                            }
                            Err(e) => tool_err(&e),
                        }
                    }
                    _ => tool_err("请指定 files 列表或设置 all=true"),
                }
            }
        }

        "git_commit" => {
            let message = arguments["message"].as_str().unwrap_or("").trim().to_string();
            if message.is_empty() {
                return tool_err("缺少 commit message");
            }
            if message.len() > MAX_COMMIT_MSG_BYTES {
                return tool_err(&format!(
                    "commit message 过长 ({}B > {}B)",
                    message.len(),
                    MAX_COMMIT_MSG_BYTES
                ));
            }
            // 防御：拒绝空提交
            // 先检查是否有暂存的变更
            let staged_empty = run_git(project_root, &["diff", "--staged", "--quiet"]).is_ok();
            if staged_empty {
                return tool_err(
                    "暂存区为空，没有可提交的变更。请先用 git_add 暂存文件后再提交。"
                );
            }
            match run_git(project_root, &["commit", "-m", &message]) {
                Ok(out) => tool_ok(truncate_output(&format!("✓ git commit\n\n{}", out.trim()), MAX_OUTPUT_BYTES)),
                Err(e) => tool_err(&e),
            }
        }

        _ => tool_err(&format!("未知 Git 工具: {name}")),
    }
}

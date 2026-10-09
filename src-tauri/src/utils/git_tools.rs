//! Built-in Git tools — structured version control operations.
//!
//! Exposes structured git operations through the system `git` CLI, replacing
//! hand-assembled git commands in `execute_command`. All calls use argument
//! arrays (never shell strings), eliminating injection risk.
//!
//! Design reference: Claude Code / Cursor Git tools.

use crate::core::models::{FileChange, ToolFunctionSpec, ToolResult, ToolSpec};
use crate::ierr;
use serde_json::{json, Value};
use std::process::Command;

// ====== Constants ======

const MAX_OUTPUT_BYTES: usize = 80_000;
const MAX_COMMIT_MSG_BYTES: usize = 10_000;

// ====== Helpers ======

// tool_ok / tool_err are provided centrally by utils::tool_result (single source repo-wide)
pub use crate::utils::tool_result::{tool_err, tool_ok};

// Truncation is provided centrally by utils::truncate (imported under a semantic name)
use crate::utils::truncate::truncate_bytes_head as truncate_output;

/// Runs a git command and returns stdout (stderr is returned as the error message on failure).
/// Uses argument arrays; never builds shell strings.
fn run_git(project_root: &str, args: &[&str]) -> Result<String, String> {
    // Only the git binary is ever invoked
    let output = Command::new("git")
        .args(args)
        .current_dir(project_root)
        .output()
        .map_err(|e| {
            ierr!(
                "无法执行 git: {}（请确认 git 已安装并在 PATH 中）",
                "Failed to run git: {} (make sure git is installed and on PATH)",
                e
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() {
            stdout
        } else {
            stderr
        };
        return Err(ierr!(
            "git {} 失败: {}",
            "git {} failed: {}",
            args.join(" "),
            detail.trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Checks whether the directory is a git repository
pub(crate) fn is_git_repo(project_root: &str) -> bool {
    Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(project_root)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Returns the current git branch name, or None if it is not a git repository.
pub fn get_current_branch(project_root: &str) -> Option<String> {
    if !is_git_repo(project_root) {
        return None;
    }
    run_git(project_root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Lists all local branch names. The current branch comes first (marked with `*`).
pub fn list_branches(project_root: &str) -> Result<Vec<String>, String> {
    if !is_git_repo(project_root) {
        return Err(ierr!("不是 git 仓库", "Not a git repository"));
    }
    let out = run_git(project_root, &["branch", "--format=%(refname:short)"])?;
    Ok(out
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect())
}

/// Switches to the given branch (git checkout). Returns Ok on success.
pub fn checkout_branch(project_root: &str, branch_name: &str) -> Result<String, String> {
    run_git(project_root, &["checkout", branch_name])?;
    Ok(format!("已切换到分支 {branch_name}"))
}

/// Captures the latest diff of a file in the project's git repository.
/// Called after a tool writes the file; reads the patch from git diff.
/// Returns None when the directory is not a git repository or nothing changed.
pub fn capture_file_diff(project_root: &str, file_path: &str) -> Option<FileChange> {
    if !is_git_repo(project_root) {
        return None;
    }
    let patch = run_git(project_root, &["diff", "--no-color", "--", file_path]).ok()?;
    if patch.trim().is_empty() {
        return None;
    }
    let action = if patch.contains("new file mode") {
        "create"
    } else if patch.contains("deleted file mode") {
        "delete"
    } else {
        "modify"
    };
    let summary = format_diff_stats(&patch);
    Some(FileChange {
        file_path: file_path.to_string(),
        action: action.to_string(),
        diff: truncate_output(&patch, 100_000),
        summary,
    })
}

/// Parses "+N -M" stats from unified diff text
fn format_diff_stats(patch: &str) -> String {
    let added = patch
        .lines()
        .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
        .count();
    let removed = patch
        .lines()
        .filter(|l| l.starts_with('-') && !l.starts_with("---"))
        .count();
    format!("+{} -{}", added, removed)
}

// ====== Tool Specs ======

pub fn get_git_tool_specs() -> Vec<ToolSpec> {
    vec![
        // ---- Read-only tools ----
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
        // ---- Write tools ----
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

// ====== Tool Execution ======

pub fn execute_git_tool(name: &str, arguments: &Value, project_root: &str) -> ToolResult {
    // Precondition: verify the directory is a git repository
    if !is_git_repo(project_root) {
        return tool_err(&ierr!(
            "当前目录不是 git 仓库（或 git 未安装）",
            "Current directory is not a git repository (or git is not installed)"
        ));
    }

    match name {
        "git_status" => match run_git(project_root, &["status", "--porcelain=v2", "--branch"]) {
            Ok(out) => {
                let result = if out.trim().is_empty() {
                    "✓ 工作区干净，没有待提交的变更。".to_string()
                } else {
                    format!("git status:\n\n{}", out.trim())
                };
                tool_ok(truncate_output(&result, MAX_OUTPUT_BYTES))
            }
            Err(e) => tool_err(&e),
        },

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
                    Ok(out) => tool_ok(format!(
                        "✓ git add --all\n{}",
                        if out.trim().is_empty() {
                            "(无输出)"
                        } else {
                            out.trim()
                        }
                    )),
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
                            return tool_err(&ierr!("files 列表为空", "The files list is empty"));
                        }
                        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                        match run_git(project_root, &arg_refs) {
                            Ok(_) => {
                                let names: Vec<&str> =
                                    args[1..].iter().map(|s| s.as_str()).collect();
                                tool_ok(format!("✓ git add {}", names.join(" ")))
                            }
                            Err(e) => tool_err(&e),
                        }
                    }
                    _ => tool_err(&ierr!(
                        "请指定 files 列表或设置 all=true",
                        "Specify the files list or set all=true"
                    )),
                }
            }
        }

        "git_commit" => {
            let message = arguments["message"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_string();
            if message.is_empty() {
                return tool_err(&ierr!("缺少 commit message", "Missing commit message"));
            }
            if message.len() > MAX_COMMIT_MSG_BYTES {
                return tool_err(&ierr!(
                    "commit message 过长 ({}B > {}B)",
                    "commit message too long ({}B > {}B)",
                    message.len(),
                    MAX_COMMIT_MSG_BYTES
                ));
            }
            // Guard: reject empty commits
            // Check whether anything is staged first
            let staged_empty = run_git(project_root, &["diff", "--staged", "--quiet"]).is_ok();
            if staged_empty {
                return tool_err(&ierr!(
                    "暂存区为空，没有可提交的变更。请先用 git_add 暂存文件后再提交。",
                    "The staging area is empty; there is nothing to commit. Stage files with git_add first, then commit."
                ));
            }
            match run_git(project_root, &["commit", "-m", &message]) {
                Ok(out) => tool_ok(truncate_output(
                    &format!("✓ git commit\n\n{}", out.trim()),
                    MAX_OUTPUT_BYTES,
                )),
                Err(e) => tool_err(&e),
            }
        }

        _ => tool_err(&ierr!("未知 Git 工具: {}", "Unknown git tool: {}", name)),
    }
}

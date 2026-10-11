//! Git integration commands (branch query/switch/file revert).
//! The actual Git operations live in utils/git_tools.rs; this file only forwards IPC parameters and maps errors.
//!
//! HOR-4: every command spawns a `git` subprocess, which can take seconds on large
//! repositories. Tauri runs SYNC commands on the main thread — a UI freeze per
//! branch query. All commands here are therefore async and run their process work
//! on the blocking pool.

use crate::ierr;
use crate::utils::git_tools;

/// Get the current project's Git branch name.
/// Returns null if the directory is not a git repository.
#[tauri::command]
pub async fn get_git_branch(project_path: String) -> Option<String> {
    tokio::task::spawn_blocking(move || git_tools::get_current_branch(&project_path))
        .await
        .ok()
        .flatten()
}

/// List all local Git branches of the current project.
/// Returns an error message if the directory is not a git repository.
#[tauri::command]
pub async fn list_git_branches(project_path: String) -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(move || git_tools::list_branches(&project_path))
        .await
        .map_err(|e| e.to_string())?
}

/// Switch to the given Git branch (git checkout).
/// Returns a confirmation message on success, or Git's error message on failure.
#[tauri::command]
pub async fn switch_git_branch(
    project_path: String,
    branch_name: String,
) -> Result<String, String> {
    tokio::task::spawn_blocking(move || git_tools::checkout_branch(&project_path, &branch_name))
        .await
        .map_err(|e| e.to_string())?
}

/// Revert one file to its HEAD version (git checkout HEAD -- <path>); shared by the
/// single/batch commands. Runs on the caller's (blocking) thread.
fn revert_one(
    git: &std::path::Path,
    project_path: &str,
    file_path: &str,
) -> Result<String, String> {
    let status = std::process::Command::new(git)
        .args(["checkout", "HEAD", "--", file_path])
        .current_dir(project_path)
        .status()
        .map_err(|e| ierr!("git 执行失败: {}", "Failed to run git: {}", e))?;
    if status.success() {
        Ok(format!("已恢复文件: {file_path}"))
    } else {
        Err(ierr!("git checkout 失败", "git checkout failed"))
    }
}

/// Restore a file to its HEAD version with git checkout, discarding all uncommitted changes.
/// Applicable to file modifications made by write_file / replace_in_file / delete_file.
#[tauri::command]
pub async fn revert_file_change(project_path: String, file_path: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        // SEC-06: PATH-only git resolution (cwd = project dir must not be searched first)
        let git = crate::utils::exec_resolve::resolve_executable("git")
            .ok_or_else(|| ierr!("未在 PATH 中找到 git", "git not found on PATH"))?;
        revert_one(&git, &project_path, &file_path)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Restore multiple files to their HEAD versions in one batch, discarding all uncommitted changes.
/// Applicable to undoing all write_file / replace_in_file / delete_file operations in a single turn.
#[tauri::command]
pub async fn revert_file_changes_batch(
    project_path: String,
    file_paths: Vec<String>,
) -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(move || {
        // SEC-06: PATH-only git resolution (see revert_file_change)
        let git = crate::utils::exec_resolve::resolve_executable("git")
            .ok_or_else(|| ierr!("未在 PATH 中找到 git", "git not found on PATH"))?;
        let mut results = Vec::new();
        for file_path in &file_paths {
            match revert_one(&git, &project_path, file_path) {
                Ok(msg) => results.push(msg),
                // Per-file failure is reported in-band, matching the historic contract
                Err(e) => results.push(e),
            }
        }
        Ok(results)
    })
    .await
    .map_err(|e| e.to_string())?
}

//! Git integration commands (branch query/switch/file revert).
//! The actual Git operations live in utils/git_tools.rs; this file only forwards IPC parameters and maps errors.

use crate::utils::git_tools;

use crate::ierr;

/// Get the current project's Git branch name.
/// Returns null if the directory is not a git repository.
#[tauri::command]
pub fn get_git_branch(project_path: String) -> Option<String> {
    git_tools::get_current_branch(&project_path)
}

/// List all local Git branches of the current project.
/// Returns an error message if the directory is not a git repository.
#[tauri::command]
pub fn list_git_branches(project_path: String) -> Result<Vec<String>, String> {
    git_tools::list_branches(&project_path)
}

/// Switch to the given Git branch (git checkout).
/// Returns a confirmation message on success, or Git's error message on failure.
#[tauri::command]
pub fn switch_git_branch(project_path: String, branch_name: String) -> Result<String, String> {
    git_tools::checkout_branch(&project_path, &branch_name)
}

/// Restore a file to its HEAD version with git checkout, discarding all uncommitted changes.
/// Applicable to file modifications made by write_file / replace_in_file / delete_file.
#[tauri::command]
pub fn revert_file_change(project_path: String, file_path: String) -> Result<String, String> {
    let status = std::process::Command::new("git")
        .args(["checkout", "HEAD", "--", &file_path])
        .current_dir(&project_path)
        .status()
        .map_err(|e| ierr!("git 执行失败: {}", "Failed to run git: {}", e))?;
    if status.success() {
        Ok(format!("已恢复文件: {file_path}"))
    } else {
        Err(ierr!("git checkout 失败", "git checkout failed"))
    }
}

/// Restore multiple files to their HEAD versions in one batch, discarding all uncommitted changes.
/// Applicable to undoing all write_file / replace_in_file / delete_file operations in a single turn.
#[tauri::command]
pub fn revert_file_changes_batch(
    project_path: String,
    file_paths: Vec<String>,
) -> Result<Vec<String>, String> {
    let mut results = Vec::new();
    for file_path in &file_paths {
        let status = std::process::Command::new("git")
            .args(["checkout", "HEAD", "--", file_path])
            .current_dir(&project_path)
            .status()
            .map_err(|e| ierr!("git 执行失败: {}", "Failed to run git: {}", e))?;
        if status.success() {
            results.push(format!("已恢复: {file_path}"));
        } else {
            results.push(ierr!("恢复失败: {}", "Revert failed: {}", file_path));
        }
    }
    Ok(results)
}

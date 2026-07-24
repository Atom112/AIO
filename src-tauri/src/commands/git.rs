use crate::utils::git_tools;
use std::process::Command;

/// 获取当前项目的 Git 分支名称。
/// 如果不是 git 仓库则返回 null。
#[tauri::command]
pub fn get_git_branch(project_path: String) -> Option<String> {
    git_tools::get_current_branch(&project_path)
}

/// 列出当前项目的所有 Git 本地分支。
/// 如果不是 git 仓库则返回错误信息。
#[tauri::command]
pub fn list_git_branches(project_path: String) -> Result<Vec<String>, String> {
    git_tools::list_branches(&project_path)
}

/// 切换到指定的 Git 分支（git checkout）。
/// 成功后返回确认信息；失败时返回 Git 的错误消息。
#[tauri::command]
pub fn switch_git_branch(project_path: String, branch_name: String) -> Result<String, String> {
    git_tools::checkout_branch(&project_path, &branch_name)
}

/// 使用 git checkout 恢复到 HEAD 版本，撤销文件的所有未提交变更。
/// 适用于 write_file / replace_in_file / delete_file 产生的文件修改。
#[tauri::command]
pub fn revert_file_change(project_path: String, file_path: String) -> Result<String, String> {
    let status = std::process::Command::new("git")
        .args(["checkout", "HEAD", "--", &file_path])
        .current_dir(&project_path)
        .status()
        .map_err(|e| format!("git 执行失败: {e}"))?;
    if status.success() {
        Ok(format!("已恢复文件: {file_path}"))
    } else {
        Err("git checkout 失败".into())
    }
}

/// 批量恢复多个文件到 HEAD 版本，撤销所有未提交的变更。
/// 适用于撤销单轮对话中的所有 write_file / replace_in_file / delete_file 操作。
#[tauri::command]
pub fn revert_file_changes_batch(project_path: String, file_paths: Vec<String>) -> Result<Vec<String>, String> {
    let mut results = Vec::new();
    for file_path in &file_paths {
        let status = std::process::Command::new("git")
            .args(["checkout", "HEAD", "--", file_path])
            .current_dir(&project_path)
            .status()
            .map_err(|e| format!("git 执行失败: {e}"))?;
        if status.success() {
            results.push(format!("已恢复: {file_path}"));
        } else {
            results.push(format!("恢复失败: {file_path}"));
        }
    }
    Ok(results)
}
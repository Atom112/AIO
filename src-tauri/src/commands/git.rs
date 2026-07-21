use crate::utils::git_tools;

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
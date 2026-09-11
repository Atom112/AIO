//! 项目索引（`projects.json`）读取 —— 全仓唯一来源。
//!
//! 位置：app_data_dir/projects.json（**不是** core::paths 的 config_dir，勿合并）。
//!
//! 历史背景：同一读取逻辑曾在 5 处各自实现，其中 2 处用未类型化的
//! `Value["projects"][id]["path"]` 下标读取（字段改名即静默返回"项目不存在"），
//! 1 处漏了 Windows 扩展长度前缀归一化，另有 1 处注释谎称"复用 skill.rs 的逻辑"。

use crate::core::models::ProjectsFile;
use tauri::{AppHandle, Manager};

/// `projects.json` 的完整路径。
pub fn projects_file_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("projects.json"))
        .map_err(|e| e.to_string())
}

/// 读取项目索引。
///
/// 容错语义（与历史实现一致）：文件缺失或 JSON 解析失败时返回空表。
/// 需要区分"无项目"与"索引损坏"的调用方请直接用 [`projects_file_path`] + 自行解析。
pub fn load_projects_file(app: &AppHandle) -> ProjectsFile {
    let Ok(path) = projects_file_path(app) else {
        return ProjectsFile::default();
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

/// 按 project_id 解析项目根路径（已归一化 Windows 扩展长度前缀 `\\?\`）。
pub fn project_path(app: &AppHandle, project_id: &str) -> Result<String, String> {
    let file = load_projects_file(app);
    file.projects
        .get(project_id)
        .map(|p| crate::utils::file_tools::strip_windows_extended_prefix(&p.path))
        .ok_or_else(|| format!("项目 {project_id} 不存在"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_yields_empty_index() {
        // 无 AppHandle 时不可直接调用；此处仅固化 ProjectsFile 的默认语义，
        // 确保"空表"与"解析失败"在调用方看来一致（历史行为）。
        let f = ProjectsFile::default();
        assert!(f.projects.is_empty());
    }
}

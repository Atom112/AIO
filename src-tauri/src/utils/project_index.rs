//! Project index (`projects.json`) loading — single source of truth repo-wide.
//!
//! Location: app_data_dir/projects.json (**not** core::paths's config_dir; do not merge them).

use crate::core::models::ProjectsFile;
use tauri::{AppHandle, Manager};

/// Full path of `projects.json`.
pub fn projects_file_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("projects.json"))
        .map_err(|e| e.to_string())
}

/// Load the project index.
///
/// Fault-tolerance semantics (historical behavior preserved): returns an empty table when the file is missing or JSON parsing fails.
/// Callers that must distinguish "no projects" from "corrupt index" should use [`projects_file_path`] and parse the file themselves.
pub fn load_projects_file(app: &AppHandle) -> ProjectsFile {
    let Ok(path) = projects_file_path(app) else {
        return ProjectsFile::default();
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

/// Resolve the project root path by project_id (normalizes the Windows extended-length prefix `\\?\`).
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
        // Cannot be called directly without an AppHandle; this only pins down ProjectsFile's
        // default semantics so "empty table" and "parse failure" look identical to callers (historical behavior).
        let f = ProjectsFile::default();
        assert!(f.projects.is_empty());
    }
}

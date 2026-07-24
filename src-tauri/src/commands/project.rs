//! 项目 CRUD 命令。
//!
//! 项目是绑定到文件系统目录的逻辑分组单元，每个项目在 `<project_dir>/.aio/` 下
//! 存放专属的 skills.json 和 mcp-servers.json。

use crate::core::models::{Project, ProjectsFile, AgentMode};
use crate::core::state::DbState;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

const PROJECTS_FILE: &str = "projects.json";
const AIO_DIR: &str = ".aio";

fn projects_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join(PROJECTS_FILE))
        .map_err(|e| e.to_string())
}

fn load_projects_file(app: &AppHandle) -> ProjectsFile {
    let Ok(path) = projects_file_path(app) else {
        return ProjectsFile::default();
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

fn save_projects_file(app: &AppHandle, file: &ProjectsFile) -> Result<(), String> {
    let path = projects_file_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(file).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

fn now_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    secs.to_string()
}

/// 初始化项目的 .aio/ 目录结构。
fn init_project_dir(project_path: &str) -> Result<(), String> {
    let aio_dir = PathBuf::from(project_path).join(AIO_DIR);
    std::fs::create_dir_all(&aio_dir).map_err(|e| format!("创建 .aio 目录失败: {}", e))?;

    // 写入项目元数据
    let _project_json = aio_dir.join("project.json");

    // 初始化空的 skills.json
    let skills_path = aio_dir.join("skills.json");
    if !skills_path.exists() {
        let empty_skills = serde_json::json!({
            "version": 1,
            "updatedAt": "",
            "skills": {}
        });
        std::fs::write(
            &skills_path,
            serde_json::to_string_pretty(&empty_skills).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("创建 skills.json 失败: {}", e))?;
    }

    // 初始化 mcp-servers.json（包含内置文件系统 MCP server）
    let mcp_path = aio_dir.join("mcp-servers.json");
    if !mcp_path.exists() {
        // 获取当前可执行文件路径，作为 --fs-server 的 command
        let exe_path = std::env::current_exe()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "aio".to_string());

        let mcp_config = serde_json::json!({
            "version": 1,
            "updatedAt": "",
            "servers": {
                "__aio-filesystem__": {
                    "id": "__aio-filesystem__",
                    "displayName": "项目文件系统 (AIO 内置)",
                    "transport": {
                        "transport": "stdio",
                        "command": exe_path,
                        "args": ["--fs-server", project_path],
                        "env": {}
                    },
                    "enabledTools": [],
                    "autoStart": true,
                    "hasStoredSecret": false
                }
            }
        });
        std::fs::write(
            &mcp_path,
            serde_json::to_string_pretty(&mcp_config).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("创建 mcp-servers.json 失败: {}", e))?;
    }

    Ok(())
}

/// 创建新项目。
#[tauri::command]
pub fn create_project(app: AppHandle, state: tauri::State<'_, DbState>, name: String, path: String) -> Result<Project, String> {
    if name.trim().is_empty() {
        return Err("项目名称不能为空".into());
    }
    let path_clean = path.trim().to_string();
    if path_clean.is_empty() {
        return Err("项目路径不能为空".into());
    }
    let canonical = std::fs::canonicalize(&path_clean).map_err(|e| format!("路径无效: {}", e))?;
    let canonical_str = canonical.to_string_lossy().to_string();

    // 检查路径是否已被其他项目使用
    let mut file = load_projects_file(&app);
    if file.projects.values().any(|p| p.path == canonical_str) {
        return Err("该目录已绑定到其他项目".into());
    }

    // 初始化 .aio/ 目录
    init_project_dir(&canonical_str)?;

    let id = uuid::Uuid::new_v4().to_string();
    let assistant_id = format!("asst-{}", id);
    let ts = now_timestamp();
    let project = Project {
        id: id.clone(),
        name: name.trim().to_string(),
        path: canonical_str,
        created_at: ts.clone(),
        updated_at: ts.clone(),
        assistant_id: assistant_id.clone(),
    };

    // 创建对应助理记录
    {
        let conn = state.0.lock();
        let agent_mode_str = serde_json::to_string(&AgentMode::Normal)
            .unwrap_or_else(|_| "\"normal\"".to_string())
            .trim_matches('"')
            .to_string();
        conn.execute(
            "INSERT INTO assistants (id, name, prompt, model_id, mcp_server_ids, skill_ids, project_id, agent_mode, assistant_type) VALUES (?1, ?2, '', NULL, '[\"__aio-filesystem__\"]', '[]', ?3, ?4, 'project')",
            rusqlite::params![assistant_id, project.name, project.id, agent_mode_str],
        ).map_err(|e| format!("创建项目助理失败: {}", e))?;
    }

    file.projects.insert(id, project.clone());
    file.updated_at = now_timestamp();
    save_projects_file(&app, &file)?;

    Ok(project)
}

/// 列出所有项目。
#[tauri::command]
pub fn list_projects(app: AppHandle) -> Result<Vec<Project>, String> {
    let file = load_projects_file(&app);
    let mut projects: Vec<Project> = file.projects.into_values().collect();
    projects.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(projects)
}

/// 更新项目名称。
#[tauri::command]
pub fn update_project(app: AppHandle, id: String, name: String) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("项目名称不能为空".into());
    }
    let mut file = load_projects_file(&app);
    match file.projects.get_mut(&id) {
        Some(project) => {
            project.name = name.trim().to_string();
            project.updated_at = now_timestamp();
            file.updated_at = now_timestamp();
            save_projects_file(&app, &file)
        }
        None => Err("项目不存在".into()),
    }
}

/// 删除项目记录（.aio/ 目录保留不删，保护用户数据）。
#[tauri::command]
pub fn delete_project(app: AppHandle, id: String) -> Result<(), String> {
    let mut file = load_projects_file(&app);
    if file.projects.remove(&id).is_some() {
        file.updated_at = now_timestamp();
        save_projects_file(&app, &file)
    } else {
        Err("项目不存在".into())
    }
}

/// 用系统文件管理器打开项目目录。
#[tauri::command]
pub fn open_project_directory(_app: AppHandle, path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }
    Ok(())
}

/// 通过路径查找项目 ID（用于从 .aio/ 目录反向定位）。
#[tauri::command]
pub fn get_project_by_path(app: AppHandle, path: String) -> Result<Option<Project>, String> {
    let canonical = std::fs::canonicalize(&path).map_err(|e| format!("路径无效: {}", e))?;
    let canonical_str = canonical.to_string_lossy().to_string();
    let file = load_projects_file(&app);
    Ok(file
        .projects
        .into_values()
        .find(|p| p.path == canonical_str))
}

/// 检查目录是否可绑定为项目（目录是否存在、是否已被绑定）。
#[tauri::command]
pub fn validate_project_path(app: AppHandle, path: String) -> Result<serde_json::Value, String> {
    let canonical = match std::fs::canonicalize(&path) {
        Ok(c) => c.to_string_lossy().to_string(),
        Err(e) => {
            return Ok(serde_json::json!({
                "valid": false,
                "reason": format!("路径无效: {}", e)
            }));
        }
    };
    let is_dir = std::path::Path::new(&canonical).is_dir();
    if !is_dir {
        return Ok(serde_json::json!({
            "valid": false,
            "reason": "路径不是目录"
        }));
    }
    let file = load_projects_file(&app);
    let existing = file.projects.values().find(|p| p.path == canonical);
    Ok(serde_json::json!({
        "valid": existing.is_none(),
        "reason": existing.map(|p| format!("该目录已绑定到项目: {}", p.name)).unwrap_or_default(),
        "canonicalPath": canonical,
    }))
}

/// 返回 .aio/ 目录下的 skill 文件路径（供 skill.rs 使用）。
pub fn project_skills_path(project_path: &str) -> PathBuf {
    PathBuf::from(project_path)
        .join(AIO_DIR)
        .join("skills.json")
}

/// 返回 .aio/ 目录下的 MCP server 文件路径（供 mcp/mod.rs 使用）。
pub fn project_mcp_servers_path(project_path: &str) -> PathBuf {
    PathBuf::from(project_path)
        .join(AIO_DIR)
        .join("mcp-servers.json")
}

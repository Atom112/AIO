//! Agent skill/artifact read tool specs and execution (read_skill / read_artifact).

use crate::commands::llm::*;
use crate::ierr;
use serde_json::json;
use tauri::AppHandle;

/// ToolSpec of the read_skill tool: lets the model read the full instructions of an enabled skill on demand.
/// Works with the frontend "skill catalog" injection, avoiding sending every skill's full content with each request.
pub(crate) fn skill_read_tool_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".into(),
        function: ToolFunctionSpec {
            name: "read_skill".into(),
            description: "读取一个已启用 Skill 的完整说明与使用指令。当任务涉及某个技能（如文档处理、表格、演示文稿等）时，先用此工具获取其完整内容再执行。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "技能名称（如 docx、xlsx）"}
                },
                "required": ["name"]
            }),
        },
    }
}

/// Execute read_skill: look up a skill by name (case-insensitive) or ID and return its full content.
pub(crate) fn execute_skill_read(skills: &[SkillConfig], name: &str) -> Result<ToolResult, String> {
    if name.is_empty() {
        return Err(ierr!(
            "read_skill 缺少 name 参数",
            "read_skill is missing the name parameter"
        ));
    }
    if let Some(skill) = skills
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(name) || s.id == name)
    {
        Ok(file_tools::tool_ok(format!(
            "[Skill: {}]\n{}",
            skill.name, skill.content
        )))
    } else {
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        Err(format!(
            "未找到技能: {name}；可用技能: {}",
            names.join(", ")
        ))
    }
}

/// ToolSpec of the read_artifact tool: reads history archives stored locally during compression (snapcompact-style).
/// When summary generation fails or early details are needed, the model calls this tool with the id from a [历史归档 #id] placeholder.
pub(crate) fn artifact_read_tool_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".into(),
        function: ToolFunctionSpec {
            name: "read_artifact".into(),
            description: "读取本地历史归档的完整内容（压缩早期对话时自动归档；需要早期细节时按 [历史归档 #id] 占位中的 id 调用）。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "artifact_id": {"type": "string", "description": "历史归档 ID（如 a1b2c3d4…）"}
                },
                "required": ["artifact_id"]
            }),
        },
    }
}

/// Execute read_artifact: read {app_data}/archives/{id}.txt and return it truncated.
pub(crate) fn execute_artifact_read(
    app: &AppHandle,
    artifact_id: &str,
) -> Result<ToolResult, String> {
    if artifact_id.is_empty() {
        return Err(ierr!(
            "read_artifact 缺少 artifact_id 参数",
            "read_artifact is missing the artifact_id parameter"
        ));
    }
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| {
            ierr!(
                "获取数据目录失败: {}",
                "Failed to resolve the app data directory: {}",
                e
            )
        })?
        .join("archives");
    let path = dir.join(format!("{}.txt", artifact_id));
    if !path.exists() {
        return Err(ierr!(
            "未找到历史归档 #{}（档案已清理或目录不存在）",
            "Archive #{} not found (already cleaned up or the directory is missing)",
            artifact_id
        ));
    }
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    Ok(file_tools::tool_ok(format!(
        "[历史归档 #{}]\n{}",
        artifact_id,
        truncate_chars_head_tail(&content, 30_000)
    )))
}

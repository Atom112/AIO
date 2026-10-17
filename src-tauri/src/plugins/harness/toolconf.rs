//! User-level config-file access for the harness tool settings pages.
//!
//! The CLI tools' own config files are the single source of truth: AIO reads
//! and writes them directly (the same file the CLI reads/writes), giving
//! near-native parity — anything configurable from the CLI is configurable in
//! the GUI, either through a structured form (later PRs) or through the
//! validated raw editor this module serves.
//!
//! Safety model:
//! - only USER-level files declared in the registry ([`registry::ToolSetupSpec::config_files`])
//!   are writable — never managed/enterprise layers, never secrets files;
//! - every write is validated by re-parsing (json/jsonc → serde_json after
//!   comment stripping, toml → toml, yaml → serde_yaml) and rejected on error;
//! - the first AIO write to an existing file creates a one-time `<path>.aio-bak`
//!   backup of the pre-AIO state (subsequent writes keep it);
//! - writes go through `fs_util::atomic_write` (temp + rename);
//! - a stale-mtime guard rejects writes when the file changed externally since
//!   the editor read it.
//!
//! Formats: comment preservation is a per-format concern — JSON/TOML keep
//! formatting by whole-file rewrite of parsed key order (comments lost for
//! JSON, TOML keeps them once structured edits move to toml_edit); YAML
//! rewrites lose comments (the UI warns; omp gets the official
//! `omp config set` channel in a later PR to avoid this). The raw editor is
//! always byte-faithful for content the user types.

use crate::ierr;
use crate::plugins::harness::registry;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};

/// One resolvable config slot as reported to the frontend.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConfigSlotInfo {
    pub id: String,
    pub label: String,
    /// json | jsonc | toml | yaml
    pub format: String,
    /// Resolved absolute path.
    pub path: String,
    pub exists: bool,
    pub size_bytes: Option<u64>,
    /// Unix epoch seconds of the last modification (staleness guard).
    pub modified: Option<u64>,
    /// A pre-AIO backup file exists next to the config.
    pub has_backup: bool,
    /// YAML slots: whole-file rewrites lose comments (the UI warns).
    pub comments_lost_on_write: bool,
}

/// One config file's content + staleness token.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConfigFileContent {
    pub content: String,
    /// Unix epoch seconds at read time; passed back on write for the
    /// external-change guard (None when the file did not exist).
    pub modified: Option<u64>,
}

/// Resolve a registry path template: `~/…` → home, `%APPDATA%/…` → roaming
/// app-data (Windows), `$DSH_HOME/…` → DSH home (DSH_HOME env, `~/.dsh` fallback).
pub(crate) fn resolve_template_path(raw: &str) -> Option<PathBuf> {
    if let Some(rest) = raw.strip_prefix("~/") {
        let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        return Some(std::path::Path::new(&std::env::var_os(home_var)?).join(rest));
    }
    if let Some(rest) = raw.strip_prefix("%APPDATA%/") {
        return Some(std::path::Path::new(&std::env::var_os("APPDATA")?).join(rest));
    }
    if let Some(rest) = raw.strip_prefix("$DSH_HOME/") {
        let dsh = std::env::var_os("DSH_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
                std::env::var_os(home_var).map(|h| Path::new(&h).join(".dsh"))
            })?;
        return Some(dsh.join(rest));
    }
    Some(PathBuf::from(raw))
}

/// Look up one tool's declared slot by id.
fn slot_spec(tool_id: &str, slot_id: &str) -> Result<registry::ConfigFileSlotSpec, String> {
    let slots = registry::lookup(tool_id)
        .ok_or_else(|| {
            ierr!(
                "未知的 harness 工具: {tool_id}",
                "Unknown harness tool: {tool_id}"
            )
        })?
        .setup
        .config_files;
    slots.into_iter().find(|s| s.id == slot_id).ok_or_else(|| {
        ierr!(
            "未知的配置文件槽位: {slot_id}",
            "Unknown config file slot: {slot_id}"
        )
    })
}

fn backup_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.push_str(".aio-bak");
    path.with_file_name(name)
}

/// List the tool's config slots with live file state.
pub fn slots(tool_id: &str) -> Result<Vec<ConfigSlotInfo>, String> {
    let spec = registry::lookup(tool_id).ok_or_else(|| {
        ierr!(
            "未知的 harness 工具: {tool_id}",
            "Unknown harness tool: {tool_id}"
        )
    })?;
    let mut out = Vec::new();
    for slot in &spec.setup.config_files {
        let Some(path) = resolve_template_path(&slot.path) else {
            continue; // prefix var unavailable on this platform (e.g. %APPDATA%)
        };
        let meta = std::fs::metadata(&path).ok();
        let exists = meta.is_some();
        out.push(ConfigSlotInfo {
            id: slot.id.clone(),
            label: slot.label.clone(),
            format: slot.format.clone(),
            path: path.to_string_lossy().into_owned(),
            exists,
            size_bytes: meta.as_ref().map(|m| m.len()),
            modified: meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
            has_backup: backup_path(&path).exists(),
            comments_lost_on_write: slot.format == "yaml",
        });
    }
    Ok(out)
}

/// Read one slot's content (2 MB safety cap — these are config files).
pub fn read(tool_id: &str, slot_id: &str) -> Result<ConfigFileContent, String> {
    let slot = slot_spec(tool_id, slot_id)?;
    let path = resolve_template_path(&slot.path)
        .ok_or_else(|| ierr!("无法解析配置路径", "Cannot resolve the config path"))?;
    if !path.exists() {
        return Ok(ConfigFileContent {
            content: String::new(),
            modified: None,
        });
    }
    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if meta.len() > 2 * 1024 * 1024 {
        return Err(ierr!(
            "配置文件超过 2MB，拒绝读取",
            "Config file exceeds 2MB; refusing to read"
        ));
    }
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    Ok(ConfigFileContent { content, modified })
}

/// Strip JSONC comments (// and /* */) outside string literals, preserving
/// newlines so error positions stay meaningful. Minimal, allocation-light.
fn strip_jsonc_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut in_string = false;
    let mut escaped = false;
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let ch_len = src[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        let ch = bytes[i];
        if in_string {
            out.push_str(&src[i..i + ch_len]);
            if escaped {
                escaped = false;
            } else if ch == b'\\' {
                escaped = true;
            } else if ch == b'"' {
                in_string = false;
            }
            i += ch_len;
            continue;
        }
        if ch == b'"' {
            in_string = true;
            out.push('"');
            i += ch_len;
            continue;
        }
        if ch == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            // line comment: skip to newline (kept for position stability)
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if ch == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                if bytes[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        out.push_str(&src[i..i + ch_len]);
        i += ch_len;
    }
    out
}

/// Validate content by the slot's format. Returns a human-readable error.
fn validate(format: &str, content: &str) -> Result<(), String> {
    match format {
        "json" => serde_json::from_str::<serde_json::Value>(content)
            .map(|_| ())
            .map_err(|e| e.to_string()),
        "jsonc" => {
            let stripped = strip_jsonc_comments(content);
            serde_json::from_str::<serde_json::Value>(&stripped)
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
        "toml" => toml::from_str::<toml::Value>(content)
            .map(|_| ())
            .map_err(|e| e.to_string()),
        "yaml" => serde_yaml::from_str::<serde_yaml::Value>(content)
            .map(|_| ())
            .map_err(|e| e.to_string()),
        // Instruction/context files: user-authored prose, no syntax to check.
        "markdown" => Ok(()),
        other => Err(format!("unknown slot format: {other}")),
    }
}

/// Write one slot: validate → one-time pre-AIO backup → stale-mtime guard →
/// atomic write. `expected_modified` is the mtime captured at read time
/// (None = the file did not exist when the editor loaded it).
pub fn write(
    tool_id: &str,
    slot_id: &str,
    content: &str,
    expected_modified: Option<u64>,
) -> Result<(), String> {
    let slot = slot_spec(tool_id, slot_id)?;
    let path = resolve_template_path(&slot.path)
        .ok_or_else(|| ierr!("无法解析配置路径", "Cannot resolve the config path"))?;
    if content.len() > 2 * 1024 * 1024 {
        return Err(ierr!(
            "内容超过 2MB，拒绝写入",
            "Content exceeds 2MB; refusing to write"
        ));
    }
    validate(&slot.format, content)?;

    // Staleness guard: reject when the file changed externally since the read.
    if let Ok(current_meta) = std::fs::metadata(&path) {
        let current = current_meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        if current != expected_modified {
            return Err(ierr!(
                "文件在编辑期间被外部修改，请重新加载后再保存",
                "The file changed externally while editing; reload and save again"
            ));
        }
    } else if expected_modified.is_some() {
        return Err(ierr!(
            "文件已被外部删除，请重新加载后再保存",
            "The file was deleted externally; reload and save again"
        ));
    }

    // One-time pre-AIO backup of the existing file.
    if path.exists() {
        let bak = backup_path(&path);
        if !bak.exists() {
            std::fs::copy(&path, &bak).map_err(|e| e.to_string())?;
        }
    }

    // Parent dirs (creating a fresh config for a not-yet-installed tool is fine).
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    crate::core::fs_util::atomic_write(&path, content.as_bytes()).map_err(|e| e.to_string())
}

/// Restore the pre-AIO backup over the current file (atomic).
pub fn restore_backup(tool_id: &str, slot_id: &str) -> Result<(), String> {
    let slot = slot_spec(tool_id, slot_id)?;
    let path = resolve_template_path(&slot.path)
        .ok_or_else(|| ierr!("无法解析配置路径", "Cannot resolve the config path"))?;
    let bak = backup_path(&path);
    if !bak.exists() {
        return Err(ierr!("没有可还原的备份", "No backup available to restore"));
    }
    let content = std::fs::read(&bak).map_err(|e| e.to_string())?;
    crate::core::fs_util::atomic_write(&path, &content).map_err(|e| e.to_string())
}

/// DSH profile layer listing (read-only overview of the profile stack).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DshProfileInfo {
    pub name: String,
    /// Directory under $DSH_HOME/profiles.
    pub path: String,
}

pub fn dsh_profiles() -> Vec<DshProfileInfo> {
    let Some(dsh) = resolve_template_path("$DSH_HOME/profiles") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dsh) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let name = entry.file_name().to_string_lossy().into_owned();
                // node_modules-style internal dirs are not profiles.
                if name.starts_with('.') || name == "node_modules" {
                    continue;
                }
                out.push(DshProfileInfo {
                    path: entry.path().to_string_lossy().into_owned(),
                    name,
                });
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

// ===== Markdown collections (skills / prompt templates / commands) =====
//
// The tool's own discovery directories (claude ~/.claude/skills/<dir>/SKILL.md,
// goose prompts/*.md, opencode commands/*.md): list/create/edit/delete with
// the same atomic-write + backup discipline as the config slots.

/// One markdown collection entry (a skill / prompt / command).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MarkdownEntryInfo {
    /// Entry name (skill dir name / file stem).
    pub name: String,
    /// Absolute file path of the entry's markdown file.
    pub path: String,
    /// First markdown heading (or first non-empty line) — one-line preview.
    pub summary: String,
}

/// Look up one declared markdown dir for a tool.
fn md_dir_spec(tool_id: &str, kind: &str) -> Result<registry::MarkdownDirSpec, String> {
    let spec = registry::lookup(tool_id).ok_or_else(|| {
        ierr!(
            "未知的 harness 工具: {tool_id}",
            "Unknown harness tool: {tool_id}"
        )
    })?;
    spec.setup
        .markdown_dirs
        .into_iter()
        .find(|d| d.kind == kind)
        .ok_or_else(|| {
            ierr!(
                "未声明的目录类型: {kind}",
                "Undeclared markdown directory: {kind}"
            )
        })
}

/// Entry name validation: filename-safe slug (lowercase letters, digits,
/// hyphens; also dots for flat stems). Rejects path separators and anything
/// that could escape the directory.
fn valid_md_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        && !name.starts_with('.')
        && !name.contains("..")
}

/// Resolve one entry's markdown file path by shape. The name is validated
/// first — entries never escape the declared directory.
fn md_entry_path(dir: &registry::MarkdownDirSpec, name: &str) -> Result<PathBuf, String> {
    if !valid_md_name(name) {
        return Err(ierr!(
            "名称只能包含字母、数字、连字符（≤64 字符）",
            "Names may only contain letters, digits and hyphens (max 64 chars)"
        ));
    }
    let base = resolve_template_path(&dir.path)
        .ok_or_else(|| ierr!("无法解析目录路径", "Cannot resolve the directory path"))?;
    Ok(match dir.shape.as_str() {
        "skill-dir" => base.join(name).join("SKILL.md"),
        _ => base.join(format!("{name}.md")),
    })
}

/// List a collection's entries (name, path, first-heading summary).
pub fn markdown_entries(tool_id: &str, kind: &str) -> Result<Vec<MarkdownEntryInfo>, String> {
    let dir = md_dir_spec(tool_id, kind)?;
    let base = resolve_template_path(&dir.path)
        .ok_or_else(|| ierr!("无法解析目录路径", "Cannot resolve the directory path"))?;
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&base) {
        for entry in entries.flatten() {
            let (name, path) = match dir.shape.as_str() {
                "skill-dir" => {
                    if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        continue;
                    }
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') {
                        continue;
                    }
                    let md = entry.path().join("SKILL.md");
                    if !md.exists() {
                        continue;
                    }
                    (name, md)
                }
                _ => {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if !name.ends_with(".md") || name.starts_with('.') {
                        continue;
                    }
                    (name.trim_end_matches(".md").to_string(), entry.path())
                }
            };
            out.push(MarkdownEntryInfo {
                name,
                path: path.to_string_lossy().into_owned(),
                summary: first_heading(&path),
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// First markdown heading (or first non-empty line, capped) of a file.
fn first_heading(path: &Path) -> String {
    let Ok(content) = std::fs::read_to_string(path) else {
        return String::new();
    };
    for line in content.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let stripped = t.trim_start_matches('#').trim();
        if !stripped.is_empty() {
            return stripped.chars().take(80).collect();
        }
    }
    String::new()
}

/// Read one entry's markdown content.
pub fn markdown_read(tool_id: &str, kind: &str, name: &str) -> Result<String, String> {
    let dir = md_dir_spec(tool_id, kind)?;
    let path = md_entry_path(&dir, name)?;
    if !path.exists() {
        return Ok(String::new());
    }
    std::fs::read_to_string(&path).map_err(|e| e.to_string())
}

/// Write (create or overwrite) one entry. Skill-dir shapes create the entry
/// directory. Atomic write; existing entries keep the one-time `.aio-bak`
/// convention via the generic backup helper.
pub fn markdown_write(tool_id: &str, kind: &str, name: &str, content: &str) -> Result<(), String> {
    let dir = md_dir_spec(tool_id, kind)?;
    let path = md_entry_path(&dir, name)?;
    if content.len() > 1024 * 1024 {
        return Err(ierr!(
            "内容超过 1MB，拒绝写入",
            "Content exceeds 1MB; refusing to write"
        ));
    }
    if path.exists() {
        let bak = backup_path(&path);
        if !bak.exists() {
            std::fs::copy(&path, &bak).map_err(|e| e.to_string())?;
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    crate::core::fs_util::atomic_write(&path, content.as_bytes()).map_err(|e| e.to_string())
}

/// Delete one entry (skill-dir: the whole skill directory; flat: the file).
pub fn markdown_delete(tool_id: &str, kind: &str, name: &str) -> Result<(), String> {
    let dir = md_dir_spec(tool_id, kind)?;
    let path = md_entry_path(&dir, name)?;
    let target = match dir.shape.as_str() {
        "skill-dir" => path
            .parent()
            .ok_or_else(|| ierr!("无效路径", "Invalid path"))?
            .to_path_buf(),
        _ => path,
    };
    if !target.exists() {
        return Ok(());
    }
    // Safety: the resolved target must live under the declared directory.
    let base = resolve_template_path(&dir.path)
        .ok_or_else(|| ierr!("无法解析目录路径", "Cannot resolve the directory path"))?;
    if !target.starts_with(&base) {
        return Err(ierr!("路径越界", "Path escapes the declared directory"));
    }
    if target.is_dir() {
        std::fs::remove_dir_all(&target).map_err(|e| e.to_string())
    } else {
        std::fs::remove_file(&target).map_err(|e| e.to_string())
    }
}

// ===== MCP server management (plan PR E) =====
//
// Each tool keeps its MCP servers in its own config file — this layer edits
// the tool's native shape (claude ~/.claude.json mcpServers, gemini/qwen
// settings.json mcpServers, opencode opencode.json mcp, codex config.toml
// [mcp_servers.<name>], goose config.yaml extensions). omp/dsh: not covered
// (their raw editors serve).

/// One MCP server entry (normalized across tools).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpEntryInfo {
    pub name: String,
    /// stdio | http
    #[serde(rename = "type")]
    pub transport: String,
    /// stdio: launch command; goose: cmd field.
    pub command: String,
    /// http: endpoint URL.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<(String, String)>,
    /// goose extensions carry an enabled flag.
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// Where a tool's MCP servers live: (slot id, config key, format family).
fn mcp_location(tool_id: &str) -> Option<(&'static str, &'static str)> {
    Some(match tool_id {
        // ~/.claude.json → mcpServers (user-level MCP; project .mcp.json is
        // workspace-scoped and stays to the user).
        "claude" => ("user-state", "mcpServers"),
        "gemini" | "qwen" => ("settings", "mcpServers"),
        "opencode" => ("global", "mcp"),
        "codex" => ("config", "mcp_servers"),
        "goose" => ("config", "extensions"),
        _ => return None,
    })
}

/// Read the tool's MCP servers (empty for tools without coverage).
pub fn mcp_list(tool_id: &str) -> Result<Vec<McpEntryInfo>, String> {
    let Some((slot_id, key)) = mcp_location(tool_id) else {
        return Ok(Vec::new());
    };
    let slot = slot_spec(tool_id, slot_id)?;
    let path = resolve_template_path(&slot.path)
        .ok_or_else(|| ierr!("无法解析配置路径", "Cannot resolve the config path"))?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    match tool_id {
        "codex" => {
            let doc: toml_edit::DocumentMut = content
                .parse()
                .map_err(|e| format!("config.toml 解析失败: {e}"))?;
            let Some(servers) = doc.get("mcp_servers").and_then(|i| i.as_table()) else {
                return Ok(Vec::new());
            };
            let mut out = Vec::new();
            for (name, item) in servers.iter() {
                if let Some(t) = item.as_table() {
                    out.push(McpEntryInfo {
                        name: name.to_string(),
                        transport: if t.get("url").is_some() {
                            "http".into()
                        } else {
                            "stdio".into()
                        },
                        command: t
                            .get("command")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        url: t.get("url").and_then(|v| v.as_str()).map(str::to_string),
                        args: t
                            .get("args")
                            .and_then(|v| v.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|x| x.as_str().map(str::to_string))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        env: t
                            .get("env")
                            .and_then(|v| v.as_table())
                            .map(|e| {
                                e.iter()
                                    .filter_map(|(k, v)| {
                                        v.as_str().map(|s| (k.to_string(), s.to_string()))
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        enabled: None,
                    });
                }
            }
            out.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(out)
        }
        "goose" => {
            let root: serde_yaml::Value =
                serde_yaml::from_str(&content).map_err(|e| format!("config.yaml 解析失败: {e}"))?;
            let Some(map) = root.get("extensions").and_then(|v| v.as_mapping()) else {
                return Ok(Vec::new());
            };
            let mut out = Vec::new();
            for (k, v) in map {
                let name = k.as_str().unwrap_or_default().to_string();
                if name.is_empty() {
                    continue;
                }
                let get = |key: &str| v.get(key).and_then(|x| x.as_str()).map(str::to_string);
                out.push(McpEntryInfo {
                    transport: "stdio".into(),
                    command: get("cmd").or_else(|| get("command")).unwrap_or_default(),
                    url: None,
                    args: v
                        .get("args")
                        .and_then(|x| x.as_sequence())
                        .map(|a| {
                            a.iter()
                                .filter_map(|i| i.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default(),
                    env: Vec::new(),
                    enabled: v.get("enabled").and_then(|x| x.as_bool()),
                    name,
                });
            }
            out.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(out)
        }
        _ => {
            // JSON tools: claude/gemini/qwen/opencode — entry objects keyed by name.
            let root: serde_json::Value =
                serde_json::from_str(&content).map_err(|e| e.to_string())?;
            let Some(map) = root.get(key).and_then(|v| v.as_object()) else {
                return Ok(Vec::new());
            };
            let mut out = Vec::new();
            for (name, e) in map {
                let get = |k: &str| e.get(k).and_then(|v| v.as_str()).map(str::to_string);
                let url = get("url");
                out.push(McpEntryInfo {
                    name: name.clone(),
                    transport: if url.is_some() {
                        "http".into()
                    } else {
                        "stdio".into()
                    },
                    command: get("command").or_else(|| get("cmd")).unwrap_or_default(),
                    url,
                    args: e
                        .get("args")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default(),
                    env: e
                        .get("env")
                        .and_then(|v| v.as_object())
                        .map(|o| {
                            o.iter()
                                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                                .collect()
                        })
                        .unwrap_or_default(),
                    enabled: e.get("enabled").and_then(|v| v.as_bool()),
                });
            }
            out.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(out)
        }
    }
}

/// Write one MCP server entry (add/update). The entry JSON carries
/// {command?, url?, args?, env?} — mapped into the tool's native shape.
pub fn mcp_set(tool_id: &str, name: &str, entry: &McpEntryInfo) -> Result<(), String> {
    let Some((slot_id, key)) = mcp_location(tool_id) else {
        return Err(ierr!(
            "该工具的 MCP 管理暂不支持（请用原始编辑器）",
            "MCP management is not available for this tool (use the raw editor)"
        ));
    };
    if !valid_md_name(name) {
        return Err(ierr!(
            "名称只能包含字母、数字、连字符",
            "Names may only contain letters, digits and hyphens"
        ));
    }
    let slot = slot_spec(tool_id, slot_id)?;
    let path = resolve_template_path(&slot.path)
        .ok_or_else(|| ierr!("无法解析配置路径", "Cannot resolve the config path"))?;
    let is_http = entry.transport == "http";
    if !is_http && entry.command.trim().is_empty() {
        return Err(ierr!(
            "stdio 服务器需要命令",
            "stdio servers require a command"
        ));
    }
    if is_http && entry.url.as_deref().unwrap_or("").trim().is_empty() {
        return Err(ierr!("http 服务器需要 URL", "http servers require a URL"));
    }
    match tool_id {
        "codex" => {
            let content = if path.exists() {
                std::fs::read_to_string(&path).map_err(|e| e.to_string())?
            } else {
                String::new()
            };
            let mut doc: toml_edit::DocumentMut = content
                .parse()
                .map_err(|e| format!("config.toml 解析失败: {e}"))?;
            let mut table = toml_edit::Table::new();
            if is_http {
                table["url"] = toml_edit::value(entry.url.clone().unwrap_or_default());
            } else {
                table["command"] = toml_edit::value(entry.command.trim());
                if !entry.args.is_empty() {
                    let mut arr = toml_edit::Array::new();
                    for a in &entry.args {
                        arr.push(a.as_str());
                    }
                    table["args"] = toml_edit::value(arr);
                }
                if !entry.env.is_empty() {
                    let mut env = toml_edit::InlineTable::new();
                    for (k, v) in &entry.env {
                        env.insert(k, toml_edit::Value::from(v.as_str()));
                    }
                    table["env"] = toml_edit::value(env);
                }
            }
            let servers = doc
                .entry("mcp_servers")
                .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
            servers[name] = toml_edit::Item::Table(table);
            write_with_fresh_mtime(tool_id, slot_id, &path, &doc.to_string())
        }
        "goose" => {
            // Full YAML round-trip (comments lost — one-time backup covers).
            let content = if path.exists() {
                std::fs::read_to_string(&path).map_err(|e| e.to_string())?
            } else {
                String::new()
            };
            let mut root: serde_yaml::Value = if content.trim().is_empty() {
                serde_yaml::Value::Mapping(Default::default())
            } else {
                serde_yaml::from_str(&content).map_err(|e| format!("config.yaml 解析失败: {e}"))?
            };
            if !root.is_mapping() {
                return Err(ierr!(
                    "配置文件顶层不是映射",
                    "The config file's top level is not a mapping"
                ));
            }
            let mut entry_map = serde_yaml::Mapping::new();
            entry_map.insert("type".into(), "stdio".into());
            entry_map.insert("cmd".into(), entry.command.trim().into());
            if !entry.args.is_empty() {
                entry_map.insert(
                    "args".into(),
                    serde_yaml::Value::Sequence(
                        entry.args.iter().map(|a| a.as_str().into()).collect(),
                    ),
                );
            }
            entry_map.insert("enabled".into(), entry.enabled.unwrap_or(true).into());
            let map = root.as_mapping_mut().unwrap();
            let ext = map
                .entry("extensions".into())
                .or_insert(serde_yaml::Value::Mapping(Default::default()));
            ext.as_mapping_mut()
                .ok_or_else(|| ierr!("extensions 不是映射", "extensions is not a mapping"))?
                .insert(name.into(), serde_yaml::Value::Mapping(entry_map));
            let serialized = serde_yaml::to_string(&root).map_err(|e| e.to_string())?;
            write_with_fresh_mtime(tool_id, slot_id, &path, &serialized)
        }
        _ => {
            // JSON tools: write the tool's native entry shape.
            let mut root: serde_json::Value = if path.exists() {
                let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
                serde_json::from_str(&content).map_err(|e| e.to_string())?
            } else {
                serde_json::Value::Null
            };
            if root.is_null() {
                root = serde_json::Map::new().into();
            }
            if !root.is_object() {
                return Err(ierr!(
                    "配置文件顶层不是对象",
                    "The config file's top level is not an object"
                ));
            }
            let native: serde_json::Value = match tool_id {
                "claude" => {
                    if is_http {
                        json!({"type": "http", "url": entry.url})
                    } else {
                        json!({"type": "stdio", "command": entry.command.trim(), "args": entry.args, "env": env_object(entry.env.clone())})
                    }
                }
                "opencode" => {
                    if is_http {
                        json!({"type": "remote", "url": entry.url})
                    } else {
                        json!({"type": "local", "command": entry.command.trim(), "args": entry.args, "enabled": true})
                    }
                }
                _ => {
                    // gemini/qwen: plain command shape.
                    if is_http {
                        json!({"url": entry.url})
                    } else {
                        json!({"command": entry.command.trim(), "args": entry.args, "env": env_object(entry.env.clone())})
                    }
                }
            };
            // Ensure the parent key exists as an object.
            if !root.get(key).map(|v| v.is_object()).unwrap_or(false) {
                json_set(&mut root, key, serde_json::Map::new().into());
            }
            json_set(&mut root, &format!("{key}.{name}"), native);
            let serialized = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
            write_with_fresh_mtime(tool_id, slot_id, &path, &serialized)
        }
    }
}

/// Delete one MCP server entry.
pub fn mcp_delete(tool_id: &str, name: &str) -> Result<(), String> {
    let Some((slot_id, key)) = mcp_location(tool_id) else {
        return Err(ierr!(
            "该工具的 MCP 管理暂不支持",
            "MCP management is not available for this tool"
        ));
    };
    let slot = slot_spec(tool_id, slot_id)?;
    let path = resolve_template_path(&slot.path)
        .ok_or_else(|| ierr!("无法解析配置路径", "Cannot resolve the config path"))?;
    if !path.exists() {
        return Ok(());
    }
    match tool_id {
        "codex" => {
            let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let mut doc: toml_edit::DocumentMut = content
                .parse()
                .map_err(|e| format!("config.toml 解析失败: {e}"))?;
            if let Some(servers) = doc.get_mut("mcp_servers").and_then(|i| i.as_table_mut()) {
                servers.remove(name);
            }
            write_with_fresh_mtime(tool_id, slot_id, &path, &doc.to_string())
        }
        "goose" => {
            let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let mut root: serde_yaml::Value =
                serde_yaml::from_str(&content).map_err(|e| format!("config.yaml 解析失败: {e}"))?;
            if let Some(map) = root.get_mut("extensions").and_then(|v| v.as_mapping_mut()) {
                map.remove(name);
            }
            let serialized = serde_yaml::to_string(&root).map_err(|e| e.to_string())?;
            write_with_fresh_mtime(tool_id, slot_id, &path, &serialized)
        }
        _ => {
            let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let mut root: serde_json::Value =
                serde_json::from_str(&content).map_err(|e| e.to_string())?;
            if let Some(servers) = root.get_mut(key).and_then(|v| v.as_object_mut()) {
                servers.remove(name);
            }
            let serialized = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
            write_with_fresh_mtime(tool_id, slot_id, &path, &serialized)
        }
    }
}

/// serde_json object from env pairs (insertion order preserved).
fn env_object(env: Vec<(String, String)>) -> serde_json::Map<String, serde_json::Value> {
    let mut m = serde_json::Map::new();
    for (k, v) in env {
        m.insert(k, serde_json::Value::String(v));
    }
    m
}

// ===== Structured key layer (plan PR B: curated per-tool keys) =====
//
// The tool's own config file stays the source of truth; this layer exposes a
// curated set of well-known keys as typed descriptors the settings page
// renders as form rows. Setting a key rewrites the file (nested dotted paths,
// unknown keys preserved); setting it to null REMOVES the key (revert to the
// tool's default), mirroring how CLI users edit these files by hand.

/// One curated key's current value (mirrored to the frontend; internally
/// tagged by `kind`: str/bool/num/strList/kv).
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StructuredValue {
    Str { value: Option<String> },
    Bool { value: Option<bool> },
    Num { value: Option<f64> },
    StrList { value: Vec<String> },
    Kv { value: Vec<(String, String)> },
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StructuredKeyInfo {
    /// Dotted path into the config file ("permissions.defaultMode").
    pub key: String,
    /// Display label (English; the GUI i18n's known keys).
    pub label: String,
    /// string | enum | bool | number | string-list | kv
    pub kind: String,
    /// Enum values when kind == enum.
    pub options: Vec<String>,
    /// Current value (None-shaped = key absent → tool default).
    pub value: StructuredValue,
    /// Tool-provided description (omp's config catalog carries one; shown as a
    /// tooltip on the label row).
    pub description: Option<String>,
}

/// Curated declaration: (dotted key, label, kind, enum options).
type KeyDecl = (
    &'static str,
    &'static str,
    &'static str,
    &'static [&'static str],
);

/// Which backend serves a tool's structured keys:
/// - Json: read-modify-write on the tool's settings.json (PR B tools);
/// - Toml: toml_edit on codex's config.toml — comments and table layout
///   preserved through structured edits;
/// - YamlLines: line-surgery on goose's config.yaml — only the one
///   `KEY: value` line is touched, so comments elsewhere survive;
/// - OmpCli: omp's own official channel (`omp config list --json` reads the
///   full typed catalog; `omp config set/reset` writes with omp's parser —
///   zero schema reimplementation, perfect parity with the CLI).
enum StructuredBackend {
    Json { slot_id: String, decl: Vec<KeyDecl> },
    Toml { slot_id: String, decl: Vec<KeyDecl> },
    YamlLines { slot_id: String, decl: Vec<KeyDecl> },
    OmpCli,
}

impl std::fmt::Debug for StructuredBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StructuredBackend::Json { .. } => write!(f, "Json"),
            StructuredBackend::Toml { .. } => write!(f, "Toml"),
            StructuredBackend::YamlLines { .. } => write!(f, "YamlLines"),
            StructuredBackend::OmpCli => write!(f, "OmpCli"),
        }
    }
}

/// Per-tool structured backend + curated key declarations.
fn structured_decl(tool_id: &str) -> Option<StructuredBackend> {
    match tool_id {
        "claude" => Some(StructuredBackend::Json {
            slot_id: "settings".to_string(),
            decl: vec![
                ("model", "Default model", "string", &[]),
                (
                    "permissions.defaultMode",
                    "Default permission mode",
                    "enum",
                    &[
                        "default",
                        "acceptEdits",
                        "askEdits",
                        "viewOnly",
                        "bypassPermissions",
                        "plan",
                    ],
                ),
                ("alwaysThinkingEnabled", "Always thinking", "bool", &[]),
                ("cleanupPeriodDays", "Session cleanup (days)", "number", &[]),
                ("permissions.allow", "Allow rules", "string-list", &[]),
                ("permissions.ask", "Ask rules", "string-list", &[]),
                ("permissions.deny", "Deny rules", "string-list", &[]),
                ("env", "Environment variables", "kv", &[]),
            ],
        }),
        "gemini" | "qwen" => Some(StructuredBackend::Json {
            slot_id: "settings".to_string(),
            decl: vec![
                ("model.name", "Default model", "string", &[]),
                (
                    "tools.approvalMode",
                    "Approval mode",
                    "enum",
                    &["default", "auto_edit", "yolo", "plan"],
                ),
                ("context.fileName", "Context file name", "string", &[]),
            ],
        }),
        "opencode" => Some(StructuredBackend::Json {
            slot_id: "global".to_string(),
            decl: vec![
                ("model", "Default model", "string", &[]),
                ("autoupdate", "Auto update", "bool", &[]),
            ],
        }),
        // codex config.toml — verified keys from the official config reference.
        "codex" => Some(StructuredBackend::Toml {
            slot_id: "config".to_string(),
            decl: vec![
                ("model", "Default model", "string", &[]),
                (
                    "approval_policy",
                    "Approval policy",
                    "enum",
                    &["untrusted", "on-request", "never"],
                ),
                (
                    "sandbox_mode",
                    "Sandbox mode",
                    "enum",
                    &["read-only", "workspace-write", "danger-full-access"],
                ),
                ("model_context_window", "Context window", "number", &[]),
                ("notify", "Notify command", "string-list", &[]),
            ],
        }),
        // goose config.yaml — top-level scalars, line-surgery friendly.
        "goose" => Some(StructuredBackend::YamlLines {
            slot_id: "config".to_string(),
            decl: vec![
                ("GOOSE_PROVIDER", "Provider", "string", &[]),
                ("GOOSE_MODEL", "Default model", "string", &[]),
                (
                    "GOOSE_MODE",
                    "Tool execution mode",
                    "enum",
                    &["auto", "approve", "smart_approve", "chat"],
                ),
                ("GOOSE_TEMPERATURE", "Temperature", "number", &[]),
                ("GOOSE_MAX_TOKENS", "Max tokens", "number", &[]),
                ("GOOSE_MAX_TURNS", "Max turns", "number", &[]),
                ("GOOSE_TOOLSHIM", "Tool interpretation", "bool", &[]),
            ],
        }),
        // omp: the official config channel (full typed catalog via CLI).
        "omp" => Some(StructuredBackend::OmpCli),
        _ => None,
    }
}

/// Read a dotted path from a parsed JSON object.
fn json_get<'a>(root: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut cur = root;
    for part in path.split('.') {
        cur = cur.get(part)?;
    }
    Some(cur)
}

/// Set (or remove on `value = null`) a dotted path in a parsed JSON object,
/// creating intermediate objects as needed. Returns whether the tree changed.
fn json_set(root: &mut serde_json::Value, path: &str, value: serde_json::Value) -> bool {
    let parts: Vec<&str> = path.split('.').collect();
    let mut cur = root;
    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            if value.is_null() {
                return if let Some(map) = cur.as_object_mut() {
                    map.remove(*part).is_some()
                } else {
                    false
                };
            }
            if !cur.is_object() {
                *cur = serde_json::Map::new().into();
            }
            return if let Some(map) = cur.as_object_mut() {
                let changed = map.get(*part) != Some(&value);
                map.insert(part.to_string(), value);
                changed
            } else {
                false
            };
        }
        if !cur.is_object() {
            *cur = serde_json::Map::new().into();
        }
        let obj = cur.as_object_mut().unwrap();
        cur = obj
            .entry(part.to_string())
            .or_insert_with(|| serde_json::Map::new().into());
    }
    false
}

/// Convert a JSON value into the descriptor's StructuredValue shape.
fn to_structured(kind: &str, v: &serde_json::Value) -> StructuredValue {
    match kind {
        "bool" => StructuredValue::Bool { value: v.as_bool() },
        "number" => StructuredValue::Num { value: v.as_f64() },
        "enum" => StructuredValue::Str {
            value: v.as_str().map(str::to_string),
        },
        "string-list" => StructuredValue::StrList {
            value: v
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        },
        "kv" => StructuredValue::Kv {
            value: v
                .as_object()
                .map(|o| {
                    o.iter()
                        .filter_map(|(k, x)| x.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                })
                .unwrap_or_default(),
        },
        _ => StructuredValue::Str {
            value: v.as_str().map(str::to_string),
        },
    }
}

/// Structured view of the tool's curated config keys (empty for tools without
/// declarations — the raw editor still serves them).
pub fn structured_keys(tool_id: &str) -> Result<Vec<StructuredKeyInfo>, String> {
    match structured_decl(tool_id) {
        None => Ok(Vec::new()),
        Some(StructuredBackend::Json { slot_id, decl }) => {
            structured_keys_json(tool_id, &slot_id, &decl)
        }
        Some(StructuredBackend::Toml { slot_id, decl }) => {
            structured_keys_toml(tool_id, &slot_id, &decl)
        }
        Some(StructuredBackend::YamlLines { slot_id, decl }) => {
            structured_keys_yaml(tool_id, &slot_id, &decl)
        }
        Some(StructuredBackend::OmpCli) => structured_keys_omp(),
    }
}

/// JSON backend: read the settings file (defaults when absent), map the
/// declared keys. opencode guard: a .jsonc config disables the section.
fn structured_keys_json(
    tool_id: &str,
    slot_id: &str,
    decl: &[KeyDecl],
) -> Result<Vec<StructuredKeyInfo>, String> {
    let slot = slot_spec(tool_id, slot_id)?;
    let path = resolve_template_path(&slot.path)
        .ok_or_else(|| ierr!("无法解析配置路径", "Cannot resolve the config path"))?;
    // opencode: when the user runs on a .jsonc file, structured writes would
    // create a parallel .json — refuse and let the raw editor serve instead.
    if tool_id == "opencode" && !path.exists() {
        if let Ok(jsonc) = slot_spec(tool_id, "global-jsonc") {
            if let Some(p) = resolve_template_path(&jsonc.path) {
                if p.exists() {
                    return Ok(Vec::new());
                }
            }
        }
    }
    let root: serde_json::Value = if path.exists() {
        let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        serde_json::from_str(&content).map_err(|e| e.to_string())?
    } else {
        serde_json::Value::Null
    };
    let mut out = Vec::new();
    for (key, label, kind, options) in decl {
        let value = json_get(&root, key)
            .map(|v| to_structured(kind, v))
            .unwrap_or_else(|| default_structured(kind));
        out.push(StructuredKeyInfo {
            key: key.to_string(),
            label: label.to_string(),
            kind: kind.to_string(),
            options: options.iter().map(|s| s.to_string()).collect(),
            value,
            description: None,
        });
    }
    Ok(out)
}

/// TOML backend (codex): toml_edit parse, dotted-path read.
fn structured_keys_toml(
    tool_id: &str,
    slot_id: &str,
    decl: &[KeyDecl],
) -> Result<Vec<StructuredKeyInfo>, String> {
    let slot = slot_spec(tool_id, slot_id)?;
    let path = resolve_template_path(&slot.path)
        .ok_or_else(|| ierr!("无法解析配置路径", "Cannot resolve the config path"))?;
    let doc: toml_edit::DocumentMut = if path.exists() {
        let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        content
            .parse()
            .map_err(|e| format!("config.toml 解析失败: {e}"))?
    } else {
        toml_edit::DocumentMut::new()
    };
    let mut out = Vec::new();
    for (key, label, kind, options) in decl {
        let item = doc.get(key);
        let value = match item {
            Some(toml_edit::Item::Value(v)) => toml_value_to_structured(kind, v),
            _ => default_structured(kind),
        };
        out.push(StructuredKeyInfo {
            key: key.to_string(),
            label: label.to_string(),
            kind: kind.to_string(),
            options: options.iter().map(|s| s.to_string()).collect(),
            value,
            description: None,
        });
    }
    Ok(out)
}

/// toml_edit value → StructuredValue by the declared kind.
fn toml_value_to_structured(kind: &str, v: &toml_edit::Value) -> StructuredValue {
    match kind {
        "bool" => StructuredValue::Bool { value: v.as_bool() },
        "number" => StructuredValue::Num {
            value: v.as_integer().map(|i| i as f64).or_else(|| v.as_float()),
        },
        "string-list" => StructuredValue::StrList {
            value: v
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        },
        _ => StructuredValue::Str {
            value: v.as_str().map(str::to_string),
        },
    }
}

/// YamlLines backend (goose): parse the file for CURRENT values (display only
/// — writes go through line surgery). Top-level scalars only.
fn structured_keys_yaml(
    tool_id: &str,
    slot_id: &str,
    decl: &[KeyDecl],
) -> Result<Vec<StructuredKeyInfo>, String> {
    let slot = slot_spec(tool_id, slot_id)?;
    let path = resolve_template_path(&slot.path)
        .ok_or_else(|| ierr!("无法解析配置路径", "Cannot resolve the config path"))?;
    let root: serde_yaml::Value = if path.exists() {
        let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        serde_yaml::from_str(&content).map_err(|e| format!("config.yaml 解析失败: {e}"))?
    } else {
        serde_yaml::Value::Null
    };
    let mut out = Vec::new();
    for (key, label, kind, options) in decl {
        let v = root.get(*key);
        let value = match v {
            Some(serde_yaml::Value::Bool(b)) => StructuredValue::Bool { value: Some(*b) },
            Some(serde_yaml::Value::Number(n)) => StructuredValue::Num { value: n.as_f64() },
            Some(serde_yaml::Value::String(s)) => StructuredValue::Str {
                value: Some(s.clone()),
            },
            _ => default_structured(kind),
        };
        out.push(StructuredKeyInfo {
            key: key.to_string(),
            label: label.to_string(),
            kind: kind.to_string(),
            options: options.iter().map(|s| s.to_string()).collect(),
            value,
            description: None,
        });
    }
    Ok(out)
}

/// omp backend: `omp config list --json` — the full typed catalog. The key
/// count is large (~450); the GUI groups by key prefix. When omp is not on
/// PATH, an empty catalog is returned (the section hides; raw editor serves).
fn structured_keys_omp() -> Result<Vec<StructuredKeyInfo>, String> {
    let output = run_omp_config(&["config", "list", "--json"])?;
    let catalog: serde_json::Value =
        serde_json::from_str(&output).map_err(|e| format!("omp config list 解析失败: {e}"))?;
    let Some(map) = catalog.as_object() else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (key, meta) in map {
        let ty = meta
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("string");
        let desc = meta
            .get("description")
            .and_then(|d| d.as_str())
            .filter(|d| !d.is_empty())
            .map(str::to_string);
        let raw = meta.get("value");
        // omp enums don't list their options in the catalog — render as text
        // (omp validates on set and its error lists the valid values).
        let kind = match ty {
            "boolean" => "bool",
            "number" => "number",
            "array" => "string-list",
            "record" => "kv",
            _ => "string",
        };
        let value = raw
            .map(|v| to_structured(kind, v))
            .unwrap_or_else(|| default_structured(kind));
        // Key path doubles as the label (omp's own vocabulary).
        out.push(StructuredKeyInfo {
            key: key.clone(),
            label: key.clone(),
            kind: kind.to_string(),
            options: Vec::new(),
            value,
            description: desc,
        });
    }
    out.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(out)
}

/// Default (unset) StructuredValue by kind.
fn default_structured(kind: &str) -> StructuredValue {
    match kind {
        "bool" => StructuredValue::Bool { value: None },
        "number" => StructuredValue::Num { value: None },
        "string-list" => StructuredValue::StrList { value: Vec::new() },
        "kv" => StructuredValue::Kv { value: Vec::new() },
        _ => StructuredValue::Str { value: None },
    }
}

/// Set one curated key. `value` of JSON null removes the key (tool default).
/// Every backend preserves the rest of the file; writes ride the standard
/// pipeline (validate → backup → stale guard → atomic) where a file is
/// involved, or omp's own parser for the CLI channel.
pub fn structured_set(tool_id: &str, key: &str, value: serde_json::Value) -> Result<(), String> {
    match structured_decl(tool_id) {
        None => Err(ierr!(
            "该工具没有结构化配置键",
            "This tool has no structured config keys"
        )),
        Some(StructuredBackend::Json { slot_id, decl }) => {
            if !decl.iter().any(|(k, ..)| *k == key) {
                return Err(ierr!(
                    "未声明的配置键: {key}",
                    "Undeclared config key: {key}"
                ));
            }
            structured_set_json(tool_id, &slot_id, key, value)
        }
        Some(StructuredBackend::Toml { slot_id, decl }) => {
            if !decl.iter().any(|(k, ..)| *k == key) {
                return Err(ierr!(
                    "未声明的配置键: {key}",
                    "Undeclared config key: {key}"
                ));
            }
            structured_set_toml(tool_id, &slot_id, key, value)
        }
        Some(StructuredBackend::YamlLines { slot_id, decl }) => {
            if !decl.iter().any(|(k, ..)| *k == key) {
                return Err(ierr!(
                    "未声明的配置键: {key}",
                    "Undeclared config key: {key}"
                ));
            }
            structured_set_yaml(tool_id, &slot_id, key, value)
        }
        Some(StructuredBackend::OmpCli) => structured_set_omp(key, &value),
    }
}

/// JSON backend: read → json_set → pretty-serialize → pipeline write.
fn structured_set_json(
    tool_id: &str,
    slot_id: &str,
    key: &str,
    value: serde_json::Value,
) -> Result<(), String> {
    let slot = slot_spec(tool_id, slot_id)?;
    let path = resolve_template_path(&slot.path)
        .ok_or_else(|| ierr!("无法解析配置路径", "Cannot resolve the config path"))?;
    let mut root: serde_json::Value = if path.exists() {
        let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        serde_json::from_str(&content).map_err(|e| e.to_string())?
    } else {
        serde_json::Value::Null
    };
    if root.is_null() {
        root = serde_json::Map::new().into();
    }
    if !root.is_object() {
        return Err(ierr!(
            "配置文件顶层不是对象",
            "The config file's top level is not an object"
        ));
    }
    json_set(&mut root, key, value);
    let serialized = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    write_with_fresh_mtime(tool_id, slot_id, &path, &serialized)
}

/// TOML backend (codex): toml_edit set/remove — comments and table layout are
/// preserved byte-faithfully outside the edited line.
fn structured_set_toml(
    tool_id: &str,
    slot_id: &str,
    key: &str,
    value: serde_json::Value,
) -> Result<(), String> {
    let slot = slot_spec(tool_id, slot_id)?;
    let path = resolve_template_path(&slot.path)
        .ok_or_else(|| ierr!("无法解析配置路径", "Cannot resolve the config path"))?;
    let mut doc: toml_edit::DocumentMut = if path.exists() {
        let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        content
            .parse()
            .map_err(|e| format!("config.toml 解析失败: {e}"))?
    } else {
        toml_edit::DocumentMut::new()
    };
    if value.is_null() {
        doc.remove(key);
    } else {
        let toml_value = json_to_toml_value(&value).ok_or_else(|| {
            ierr!(
                "该值无法写入 TOML（对象/嵌套结构请用原始编辑器）",
                "This value cannot be written as TOML (use the raw editor for objects)"
            )
        })?;
        doc[key] = toml_edit::Item::Value(toml_value);
    }
    let serialized = doc.to_string();
    write_with_fresh_mtime(tool_id, slot_id, &path, &serialized)
}

/// serde_json value → toml_edit value (scalars + arrays; objects are out of
/// scope for the curated keys — the raw editor serves those).
fn json_to_toml_value(v: &serde_json::Value) -> Option<toml_edit::Value> {
    match v {
        serde_json::Value::Bool(b) => Some((*b).into()),
        serde_json::Value::Number(n) => n
            .as_i64()
            .map(|i| i.into())
            .or_else(|| n.as_f64().map(|f| f.into())),
        serde_json::Value::String(s) => Some(s.as_str().into()),
        serde_json::Value::Array(a) => {
            let mut arr = toml_edit::Array::new();
            for item in a {
                arr.push(json_to_toml_value(item)?);
            }
            Some(arr.into())
        }
        _ => None,
    }
}

/// YamlLines backend (goose): line surgery — replace/remove the single
/// `KEY: value` line (top-level keys only), leaving every other line
/// (comments included) byte-identical. Missing key appends at the end.
fn structured_set_yaml(
    tool_id: &str,
    slot_id: &str,
    key: &str,
    value: serde_json::Value,
) -> Result<(), String> {
    let slot = slot_spec(tool_id, slot_id)?;
    let path = resolve_template_path(&slot.path)
        .ok_or_else(|| ierr!("无法解析配置路径", "Cannot resolve the config path"))?;
    let original = if path.exists() {
        std::fs::read_to_string(&path).map_err(|e| e.to_string())?
    } else {
        String::new()
    };
    let updated = yaml_line_surgery(&original, key, &value)?;
    write_with_fresh_mtime(tool_id, slot_id, &path, &updated)
}

/// Replace/remove/append one top-level `KEY: value` line in a YAML document.
/// null removes the line; scalars serialize as plain YAML (strings quoted when
/// they contain YAML special characters).
fn yaml_line_surgery(src: &str, key: &str, value: &serde_json::Value) -> Result<String, String> {
    let lines: Vec<&str> = src.lines().collect();
    let prefix = format!("{key}:");
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 1);
    let mut found = false;
    for line in lines {
        let trimmed = line.trim_start();
        // Top-level match: the line starts with the key (no leading spaces) and
        // is a mapping entry, not a nested key with the same suffix.
        if !found
            && line.starts_with(&prefix)
            && (trimmed == prefix || trimmed.starts_with(&format!("{prefix} ")))
        {
            found = true;
            if value.is_null() {
                continue; // remove the line
            }
            out.push(format!("{key}: {}", yaml_scalar(value)));
            continue;
        }
        out.push(line.to_string());
    }
    if !found && !value.is_null() {
        if !out.is_empty() && !out.last().map(|l| l.is_empty()).unwrap_or(true) {
            out.push(String::new());
        }
        out.push(format!("{key}: {}", yaml_scalar(value)));
    }
    let mut result = out.join("\n");
    if src.ends_with('\n') || !result.is_empty() {
        result.push('\n');
    }
    Ok(result)
}

/// Serialize a JSON scalar as a YAML scalar (strings quoted when needed).
fn yaml_scalar(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => {
            let needs_quotes = s.is_empty()
                || s.contains(':')
                || s.contains('#')
                || s.contains('"')
                || s.contains('\'')
                || s.contains('\n')
                || s.trim() != s
                || s.starts_with('[')
                || s.starts_with('{')
                || s.starts_with('&')
                || s.starts_with('*')
                || s.starts_with('!')
                || s.starts_with('%')
                || s.starts_with('@')
                || s.starts_with('`')
                || s == "true"
                || s == "false"
                || s == "null"
                || s == "~";
            if needs_quotes {
                serde_yaml::to_string(s)
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            } else {
                s.clone()
            }
        }
        other => serde_yaml::to_string(other)
            .unwrap_or_default()
            .trim()
            .to_string(),
    }
}

/// omp backend: `omp config set <key> <value>` (omp's own parser/validator —
/// its error text, including valid enum values, surfaces to the UI). null →
/// `omp config reset <key>` (omp's official revert).
fn structured_set_omp(key: &str, value: &serde_json::Value) -> Result<(), String> {
    if value.is_null() {
        run_omp_config(&["config", "reset", key])?;
        return Ok(());
    }
    let arg = omp_arg_string(value);
    run_omp_config(&["config", "set", key, &arg])?;
    Ok(())
}

/// Convert a JSON value to omp's `config set` value-string convention
/// (booleans true/false; numbers decimal; arrays/records as JSON strings —
/// omp parses these against the key's schema type).
fn omp_arg_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Run `omp <args…>`, capture stdout, surface stderr on non-zero exit.
/// PATH-only resolution (SEC-06); no console window on Windows.
fn run_omp_config(args: &[&str]) -> Result<String, String> {
    use std::process::Command as StdCommand;
    let program = crate::utils::exec_resolve::resolve_or_keep("omp");
    let mut cmd = StdCommand::new(&program);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = cmd
        .output()
        .map_err(|e| format!("无法启动 omp ({program}): {e}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Err(format!(
            "omp config 失败: {}",
            if err.is_empty() { out } else { err }
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Read-modify-write helper: capture the file's current mtime (or None when
/// absent) right before writing so the pipeline's staleness guard passes for
/// our own read.
fn write_with_fresh_mtime(
    tool_id: &str,
    slot_id: &str,
    path: &Path,
    serialized: &str,
) -> Result<(), String> {
    let expected = if path.exists() {
        std::fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
    } else {
        None
    };
    write(tool_id, slot_id, serialized, expected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn jsonc_comment_stripping() {
        let src = r#"{
  // line comment
  "$schema": "https://opencode.ai/config.json", /* block
     comment */ "model": "anthropic/claude-sonnet-4-5",
  "url": "https://example.com/a//b" // trailing
}"#;
        let stripped = strip_jsonc_comments(src);
        let v: serde_json::Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(v["model"], "anthropic/claude-sonnet-4-5");
        assert_eq!(v["url"], "https://example.com/a//b");
        // string containing comment markers stays intact
        assert!(stripped.contains("https://example.com/a//b"));
    }

    #[test]
    fn validation_by_format() {
        assert!(validate("json", r#"{"a":1}"#).is_ok());
        assert!(validate("json", r#"{"a":}"#).is_err());
        assert!(validate("jsonc", "{\n// c\n\"a\": 1\n}").is_ok());
        assert!(validate("toml", "model = \"gpt-5.2\"\n").is_ok());
        assert!(validate("toml", "model =\n").is_err());
        assert!(validate("yaml", "model: gpt-5.2\n").is_ok());
        assert!(validate("yaml", "model: [unclosed\n").is_err());
    }

    #[test]
    fn backup_path_shape() {
        let p = backup_path(Path::new("C:/x/.claude/settings.json"));
        assert!(p.to_string_lossy().ends_with("settings.json.aio-bak"));
    }

    #[test]
    fn unknown_slot_rejected() {
        assert!(slot_spec("claude", "nope").is_err());
        assert!(slot_spec("claude", "settings").is_ok());
        // custom agents declare no slots
        assert!(slot_spec("nonexistent", "settings").is_err());
    }

    #[test]
    fn json_set_nested_and_remove() {
        let mut root: serde_json::Value =
            serde_json::from_str(r#"{"a":1,"permissions":{"allow":["Bash(git *)"]}}"#).unwrap();
        // nested set creates intermediate objects
        assert!(json_set(
            &mut root,
            "permissions.defaultMode",
            json!("acceptEdits")
        ));
        assert_eq!(root["permissions"]["defaultMode"], "acceptEdits");
        assert_eq!(root["permissions"]["allow"][0], "Bash(git *)");
        // same value → no change reported
        assert!(!json_set(
            &mut root,
            "permissions.defaultMode",
            json!("acceptEdits")
        ));
        // null removes the leaf only
        assert!(json_set(&mut root, "permissions.defaultMode", json!(null)));
        assert!(root["permissions"].get("defaultMode").is_none());
        assert_eq!(root["permissions"]["allow"][0], "Bash(git *)");
        // deep path materializes objects
        json_set(&mut root, "x.y.z", json!(true));
        assert_eq!(root["x"]["y"]["z"], true);
    }

    #[test]
    fn structured_declarations_cover_all_backends() {
        // JSON tools (PR B)
        for tool in ["claude", "gemini", "qwen", "opencode"] {
            let decl = structured_decl(tool);
            assert!(decl.is_some(), "{tool} should declare structured keys");
            match decl.unwrap() {
                StructuredBackend::Json { slot_id, decl } => {
                    let expected = if tool == "opencode" {
                        "global"
                    } else {
                        "settings"
                    };
                    assert_eq!(slot_id, expected);
                    assert!(!decl.is_empty());
                }
                other => panic!("{tool} should be the Json backend, got {other:?}"),
            }
        }
        // codex → Toml, goose → YamlLines, omp → OmpCli (PR C)
        assert!(matches!(
            structured_decl("codex"),
            Some(StructuredBackend::Toml { .. })
        ));
        assert!(matches!(
            structured_decl("goose"),
            Some(StructuredBackend::YamlLines { .. })
        ));
        assert!(matches!(
            structured_decl("omp"),
            Some(StructuredBackend::OmpCli)
        ));
        // dsh/custom: raw editor only
        for tool in ["dsh", "nope"] {
            assert!(structured_decl(tool).is_none());
        }
    }

    #[test]
    fn structured_set_rejects_undeclared_keys() {
        assert!(structured_set("claude", "nope.hack", json!(1)).is_err());
        assert!(structured_set("codex", "nope.hack", json!(1)).is_err());
        assert!(structured_set("goose", "nope.hack", json!(1)).is_err());
    }

    /// toml_edit keeps comments and table layout through structured edits
    /// (a removed key takes its OWN comments with it — toml_edit decor
    /// semantics — while untouched keys keep theirs byte-identically).
    #[test]
    fn toml_edit_preserves_comments() {
        let src =
            "model = \"gpt-5.2\" # inline\n\n# keep me\nother = 1\n\n[not_touched]\nkey = 1\n";
        let mut doc: toml_edit::DocumentMut = src.parse().unwrap();
        doc["sandbox_mode"] = toml_edit::Item::Value("workspace-write".into());
        let out = doc.to_string();
        assert!(out.contains("model = \"gpt-5.2\" # inline"));
        assert!(out.contains("# keep me\nother = 1"));
        assert!(out.contains("[not_touched]"));
        assert!(out.contains("sandbox_mode = \"workspace-write\""));
        // removal drops the key and its own decor; others untouched
        let mut doc2: toml_edit::DocumentMut = out.parse().unwrap();
        doc2.remove("model");
        let out2 = doc2.to_string();
        assert!(!out2.contains("model ="));
        assert!(out2.contains("# keep me\nother = 1"));
        assert!(out2.contains("sandbox_mode = \"workspace-write\""));
        assert!(out2.contains("[not_touched]\nkey = 1"));
    }

    /// YAML line surgery: replace/remove/append one top-level key, comments
    /// and unrelated lines byte-identical.
    #[test]
    fn yaml_line_surgery_roundtrip() {
        let src = "# goose config\nGOOSE_PROVIDER: anthropic\nGOOSE_MODE: approve # note\n\nextensions:\n  developer:\n    enabled: true\n";
        // replace
        let out = yaml_line_surgery(src, "GOOSE_MODE", &json!("smart_approve")).unwrap();
        assert!(out.contains("GOOSE_MODE: smart_approve"));
        assert!(out.contains("# goose config"));
        assert!(out.contains("GOOSE_PROVIDER: anthropic"));
        assert!(out.contains("extensions:"));
        assert!(!out.contains("approve # note"));
        // remove
        let out2 = yaml_line_surgery(&out, "GOOSE_PROVIDER", &json!(null)).unwrap();
        assert!(!out2.contains("GOOSE_PROVIDER"));
        assert!(out2.contains("GOOSE_MODE: smart_approve"));
        // append when missing
        let out3 = yaml_line_surgery(&out2, "GOOSE_TEMPERATURE", &json!(0.7)).unwrap();
        assert!(out3.contains("GOOSE_TEMPERATURE: 0.7"));
        // string needing quotes
        let out4 = yaml_line_surgery(&out2, "GOOSE_MODEL", &json!("claude: 3.5")).unwrap();
        assert!(
            out4.contains("GOOSE_MODEL: 'claude: 3.5'")
                || out4.contains("GOOSE_MODEL: \"claude: 3.5\"")
        );
        // nested keys with the same suffix are NOT touched
        let nested = "extensions:\n  GOOSE_MODE: nope\n";
        let out5 = yaml_line_surgery(nested, "GOOSE_MODE", &json!("auto")).unwrap();
        assert!(out5.contains("  GOOSE_MODE: nope"));
        assert!(out5.contains("GOOSE_MODE: auto"));
    }

    /// omp arg serialization follows omp's config set conventions.
    #[test]
    fn omp_arg_string_conventions() {
        assert_eq!(omp_arg_string(&json!(true)), "true");
        assert_eq!(omp_arg_string(&json!(42)), "42");
        assert_eq!(omp_arg_string(&json!("high")), "high");
        assert_eq!(omp_arg_string(&json!(["a", "b"])), r#"["a","b"]"#);
        assert_eq!(
            omp_arg_string(&json!({"bash": "prompt"})),
            r#"{"bash":"prompt"}"#
        );
    }
}
/// Markdown entries: name validation rejects traversal/unsafe chars.
#[test]
fn markdown_name_validation() {
    assert!(valid_md_name("my-skill"));
    assert!(valid_md_name("Skill.2"));
    assert!(!valid_md_name(""));
    assert!(!valid_md_name("../escape"));
    assert!(!valid_md_name("a/b"));
    assert!(!valid_md_name(".hidden"));
    assert!(!valid_md_name("has space"));
    assert!(!valid_md_name(&"x".repeat(65)));
}

/// Markdown validation arm accepts any prose (no syntax to check).
#[test]
fn markdown_validation_is_lenient() {
    assert!(validate("markdown", "# anything {even braces").is_ok());
    assert!(validate("markdown", "").is_ok());
}

/// MCP location matrix: six supported tools, omp/dsh served by raw editors.
#[test]
fn mcp_location_matrix() {
    assert_eq!(mcp_location("claude"), Some(("user-state", "mcpServers")));
    assert_eq!(mcp_location("gemini"), Some(("settings", "mcpServers")));
    assert_eq!(mcp_location("qwen"), Some(("settings", "mcpServers")));
    assert_eq!(mcp_location("opencode"), Some(("global", "mcp")));
    assert_eq!(mcp_location("codex"), Some(("config", "mcp_servers")));
    assert_eq!(mcp_location("goose"), Some(("config", "extensions")));
    assert_eq!(mcp_location("omp"), None);
    assert_eq!(mcp_location("dsh"), None);
}

/// First heading extraction for the entry list previews.
#[test]
fn first_heading_extraction() {
    let dir = std::env::temp_dir().join("aio-md-test");
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("a.md");
    std::fs::write(&f, "\n\n# My Skill Title\nbody").unwrap();
    assert_eq!(first_heading(&f), "My Skill Title");
    let f2 = dir.join("b.md");
    std::fs::write(&f2, "plain first line").unwrap();
    assert_eq!(first_heading(&f2), "plain first line");
    let _ = std::fs::remove_dir_all(&dir);
}

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
use serde::Serialize;
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
}

/// Curated declaration: (dotted key, label, kind, enum options).
type KeyDecl = (
    &'static str,
    &'static str,
    &'static str,
    &'static [&'static str],
);

/// Which slot a tool's structured keys live in + the curated key declarations.
fn structured_decl(tool_id: &str) -> Option<(String, Vec<KeyDecl>)> {
    // (dotted key, label, kind, enum options)
    let decl: Vec<(
        &'static str,
        &'static str,
        &'static str,
        &'static [&'static str],
    )> = match tool_id {
        "claude" => vec![
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
        "gemini" | "qwen" => vec![
            ("model.name", "Default model", "string", &[]),
            (
                "tools.approvalMode",
                "Approval mode",
                "enum",
                &["default", "auto_edit", "yolo", "plan"],
            ),
            ("context.fileName", "Context file name", "string", &[]),
        ],
        "opencode" => vec![
            ("model", "Default model", "string", &[]),
            ("autoupdate", "Auto update", "bool", &[]),
        ],
        _ => return None,
    };
    // All structured targets are the tool's primary JSON settings slot.
    let slot = match tool_id {
        "claude" | "gemini" | "qwen" | "opencode" => "settings".to_string(),
        _ => return None,
    };
    // opencode's primary slot is the "global" one (id differs).
    let slot = if tool_id == "opencode" {
        "global".to_string()
    } else {
        slot
    };
    Some((slot, decl))
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
    let Some((slot_id, decl)) = structured_decl(tool_id) else {
        return Ok(Vec::new());
    };
    let slot = slot_spec(tool_id, &slot_id)?;
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
            .unwrap_or_else(|| match kind {
                "bool" => StructuredValue::Bool { value: None },
                "number" => StructuredValue::Num { value: None },
                "string-list" => StructuredValue::StrList { value: Vec::new() },
                "kv" => StructuredValue::Kv { value: Vec::new() },
                _ => StructuredValue::Str { value: None },
            });
        out.push(StructuredKeyInfo {
            key: key.to_string(),
            label: label.to_string(),
            kind: kind.to_string(),
            options: options.iter().map(|s| s.to_string()).collect(),
            value,
        });
    }
    Ok(out)
}

/// Set one curated key. `value` of JSON null removes the key (tool default).
/// Preserves every other key; keeps 2-space pretty formatting; the standard
/// write pipeline (validate → backup → stale guard → atomic) applies via the
/// file-level read-modify-write below.
pub fn structured_set(tool_id: &str, key: &str, value: serde_json::Value) -> Result<(), String> {
    let Some((slot_id, decl)) = structured_decl(tool_id) else {
        return Err(ierr!(
            "该工具没有结构化配置键",
            "This tool has no structured config keys"
        ));
    };
    if !decl.iter().any(|(k, ..)| *k == key) {
        return Err(ierr!(
            "未声明的配置键: {key}",
            "Undeclared config key: {key}"
        ));
    }
    let slot = slot_spec(tool_id, &slot_id)?;
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
    let expected = if path.exists() {
        std::fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
    } else {
        None
    };
    write(tool_id, &slot_id, &serialized, expected)
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
    fn structured_declarations_cover_json_tools() {
        for tool in ["claude", "gemini", "qwen", "opencode"] {
            let decl = structured_decl(tool);
            assert!(decl.is_some(), "{tool} should declare structured keys");
            let (slot, keys) = decl.unwrap();
            let expected_slot = if tool == "opencode" {
                "global"
            } else {
                "settings"
            };
            assert_eq!(slot, expected_slot);
            assert!(!keys.is_empty());
        }
        // TOML/YAML tools get no structured keys in PR B (raw editor serves them)
        for tool in ["codex", "omp", "goose", "dsh"] {
            assert!(structured_decl(tool).is_none());
        }
    }

    #[test]
    fn structured_set_rejects_undeclared_keys() {
        assert!(structured_set("claude", "nope.hack", json!(1)).is_err());
    }
}

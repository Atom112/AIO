//! 内置文件系统工具 — 原生函数调用（非 MCP 子进程）。
//!
//! 原 `mcp_fs_server.rs` 通过 stdio JSON-RPC 子进程提供文件操作，
//! 但子进程断连导致 Agent 文件操作频繁失败。现改为 in-process 直接调用，
//! 沙箱逻辑（`safe_path`）保持不变，工具名与参数完全兼容，前端无须改动。
//!
//! 供 `run_agent_turn` 在工具分发时直接调用，跳过 MCP 通道。

use crate::core::models::{ToolFunctionSpec, ToolResult, ToolResultContent, ToolSpec};
use serde_json::{json, Value};
use std::path::PathBuf;
use tauri::Manager;

// ====== 常量 ======

const MAX_R: u64 = 1_000_000; // 读文件上限 1MB
const MAX_W: u64 = 5_000_000; // 写文件上限 5MB
const MAX_S: usize = 10_000; // 列表/搜索上限 10K 条

// ====== 辅助函数 ======

/// 路径沙箱：解析请求的路径，确保在 `allowed_dir` 以内。
fn safe_path(allowed_dir: &str, requested: &str, must_exist: bool) -> Result<PathBuf, String> {
    let base = std::fs::canonicalize(allowed_dir).map_err(|e| format!("项目目录不可访问: {e}"))?;
    let p = PathBuf::from(requested);
    let resolved = if p.is_absolute() { p } else { base.join(&p) };
    let target = if must_exist {
        std::fs::canonicalize(&resolved).map_err(|e| format!("路径不可访问: {e}"))?
    } else {
        match std::fs::canonicalize(&resolved) {
            Ok(c) => c,
            Err(_) => {
                // 文件尚不存在：逐级上溯找到存在的父目录再拼接
                let mut exist = resolved.clone();
                let mut trail: Vec<std::ffi::OsString> = Vec::new();
                while !exist.exists() {
                    if let Some(n) = exist.file_name().map(|n| n.to_os_string()) {
                        trail.push(n);
                    }
                    if !exist.pop() {
                        let mut r = PathBuf::new();
                        for c in resolved.components() {
                            match c {
                                std::path::Component::ParentDir => {
                                    r.pop();
                                }
                                std::path::Component::CurDir => {}
                                o => {
                                    r.push(o);
                                }
                            }
                        }
                        return Ok(r);
                    }
                }
                let canon =
                    std::fs::canonicalize(&exist).map_err(|e| format!("路径不可访问: {e}"))?;
                let mut r = canon;
                for n in trail.into_iter().rev() {
                    r.push(n);
                }
                r
            }
        }
    };
    if !target.starts_with(&base) {
        return Err(format!("路径越界: '{requested}' 不在项目目录内"));
    }
    Ok(target)
}

fn is_binary(p: &std::path::Path) -> bool {
    [
        "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "svg", "pdf", "docx", "pptx", "xlsx",
        "zip", "tar", "gz", "rar", "exe", "dll", "so", "dylib", "wasm", "bin", "mp3", "mp4", "avi",
        "mov", "wav", "ogg", "woff", "woff2", "ttf", "eot",
    ]
    .contains(&p.extension().and_then(|e| e.to_str()).unwrap_or(""))
}

fn sz(b: u64) -> String {
    if b >= 1_000_000 {
        format!("{:.1}MB", b as f64 / 1_000_000.0)
    } else if b >= 1_000 {
        format!("{:.1}KB", b as f64 / 1_000.0)
    } else {
        format!("{b}B")
    }
}

pub fn tool_ok(text: String) -> ToolResult {
    ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({"text": text}),
        }],
        is_error: false,
    }
}

pub fn tool_err(msg: &str) -> ToolResult {
    ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({"text": format!("[Error] {msg}")}),
        }],
        is_error: true,
    }
}

// ====== 工具定义 ======

/// 返回内置文件工具的 ToolSpec 列表（直接喂给 LLM 的 tools 字段）。
pub fn get_file_tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "read_file".into(),
                description: "读取项目目录内的文件内容".into(),
                parameters: json!({"type":"object","properties":{"path":{"type":"string","description":"文件路径"}},"required":["path"]}),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "write_file".into(),
                description: "创建或覆盖文件".into(),
                parameters: json!({"type":"object","properties":{"path":{"type":"string","description":"文件路径"},"content":{"type":"string","description":"文件内容"}},"required":["path","content"]}),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "list_directory".into(),
                description: "列出目录内容".into(),
                parameters: json!({"type":"object","properties":{"path":{"type":"string","description":"目录路径, 默认项目根目录"}}}),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "search_files".into(),
                description: "按glob模式搜索文件".into(),
                parameters: json!({"type":"object","properties":{"pattern":{"type":"string","description":"glob模式,如 'src/**/*.ts'"},"basePath":{"type":"string","description":"起始目录"}},"required":["pattern"]}),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "search_content".into(),
                description: "搜索文件内容(正则)".into(),
                parameters: json!({"type":"object","properties":{"pattern":{"type":"string","description":"正则表达式"},"path":{"type":"string","description":"目标路径"}},"required":["pattern"]}),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "delete_file".into(),
                description: "删除文件".into(),
                parameters: json!({"type":"object","properties":{"path":{"type":"string","description":"文件路径"}},"required":["path"]}),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "make_directory".into(),
                description: "创建目录".into(),
                parameters: json!({"type":"object","properties":{"path":{"type":"string","description":"目录路径"}},"required":["path"]}),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "replace_in_file".into(),
                description: "在文件中执行精确字符串替换（old_string → new_string）。old_string 必须在文件中恰好出现一次，否则会报错要求提供更多上下文使匹配唯一".into(),
                parameters: json!({"type":"object","properties":{"path":{"type":"string","description":"文件路径"},"old_string":{"type":"string","description":"要被替换的原始字符串（必须与文件内容精确匹配）"},"new_string":{"type":"string","description":"替换后的新字符串"}},"required":["path","old_string","new_string"]}),
            },
        },
    ]
}

// ====== 工具执行 ======

/// 直接执行一个文件工具，返回 ToolResult。
///
/// `project_root` 是项目根目录的绝对路径，所有路径操作均限定在该目录内。
pub fn execute_file_tool(name: &str, arguments: &Value, project_root: &str) -> ToolResult {
    match name {
        "read_file" => {
            let path = arguments["path"].as_str().unwrap_or("");
            if path.is_empty() {
                return tool_err("缺少 path");
            }
            let t = match safe_path(project_root, path, true) {
                Ok(t) => t,
                Err(e) => return tool_err(&e),
            };
            if !t.is_file() {
                return tool_err(&format!("不是文件: {path}"));
            }
            if is_binary(&t) {
                return tool_err(&format!("二进制文件: {path}"));
            }
            let meta = match std::fs::metadata(&t) {
                Ok(m) => m,
                Err(e) => return tool_err(&e.to_string()),
            };
            let c = if meta.len() > MAX_R {
                let b = match std::fs::read(&t) {
                    Ok(b) => b,
                    Err(e) => return tool_err(&e.to_string()),
                };
                format!(
                    "⚠ 文件过大 ({}KB/{}KB截断)\n\n{}",
                    meta.len() / 1024,
                    MAX_R / 1024,
                    String::from_utf8_lossy(&b[..MAX_R as usize])
                )
            } else {
                String::from_utf8_lossy(&match std::fs::read(&t) {
                    Ok(b) => b,
                    Err(e) => return tool_err(&e.to_string()),
                })
                .into()
            };
            tool_ok(c)
        }
        "write_file" => {
            let path = arguments["path"].as_str().unwrap_or("");
            let content = arguments["content"].as_str().unwrap_or("");
            if path.is_empty() {
                return tool_err("缺少 path");
            }
            if content.len() as u64 > MAX_W {
                return tool_err(&format!("内容过大 (>{MAX_W}B)"));
            }
            let t = match safe_path(project_root, path, false) {
                Ok(t) => t,
                Err(e) => return tool_err(&e),
            };
            if t.exists() && t.is_dir() {
                return tool_err(&format!("目标是目录: {path}"));
            }
            if let Some(p) = t.parent() {
                let _ = std::fs::create_dir_all(p);
            }
            match std::fs::write(&t, content) {
                Ok(_) => tool_ok(format!("✓ 写入 {path} ({}B)", content.len())),
                Err(e) => tool_err(&format!("写入失败: {e}")),
            }
        }
        "list_directory" => {
            let p = arguments["path"].as_str().unwrap_or(".");
            let t = match safe_path(project_root, p, true) {
                Ok(t) => t,
                Err(e) => return tool_err(&e),
            };
            if !t.is_dir() {
                return tool_err(&format!("不是目录: {p}"));
            }
            let mut v = Vec::new();
            let mut n = 0;
            if let Ok(d) = std::fs::read_dir(&t) {
                for e in d.flatten() {
                    if n >= MAX_S {
                        v.push(format!("... 更多 (>{MAX_S})"));
                        break;
                    }
                    v.push(format!(
                        "{} {}  {}",
                        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                            "📁"
                        } else {
                            "📄"
                        },
                        e.file_name().to_string_lossy(),
                        e.metadata().map(|m| sz(m.len())).unwrap_or_default()
                    ));
                    n += 1;
                }
            }
            tool_ok(format!(
                "📂 {} ({}项):\n{}",
                if p == "." { "项目根目录" } else { p },
                n,
                v.join("\n")
            ))
        }
        "search_files" => {
            let pattern = arguments["pattern"].as_str().unwrap_or("**/*");
            let bp = arguments["basePath"].as_str().unwrap_or(".");
            let t = match safe_path(project_root, bp, true) {
                Ok(t) => t,
                Err(e) => return tool_err(&e),
            };
            let mut v = Vec::new();
            let mut n = 0;
            if let Ok(paths) = glob::glob(&t.join(pattern).to_string_lossy()) {
                for p in paths.flatten().filter(|p| p.is_file()) {
                    if n >= MAX_S {
                        v.push(format!("... (>{MAX_S})"));
                        break;
                    }
                    if let Ok(rel) = p.strip_prefix(&t) {
                        v.push(format!(
                            "📄 {}  {}",
                            rel.display(),
                            std::fs::metadata(&p)
                                .map(|m| sz(m.len()))
                                .unwrap_or_default()
                        ));
                        n += 1;
                    }
                }
            }
            let d = v.join("\n");
            tool_ok(format!(
                "🔍 '{}' ({}项):\n{}",
                pattern,
                n,
                if d.is_empty() { "(无匹配)" } else { &d }
            ))
        }
        "search_content" => {
            let pattern = arguments["pattern"].as_str().unwrap_or("");
            if pattern.is_empty() {
                return tool_err("缺少 pattern");
            }
            let p = arguments["path"].as_str().unwrap_or(".");
            let t = match safe_path(project_root, p, true) {
                Ok(t) => t,
                Err(e) => return tool_err(&e),
            };
            let re = match regex::Regex::new(pattern) {
                Ok(r) => r,
                Err(e) => return tool_err(&format!("无效正则: {e}")),
            };
            let mut v = Vec::new();
            let (mut scanned, mut matched) = (0, 0);
            let g = if t.is_dir() {
                t.join("**/*")
            } else {
                t.clone()
            };
            if let Ok(paths) = glob::glob(&g.to_string_lossy()) {
                for p in paths.flatten() {
                    if scanned >= MAX_S {
                        break;
                    }
                    if !p.is_file() || is_binary(&p) {
                        continue;
                    }
                    scanned += 1;
                    let c = match std::fs::read_to_string(&p) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };
                    let mut fc = 0;
                    for (i, line) in c.lines().enumerate() {
                        if re.is_match(line) {
                            if fc == 0 {
                                if let Ok(rel) = p.strip_prefix(&t) {
                                    v.push(format!("\n📄 {}:", rel.display()));
                                    matched += 1;
                                }
                            }
                            if fc < 10 {
                                v.push(format!(
                                    "  {}: {}",
                                    i + 1,
                                    line.trim().chars().take(200).collect::<String>()
                                ));
                            }
                            fc += 1;
                        }
                    }
                    if fc > 10 {
                        v.push(format!("  ... (共{fc}处)"));
                    }
                }
            }
            let d = v.join("\n");
            tool_ok(format!(
                "🔍 '{pattern}' — {matched}文件有匹配, 扫描{scanned}个:\n{}",
                if d.is_empty() { "(无匹配)" } else { &d }
            ))
        }
        "delete_file" => {
            let path = arguments["path"].as_str().unwrap_or("");
            if path.is_empty() {
                return tool_err("缺少 path");
            }
            let t = match safe_path(project_root, path, true) {
                Ok(t) => t,
                Err(e) => return tool_err(&e),
            };
            if !t.exists() {
                return tool_err(&format!("不存在: {path}"));
            }
            if t.is_dir() {
                return tool_err(&format!("是目录: {path}"));
            }
            match std::fs::remove_file(&t) {
                Ok(_) => tool_ok(format!("✓ 删除 {path}")),
                Err(e) => tool_err(&format!("{e}")),
            }
        }
        "make_directory" => {
            let path = arguments["path"].as_str().unwrap_or("");
            if path.is_empty() {
                return tool_err("缺少 path");
            }
            let t = match safe_path(project_root, path, false) {
                Ok(t) => t,
                Err(e) => return tool_err(&e),
            };
            if t.exists() {
                return tool_err(&format!("已存在: {path}"));
            }
            match std::fs::create_dir_all(&t) {
                Ok(_) => tool_ok(format!("✓ 创建 {path}")),
                Err(e) => tool_err(&format!("{e}")),
            }
        }
        "replace_in_file" => {
            let path = arguments["path"].as_str().unwrap_or("");
            let old_s = arguments["old_string"].as_str().unwrap_or("");
            let new_s = arguments["new_string"].as_str().unwrap_or("");
            if path.is_empty() {
                return tool_err("缺少 path");
            }
            if old_s.is_empty() {
                return tool_err("old_string 不能为空（若要前置/追加内容，请包含周围的上下文行）");
            }
            let t = match safe_path(project_root, path, true) {
                Ok(t) => t,
                Err(e) => return tool_err(&e),
            };
            if !t.is_file() {
                return tool_err(&format!("不是文件: {path}"));
            }
            if is_binary(&t) {
                return tool_err(&format!("二进制文件: {path}"));
            }
            let content = match std::fs::read_to_string(&t) {
                Ok(c) => c,
                Err(e) => return tool_err(&format!("读取失败: {e}")),
            };
            // 统计 old_string 出现次数及行号
            let mut matches: Vec<usize> = Vec::new();
            let mut pos = 0;
            while let Some(found) = content[pos..].find(old_s) {
                let abs_pos = pos + found;
                let line = content[..abs_pos].chars().filter(|&c| c == '\n').count() + 1;
                matches.push(line);
                pos = abs_pos + 1;
            }
            match matches.len() {
                0 => {
                    tool_err(&format!(
                        "未找到匹配的 old_string。请用 read_file 重新读取文件内容，确保 old_string 与文件中的文本完全一致（包括缩进和换行）。\n文件: {path}"
                    ))
                }
                1 => {
                    let new_content = content.replacen(old_s, new_s, 1);
                    if new_content.len() as u64 > MAX_W {
                        return tool_err(&format!("替换后内容过大 (>{MAX_W}B)"));
                    }
                    match std::fs::write(&t, &new_content) {
                        Ok(_) => tool_ok(format!(
                            "✓ 替换 {path}\n第 {} 行: {} 处匹配已替换",
                            matches[0], 1
                        )),
                        Err(e) => tool_err(&format!("写入失败: {e}")),
                    }
                }
                n => {
                    let lines_str = matches.iter()
                        .map(|l| l.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    tool_err(&format!(
                        "old_string 在文件中出现了 {n} 次（行 {lines_str}）。请增加更多周围的上下文行使匹配唯一，然后重试。"
                    ))
                }
            }
        }
        _ => tool_err(&format!("未知工具: {name}")),
    }
}

/// 根据 project_id 解析项目根目录绝对路径。
pub fn resolve_project_root(
    app: &tauri::AppHandle,
    project_id: Option<&str>,
) -> Result<String, String> {
    let pid = project_id.ok_or_else(|| "未指定项目".to_string())?;
    let idx_path = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取 AppData 目录失败: {e}"))?
        .join("projects.json");
    let content =
        std::fs::read_to_string(&idx_path).map_err(|e| format!("读取项目索引失败: {e}"))?;
    let file: Value =
        serde_json::from_str(&content).map_err(|e| format!("解析项目索引失败: {e}"))?;
    file["projects"][pid]["path"]
        .as_str()
        .map(strip_windows_extended_prefix)
        .ok_or_else(|| format!("项目 {} 不存在", pid))
}

/// 去掉 Windows 扩展长度路径前缀 `\\?\`（由 `std::fs::canonicalize` 产生）。
/// 该前缀对文件操作透明，但暴露给 LLM 时会造成困惑。
pub fn strip_windows_extended_prefix(path: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        path.strip_prefix("\\\\?\\").unwrap_or(path).to_string()
    }
    #[cfg(not(target_os = "windows"))]
    {
        path.to_string()
    }
}

//! 内置文件系统工具 — 原生函数调用（非 MCP 子进程）。
//!
//! 原 `mcp_fs_server.rs` 通过 stdio JSON-RPC 子进程提供文件操作，
//! 但子进程断连导致 Agent 文件操作频繁失败。现改为 in-process 直接调用，
//! 沙箱逻辑（`safe_path`）保持不变，工具名与参数完全兼容，前端无须改动。
//!
//! 供 `run_agent_turn` 在工具分发时直接调用，跳过 MCP 通道。

use crate::core::models::{ToolFunctionSpec, ToolResult, ToolSpec};
use crate::ierr;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

// ====== 常量 ======

const MAX_R: u64 = 1_000_000; // 读文件上限 1MB
const MAX_W: u64 = 5_000_000; // 写文件上限 5MB
const MAX_S: usize = 10_000; // 列表/搜索上限 10K 条

// ====== 辅助函数 ======

/// 计算文件内容的 SHA-256 十六进制摘要（hashline 协议：编辑按 hash 锚定防漂移）。
fn sha256_hex(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    hex::encode(hasher.finalize())
}

/// 路径沙箱：解析请求的路径，确保在 `allowed_dir` 以内。
///
/// 安全不变式：无论目标是否存在、能否 canonicalize，最终都必须经过末尾的
/// 越界检查后才能放行 —— 任何分支都不得提前 return（历史上上溯到根时曾提前返回，
/// 导致不存在的盘符/UNC 根路径绕过校验）。该函数同时被进程内文件工具与
/// `mcp_fs_server`（stdio 文件服务器）复用。
pub(crate) fn safe_path(
    allowed_dir: &str,
    requested: &str,
    must_exist: bool,
) -> Result<PathBuf, String> {
    let base = std::fs::canonicalize(allowed_dir).map_err(|e| {
        ierr!(
            "项目目录不可访问: {}",
            "Project directory is inaccessible: {}",
            e
        )
    })?;
    let p = PathBuf::from(requested);
    let resolved = if p.is_absolute() { p } else { base.join(&p) };
    let target = if must_exist {
        std::fs::canonicalize(&resolved)
            .map_err(|e| ierr!("路径不可访问: {}", "Path is inaccessible: {}", e))?
    } else {
        match std::fs::canonicalize(&resolved) {
            Ok(c) => c,
            Err(_) => {
                // 文件尚不存在：逐级上溯找到存在的父目录再拼接
                let mut exist = resolved.clone();
                let mut trail: Vec<std::ffi::OsString> = Vec::new();
                let mut walked_past_root = false;
                while !exist.exists() {
                    if let Some(n) = exist.file_name().map(|n| n.to_os_string()) {
                        trail.push(n);
                    }
                    if !exist.pop() {
                        walked_past_root = true;
                        break;
                    }
                }
                if walked_past_root {
                    // 全路径无一存在（如不存在的盘符/UNC 根）：无法 canonicalize 锚点，
                    // 退化为纯词法重建（展开 ../..），随后仍须经过末尾的越界检查。
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
                    r
                } else {
                    let canon = std::fs::canonicalize(&exist)
                        .map_err(|e| ierr!("路径不可访问: {}", "Path is inaccessible: {}", e))?;
                    let mut r = canon;
                    for n in trail.into_iter().rev() {
                        r.push(n);
                    }
                    r
                }
            }
        }
    };
    if !target.starts_with(&base) {
        return Err(ierr!(
            "路径越界: '{}' 不在项目目录内",
            "Path out of bounds: '{}' is not inside the project directory",
            requested
        ));
    }
    Ok(target)
}

pub(crate) fn is_binary(p: &std::path::Path) -> bool {
    [
        "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "svg", "pdf", "docx", "pptx", "xlsx",
        "zip", "tar", "gz", "rar", "exe", "dll", "so", "dylib", "wasm", "bin", "mp3", "mp4", "avi",
        "mov", "wav", "ogg", "woff", "woff2", "ttf", "eot",
    ]
    .contains(&p.extension().and_then(|e| e.to_str()).unwrap_or(""))
}

pub(crate) fn sz(b: u64) -> String {
    if b >= 1_000_000 {
        format!("{:.1}MB", b as f64 / 1_000_000.0)
    } else if b >= 1_000 {
        format!("{:.1}KB", b as f64 / 1_000.0)
    } else {
        format!("{b}B")
    }
}

// tool_ok / tool_err 统一由 utils::tool_result 提供（消除全仓 8+5 份副本）
pub use crate::utils::tool_result::{tool_err, tool_ok};

// ====== 工具定义 ======

/// 返回内置文件工具的 ToolSpec 列表（直接喂给 LLM 的 tools 字段）。
pub fn get_file_tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "read_file".into(),
                description: "读取项目目录内的文件内容，输出带 1-based 行号（格式 \"    1| 内容\"，可直接用于 replace_in_file 的 start_line/end_line）。默认最多读取 300 行 / 32KB，超出时返回头部+尾部并标注省略的行号区间；如需完整内容请显式传入更大的 max_lines 或 max_bytes。with_hash=true 时返回文件 SHA-256 摘要（hashline 协议：编辑时用 anchor_hash 锚定防漂移，写/替换工具会返回新 hash 供链式使用）".into(),
                parameters: json!({
                    "type":"object",
                    "properties":{
                        "path":{"type":"string","description":"文件路径"},
                        "max_lines":{"type":"integer","description":"最大读取行数（默认 300；传 -1 表示不限行数）"},
                        "max_bytes":{"type":"integer","description":"最大读取字节数（默认 32768）"},
                        "with_hash":{"type":"boolean","description":"是否返回文件 SHA-256 摘要（默认 false）"}
                    },
                    "required":["path"]
                }),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "write_file".into(),
                description: "创建或覆盖文件（父目录不存在时自动创建，单次上限 5MB）。新建文件用本工具；覆盖已有文件前必须先 read_file 确认当前内容——覆盖是不可撤销的整体替换，会把未包含在 content 中的原有内容全部丢弃。返回新文件的 SHA-256 摘要，可作为后续 replace_in_file 的 anchor_hash。已有文件的小范围修改优先用 replace_in_file（只需传改动片段，更省 token 且不易误覆盖）".into(),
                parameters: json!({"type":"object","properties":{"path":{"type":"string","description":"文件路径（相对项目根目录）"},"content":{"type":"string","description":"文件的完整新内容"}},"required":["path","content"]}),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "list_directory".into(),
                description: "列出目录内容（名称 + 类型标记 + 大小，单目录上限 10000 条）。探索项目结构时先用本工具或 search_files 定位目标，再 read_file，避免盲读大文件浪费上下文".into(),
                parameters: json!({"type":"object","properties":{"path":{"type":"string","description":"目录路径, 默认项目根目录"}}}),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "search_files".into(),
                description: "按文件名 glob 模式查找文件（如 'src/**/*.ts'），上限 10000 条。按名字找文件时优先用本工具而非逐层 list_directory；找到后再用 read_file 读取".into(),
                parameters: json!({"type":"object","properties":{"pattern":{"type":"string","description":"glob模式,如 'src/**/*.ts'"},"basePath":{"type":"string","description":"起始目录"}},"required":["pattern"]}),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "search_content".into(),
                description: "在文件内容中搜索正则表达式并返回命中行（含文件路径与行号）。找符号定义/调用点/配置项时优先用本工具定位到具体文件与行号，再 read_file 精读该区域，避免通读整个文件".into(),
                parameters: json!({"type":"object","properties":{"pattern":{"type":"string","description":"正则表达式"},"path":{"type":"string","description":"目标路径"}},"required":["pattern"]}),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "delete_file".into(),
                description: "删除文件（不可恢复）。删除前应确认路径正确且该文件确实不再需要；目录无法用本工具删除（仅文件）。删除属于变更类操作，在需要确认的模式下会请求用户批准".into(),
                parameters: json!({"type":"object","properties":{"path":{"type":"string","description":"文件路径"}},"required":["path"]}),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "make_directory".into(),
                description: "创建目录（含缺失的父级目录，递归创建；已存在时直接成功）。新建嵌套路径的文件可直接用 write_file（父目录自动创建），本工具用于显式建目录结构".into(),
                parameters: json!({"type":"object","properties":{"path":{"type":"string","description":"目录路径"}},"required":["path"]}),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "replace_in_file".into(),
                description: "在文件中执行替换。两种模式（二选一）：1) old_string 精确替换——old_string 必须恰好出现一次；2) 行范围锚定——old_string 留空并传 start_line/end_line（1-based），工具按当前文件内容取该行范围替换为 new_string，大段修改时无需回传整段旧代码，更省 token；若行号已过期会报错，请重新 read_file".into(),
                parameters: json!({"type":"object","properties":{"path":{"type":"string","description":"文件路径"},"old_string":{"type":"string","description":"要被替换的原始字符串（必须与文件内容精确匹配）；行范围模式下留空"},"new_string":{"type":"string","description":"替换后的新字符串（可含换行）"},"start_line":{"type":"integer","description":"行范围锚定模式：起始行号（1-based）"},"end_line":{"type":"integer","description":"行范围锚定模式：结束行号（1-based，默认等于 start_line）"},"anchor_hash":{"type":"string","description":"hashline 协议：read_file(with_hash=true) 或上次写/替换返回的文件 SHA-256 摘要。提供时先校验文件未被修改，防止按过期上下文误改"}},"required":["path","new_string"]}),
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
                return tool_err(&ierr!("缺少 path", "Missing path"));
            }
            // 读取上限：默认 300 行 / 32KB（token 效率：防止大文件全文注入上下文）；
            // max_lines 传 -1 表示不限行数，max_bytes 传 -1 表示不限制字节（仍受 MAX_R 硬上限约束）。
            let max_lines_arg = arguments["max_lines"].as_i64().unwrap_or(300);
            let max_lines = if max_lines_arg < 0 {
                usize::MAX
            } else {
                (max_lines_arg as u64).clamp(1, 1_000_000) as usize
            };
            let max_bytes_arg = arguments["max_bytes"].as_i64().unwrap_or(32 * 1024);
            let max_bytes = if max_bytes_arg < 0 {
                usize::MAX
            } else {
                (max_bytes_arg as u64).clamp(1024, MAX_R) as usize
            };
            let t = match safe_path(project_root, path, true) {
                Ok(t) => t,
                Err(e) => return tool_err(&e),
            };
            if !t.is_file() {
                return tool_err(&ierr!("不是文件: {}", "Not a file: {}", path));
            }
            if is_binary(&t) {
                return tool_err(&ierr!("二进制文件: {}", "Binary file: {}", path));
            }
            let meta = match std::fs::metadata(&t) {
                Ok(m) => m,
                Err(e) => return tool_err(&e.to_string()),
            };
            // 一次性读取字节（1MB 硬上限），并计算文件 SHA-256（hashline 协议用）
            let bytes = match std::fs::read(&t) {
                Ok(b) => b,
                Err(e) => return tool_err(&e.to_string()),
            };
            let hash_len = bytes.len().min(MAX_R as usize);
            let file_hash = sha256_hex(&bytes[..hash_len]);
            let raw = String::from_utf8_lossy(&bytes[..hash_len]).into_owned();
            let over_size = meta.len() > MAX_R;
            let total_lines = raw.lines().count();
            // 行号输出（1-based，cat -n 风格）：模型可精确引用行号（replace_in_file
            // 的行范围模式、错误报告等）。行号在行数截断之前编号，保持文件真实位置。
            let numbered: Vec<String> = raw
                .lines()
                .enumerate()
                .map(|(i, l)| format!("{:>5}| {}", i + 1, l))
                .collect();
            // 行数限制（头部+尾部，保留关键首尾信息；省略标记标注被省略的行号区间）
            let limited = if total_lines > max_lines {
                let head_lines = (max_lines * 6) / 10;
                let tail_lines = max_lines - head_lines;
                let head_str = numbered[..head_lines.min(numbered.len())].join("\n");
                let tail_str = numbered[numbered.len().saturating_sub(tail_lines)..].join("\n");
                format!(
                    "{}\n\n… [已省略第 {}–{} 行，共 {} 行；如需更多内容请增大 max_lines] …\n\n{}",
                    head_str,
                    head_lines + 1,
                    total_lines - tail_lines,
                    total_lines,
                    tail_str
                )
            } else {
                numbered.join("\n")
            };
            // 文件过大警告置于编号内容之前（不占用行号）
            let limited = if over_size {
                format!(
                    "⚠ 文件过大 ({}KB/{}KB截断)\n\n{}",
                    meta.len() / 1024,
                    MAX_R / 1024,
                    limited
                )
            } else {
                limited
            };
            // 字节限制（UTF-8 安全边界，头部+尾部）
            let c = if limited.len() > max_bytes {
                let head_bytes = (max_bytes * 6) / 10;
                let tail_bytes = max_bytes - head_bytes;
                let head_end = limited.floor_char_boundary(head_bytes.min(limited.len()));
                let tail_start =
                    limited.floor_char_boundary(limited.len().saturating_sub(tail_bytes));
                if tail_start > head_end {
                    format!(
                        "{}\n… [已省略 {} 字符] …\n{}",
                        &limited[..head_end],
                        limited[head_end..tail_start].chars().count(),
                        &limited[tail_start..]
                    )
                } else {
                    let end = limited.floor_char_boundary(max_bytes.min(limited.len()));
                    format!("{}\n… [已截断: 共{}字符]", &limited[..end], limited.len())
                }
            } else {
                limited
            };
            // with_hash=true：追加文件摘要，供 replace_in_file 的 anchor_hash 锚定
            let with_hash = arguments["with_hash"].as_bool().unwrap_or(false);
            if with_hash {
                tool_ok(format!("{}\n\n[HASH sha256:{}]", c, file_hash))
            } else {
                tool_ok(c)
            }
        }
        "write_file" => {
            let path = arguments["path"].as_str().unwrap_or("");
            let content = arguments["content"].as_str().unwrap_or("");
            if path.is_empty() {
                return tool_err(&ierr!("缺少 path", "Missing path"));
            }
            if content.len() as u64 > MAX_W {
                return tool_err(&ierr!("内容过大 (>{}B)", "Content too large (>{}B)", MAX_W));
            }
            let t = match safe_path(project_root, path, false) {
                Ok(t) => t,
                Err(e) => return tool_err(&e),
            };
            if t.exists() && t.is_dir() {
                return tool_err(&ierr!("目标是目录: {}", "Target is a directory: {}", path));
            }
            if let Some(p) = t.parent() {
                let _ = std::fs::create_dir_all(p);
            }
            match std::fs::write(&t, content) {
                Ok(_) => tool_ok(format!(
                    "✓ 写入 {path} ({}B)\n[HASH sha256:{}]",
                    content.len(),
                    sha256_hex(content.as_bytes())
                )),
                Err(e) => tool_err(&ierr!("写入失败: {}", "Write failed: {}", e)),
            }
        }
        "list_directory" => {
            let p = arguments["path"].as_str().unwrap_or(".");
            let t = match safe_path(project_root, p, true) {
                Ok(t) => t,
                Err(e) => return tool_err(&e),
            };
            if !t.is_dir() {
                return tool_err(&ierr!("不是目录: {}", "Not a directory: {}", p));
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
                return tool_err(&ierr!("缺少 pattern", "Missing pattern"));
            }
            let p = arguments["path"].as_str().unwrap_or(".");
            let t = match safe_path(project_root, p, true) {
                Ok(t) => t,
                Err(e) => return tool_err(&e),
            };
            let re = match regex::Regex::new(pattern) {
                Ok(r) => r,
                Err(e) => return tool_err(&ierr!("无效正则: {}", "Invalid regex: {}", e)),
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
                return tool_err(&ierr!("缺少 path", "Missing path"));
            }
            let t = match safe_path(project_root, path, true) {
                Ok(t) => t,
                Err(e) => return tool_err(&e),
            };
            if !t.exists() {
                return tool_err(&ierr!("不存在: {}", "Does not exist: {}", path));
            }
            if t.is_dir() {
                return tool_err(&ierr!("是目录: {}", "Is a directory: {}", path));
            }
            match std::fs::remove_file(&t) {
                Ok(_) => tool_ok(format!("✓ 删除 {path}")),
                Err(e) => tool_err(&format!("{e}")),
            }
        }
        "make_directory" => {
            let path = arguments["path"].as_str().unwrap_or("");
            if path.is_empty() {
                return tool_err(&ierr!("缺少 path", "Missing path"));
            }
            let t = match safe_path(project_root, path, false) {
                Ok(t) => t,
                Err(e) => return tool_err(&e),
            };
            if t.exists() {
                return tool_err(&ierr!("已存在: {}", "Already exists: {}", path));
            }
            match std::fs::create_dir_all(&t) {
                Ok(_) => tool_ok(format!("✓ 创建 {path}")),
                Err(e) => tool_err(&format!("{e}")),
            }
        }
        "replace_in_file" => {
            let path = arguments["path"].as_str().unwrap_or("");
            let mut old_s = arguments["old_string"].as_str().unwrap_or("").to_string();
            let new_s = arguments["new_string"].as_str().unwrap_or("");
            if path.is_empty() {
                return tool_err(&ierr!("缺少 path", "Missing path"));
            }
            let t = match safe_path(project_root, path, true) {
                Ok(t) => t,
                Err(e) => return tool_err(&e),
            };
            if !t.is_file() {
                return tool_err(&ierr!("不是文件: {}", "Not a file: {}", path));
            }
            if is_binary(&t) {
                return tool_err(&ierr!("二进制文件: {}", "Binary file: {}", path));
            }
            let content = match std::fs::read_to_string(&t) {
                Ok(c) => c,
                Err(e) => return tool_err(&ierr!("读取失败: {}", "Read failed: {}", e)),
            };
            // hashline 协议：anchor_hash 锚定防漂移——文件当前摘要必须与模型持有的 hash 一致，
            // 否则说明文件已被修改（或模型上下文过期），要求重新 read_file(with_hash=true)。
            let anchor_hash = arguments["anchor_hash"].as_str().unwrap_or("");
            if !anchor_hash.is_empty() {
                let current_hash = sha256_hex(content.as_bytes());
                if current_hash != anchor_hash {
                    return tool_err(&ierr!(
                        "[HASH MISMATCH] 文件摘要与 anchor_hash 不一致（期望 {}，当前 {}）。文件可能已被修改，请重新 read_file(with_hash=true) 获取最新 hash。",
                        "[HASH MISMATCH] File digest does not match anchor_hash (expected {}, got {}). The file may have been modified; re-run read_file(with_hash=true) to get the latest hash.",
                        anchor_hash,
                        current_hash
                    ));
                }
            }
            // 行范围锚定模式（token 效率）：old_string 留空 + start_line/end_line 时，
            // 由工具从当前文件内容构造待替换文本，模型无需回传整段旧代码。
            if old_s.is_empty() {
                match arguments["start_line"].as_i64() {
                    None => {
                        return tool_err(&ierr!(
                            "old_string 与 start_line 至少提供其一：精确匹配用 old_string；行范围锚定用 start_line/end_line（old_string 留空）",
                            "Provide old_string or start_line: use old_string for exact matching, or start_line/end_line for line-range anchoring (leave old_string empty)"
                        ))
                    }
                    Some(sl) => {
                        let el = arguments["end_line"].as_i64().unwrap_or(sl);
                        if sl < 1 || el < sl {
                            return tool_err(&ierr!(
                                "start_line/end_line 无效（1-based，end_line >= start_line）",
                                "Invalid start_line/end_line (1-based, end_line >= start_line)"
                            ));
                        }
                        // 构造行范围文本（不含行尾换行，匹配后保留原有换行结构）
                        let lines: Vec<&str> = content.split('\n').collect();
                        let last_empty = lines.last().is_some_and(|s| s.is_empty());
                        let total = if last_empty { lines.len() - 1 } else { lines.len() };
                        if el as usize > total {
                            return tool_err(&ierr!(
                                "行号越界：文件共 {} 行，end_line={}。若文件已被修改，请重新 read_file 获取最新行号。",
                                "Line number out of range: the file has {} lines, end_line={}. If the file has changed, re-run read_file to get the latest line numbers.",
                                total,
                                el
                            ));
                        }
                        let sl_u = sl as usize;
                        let el_u = el as usize;
                        old_s = lines[sl_u - 1..el_u].join("\n");
                    }
                }
            }
            if old_s.is_empty() {
                return tool_err(&ierr!(
                    "old_string 不能为空（若要前置/追加内容，请包含周围的上下文行）",
                    "old_string must not be empty (to prepend/append content, include surrounding context lines)"
                ));
            }
            // 统计 old_string 出现次数及行号
            let mut matches: Vec<usize> = Vec::new();
            let mut pos = 0;
            while let Some(found) = content[pos..].find(old_s.as_str()) {
                let abs_pos = pos + found;
                let line = content[..abs_pos].chars().filter(|&c| c == '\n').count() + 1;
                matches.push(line);
                pos = abs_pos + 1;
            }
            match matches.len() {
                0 => {
                    tool_err(&ierr!(
                        "未找到匹配的 old_string。请用 read_file 重新读取文件内容，确保 old_string 与文件中的文本完全一致（包括缩进和换行）。\n文件: {}",
                        "old_string not found. Re-read the file with read_file and make sure old_string matches the file text exactly (including indentation and line breaks).\nFile: {}",
                        path
                    ))
                }
                1 => {
                    let new_content = content.replacen(old_s.as_str(), new_s, 1);
                    if new_content.len() as u64 > MAX_W {
                        return tool_err(&ierr!(
                            "替换后内容过大 (>{}B)",
                            "Content too large after replacement (>{}B)",
                            MAX_W
                        ));
                    }
                    match std::fs::write(&t, &new_content) {
                        Ok(_) => tool_ok(format!(
                            "✓ 替换 {path}\n第 {} 行: {} 处匹配已替换\n[HASH sha256:{}]",
                            matches[0],
                            1,
                            sha256_hex(new_content.as_bytes())
                        )),
                        Err(e) => tool_err(&ierr!("写入失败: {}", "Write failed: {}", e)),
                    }
                }
                n => {
                    let lines_str = matches.iter()
                        .map(|l| l.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    tool_err(&ierr!(
                        "old_string 在文件中出现了 {} 次（行 {}）。请增加更多周围的上下文行使匹配唯一，然后重试。",
                        "old_string appears {} times in the file (lines {}). Add more surrounding context lines to make the match unique, then retry.",
                        n,
                        lines_str
                    ))
                }
            }
        }
        _ => tool_err(&ierr!("未知工具: {}", "Unknown tool: {}", name)),
    }
}

/// 根据 project_id 解析项目根目录绝对路径。
pub fn resolve_project_root(
    app: &tauri::AppHandle,
    project_id: Option<&str>,
) -> Result<String, String> {
    let pid = project_id.ok_or_else(|| ierr!("未指定项目", "No project specified"))?;
    // 路径解析统一在 utils::project_index（含 Windows 扩展前缀归一化）
    crate::utils::project_index::project_path(app, pid)
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

#[cfg(test)]
mod safe_path_tests {
    use super::{execute_file_tool, safe_path};
    use std::fs;
    use std::path::PathBuf;

    fn fresh_base(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aio_safepath_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 绝对路径指向不存在的盘符/UNC 根（walk-up 走到根仍无锚点）——
    /// 修复前此分支提前 return、绕过末尾的越界检查。
    #[test]
    fn rejects_absolute_path_on_nonexistent_root() {
        let base = fresh_base("noroot");
        let base_s = base.to_string_lossy().to_string();
        #[cfg(target_os = "windows")]
        let outside = "Q:\\__aio_never_exists_drive__\\x\\evil.txt";
        #[cfg(not(target_os = "windows"))]
        let outside = "/__aio_never_exists_dir__/x/evil.txt";
        let r = safe_path(&base_s, outside, false);
        assert!(r.is_err(), "越界路径必须被拒绝，实际得到 {r:?}");
        let _ = fs::remove_dir_all(&base);
    }

    /// 通过 .. 上溯逃逸项目目录。
    #[test]
    fn rejects_parent_traversal_escape() {
        let base = fresh_base("dotdot");
        let base_s = base.to_string_lossy().to_string();
        let r = safe_path(&base_s, "../../../../outside_evil.txt", false);
        assert!(r.is_err(), ".. 逃逸必须被拒绝，实际得到 {r:?}");
        let _ = fs::remove_dir_all(&base);
    }

    /// 项目目录内不存在的新文件（含不存在的中级目录）应允许写入。
    #[test]
    fn allows_new_file_inside_project() {
        let base = fresh_base("inside");
        let base_s = base.to_string_lossy().to_string();
        let r = safe_path(&base_s, "new_dir/new_file.txt", false).unwrap();
        let canon_base = fs::canonicalize(&base).unwrap();
        assert!(
            r.starts_with(&canon_base),
            "结果 {r:?} 应位于 {canon_base:?} 内"
        );
        assert!(r.ends_with("new_file.txt"));
        let _ = fs::remove_dir_all(&base);
    }

    /// must_exist=true 时目标必须真实存在。
    #[test]
    fn must_exist_rejects_missing_path() {
        let base = fresh_base("mustexist");
        let base_s = base.to_string_lossy().to_string();
        assert!(safe_path(&base_s, "definitely_missing.txt", true).is_err());
        fs::write(base.join("real.txt"), b"x").unwrap();
        let r = safe_path(&base_s, "real.txt", true).unwrap();
        assert!(r.starts_with(fs::canonicalize(&base).unwrap()));
        let _ = fs::remove_dir_all(&base);
    }

    fn text_of(r: &super::ToolResult) -> String {
        r.content
            .first()
            .and_then(|c| c.data.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }

    /// read_file 输出带 1-based 行号；行数截断后行号保持文件真实位置，
    /// 省略标记标注被省略的行号区间（供 replace_in_file 行范围模式引用）。
    #[test]
    fn read_file_outputs_real_line_numbers() {
        let base = fresh_base("lineno");
        let base_s = base.to_string_lossy().to_string();
        // 10 行文件，max_lines=5 → 头 3 行 + 尾 2 行，省略第 4–8 行
        let body: String = (1..=10).map(|i| format!("line{i}\n")).collect();
        fs::write(base.join("ten.txt"), body).unwrap();

        let args = serde_json::json!({"path": "ten.txt", "max_lines": 5, "max_bytes": -1});
        let r = execute_file_tool("read_file", &args, &base_s);
        let text = text_of(&r);
        assert!(text.contains("    1| line1"), "首行应带行号: {text}");
        assert!(text.contains("    3| line3"), "头部末行: {text}");
        assert!(
            text.contains("    9| line9"),
            "尾部行号应为真实位置: {text}"
        );
        assert!(text.contains("   10| line10"), "末行: {text}");
        assert!(text.contains("已省略第 4–8 行"), "省略区间: {text}");
        let _ = fs::remove_dir_all(&base);
    }

    /// 行号输出与 hashline 协议兼容（with_hash 摘要仍在末尾）。
    #[test]
    fn read_file_line_numbers_with_hash() {
        let base = fresh_base("lineno-hash");
        let base_s = base.to_string_lossy().to_string();
        fs::write(base.join("h.txt"), "hello\nworld\n").unwrap();
        let args = serde_json::json!({"path": "h.txt", "with_hash": true});
        let r = execute_file_tool("read_file", &args, &base_s);
        let text = text_of(&r);
        assert!(text.contains("    1| hello"));
        assert!(text.contains("    2| world"));
        assert!(text.contains("[HASH sha256:"), "hashline 应保留: {text}");
        let _ = fs::remove_dir_all(&base);
    }
}

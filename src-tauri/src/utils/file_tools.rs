//! Built-in filesystem tools — native in-process calls (not MCP subprocesses).
//!
//! Previously served by `mcp_fs_server.rs` over a stdio JSON-RPC subprocess, whose disconnections
//! made Agent file operations fail frequently; now called in-process. The sandboxing logic
//! (`safe_path`) is unchanged and tool names/parameters are fully compatible, so the frontend needs no changes.
//!
//! Called directly by `run_agent_turn` during tool dispatch, bypassing the MCP channel.

use crate::core::models::{ToolFunctionSpec, ToolResult, ToolSpec};
use crate::core::tool_names;
use crate::ierr;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

// ====== Constants ======

const MAX_R: u64 = 1_000_000; // read size cap: 1MB
const MAX_W: u64 = 5_000_000; // write size cap: 5MB
const MAX_S: usize = 10_000; // list/search cap: 10K entries
/// Hard cap on any single file read into memory by the file tools (SEC-04): files
/// beyond this size are refused outright instead of being slurped whole — the model
/// is told to use command-line tools for them. Keeps a multi-GB file from OOM-ing
/// the agent turn while staying generous enough for real source files.
const MAX_READ_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// Per-file content cap for the search_content scan (SEC-04): a multi-GB log file
/// must not be slurped whole; matches within the first 1MB are what the model needs.
const SEARCH_SCAN_FILE_CAP: u64 = 1_000_000;

// ====== Helpers ======

/// Computes the SHA-256 hex digest of the file content (hashline protocol: edits are anchored by hash to prevent drift).
fn sha256_hex(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    hex::encode(hasher.finalize())
}

/// Streams a file through SHA-256 without loading it into memory (SEC-04: hashing a
/// multi-GB file must not allocate proportionally). The digest is identical to
/// [`sha256_hex`] over the whole file — the hashline protocol's anchor values stay
/// stable no matter which path computed them.
fn sha256_hex_file(path: &std::path::Path) -> Result<String, String> {
    use std::io::Read;
    let mut hasher = Sha256::new();
    let mut reader = std::io::BufReader::new(std::fs::File::open(path).map_err(|e| e.to_string())?);
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Reads at most `cap` bytes of a file (UTF-8 lossy; missing file → empty string).
/// For preview/scan purposes only — bounded reads must never grow to file size.
pub(crate) fn read_bytes_capped(path: &std::path::Path, cap: u64) -> String {
    use std::io::Read;
    let mut buf: Vec<u8> = Vec::new();
    if let Ok(mut f) = std::fs::File::open(path) {
        let _ = (&mut f).take(cap).read_to_end(&mut buf);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// Bilingual refusal message for files above [`MAX_READ_FILE_BYTES`] (SEC-04).
fn too_large_error(actual: u64) -> String {
    ierr!(
        "文件过大（{}，单文件上限 {}）：拒绝读入内存以防资源耗尽。请改用 execute_command 配合命令行工具（如 sed/awk/分页读取）处理",
        "File too large ({}; single-file cap {}): refusing to load it into memory to avoid resource exhaustion. Use execute_command with command-line tools (sed/awk/paged reads) instead",
        sz(actual),
        sz(MAX_READ_FILE_BYTES)
    )
}

/// Whether the tool is a mutating single-path file tool. Such tools MUST be dispatched
/// in-process (execute_file_tool) via the write-lock choke point; their MCP variants
/// (built-in fs server) are filtered out of agent tool sets for exactly this reason.
pub(crate) fn is_mutating_file_tool(name: &str) -> bool {
    matches!(
        name,
        tool_names::WRITE_FILE
            | tool_names::REPLACE_IN_FILE
            | tool_names::DELETE_FILE
            | tool_names::MAKE_DIRECTORY
    )
}

/// SEC-5: reparse-point swap narrowing. Re-runs the sandbox resolution for the target
/// immediately before the mutation and requires it to produce the SAME canonical path
/// as the first validation — a junction/reparse point swapped in between (e.g. while
/// the tool was reading the old content) changes the canonicalization and is
/// rejected. This narrows the validate-to-write TOCTOU window from the whole tool
/// body down to the microseconds between this check and the actual file operation;
/// complete elimination would need FILE_FLAG_OPEN_REPARSE_POINT / O_NOFOLLOW
/// semantics on every syscall, which is out of scope here.
fn revalidate_target_stable(
    project_root: &str,
    rel: &str,
    validated: &std::path::Path,
) -> Result<(), String> {
    match safe_path(project_root, rel, false) {
        Ok(recheck) if recheck == *validated => Ok(()),
        _ => Err(ierr!(
            "目标路径在操作期间发生了变化（可能被替换为链接/挂载点），已拒绝写入",
            "The target path changed during the operation (possibly swapped for a link/mount point); write refused"
        )),
    }
}

/// Atomic file replace: write a temp sibling, then rename over the target, so concurrent
/// readers never observe a torn file and a crash mid-write cannot leave the target half
/// written. The target's permissions (mode bits / readonly) are copied onto the temp file
/// before the rename so an edit does not silently drop the executable bit or the
/// readonly attribute. Windows AV/indexer may briefly hold the target, making rename
/// fail; retries, then falls back to an in-place write (writers are still serialized by
/// the per-file write lock at the `execute_builtin_tool` choke point).
fn atomic_write(target: &std::path::Path, content: &str) -> Result<(), String> {
    let Some(dir) = target.parent() else {
        return Err(ierr!(
            "目标缺少父目录: {}",
            "Target has no parent directory: {}",
            target.display()
        ));
    };
    // Permission preservation is Unix-scoped: fs::permissions only carries mode bits
    // (exec bit included) there. On Windows it carries only the readonly flag — copying
    // it onto the tmp file would make the rename-over-target fail (readonly targets
    // cannot be replaced), and true ACL preservation needs ReplaceFile, so the swap
    // accepts directory-inherited ACLs there (documented limitation).
    #[cfg(not(windows))]
    let target_perms = std::fs::metadata(target).ok().map(|m| m.permissions());
    #[cfg(windows)]
    let target_perms: Option<std::fs::Permissions> = None;
    for _ in 0..3 {
        let tmp = dir.join(format!(".aio-tmp-{}", uuid::Uuid::new_v4().simple()));
        if let Err(e) = std::fs::write(&tmp, content) {
            let _ = std::fs::remove_file(&tmp);
            return Err(ierr!("写入失败: {}", "Write failed: {}", e));
        }
        if let Some(p) = &target_perms {
            let _ = std::fs::set_permissions(&tmp, p.clone());
        }
        match std::fs::rename(&tmp, target) {
            Ok(_) => return Ok(()),
            Err(_) => {
                let _ = std::fs::remove_file(&tmp);
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
    // Fallback: in-place write (still serialized by the per-file write lock). Atomicity
    // is knowingly traded away here — leave a trace for post-mortem torn-write debugging.
    tracing::warn!(
        "[atomic_write] rename 反复失败，回退就地写入（放弃原子性）: {}",
        target.display()
    );
    std::fs::write(target, content).map_err(|e| ierr!("写入失败: {}", "Write failed: {}", e))
}

/// Resolves the canonical target path of a single-path mutating file tool (lock key for
/// the per-file write lock). Returns `(canonical_path, exists)`; `None` when the tool is
/// not a single-path mutation or the path cannot be resolved (the tool itself reports the
/// detailed error right after, without lock protection).
pub(crate) fn mutating_tool_target(
    project_root: &str,
    tool_name: &str,
    arguments: &Value,
) -> Option<(PathBuf, bool)> {
    if !is_mutating_file_tool(tool_name) {
        return None;
    }
    let path = arguments["path"].as_str().unwrap_or("");
    if path.is_empty() {
        return None;
    }
    let target = safe_path(project_root, path, false).ok()?;
    let exists = target.exists();
    Some((target, exists))
}

/// Path sandbox: resolves the requested path and ensures it stays within `allowed_dir`.
///
/// Safety invariant: regardless of whether the target exists or can be canonicalized, it
/// must pass the final out-of-bounds check before being allowed — no branch may return
/// early. Reused by both the in-process file tools and `mcp_fs_server`
/// (the stdio file server).
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
                // Target does not exist yet: walk up level by level to an existing parent dir, then re-append
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
                    // No component of the path exists (e.g. a nonexistent drive/UNC root): no canonicalize
                    // anchor, so fall back to purely lexical reconstruction (expanding ../..), still subject to the final out-of-bounds check.
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

// tool_ok / tool_err are provided solely by utils::tool_result (single source of truth repo-wide)
pub use crate::utils::tool_result::{tool_err, tool_ok};

// ====== Tool definitions ======

/// Returns the ToolSpec list of the built-in file tools (fed directly to the LLM's tools field).
pub fn get_file_tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: tool_names::READ_FILE.into(),
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
                name: tool_names::WRITE_FILE.into(),
                description: "创建或覆盖文件（父目录不存在时自动创建，单次上限 5MB）。新建文件用本工具；覆盖已有文件前必须先 read_file 确认当前内容——覆盖是不可撤销的整体替换，会把未包含在 content 中的原有内容全部丢弃。返回新文件的 SHA-256 摘要，可作为后续 replace_in_file 的 anchor_hash。已有文件的小范围修改优先用 replace_in_file（只需传改动片段，更省 token 且不易误覆盖）；文件被其他智能体修改过后，覆盖时必须携带 expect_hash（当前内容哈希），不匹配会报错".into(),
                parameters: json!({"type":"object","properties":{"path":{"type":"string","description":"文件路径（相对项目根目录）"},"content":{"type":"string","description":"文件的完整新内容"},"expect_hash":{"type":"string","description":"写入前校验的文件当前 SHA-256（read_file(with_hash=true) 获取）。文件被其他智能体修改过后为必填；不匹配则报错，需重新读取最新内容"}},"required":["path","content"]}),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: tool_names::LIST_DIRECTORY.into(),
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
                name: tool_names::SEARCH_CONTENT.into(),
                description: "在文件内容中搜索正则表达式并返回命中行（含文件路径与行号）。找符号定义/调用点/配置项时优先用本工具定位到具体文件与行号，再 read_file 精读该区域，避免通读整个文件".into(),
                parameters: json!({"type":"object","properties":{"pattern":{"type":"string","description":"正则表达式"},"path":{"type":"string","description":"目标路径"}},"required":["pattern"]}),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: tool_names::DELETE_FILE.into(),
                description: "删除文件（不可恢复）。删除前应确认路径正确且该文件确实不再需要；目录无法用本工具删除（仅文件）。删除属于变更类操作，在需要确认的模式下会请求用户批准".into(),
                parameters: json!({"type":"object","properties":{"path":{"type":"string","description":"文件路径"}},"required":["path"]}),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: tool_names::MAKE_DIRECTORY.into(),
                description: "创建目录（含缺失的父级目录，递归创建；已存在时直接成功）。新建嵌套路径的文件可直接用 write_file（父目录自动创建），本工具用于显式建目录结构".into(),
                parameters: json!({"type":"object","properties":{"path":{"type":"string","description":"目录路径"}},"required":["path"]}),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: tool_names::REPLACE_IN_FILE.into(),
                description: "在文件中执行替换。两种模式（二选一）：1) old_string 精确替换——old_string 必须恰好出现一次；2) 行范围锚定——old_string 留空并传 start_line/end_line（1-based），工具按当前文件内容取该行范围替换为 new_string，大段修改时无需回传整段旧代码，更省 token；若行号已过期会报错，请重新 read_file".into(),
                parameters: json!({"type":"object","properties":{"path":{"type":"string","description":"文件路径"},"old_string":{"type":"string","description":"要被替换的原始字符串（必须与文件内容精确匹配）；行范围模式下留空"},"new_string":{"type":"string","description":"替换后的新字符串（可含换行）"},"start_line":{"type":"integer","description":"行范围锚定模式：起始行号（1-based）"},"end_line":{"type":"integer","description":"行范围锚定模式：结束行号（1-based，默认等于 start_line）"},"anchor_hash":{"type":"string","description":"hashline 协议：read_file(with_hash=true) 或上次写/替换返回的文件 SHA-256 摘要。提供时先校验文件未被修改，防止按过期上下文误改"}},"required":["path","new_string"]}),
            },
        },
    ]
}

// ====== Tool execution ======

/// Executes a single file tool directly and returns a ToolResult.
///
/// `project_root` is the absolute path of the project root; all path operations are confined to that directory.
pub fn execute_file_tool(name: &str, arguments: &Value, project_root: &str) -> ToolResult {
    match name {
        tool_names::READ_FILE => {
            let path = arguments["path"].as_str().unwrap_or("");
            if path.is_empty() {
                return tool_err(&ierr!("缺少 path", "Missing path"));
            }
            // Read caps: default 300 lines / 32KB (token efficiency: keeps full large files out of the context);
            // max_lines = -1 means unlimited lines, max_bytes = -1 means unlimited bytes (still bounded by the MAX_R hard cap).
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
            // SEC-04: bounded reads — a multi-GB file must not be slurped into memory
            // before truncating. Display bytes are taken with a capped read; the
            // digest is streamed (memory O(1)) and still covers the COMPLETE file
            // (hashline protocol: the digest must match the full-content digest
            // computed by replace_in_file's anchor_hash / write_file's expect_hash
            // validation — a truncated digest would permanently break edits of
            // files > MAX_R under the cross-writer gate).
            if meta.len() > MAX_READ_FILE_BYTES {
                return tool_err(&too_large_error(meta.len()));
            }
            let file_hash = match sha256_hex_file(&t) {
                Ok(h) => h,
                Err(e) => return tool_err(&e),
            };
            let raw = read_bytes_capped(&t, MAX_R);
            let over_size = meta.len() > MAX_R;
            let total_lines = raw.lines().count();
            // Line-numbered output (1-based, cat -n style): lets the model cite exact line numbers
            // (replace_in_file line-range mode, error reporting, etc.). Lines are numbered before line truncation to preserve real file positions.
            let numbered: Vec<String> = raw
                .lines()
                .enumerate()
                .map(|(i, l)| format!("{:>5}| {}", i + 1, l))
                .collect();
            // Line limit (head + tail, keeping the key leading/trailing content; the omission marker notes the omitted line range)
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
            // The oversize warning goes before the numbered content (consumes no line number)
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
            // Byte limit (UTF-8-safe boundaries, head + tail)
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
            // with_hash=true: append the file digest for replace_in_file's anchor_hash anchoring
            let with_hash = arguments["with_hash"].as_bool().unwrap_or(false);
            if with_hash {
                tool_ok(format!("{}\n\n[HASH sha256:{}]", c, file_hash))
            } else {
                tool_ok(c)
            }
        }
        tool_names::WRITE_FILE => {
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
            // hashline protocol — expect_hash: pre-overwrite content validation (the
            // mirror of replace_in_file's anchor_hash). Enforced for cross-writer
            // scenarios by the execute_builtin_tool gate; validated here whenever given.
            let expect_hash = arguments["expect_hash"].as_str().unwrap_or("");
            if !expect_hash.is_empty() && t.is_file() {
                // SEC-04: stream the hash instead of slurping the file; refuse files
                // beyond the hard read cap (the overwrite target can't be verified).
                let len = match std::fs::metadata(&t) {
                    Ok(m) => m.len(),
                    Err(e) => return tool_err(&ierr!("读取失败: {}", "Read failed: {}", e)),
                };
                if len > MAX_READ_FILE_BYTES {
                    return tool_err(&too_large_error(len));
                }
                let current = match sha256_hex_file(&t) {
                    Ok(h) => h,
                    Err(e) => return tool_err(&e),
                };
                if current != expect_hash {
                    return tool_err(&ierr!(
                        "[HASH MISMATCH] 文件当前哈希与 expect_hash 不一致（期望 {}，当前 {}）。文件已被修改，请重新 read_file(with_hash=true) 获取最新内容后基于其重写",
                        "[HASH MISMATCH] Current file digest does not match expect_hash (expected {}, got {}). The file was modified; re-run read_file(with_hash=true) and rewrite from the fresh content",
                        expect_hash,
                        current
                    ));
                }
            }
            if let Some(p) = t.parent() {
                let _ = std::fs::create_dir_all(p);
            }
            // SEC-5: junction/reparse-point swap narrowing — the canonical target must
            // be unchanged since the validation above (which the hash read straddled).
            if let Err(e) = revalidate_target_stable(project_root, path, &t) {
                return tool_err(&e);
            }
            match atomic_write(&t, content) {
                Ok(_) => tool_ok(format!(
                    "✓ 写入 {path} ({}B)\n[HASH sha256:{}]",
                    content.len(),
                    sha256_hex(content.as_bytes())
                )),
                Err(e) => tool_err(&e),
            }
        }
        tool_names::LIST_DIRECTORY => {
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
        tool_names::SEARCH_CONTENT => {
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
                    // SEC-04: bounded per-file scan (first SEARCH_SCAN_FILE_CAP bytes);
                    // non-UTF-8 files are skipped, same as the previous read_to_string path.
                    let bytes = {
                        use std::io::Read;
                        match std::fs::File::open(&p) {
                            Ok(mut f) => {
                                let mut b = Vec::new();
                                match (&mut f).take(SEARCH_SCAN_FILE_CAP).read_to_end(&mut b) {
                                    Ok(_) => b,
                                    Err(_) => continue,
                                }
                            }
                            Err(_) => continue,
                        }
                    };
                    let c = match String::from_utf8(bytes) {
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
        tool_names::DELETE_FILE => {
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
            // SEC-5: canonical target must be unchanged since validation
            if let Err(e) = revalidate_target_stable(project_root, path, &t) {
                return tool_err(&e);
            }
            match std::fs::remove_file(&t) {
                Ok(_) => tool_ok(format!("✓ 删除 {path}")),
                Err(e) => tool_err(&format!("{e}")),
            }
        }
        tool_names::MAKE_DIRECTORY => {
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
            // SEC-5: canonical target must be unchanged since validation
            if let Err(e) = revalidate_target_stable(project_root, path, &t) {
                return tool_err(&e);
            }
            match std::fs::create_dir_all(&t) {
                Ok(_) => tool_ok(format!("✓ 创建 {path}")),
                Err(e) => tool_err(&format!("{e}")),
            }
        }
        tool_names::REPLACE_IN_FILE => {
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
            // SEC-04: hard size cap before the in-memory replace (read_to_string on a
            // multi-GB file would allocate proportionally).
            let len = match std::fs::metadata(&t) {
                Ok(m) => m.len(),
                Err(e) => return tool_err(&ierr!("读取失败: {}", "Read failed: {}", e)),
            };
            if len > MAX_READ_FILE_BYTES {
                return tool_err(&too_large_error(len));
            }
            let content = match std::fs::read_to_string(&t) {
                Ok(c) => c,
                Err(e) => return tool_err(&ierr!("读取失败: {}", "Read failed: {}", e)),
            };
            // hashline protocol: anchor_hash guards against drift — the file's current digest must match the hash the model holds,
            // otherwise the file was modified (or the model's context is stale) and it must re-run read_file(with_hash=true).
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
            // Line-range anchor mode (token efficiency): with old_string empty + start_line/end_line,
            // the tool builds the text to replace from the current file content, so the model need not echo the whole old block.
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
                        // Build the line-range text (without the trailing newline; the original newline structure is preserved after replacement)
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
            // Count old_string occurrences and their line numbers.
            // Linear scan: the newline counter advances with the cursor (a per-match
            // prefix re-scan made this quadratic — catastrophic on large files with many
            // matches); the listed lines are capped so the error stays readable and cheap.
            let mut matches: Vec<usize> = Vec::new();
            let mut pos = 0;
            let mut line = 1usize;
            while let Some(found) = content[pos..].find(old_s.as_str()) {
                let abs_pos = pos + found;
                line += content[pos..abs_pos].matches('\n').count();
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
                    // SEC-5: junction/reparse-point swap narrowing — the canonical
                    // target must be unchanged since validation (the content read
                    // and match scan straddled it).
                    if let Err(e) = revalidate_target_stable(project_root, path, &t) {
                        return tool_err(&e);
                    }
                    match atomic_write(&t, &new_content) {
                        Ok(_) => tool_ok(format!(
                            "✓ 替换 {path}\n第 {} 行: {} 处匹配已替换\n[HASH sha256:{}]",
                            matches[0],
                            1,
                            sha256_hex(new_content.as_bytes())
                        )),
                        Err(e) => tool_err(&e),
                    }
                }
                n => {
                    const MAX_LISTED_LINES: usize = 20;
                    let lines_str = matches
                        .iter()
                        .take(MAX_LISTED_LINES)
                        .map(|l| l.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let lines_str = if n > MAX_LISTED_LINES {
                        format!("{lines_str}… 共 {n} 处")
                    } else {
                        lines_str
                    };
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

/// Resolves the absolute project root path from project_id.
pub fn resolve_project_root(
    app: &tauri::AppHandle,
    project_id: Option<&str>,
) -> Result<String, String> {
    let pid = project_id.ok_or_else(|| ierr!("未指定项目", "No project specified"))?;
    // Path resolution is centralized in utils::project_index (includes Windows extended-prefix normalization)
    crate::utils::project_index::project_path(app, pid)
}

/// Strips the Windows extended-length path prefix `\\?\` (produced by `std::fs::canonicalize`).
/// The prefix is transparent to file operations but confuses the LLM when exposed.
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
    use crate::core::tool_names;
    use std::fs;
    use std::path::PathBuf;

    fn fresh_base(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aio_safepath_{}_{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Absolute path targeting a nonexistent drive/UNC root (walk-up reaches the root with no anchor)
    /// must still be rejected by the final out-of-bounds check.
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

    /// Escaping the project directory via .. traversal.
    #[test]
    fn rejects_parent_traversal_escape() {
        let base = fresh_base("dotdot");
        let base_s = base.to_string_lossy().to_string();
        let r = safe_path(&base_s, "../../../../outside_evil.txt", false);
        assert!(r.is_err(), ".. 逃逸必须被拒绝，实际得到 {r:?}");
        let _ = fs::remove_dir_all(&base);
    }

    /// A new file inside the project directory (including nonexistent intermediate directories) must be allowed.
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

    /// With must_exist=true the target must actually exist.
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

    /// read_file output carries 1-based line numbers; after line truncation the numbers keep the real file positions,
    /// and the omission marker notes the omitted line range (for replace_in_file line-range mode).
    #[test]
    fn read_file_outputs_real_line_numbers() {
        let base = fresh_base("lineno");
        let base_s = base.to_string_lossy().to_string();
        // 10-line file, max_lines=5 -> head 3 lines + tail 2 lines, omitting lines 4-8
        let body: String = (1..=10).map(|i| format!("line{i}\n")).collect();
        fs::write(base.join("ten.txt"), body).unwrap();

        let args = serde_json::json!({"path": "ten.txt", "max_lines": 5, "max_bytes": -1});
        let r = execute_file_tool(tool_names::READ_FILE, &args, &base_s);
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

    /// Line-numbered output stays compatible with the hashline protocol (the with_hash digest still trails).
    #[test]
    fn read_file_line_numbers_with_hash() {
        let base = fresh_base("lineno-hash");
        let base_s = base.to_string_lossy().to_string();
        fs::write(base.join("h.txt"), "hello\nworld\n").unwrap();
        let args = serde_json::json!({"path": "h.txt", "with_hash": true});
        let r = execute_file_tool(tool_names::READ_FILE, &args, &base_s);
        let text = text_of(&r);
        assert!(text.contains("    1| hello"));
        assert!(text.contains("    2| world"));
        assert!(text.contains("[HASH sha256:"), "hashline 应保留: {text}");
        let _ = fs::remove_dir_all(&base);
    }

    /// expect_hash guards write_file overwrites: a mismatched digest is rejected and the
    /// file is left untouched.
    #[test]
    fn write_file_expect_hash_mismatch_rejects() {
        let base = fresh_base("expect-mismatch");
        let base_s = base.to_string_lossy().to_string();
        fs::write(base.join("a.txt"), "hello").unwrap();
        let args =
            serde_json::json!({"path": "a.txt", "content": "NEW", "expect_hash": "deadbeef"});
        let r = execute_file_tool(tool_names::WRITE_FILE, &args, &base_s);
        let text = text_of(&r);
        assert!(text.contains("[HASH MISMATCH]"), "应报哈希不匹配: {text}");
        assert_eq!(fs::read_to_string(base.join("a.txt")).unwrap(), "hello");
        let _ = fs::remove_dir_all(&base);
    }

    /// A matching expect_hash lets the overwrite proceed atomically.
    #[test]
    fn write_file_expect_hash_match_overwrites() {
        let base = fresh_base("expect-match");
        let base_s = base.to_string_lossy().to_string();
        fs::write(base.join("a.txt"), "hello").unwrap();
        let hash = super::sha256_hex(b"hello");
        let args = serde_json::json!({"path": "a.txt", "content": "NEW", "expect_hash": hash});
        let r = execute_file_tool(tool_names::WRITE_FILE, &args, &base_s);
        let text = text_of(&r);
        assert!(text.contains("✓ 写入"), "应写入成功: {text}");
        assert_eq!(fs::read_to_string(base.join("a.txt")).unwrap(), "NEW");
        let _ = fs::remove_dir_all(&base);
    }

    /// Creating a new file ignores expect_hash (there is nothing to compare against).
    #[test]
    fn write_file_expect_hash_new_file_skips_validation() {
        let base = fresh_base("expect-new");
        let base_s = base.to_string_lossy().to_string();
        let args =
            serde_json::json!({"path": "n.txt", "content": "first", "expect_hash": "whatever"});
        let r = execute_file_tool(tool_names::WRITE_FILE, &args, &base_s);
        assert!(text_of(&r).contains("✓ 写入"));
        assert_eq!(fs::read_to_string(base.join("n.txt")).unwrap(), "first");
        let _ = fs::remove_dir_all(&base);
    }

    /// Atomic replace leaves no `.aio-tmp-*` residue behind, for both create and overwrite.
    #[test]
    fn atomic_write_leaves_no_temp_files() {
        let base = fresh_base("atomic");
        let base_s = base.to_string_lossy().to_string();
        execute_file_tool(
            tool_names::WRITE_FILE,
            &serde_json::json!({"path": "sub/dir/f.txt", "content": "v1"}),
            &base_s,
        );
        execute_file_tool(
            tool_names::WRITE_FILE,
            &serde_json::json!({"path": "sub/dir/f.txt", "content": "v2"}),
            &base_s,
        );
        execute_file_tool(
            tool_names::REPLACE_IN_FILE,
            &serde_json::json!({"path": "sub/dir/f.txt", "old_string": "v2", "new_string": "v3"}),
            &base_s,
        );
        assert_eq!(
            fs::read_to_string(base.join("sub/dir/f.txt")).unwrap(),
            "v3"
        );
        for e in fs::read_dir(base.join("sub/dir")).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            assert!(!name.starts_with(".aio-tmp-"), "不应残留临时文件: {name}");
        }
        let _ = fs::remove_dir_all(&base);
    }

    /// replace_in_file exact-match mode still works through the atomic write path and
    /// returns the new hash for chained hashline edits.
    #[test]
    fn replace_in_file_exact_match_replaces() {
        let base = fresh_base("replace");
        let base_s = base.to_string_lossy().to_string();
        fs::write(base.join("r.txt"), "alpha\nbeta\ngamma\n").unwrap();
        let args = serde_json::json!({"path": "r.txt", "old_string": "beta", "new_string": "BETA"});
        let r = execute_file_tool(tool_names::REPLACE_IN_FILE, &args, &base_s);
        let text = text_of(&r);
        assert!(text.contains("✓ 替换"), "应替换成功: {text}");
        assert!(text.contains("[HASH sha256:"), "应返回新哈希: {text}");
        assert_eq!(
            fs::read_to_string(base.join("r.txt")).unwrap(),
            "alpha\nBETA\ngamma\n"
        );
        let _ = fs::remove_dir_all(&base);
    }

    /// Regression (P0-2): read_file's [HASH] must be the digest of the COMPLETE file —
    /// files > 1MB must remain editable through anchor_hash (the digest previously
    /// covered only the first MAX_R bytes, making cross-writer edits a permanent
    /// mismatch-retry loop).
    #[test]
    fn read_file_hash_covers_files_over_one_mb() {
        let base = fresh_base("big-hash");
        let base_s = base.to_string_lossy().to_string();
        // 1.2MB body: > MAX_R (1MB) but < MAX_W (5MB), so write_file/replace_in_file accept it
        let body = format!("{}UNIQUE_MARKER", "a".repeat(1_199_990));
        fs::write(base.join("big.txt"), &body).unwrap();

        let r = execute_file_tool(
            tool_names::READ_FILE,
            &serde_json::json!({"path": "big.txt", "max_bytes": 1024, "with_hash": true}),
            &base_s,
        );
        let text = text_of(&r);
        let expected = super::sha256_hex(body.as_bytes());
        assert!(
            text.contains(&format!("[HASH sha256:{expected}]")),
            "返回的摘要必须覆盖完整文件: {text}"
        );

        // anchor_hash validation uses the full-content digest — must pass with the read hash
        let r = execute_file_tool(
            tool_names::REPLACE_IN_FILE,
            &serde_json::json!({"path": "big.txt", "old_string": "UNIQUE_MARKER", "new_string": "REPLACED", "anchor_hash": expected}),
            &base_s,
        );
        let text = text_of(&r);
        assert!(
            text.contains("✓ 替换"),
            "大文件 anchor_hash 应校验通过: {text}"
        );
        let _ = fs::remove_dir_all(&base);
    }

    /// Perf regression: the multi-match scan must stay linear on large files — an
    /// old_string matching ~every position of a 1.2MB file must resolve quickly as a
    /// capped multi-match error instead of re-scanning the prefix per match (quadratic).
    #[test]
    fn multi_match_scan_stays_linear_on_large_files() {
        let base = fresh_base("multimatch");
        let base_s = base.to_string_lossy().to_string();
        let body = "a".repeat(1_200_000);
        fs::write(base.join("many.txt"), &body).unwrap();

        let start = std::time::Instant::now();
        let r = execute_file_tool(
            tool_names::REPLACE_IN_FILE,
            &serde_json::json!({"path": "many.txt", "old_string": "aaa", "new_string": "bbb"}),
            &base_s,
        );
        let text = text_of(&r);
        assert!(
            text.contains("出现了") && text.contains("共 1199998 处"),
            "应报多匹配错误（含总数）: {text}"
        );
        assert!(text.matches(" 1").count() < 40, "列出的匹配行号必须截断");
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "扫描必须保持线性（实际 {:?}）",
            start.elapsed()
        );
        let _ = fs::remove_dir_all(&base);
    }

    /// Regression (P1-4): an edit must preserve the target's mode bits on Unix (the
    /// executable flag) — the tmp+rename swap used to reset them to the default mode.
    /// Windows carries only the readonly flag in fs::permissions (copying it would block
    /// the swap entirely); there the write itself is the smoke assertion.
    #[test]
    fn atomic_write_preserves_permissions() {
        let base = fresh_base("perm");
        let base_s = base.to_string_lossy().to_string();
        let target = base.join("script.sh");
        fs::write(&target, "#!/bin/sh\necho v1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        }

        execute_file_tool(
            tool_names::WRITE_FILE,
            &serde_json::json!({"path": "script.sh", "content": "#!/bin/sh\necho v2\n"}),
            &base_s,
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&target).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755, "可执行位必须保留");
            assert_eq!(
                fs::read_to_string(&target).unwrap(),
                "#!/bin/sh\necho v2\n",
                "内容必须更新"
            );
        }
        #[cfg(windows)]
        {
            assert_eq!(
                fs::read_to_string(&target).unwrap(),
                "#!/bin/sh\necho v2\n",
                "写入必须成功"
            );
        }
        let _ = fs::remove_dir_all(&base);
    }
}

//! 内置文件系统 MCP Server — 直接实现 MCP JSON-RPC over stdio。
//!
//! ⚠ **已废弃（2025-07）**：Agent 模式现在通过 `utils/file_tools.rs` 直接 in-process 调用文件工具，
//! 不再通过 stdio 子进程。本文件保留仅用于：
//! 1. 外部 MCP 客户端通过 `--fs-server <path>` 启动独立文件系统 server
//! 2. 向后兼容：已有脚本/配置可能直接调用 `aio --fs-server <path>`
//!
//! MCP 协议本身很简单（JSON-RPC 2.0 over stdin/stdout），
//! 手写实现比引入 rmcp 更轻量，且避免了 rmcp 版本 API 兼容问题。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::PathBuf;

// ====== JSON-RPC 模型 ======

#[derive(Deserialize, Debug)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: Option<String>,
    params: Option<Value>,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

fn respond(id: Option<Value>, result: Value) -> String {
    let resp = JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    };
    serde_json::to_string(&resp).unwrap()
}

fn respond_error(id: Option<Value>, code: i32, message: &str) -> String {
    let resp = JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(JsonRpcError { code, message: message.into() }),
    };
    serde_json::to_string(&resp).unwrap()
}

// ====== 沙箱 ======

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
                let mut exist = resolved.clone();
                let mut trail: Vec<std::ffi::OsString> = Vec::new();
                while !exist.exists() {
                    if let Some(n) = exist.file_name().map(|n| n.to_os_string()) { trail.push(n); }
                    if !exist.pop() {
                        let mut r = PathBuf::new();
                        for c in resolved.components() {
                            match c { std::path::Component::ParentDir => { r.pop(); } std::path::Component::CurDir => {} o => { r.push(o); } }
                        }
                        return Ok(r);
                    }
                }
                let canon = std::fs::canonicalize(&exist).map_err(|e| format!("路径不可访问: {e}"))?;
                let mut r = canon;
                for n in trail.into_iter().rev() { r.push(n); }
                r
            }
        }
    };
    if !target.starts_with(&base) { return Err(format!("路径越界: '{requested}' 不在项目目录内")); }
    Ok(target)
}

fn is_binary(p: &std::path::Path) -> bool {
    ["png","jpg","jpeg","gif","webp","bmp","ico","svg","pdf","docx","pptx","xlsx","zip","tar","gz","rar","exe","dll","so","dylib","wasm","bin","mp3","mp4","avi","mov","wav","ogg","woff","woff2","ttf","eot"]
        .contains(&p.extension().and_then(|e| e.to_str()).unwrap_or(""))
}
fn sz(b: u64) -> String {
    if b >= 1_000_000 { format!("{:.1}MB", b as f64 / 1_000_000.0) }
    else if b >= 1_000 { format!("{:.1}KB", b as f64 / 1_000.0) }
    else { format!("{b}B") }
}
const MAX_R: u64 = 1_000_000;
const MAX_W: u64 = 5_000_000;
const MAX_S: usize = 10_000;

fn tool_ok(text: String) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": false})
}
fn tool_err(msg: &str) -> Value {
    json!({"content": [{"type": "text", "text": format!("[Error] {msg}")}], "isError": true})
}

// ====== 工具实现 ======

struct FileServer { dir: String }

impl FileServer {
    fn handle(&self, name: &str, arguments: &Value) -> Value {
        let args = arguments;
        match name {
            "read_file" => {
                let path = args["path"].as_str().unwrap_or("");
                if path.is_empty() { return tool_err("缺少 path"); }
                let t = match safe_path(&self.dir, path, true) { Ok(t) => t, Err(e) => return tool_err(&e) };
                if !t.is_file() { return tool_err(&format!("不是文件: {path}")); }
                if is_binary(&t) { return tool_err(&format!("二进制文件: {path}")); }
                let meta = match std::fs::metadata(&t) { Ok(m) => m, Err(e) => return tool_err(&e.to_string()) };
                let c = if meta.len() > MAX_R {
                    let b = match std::fs::read(&t) { Ok(b) => b, Err(e) => return tool_err(&e.to_string()) };
                    format!("⚠ 文件过大 ({}KB/{}KB截断)\n\n{}", meta.len()/1024, MAX_R/1024, String::from_utf8_lossy(&b[..MAX_R as usize]))
                } else {
                    String::from_utf8_lossy(&match std::fs::read(&t) { Ok(b) => b, Err(e) => return tool_err(&e.to_string()) }).into()
                };
                tool_ok(c)
            }
            "write_file" => {
                let path = args["path"].as_str().unwrap_or("");
                let content = args["content"].as_str().unwrap_or("");
                if path.is_empty() { return tool_err("缺少 path"); }
                if content.len() as u64 > MAX_W { return tool_err(&format!("内容过大 (>{MAX_W}B)")); }
                let t = match safe_path(&self.dir, path, false) { Ok(t) => t, Err(e) => return tool_err(&e) };
                if t.exists() && t.is_dir() { return tool_err(&format!("目标是目录: {path}")); }
                if let Some(p) = t.parent() { let _ = std::fs::create_dir_all(p); }
                match std::fs::write(&t, content) {
                    Ok(_) => tool_ok(format!("✓ 写入 {path} ({}B)", content.len())),
                    Err(e) => tool_err(&format!("写入失败: {e}")),
                }
            }
            "list_directory" => {
                let p = args["path"].as_str().unwrap_or(".");
                let t = match safe_path(&self.dir, p, true) { Ok(t) => t, Err(e) => return tool_err(&e) };
                if !t.is_dir() { return tool_err(&format!("不是目录: {p}")); }
                let mut v = Vec::new(); let mut n = 0;
                if let Ok(d) = std::fs::read_dir(&t) {
                    for e in d.flatten() {
                        if n >= MAX_S { v.push(format!("... 更多 (>{MAX_S})")); break; }
                        v.push(format!("{} {}  {}", if e.file_type().map(|t|t.is_dir()).unwrap_or(false){"📁"}else{"📄"}, e.file_name().to_string_lossy(), e.metadata().map(|m|sz(m.len())).unwrap_or_default()));
                        n += 1;
                    }
                }
                tool_ok(format!("📂 {} ({}项):\n{}", if p=="." { "项目根目录" } else { p }, n, v.join("\n")))
            }
            "search_files" => {
                let pattern = args["pattern"].as_str().unwrap_or("**/*");
                let bp = args["basePath"].as_str().unwrap_or(".");
                let t = match safe_path(&self.dir, bp, true) { Ok(t) => t, Err(e) => return tool_err(&e) };
                let mut v = Vec::new(); let mut n = 0;
                if let Ok(paths) = glob::glob(&t.join(pattern).to_string_lossy()) {
                    for p in paths.flatten().filter(|p| p.is_file()) {
                        if n >= MAX_S { v.push(format!("... (>{MAX_S})")); break; }
                        if let Ok(rel) = p.strip_prefix(&t) { v.push(format!("📄 {}  {}", rel.display(), std::fs::metadata(&p).map(|m|sz(m.len())).unwrap_or_default())); n += 1; }
                    }
                }
                let d = v.join("\n");
                tool_ok(format!("🔍 '{}' ({}项):\n{}", pattern, n, if d.is_empty(){"(无匹配)"}else{&d}))
            }
            "search_content" => {
                let pattern = args["pattern"].as_str().unwrap_or("");
                if pattern.is_empty() { return tool_err("缺少 pattern"); }
                let p = args["path"].as_str().unwrap_or(".");
                let t = match safe_path(&self.dir, p, true) { Ok(t) => t, Err(e) => return tool_err(&e) };
                let re = match regex::Regex::new(pattern) { Ok(r) => r, Err(e) => return tool_err(&format!("无效正则: {e}")) };
                let mut v = Vec::new(); let (mut scanned, mut matched) = (0, 0);
                let g = if t.is_dir() { t.join("**/*") } else { t.clone() };
                if let Ok(paths) = glob::glob(&g.to_string_lossy()) {
                    for p in paths.flatten() {
                        if scanned >= MAX_S { break; }
                        if !p.is_file() || is_binary(&p) { continue; }
                        scanned += 1;
                        let c = match std::fs::read_to_string(&p) { Ok(c) => c, Err(_) => continue };
                        let mut fc = 0;
                        for (i, line) in c.lines().enumerate() {
                            if re.is_match(line) {
                                if fc == 0 { if let Ok(rel) = p.strip_prefix(&t) { v.push(format!("\n📄 {}:", rel.display())); matched += 1; } }
                                if fc < 10 { v.push(format!("  {}: {}", i+1, line.trim().chars().take(200).collect::<String>())); }
                                fc += 1;
                            }
                        }
                        if fc > 10 { v.push(format!("  ... (共{fc}处)")); }
                    }
                }
                let d = v.join("\n");
                tool_ok(format!("🔍 '{pattern}' — {matched}文件有匹配, 扫描{scanned}个:\n{}", if d.is_empty(){"(无匹配)"}else{&d}))
            }
            "delete_file" => {
                let path = args["path"].as_str().unwrap_or("");
                if path.is_empty() { return tool_err("缺少 path"); }
                let t = match safe_path(&self.dir, path, true) { Ok(t) => t, Err(e) => return tool_err(&e) };
                if !t.exists() { return tool_err(&format!("不存在: {path}")); }
                if t.is_dir() { return tool_err(&format!("是目录: {path}")); }
                match std::fs::remove_file(&t) { Ok(_) => tool_ok(format!("✓ 删除 {path}")), Err(e) => tool_err(&format!("{e}")) }
            }
            "make_directory" => {
                let path = args["path"].as_str().unwrap_or("");
                if path.is_empty() { return tool_err("缺少 path"); }
                let t = match safe_path(&self.dir, path, false) { Ok(t) => t, Err(e) => return tool_err(&e) };
                if t.exists() { return tool_err(&format!("已存在: {path}")); }
                match std::fs::create_dir_all(&t) { Ok(_) => tool_ok(format!("✓ 创建 {path}")), Err(e) => tool_err(&format!("{e}")) }
            }
            _ => tool_err(&format!("未知工具: {name}")),
        }
    }
}

// ====== MCP 协议处理 ======

fn server_info() -> Value {
    json!({
        "protocolVersion": "2025-06-18",
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name": "aio-filesystem",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": "File system tools for AIO agent mode. All paths relative to project root."
    })
}

fn tools_list() -> Value {
    json!({
        "tools": [
            {"name":"read_file","description":"读取项目目录内的文件内容","inputSchema":{"type":"object","properties":{"path":{"type":"string","description":"文件路径"}},"required":["path"]}},
            {"name":"write_file","description":"创建或覆盖文件","inputSchema":{"type":"object","properties":{"path":{"type":"string","description":"文件路径"},"content":{"type":"string","description":"文件内容"}},"required":["path","content"]}},
            {"name":"list_directory","description":"列出目录内容","inputSchema":{"type":"object","properties":{"path":{"type":"string","description":"目录路径, 默认项目根目录"}}}},
            {"name":"search_files","description":"按glob模式搜索文件","inputSchema":{"type":"object","properties":{"pattern":{"type":"string","description":"glob模式,如 'src/**/*.ts'"},"basePath":{"type":"string","description":"起始目录"}},"required":["pattern"]}},
            {"name":"search_content","description":"搜索文件内容(正则)","inputSchema":{"type":"object","properties":{"pattern":{"type":"string","description":"正则表达式"},"path":{"type":"string","description":"目标路径"}},"required":["pattern"]}},
            {"name":"delete_file","description":"删除文件","inputSchema":{"type":"object","properties":{"path":{"type":"string","description":"文件路径"}},"required":["path"]}},
            {"name":"make_directory","description":"创建目录","inputSchema":{"type":"object","properties":{"path":{"type":"string","description":"目录路径"}},"required":["path"]}}
        ]
    })
}

// ====== Entry Point ======

pub fn run(allowed_dir: String) {
    eprintln!("[aio-fs-server] starting, dir: {}", allowed_dir);
    let server = FileServer { dir: allowed_dir };
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut _initialized = false;

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[aio-fs-server] stdin read error: {e}");
                break;
            }
        };
        let line = line.trim().to_string();
        if line.is_empty() { continue; }

        eprintln!("[aio-fs-server] <- {}", &line[..line.len().min(200)]);

        let req: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[aio-fs-server] parse error: {e}");
                let _ = writeln!(stdout, "{}", respond_error(None, -32700, &format!("Parse error: {e}")));
                let _ = stdout.flush();
                continue;
            }
        };

        let method = req.method.as_deref().unwrap_or("");
        match method {
            "initialize" => {
                let resp = respond(req.id.clone(), server_info());
                eprintln!("[aio-fs-server] -> initialize response");
                let _ = writeln!(stdout, "{resp}");
                let _ = stdout.flush();
            }
            "notifications/initialized" => {
                // 客户端发来的 initialized 通知，不需要回复
                eprintln!("[aio-fs-server] received initialized notification");
                _initialized = true;
            }
            "tools/list" => {
                let resp = respond(req.id.clone(), tools_list());
                eprintln!("[aio-fs-server] -> tools/list response");
                let _ = writeln!(stdout, "{resp}");
                let _ = stdout.flush();
            }
            "tools/call" => {
                let empty_params = json!({});
                let empty_args = json!({});
                let params = req.params.as_ref().unwrap_or(&empty_params);
                let tool_name = params["name"].as_str().unwrap_or("");
                let arguments = params.get("arguments").unwrap_or(&empty_args);
                eprintln!("[aio-fs-server] call tool: {tool_name} args: {arguments}");
                let result = server.handle(tool_name, arguments);
                let resp = respond(req.id.clone(), result);
                eprintln!("[aio-fs-server] -> tool result: {}", &resp[..resp.len().min(200)]);
                let _ = writeln!(stdout, "{resp}");
                let _ = stdout.flush();
            }
            "ping" => {
                let _ = writeln!(stdout, "{}", respond(req.id.clone(), json!({})));
                let _ = stdout.flush();
            }
            _ => {
                eprintln!("[aio-fs-server] unknown method: {method}");
                let _ = writeln!(stdout, "{}", respond_error(req.id.clone(), -32601, &format!("Method not found: {method}")));
                let _ = stdout.flush();
            }
        }
    }
    eprintln!("[aio-fs-server] stdin closed, exiting");
}

// 入口在 main.rs 的 --fs-server 分支调用 run()

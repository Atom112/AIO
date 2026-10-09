//! Built-in filesystem MCP server - a direct MCP JSON-RPC implementation over stdio.
//!
//! Still the default filesystem server for new projects: `commands/project.rs` writes
//! `__aio-filesystem__` (command = current exe + `--fs-server <path>`, autoStart) into
//! the project's `mcp-servers.json` on creation, so this file is on the live code path
//! and must be maintained like the agent's in-process file tools. Retiring it later
//! requires migrating existing project configs.
//!
//! Tool implementations are fully delegated to `execute_file_tool` in
//! `utils/file_tools.rs` (single source of truth); this file only adapts the
//! MCP JSON-RPC protocol.
//!
//! The MCP protocol is simple (JSON-RPC 2.0 over stdin/stdout); a hand-written
//! implementation is lighter than rmcp and avoids rmcp version API compatibility issues.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, Write};

// ====== JSON-RPC models ======

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
        error: Some(JsonRpcError {
            code,
            message: message.into(),
        }),
    };
    serde_json::to_string(&resp).unwrap()
}

// ====== Sandbox ======
// Tool implementations and security boundaries (safe_path etc.) all come from
// the single in-process implementation in utils::file_tools; this file is only the MCP JSON-RPC adapter.

// ====== Tool implementations ======

struct FileServer {
    dir: String,
}

impl FileServer {
    fn handle(&self, name: &str, arguments: &Value) -> Value {
        // All file tools delegate to the single file_tools::execute_file_tool implementation,
        // so MCP behavior matches the in-process agent file tools exactly.
        let r = crate::utils::file_tools::execute_file_tool(name, arguments, &self.dir);
        // ToolResult (in-process shape) -> MCP JSON-RPC result shape
        let text = r
            .content
            .iter()
            .find(|c| c.kind == "text")
            .and_then(|c| c.data.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        json!({
            "content": [{"type": "text", "text": text}],
            "isError": r.is_error
        })
    }
}

// ====== MCP protocol handling ======

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
            {"name":"replace_in_file","description":"在文件中执行替换（old_string 必须唯一）或行范围锚定替换","inputSchema":{"type":"object","properties":{"path":{"type":"string","description":"文件路径"},"old_string":{"type":"string","description":"要替换的文本（须唯一）"},"new_string":{"type":"string","description":"替换文本"},"start_line":{"type":"integer","description":"起始行（行范围模式）"},"end_line":{"type":"integer","description":"结束行（行范围模式）"},"anchor_hash":{"type":"string","description":"文件 SHA-256 锚定（防过期写入）"}},"required":["path"]}},
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
        if line.is_empty() {
            continue;
        }

        eprintln!("[aio-fs-server] <- {}", &line[..line.len().min(200)]);

        let req: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[aio-fs-server] parse error: {e}");
                let _ = writeln!(
                    stdout,
                    "{}",
                    respond_error(None, -32700, &format!("Parse error: {e}"))
                );
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
                // Initialized notification from the client; no reply needed
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
                eprintln!(
                    "[aio-fs-server] -> tool result: {}",
                    &resp[..resp.len().min(200)]
                );
                let _ = writeln!(stdout, "{resp}");
                let _ = stdout.flush();
            }
            "ping" => {
                let _ = writeln!(stdout, "{}", respond(req.id.clone(), json!({})));
                let _ = stdout.flush();
            }
            _ => {
                eprintln!("[aio-fs-server] unknown method: {method}");
                let _ = writeln!(
                    stdout,
                    "{}",
                    respond_error(
                        req.id.clone(),
                        -32601,
                        &format!("Method not found: {method}")
                    )
                );
                let _ = stdout.flush();
            }
        }
    }
    eprintln!("[aio-fs-server] stdin closed, exiting");
}

// Entry point: main.rs invokes run() from its --fs-server branch

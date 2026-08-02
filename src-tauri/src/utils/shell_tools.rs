//! 内置系统命令执行工具 — 供 Agent 运行终端命令。
//!
//! 业界参考：Claude Code 的 Bash 工具（简单 schema + 权限记忆）。
//! 由 `run_agent_turn` 在工具分发时直接调用，跳过 MCP 子进程通道。
//!
//! 安全措施：
//! - 路径沙箱：工作目录限定在项目根目录
//! - 输出截断：stdout/stderr 各 50KB
//! - 超时控制：默认 60s，最大 300s
//! - 危险命令检测：词法级 token 分析，拒绝高危模式
//!   (系统破坏 / 提权 / 网络外泄 / 代码执行 / 环境注入 / fork bomb 等)
//! - 命令过滤：长度限制 4096 字符、禁止反引号/$() 命令替换
//! - Windows Job Object 沙箱：限制子进程权限
//! - 权限系统：复用现有 Allow/Ask/Deny 规则引擎

use crate::core::models::{ToolFunctionSpec, ToolResult, ToolResultContent, ToolSpec};
use serde_json::json;
use std::process::Command;
use std::time::Duration;

// ====== 常量 ======

const DEFAULT_TIMEOUT_MS: u64 = 60_000;
const MAX_TIMEOUT_MS: u64 = 300_000;
const MAX_OUTPUT_BYTES: usize = 50_000;
const MAX_COMMAND_LEN: usize = 4096;

// ====== 辅助函数 ======

fn tool_ok(text: String) -> ToolResult {
    ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({"text": text}),
        }],
        is_error: false,
    }
}

pub(crate) fn tool_err(msg: &str) -> ToolResult {
    ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({"text": format!("[Error] {msg}")}),
        }],
        is_error: true,
    }
}

/// 截断输出到指定字节数，超限时追加 "(truncated)" 标记。
fn truncate_output(output: &[u8], limit: usize) -> String {
    if output.len() <= limit {
        String::from_utf8_lossy(output).into_owned()
    } else {
        format!(
            "{}\n...(truncated, {} bytes total)",
            String::from_utf8_lossy(&output[..limit]),
            output.len()
        )
    }
}

// ====== 危险命令检测 ======

/// 危险类别 — 用于分级展示警告。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DangerCategory {
    /// 严重：系统破坏 / 提权 / fork bomb
    Critical,
    /// 高风险：网络外泄 / 环境注入
    High,
    /// 中等：代码执行 / git 危险操作
    Medium,
}

/// 危险命令检测结果。
#[derive(Debug, Clone)]
pub struct DangerInfo {
    /// 风险描述（中文，供 UI 展示）
    pub risk: &'static str,
    /// 危险类别
    pub category: DangerCategory,
}

/// 将命令拆分为 token 列表，保留引号边界。
/// 例如: `rm -rf "/path with spaces"` → `["rm", "-rf", "/path with spaces"]`
fn tokenize_command(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let bytes = command.as_bytes();

    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i] as char;

        match (ch, in_single, in_double) {
            ('\'', _, false) if !in_double => {
                // toggle single-quote (inside double-quote = literal)
                in_single = !in_single;
            }
            ('"', false, _) if !in_single => {
                // toggle double-quote (inside single-quote = literal)
                in_double = !in_double;
            }
            ('\\', _, _) => {
                // skip escaped characters inside quotes
                if i + 1 < bytes.len() {
                    i += 1;
                    current.push(bytes[i] as char);
                }
            }
            _ if ch.is_whitespace() && !in_single && !in_double => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => {
                current.push(ch);
            }
        }
        i += 1;
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Windows: 查找 `cmd /C` 包装之后的真正用户命令。
#[cfg(windows)]
fn extract_inner_command(command: &str) -> &str {
    let trimmed = command.trim();
    let lower = trimmed.to_lowercase();
    if lower.starts_with("cmd ") || lower.starts_with("cmd.exe ") {
        // 查找 /C 或 /c
        let after_cmd = &trimmed[if lower.starts_with("cmd.exe ") { 8 } else { 4 }..];
        if let Some(idx) = after_cmd.to_lowercase().find(" /c ") {
            return after_cmd[idx + 4..].trim();
        }
        if let Some(idx) = after_cmd.find(" /C ") {
            return after_cmd[idx + 4..].trim();
        }
    }
    trimmed
}

#[cfg(not(windows))]
fn extract_inner_command(command: &str) -> &str {
    command.trim()
}

/// 检查 token 是否是 `rm` 命令的别名（rm、/bin/rm、/usr/bin/rm、rmdir、del）。
fn is_rm_command(token: &str) -> bool {
    let basename = if let Some(pos) = token.rfind(['/', '\\']) {
        &token[pos + 1..]
    } else {
        token
    };
    matches!(basename, "rm" | "rmdir" | "del" | "rm.exe" | "del.exe")
}

/// 从 `curl` / `wget` 命令 token 列表中提取目标 URL。
fn extract_url_from_tokens(tokens: &[String], expected_cmd: &[&str]) -> Option<String> {
    // 跳过命令名本身，查找第一个不是 flag 的参数（不以 - 开头）
    let start_idx = tokens.iter().position(|t| {
        let lower = t.to_lowercase();
        expected_cmd.iter().any(|c| lower.ends_with(c))
    })?;
    for t in &tokens[start_idx + 1..] {
        if !t.starts_with('-') && !t.is_empty() {
            // 去掉可能的前后引号
            let url = t.trim_matches(|c: char| c == '"' || c == '\'');
            if url.contains("://") || url.contains('.') {
                return Some(url.to_string());
            }
        }
    }
    None
}

/// 检测命令是否包含高危操作（词法级别）。
///
/// 使用 token 分析而非子串匹配，避免路径前缀绕过
/// （如 `/usr/bin/sudo` 绕过 `sudo` 起始匹配）。
///
/// 返回 `Some(DangerInfo)` 表示危险，`None` 表示安全。
pub fn check_dangerous_command(command: &str) -> Option<DangerInfo> {
    let cmd = extract_inner_command(command);
    let lower_cmd = cmd.to_lowercase();
    let tokens = tokenize_command(cmd);
    let lower_tokens: Vec<String> = tokens.iter().map(|t| t.to_lowercase()).collect();

    // --- 1. 系统破坏：rm -rf + 关键系统路径 ---
    // 检测 `rm` 后面紧跟 `-rf`/`-fr`/`-r`/`-f` + 系统路径
    let system_paths = [
        "/", "/*", "/bin", "/boot", "/dev", "/etc", "/home", "/lib", "/lib64", "/media", "/mnt",
        "/opt", "/proc", "/root", "/run", "/sbin", "/srv", "/sys", "/usr", "/var", "~", "~/",
        "/tmp", "/var/tmp",
    ];

    for i in 0..tokens.len() {
        if is_rm_command(&lower_tokens[i]) {
            let args: Vec<&str> = tokens[i + 1..].iter().map(|t| t.as_str()).collect();
            let has_force = args
                .iter()
                .any(|a| *a == "-rf" || *a == "-fr" || *a == "-r" || *a == "-f");
            if has_force {
                for arg in &args {
                    if !arg.starts_with('-') {
                        for sp in &system_paths {
                            if *arg == *sp || arg.starts_with(sp) && *arg != "/tmp/" {
                                return Some(DangerInfo {
                                    risk: "危险：递归删除系统关键路径",
                                    category: DangerCategory::Critical,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // 检测 Windows: `del /s /q C:\`、`rmdir /s /q C:\`、`format C:`
    #[cfg(windows)]
    {
        for i in 0..lower_tokens.len() {
            let t = &lower_tokens[i];
            if (t == "del" || t == "del.exe" || t == "rmdir" || t == "rmdir.exe")
                && lower_tokens.get(i + 1).is_some_and(|a| a == "/s")
            {
                // 检查是否有 C:\ 之类的系统盘路径
                for arg in lower_tokens.iter().skip(i + 2) {
                    if arg.starts_with("c:\\") || arg.starts_with("c:") || arg == "c:\\" {
                        return Some(DangerInfo {
                            risk: "危险：删除/格式化系统盘",
                            category: DangerCategory::Critical,
                        });
                    }
                }
            }
            if t == "format" || t == "format.com" {
                let next = lower_tokens.get(i + 1).map(|s| s.as_str()).unwrap_or("");
                if next.starts_with("c:") || next.starts_with("c:\\") {
                    return Some(DangerInfo {
                        risk: "危险：删除/格式化系统盘",
                        category: DangerCategory::Critical,
                    });
                }
            }
        }
    }

    // --- 2. 提权：sudo / su ---
    for t in &lower_tokens {
        // 只匹配 standalone `sudo`，不匹配 e.g. `sudoedit`
        if t == "sudo" || t == "sudo.exe" {
            return Some(DangerInfo {
                risk: "危险：尝试提权执行 (sudo)",
                category: DangerCategory::Critical,
            });
        }
        if t == "su" || t == "su.exe" {
            // 检查是否后面跟了 `-c` 或 `root`
            let idx = lower_tokens.iter().position(|x| x == t).unwrap_or(0);
            let next = lower_tokens.get(idx.saturating_add(1)).map(|s| s.as_str());
            let next2 = lower_tokens.get(idx.saturating_add(2)).map(|s| s.as_str());
            if next == Some("-c") || next == Some("root") || next2 == Some("-c") {
                return Some(DangerInfo {
                    risk: "危险：尝试提权执行 (su)",
                    category: DangerCategory::Critical,
                });
            }
        }
    }

    // --- 3. 设备文件写入：dd of=/dev/... / > /dev ---
    if lower_tokens.iter().any(|t| t == "dd" || t == "dd.exe") {
        // 查找 of= 参数指向 /dev/
        for t in &tokens {
            let lower = t.to_lowercase();
            if (lower.starts_with("of=/dev/") || lower.starts_with("of=/dev/"))
                && !lower.contains("null")
                && !lower.contains("zero")
                && !lower.contains("random")
                && !lower.contains("urandom")
            {
                return Some(DangerInfo {
                    risk: "危险：直接写入设备文件 (dd)",
                    category: DangerCategory::Critical,
                });
            }
        }
    }

    // 重定向到 /dev/ 设备
    if cmd.contains("> /dev/") || cmd.contains(">> /dev/") || cmd.contains("> /dev") {
        // 允许 /dev/null、/dev/zero、/dev/random、/dev/urandom
        let has_dangerous_redirect = ["> /dev/", ">> /dev/", "> /dev"].iter().any(|pat| {
            if let Some(pos) = cmd.find(pat) {
                let suffix = &cmd[pos + pat.len()..];
                let first_word = suffix
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_end_matches(|c: char| !c.is_alphanumeric());
                !matches!(
                    first_word,
                    "null"
                        | "zero"
                        | "random"
                        | "urandom"
                        | "stdout"
                        | "stdin"
                        | "stderr"
                        | "fd"
                        | "tty"
                )
            } else {
                false
            }
        });
        if has_dangerous_redirect {
            return Some(DangerInfo {
                risk: "危险：重定向写入设备文件",
                category: DangerCategory::Critical,
            });
        }
    }

    // --- 4. fork bomb ---
    if lower_cmd.contains(":(){ :|:& };:")
        || lower_cmd.contains(":(){ :|:&};:")
        || (lower_cmd.contains("() {") && lower_cmd.contains(":&"))
    {
        return Some(DangerInfo {
            risk: "危险：fork bomb 模式",
            category: DangerCategory::Critical,
        });
    }

    // --- 5. 系统关机/重启 ---
    {
        let shutdown_cmds = [
            "shutdown",
            "reboot",
            "halt",
            "poweroff",
            "init",
            "shutdown.exe",
            "reboot.exe",
        ];
        for t in &lower_tokens {
            if shutdown_cmds.contains(&t.as_str()) {
                // `init 6` / `init 0` 才算危险，单独的 `init` 不是
                if t == "init" {
                    let idx = lower_tokens.iter().position(|x| x == t).unwrap_or(0);
                    let next = lower_tokens.get(idx.saturating_add(1)).map(|s| s.as_str());
                    if next == Some("0") || next == Some("6") {
                        return Some(DangerInfo {
                            risk: "危险：系统关机/重启",
                            category: DangerCategory::Critical,
                        });
                    }
                    continue;
                }
                return Some(DangerInfo {
                    risk: "危险：系统关机/重启",
                    category: DangerCategory::Critical,
                });
            }
        }
    }

    // --- 6. 网络外泄：curl/wget + 内网地址 ---
    {
        let download_cmds = ["curl", "curl.exe", "wget", "wget.exe"];
        if lower_tokens
            .iter()
            .any(|t| download_cmds.contains(&t.as_str()))
        {
            if let Some(url_str) = extract_url_from_tokens(&tokens, &download_cmds) {
                if let Ok(parsed) = url::Url::parse(&url_str) {
                    let host = parsed.host_str().unwrap_or("");
                    // 检查内网
                    if host.parse::<std::net::Ipv4Addr>().is_ok()
                        || host.parse::<std::net::Ipv6Addr>().is_ok()
                    {
                        return Some(DangerInfo {
                            risk: "危险：curl/wget 访问 IP 地址（疑似数据外泄）",
                            category: DangerCategory::High,
                        });
                    }
                    if ["localhost", "127.0.0.1", "[::1]", "169.254.169.254"]
                        .contains(&host.to_lowercase().as_str())
                    {
                        return Some(DangerInfo {
                            risk: "危险：curl/wget 访问内网/元数据地址",
                            category: DangerCategory::High,
                        });
                    }
                    // 非 HTTPS 的目标
                    if parsed.scheme() != "https" {
                        return Some(DangerInfo {
                            risk: "危险：curl/wget 使用非 HTTPS 协议（明文传输）",
                            category: DangerCategory::High,
                        });
                    }
                }
            }
            // curl 管道到 bash/sh
            if cmd.contains("| bash")
                || cmd.contains("| sh")
                || cmd.contains("|bash")
                || cmd.contains("|sh")
            {
                return Some(DangerInfo {
                    risk: "危险：curl/wget 管道到 shell 执行（远程代码执行）",
                    category: DangerCategory::Critical,
                });
            }
        }
    }

    // netcat 反弹 shell 检测
    {
        let nc_cmds = ["nc", "ncat", "netcat", "nc.exe", "ncat.exe", "netcat.exe"];
        if lower_tokens.iter().any(|t| nc_cmds.contains(&t.as_str()))
            && lower_tokens.iter().any(|t| t == "-e" || t == "-c")
        {
            return Some(DangerInfo {
                risk: "危险：nc/netcat 执行远程命令（反弹 shell）",
                category: DangerCategory::High,
            });
        }
    }

    // --- 7. 代码执行：eval / exec / source ---
    {
        let exec_cmds = ["eval", "exec", "source"];
        if lower_tokens.iter().any(|t| exec_cmds.contains(&t.as_str())) {
            return Some(DangerInfo {
                risk: "危险：使用 eval/exec/source 动态执行代码",
                category: DangerCategory::Medium,
            });
        }
        // 检测 `. script.sh` 模式（dot-space：bash source 语法）
        if lower_tokens.len() >= 2
            && lower_tokens[0] == "."
            && !lower_tokens[1].starts_with('/')
            && !lower_tokens[1].starts_with("./")
        {
            // `.` 后跟非路径 token 可能是 source 命令
            if lower_tokens[1].contains(".sh") || lower_tokens[1].contains(".bash") {
                return Some(DangerInfo {
                    risk: "危险：使用 source/dot 命令执行脚本",
                    category: DangerCategory::Medium,
                });
            }
        }
    }

    // --- 8. 环境注入：KEY=VALUE cmd ---
    {
        let env_inject_regex = regex::Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*=").ok();
        if let Some(re) = &env_inject_regex {
            let first = tokens.first().map(|t| t.as_str()).unwrap_or("");
            if re.is_match(first)
                && !first.contains('/')  // 排除路径中的 = 符号
                && tokens.len() > 1
            {
                // 检查是否在篡改 PATH/LD_PRELOAD/LD_LIBRARY_PATH 等关键变量
                let var_name = first.split('=').next().unwrap_or("").to_uppercase();
                if matches!(
                    var_name.as_str(),
                    "PATH" | "LD_PRELOAD" | "LD_LIBRARY_PATH" | "PYTHONPATH"
                ) {
                    return Some(DangerInfo {
                        risk: "危险：命令注入前篡改关键环境变量",
                        category: DangerCategory::High,
                    });
                }
            }
        }
    }

    // --- 9. git 危险操作 ---
    {
        let git_idx = lower_tokens
            .iter()
            .position(|t| t == "git" || t == "git.exe");
        if let Some(idx) = git_idx {
            let subcmd = lower_tokens.get(idx + 1).map(|s| s.as_str()).unwrap_or("");
            if subcmd == "push" {
                let has_force = lower_tokens
                    .get(idx + 2)
                    .is_some_and(|a| a == "--force" || a == "-f");
                let has_delete = tokens.get(idx + 2).is_some_and(|a| a == "--delete");
                if has_force {
                    return Some(DangerInfo {
                        risk: "危险：强制推送 (git push --force)",
                        category: DangerCategory::Medium,
                    });
                }
                if has_delete {
                    return Some(DangerInfo {
                        risk: "危险：删除远程分支 (git push --delete)",
                        category: DangerCategory::Medium,
                    });
                }
            }
        }
    }

    // --- 10. chmod 777 / chown 到关键路径 ---
    {
        let chmod_idx = lower_tokens
            .iter()
            .position(|t| t == "chmod" || t == "chmod.exe");
        if let Some(idx) = chmod_idx {
            let mode = lower_tokens.get(idx + 1).map(|s| s.as_str()).unwrap_or("");
            if mode == "777" || mode == "7777" || mode == "a+rwx" || mode == "ugo+rwx" {
                let target = tokens.get(idx + 2).map(|s| s.as_str()).unwrap_or("");
                if target.starts_with('/') && !target.starts_with("/tmp") {
                    return Some(DangerInfo {
                        risk: "危险：对系统路径设置 777 权限",
                        category: DangerCategory::Critical,
                    });
                }
            }
        }
    }

    None
}

// 保留旧接口兼容性（仅返回风险描述字符串），供调用方逐步迁移使用。
#[allow(dead_code)]
pub fn check_dangerous_command_str(command: &str) -> Option<&'static str> {
    check_dangerous_command(command).map(|d| d.risk)
}

// ====== 工具定义 ======

/// 返回 execute_command 的 ToolSpec。
pub fn get_command_tool_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".into(),
        function: ToolFunctionSpec {
            name: "execute_command".into(),
            description: "在项目目录中执行系统命令，返回 stdout/stderr/stdout 和退出码。用于运行构建、测试、git 命令等。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "要执行的命令。如 'cargo build', 'npm test', 'git status'。避免使用 sudo 或高危操作。"
                    },
                    "description": {
                        "type": "string",
                        "description": "简短描述命令用途，仅用于审计日志。如 '编译 Rust 项目'"
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "超时毫秒，默认 60s，最大 300s（5分钟）。长时间编译适量增大。"
                    }
                },
                "required": ["command"]
            }),
        },
    }
}

// ====== 命令执行 ======

/// 检查命令中的高危 shell 元字符序列。
fn check_shell_metacharacters(command: &str) -> Option<&'static str> {
    // 1. 反引号命令替换
    if command.contains('`') {
        return Some("禁止使用反引号 (`) 进行命令替换");
    }

    // 2. $() 命令替换（排除在单引号内的）
    // 简化处理：只要出现 $( 且在引号外就拒绝
    let mut in_single = false;
    let bytes = command.as_bytes();
    for i in 0..bytes.len().saturating_sub(1) {
        let ch = bytes[i] as char;
        match (ch, in_single) {
            ('\'', _) => in_single = !in_single,
            ('$', false) if i + 1 < bytes.len() && bytes[i + 1] as char == '(' => {
                return Some("禁止使用 $() 命令替换");
            }
            _ => {}
        }
    }

    // 3. 检测 `> /dev/` 设备文件重定向
    // 这个已经由 check_dangerous_command 覆盖，此处不重复检查

    None
}

/// 在项目根目录中执行系统命令。
///
/// # 参数
/// - `command` — 要执行的命令字符串
/// - `project_root` — 项目根目录绝对路径（工作目录）
/// - `timeout_ms` — 可选超时毫秒，默认 60s
///
/// # 返回
/// ToolResult，包含 stdout、stderr、exit_code、timed_out 信息。
pub fn execute_command(command: &str, project_root: &str, timeout_ms: Option<u64>) -> ToolResult {
    if command.trim().is_empty() {
        return tool_err("命令为空");
    }

    // 长度限制
    if command.len() > MAX_COMMAND_LEN {
        return tool_err(&format!(
            "命令过长 ({:.1}KB)，最大允许 {:.1}KB",
            command.len() as f64 / 1024.0,
            MAX_COMMAND_LEN as f64 / 1024.0
        ));
    }

    // 高危 shell 元字符检查
    if let Some(reason) = check_shell_metacharacters(command) {
        return tool_err(&format!("安全拦截: {reason}"));
    }

    let timeout = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS).min(MAX_TIMEOUT_MS);

    // 平台选择 shell
    let (shell, shell_arg) = if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };

    // 沙箱包装（Linux/macOS best-effort）：可用则替换启动 argv；否则保持原生执行
    let sandbox_argv =
        crate::utils::sandbox::sandbox_command_prefix(shell, shell_arg, command, project_root);
    let mut cmd = if let Some(argv) = sandbox_argv {
        let mut c = Command::new(&argv[0]);
        c.args(&argv[1..]);
        c
    } else {
        let mut c = Command::new(shell);
        c.arg(shell_arg).arg(command);
        c
    };
    cmd.current_dir(project_root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // 非 Windows：让命令自成进程组，便于超时/清理时把整个子树一并杀掉（AGT-05）
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return tool_err(&format!("启动命令失败: {e}")),
    };

    // Windows: 将子进程分配到限制 Job Object
    #[cfg(windows)]
    {
        if let Err(e) = crate::utils::sandbox::assign_to_job(&child) {
            // 沙箱分配失败不应阻塞执行，但记录错误到输出中
            // 注意：我们这里不 kill 进程，因为这属于非致命错误
            tracing::warn!("[shell] 无法分配 Job Object 沙箱: {e}");
        }
    }

    // 等待执行（带超时），使用 channel 避免依赖 wait_timeout（未稳定）
    use std::sync::{Arc, Mutex};
    let child_arc = Arc::new(Mutex::new(child));
    let child_clone = Arc::clone(&child_arc);

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut c = child_clone.lock().unwrap();
        let status = c.wait();
        let _ = tx.send(status);
    });

    let (exit_code, timed_out) = match rx.recv_timeout(Duration::from_millis(timeout)) {
        Ok(Ok(status)) => (status.code(), false),
        Ok(Err(_)) => return tool_err("等待命令进程失败"),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            // 超时 → 杀掉进程（非 Windows 连进程组一并清理）
            if let Ok(mut c) = child_arc.lock() {
                #[cfg(unix)]
                {
                    let pid = c.id();
                    if pid > 0 {
                        let _ = std::process::Command::new("kill")
                            .args(["-KILL", &format!("-{pid}")])
                            .status();
                    }
                }
                let _ = c.kill();
                let _ = c.wait();
            }
            (None, true)
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            return tool_err("命令进程意外终止");
        }
    };

    // 读取输出
    let stdout = {
        let mut c = child_arc.lock().unwrap();
        use std::io::Read;
        c.stdout
            .take()
            .and_then(|mut o| {
                let mut buf = Vec::new();
                std::io::BufReader::new(&mut o).read_to_end(&mut buf).ok()?;
                Some(buf)
            })
            .unwrap_or_default()
    };

    let stderr = {
        let mut c = child_arc.lock().unwrap();
        use std::io::Read;
        c.stderr
            .take()
            .and_then(|mut o| {
                let mut buf = Vec::new();
                std::io::BufReader::new(&mut o).read_to_end(&mut buf).ok()?;
                Some(buf)
            })
            .unwrap_or_default()
    };

    // 构建结果
    let mut result = String::new();
    result.push_str(&format!("$ {command}\n"));

    if timed_out {
        result.push_str(&format!("\n⏱ 命令超时 (>{timeout}ms)，已强制终止\n"));
    }

    if let Some(code) = exit_code {
        result.push_str(&format!("\nexit code: {code}\n"));
    }

    if !stdout.is_empty() {
        result.push_str(&format!(
            "\n[stdout]:\n{}\n",
            truncate_output(&stdout, MAX_OUTPUT_BYTES)
        ));
    }
    if !stderr.is_empty() {
        result.push_str(&format!(
            "\n[stderr]:\n{}\n",
            truncate_output(&stderr, MAX_OUTPUT_BYTES)
        ));
    }

    if (exit_code != Some(0)) || timed_out {
        tool_err(&result)
    } else {
        tool_ok(result)
    }
}

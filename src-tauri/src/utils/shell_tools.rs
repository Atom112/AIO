//! Built-in system command execution tool - lets the Agent run terminal commands.
//!
//! Design reference: Claude Code's Bash tool (simple schema + permission memory).
//! Invoked directly by `run_agent_turn` at tool dispatch, bypassing the MCP subprocess channel.
//!
//! Safety measures:
//! - Path sandbox: working directory confined to the project root
//! - Output truncation: stdout/stderr capped at 50KB each
//! - Timeout control: default 60s, max 300s
//! - Dangerous-command detection: lexical token analysis; Critical findings
//!   (system destruction / privilege escalation / fork bomb) are rejected in
//!   every mode; High/Medium findings log a warn and surface in the approval prompt
//! - Command filtering: 4096-char length limit; backtick/$() substitution forbidden
//! - Windows Job Object sandbox: constrains child process privileges
//! - Permission system: reuses the existing Allow/Ask/Deny rule engine

use crate::core::models::{ToolFunctionSpec, ToolResult, ToolSpec};
use serde_json::json;
use std::process::Command;
use std::time::Duration;

// ====== Constants ======

const DEFAULT_TIMEOUT_MS: u64 = 60_000;
const MAX_TIMEOUT_MS: u64 = 300_000;
const MAX_OUTPUT_BYTES: usize = 50_000;
const MAX_COMMAND_LEN: usize = 4096;

// ====== Helpers ======

// tool_ok / tool_err come from utils::tool_result (single source of truth repo-wide)
pub use crate::utils::tool_result::{tool_err, tool_ok};

// Truncation is provided by utils::truncate (name matches semantics)
use crate::core::tool_names;
use crate::utils::truncate::truncate_bytes_lossy as truncate_output;

// ====== Dangerous-command detection ======

/// Danger category - used for tiered warning display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DangerCategory {
    /// Critical: system destruction / privilege escalation / fork bomb
    Critical,
    /// High: network exfiltration / environment injection
    High,
    /// Medium: code execution / dangerous git operations
    Medium,
}

/// Result of dangerous-command detection.
#[derive(Debug, Clone)]
pub struct DangerInfo {
    /// Risk description (zh-CN, shown in the UI)
    pub risk: &'static str,
    /// Danger category
    pub category: DangerCategory,
}

/// Splits a command into a token list, preserving quote boundaries.
/// e.g. `rm -rf "/path with spaces"` → `["rm", "-rf", "/path with spaces"]`
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
            ('\\', true, _) | ('\\', _, true) => {
                // SEC-10 配套：反斜杠只在引号内作转义符；引号外它是 Windows 路径
                // 分隔符，必须原样保留——原先无条件剥反斜杠使 `C:\Windows\Temp`
                // 被切成 `c:windowstemp`，全部 `c:\` 前缀检查沦为死代码
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

/// Windows: finds the real user command behind the `cmd /C` wrapper.
#[cfg(windows)]
fn extract_inner_command(command: &str) -> &str {
    let trimmed = command.trim();
    let lower = trimmed.to_lowercase();
    if lower.starts_with("cmd ") || lower.starts_with("cmd.exe ") {
        // find /C or /c
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

/// Checks whether a token is an `rm` alias (rm, /bin/rm, /usr/bin/rm, rmdir, del).
fn is_rm_command(token: &str) -> bool {
    let basename = if let Some(pos) = token.rfind(['/', '\\']) {
        &token[pos + 1..]
    } else {
        token
    };
    matches!(basename, "rm" | "rmdir" | "del" | "rm.exe" | "del.exe")
}

/// Extracts the target URL from `curl` / `wget` command tokens.
fn extract_url_from_tokens(tokens: &[String], expected_cmd: &[&str]) -> Option<String> {
    // skip the command name itself; take the first argument that is not a flag (not starting with -)
    let start_idx = tokens.iter().position(|t| {
        let lower = t.to_lowercase();
        expected_cmd.iter().any(|c| lower.ends_with(c))
    })?;
    for t in &tokens[start_idx + 1..] {
        if !t.starts_with('-') && !t.is_empty() {
            // strip any surrounding quotes
            let url = t.trim_matches(|c: char| c == '"' || c == '\'');
            if url.contains("://") || url.contains('.') {
                return Some(url.to_string());
            }
        }
    }
    None
}

/// Detects whether a command contains high-risk operations (lexically).
///
/// **Positioning**: the enforcement layer is the OS sandbox (Linux bwrap / macOS sandbox-exec /
/// Windows AppContainer; see utils::sandbox); this check is a defense-in-depth
/// early-warning layer - Critical findings are hard-blocked unconditionally
/// (backstop for the no-approval Auto mode), while High/Medium findings only
/// annotate risk in the approval prompt. Lexical checks are inherently bypassable
/// and must not be the sole line of defense.
///
/// Uses token analysis instead of substring matching to avoid path-prefix bypasses
/// (e.g. `/usr/bin/sudo` bypassing a `sudo` prefix match).
///
/// Returns `Some(DangerInfo)` for dangerous, `None` for safe.
pub fn check_dangerous_command(command: &str) -> Option<DangerInfo> {
    let cmd = extract_inner_command(command);
    let lower_cmd = cmd.to_lowercase();
    let tokens = tokenize_command(cmd);
    let lower_tokens: Vec<String> = tokens.iter().map(|t| t.to_lowercase()).collect();

    // --- 1. System destruction: rm -rf + critical system paths ---
    // SEC-10: two-tier matching. (a) The directory ITSELF (exact) is always
    // catastrophic — `rm -rf /etc`, `rm -rf /home`, `rm -rf ~`, `rm -rf /`.
    // (b) Prefix matching applies ONLY to OS-critical trees (/etc, /usr, ...)
    // where any depth IS the system. Paths under /home/<user>/..., /tmp/...,
    // ~/..., /opt/... are USER data: the old prefix list hard-blocked e.g.
    // `rm -rf /home/u/proj/build` and `rm -rf ~/build` in every mode (Critical
    // bypasses approvals entirely) — those now go through the normal sandbox +
    // approval flow instead.
    let os_critical_prefixes: &[&str] = &[
        "/etc",
        "/bin",
        "/boot",
        "/dev",
        "/lib",
        "/lib64",
        "/proc",
        "/root",
        "/run",
        "/sbin",
        "/sys",
        "/usr",
        "c:\\windows",
    ];
    let exact_catastrophic: &[&str] = &[
        "/", "/*", "/home", "/opt", "/mnt", "/media", "/srv", "/var", "/tmp", "/var/tmp", "~",
        "/bin", "/boot", "/dev", "/etc", "/lib", "/lib64", "/proc", "/root", "/run", "/sbin",
        "/sys", "/usr", "c:", "c:\\", "c:/",
    ];
    // SEC-10: normalize trailing slashes so `/etc/` and `~/` hit the exact list
    // (`//` collapses to `/`); a leading-slash-only string normalizes to `/`.
    let norm_path = |p: &str| -> String {
        let n = p.trim_end_matches('/');
        if n.is_empty() && p.starts_with('/') {
            "/".to_string()
        } else {
            n.to_string()
        }
    };

    for i in 0..tokens.len() {
        if is_rm_command(&lower_tokens[i]) {
            let args: Vec<&str> = tokens[i + 1..].iter().map(|t| t.as_str()).collect();
            let has_force = args
                .iter()
                .any(|a| *a == "-rf" || *a == "-fr" || *a == "-r" || *a == "-f");
            if has_force {
                for arg in &args {
                    if arg.starts_with('-') || arg.is_empty() {
                        continue;
                    }
                    let norm = norm_path(arg);
                    let is_system = exact_catastrophic.contains(&norm.as_str())
                        || os_critical_prefixes.iter().any(|p| {
                            norm.starts_with(&format!("{p}/"))
                                || norm.starts_with(&format!("{p}\\"))
                        });
                    if is_system {
                        return Some(DangerInfo {
                            risk: "危险：递归删除系统关键路径",
                            category: DangerCategory::Critical,
                        });
                    }
                }
            }
        }
    }

    // Windows detection: `del /s /q C:\`, `rmdir /s /q C:\`, `format C:`
    #[cfg(windows)]
    {
        for i in 0..lower_tokens.len() {
            let t = &lower_tokens[i];
            if (t == "del" || t == "del.exe" || t == "rmdir" || t == "rmdir.exe")
                && lower_tokens.get(i + 1).is_some_and(|a| a == "/s")
            {
                // check for a system drive path such as C:\
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

    // PowerShell 递归删除：`Remove-Item -Recurse [-Force] <系统路径/~>`。
    // §11a 递归会把引号载荷送进本函数（SEC-07 旗标修复后可达），这里补 cmdlet 词法。
    // SEC-10: same two-tier rule as the rm section — exact system-dir targets or
    // OS-critical-tree prefixes are Critical; user subpaths (/home/u/..., ~/build,
    // C:\Users\...) go through sandbox + approval instead of a hard block.
    for i in 0..lower_tokens.len() {
        if lower_tokens[i] == "remove-item" {
            let args = &tokens[i + 1..];
            let has_recurse = args.iter().any(|a| {
                let la = a.to_lowercase();
                la == "-recurse" || la == "-r" || la == "-recurse:$true"
            });
            if has_recurse {
                for arg in args {
                    if arg.starts_with('-') {
                        continue;
                    }
                    let lowered = arg.to_lowercase();
                    let target = lowered.trim_matches('"').trim_matches('\'');
                    let norm = norm_path(target);
                    let is_system = exact_catastrophic.contains(&norm.as_str())
                        || os_critical_prefixes.iter().any(|p| {
                            norm.starts_with(&format!("{p}/"))
                                || norm.starts_with(&format!("{p}\\"))
                        });
                    if is_system {
                        return Some(DangerInfo {
                            risk: "危险：PowerShell 递归删除系统关键路径",
                            category: DangerCategory::Critical,
                        });
                    }
                }
            }
        }
    }

    // --- 2. Privilege escalation: sudo / su ---
    for t in &lower_tokens {
        // match standalone `sudo` only, not e.g. `sudoedit`
        if t == "sudo" || t == "sudo.exe" {
            return Some(DangerInfo {
                risk: "危险：尝试提权执行 (sudo)",
                category: DangerCategory::Critical,
            });
        }
        if t == "su" || t == "su.exe" {
            // check whether it is followed by `-c` or `root`
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

    // --- 3. Device-file writes: dd of=/dev/... / > /dev ---
    if lower_tokens.iter().any(|t| t == "dd" || t == "dd.exe") {
        // find an of= argument pointing into /dev/
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

    // redirection into a /dev/ device
    if cmd.contains("> /dev/") || cmd.contains(">> /dev/") || cmd.contains("> /dev") {
        // allow /dev/null, /dev/zero, /dev/random, /dev/urandom
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

    // --- 5. System shutdown/reboot ---
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
                // only `init 6` / `init 0` are dangerous; bare `init` is not
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

    // --- 6. Network exfiltration: curl/wget + intranet addresses ---
    {
        let download_cmds = ["curl", "curl.exe", "wget", "wget.exe"];
        if lower_tokens
            .iter()
            .any(|t| download_cmds.contains(&t.as_str()))
        {
            if let Some(url_str) = extract_url_from_tokens(&tokens, &download_cmds) {
                if let Ok(parsed) = url::Url::parse(&url_str) {
                    let host = parsed.host_str().unwrap_or("");
                    // check for an intranet host
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
                    // non-HTTPS target
                    if parsed.scheme() != "https" {
                        return Some(DangerInfo {
                            risk: "危险：curl/wget 使用非 HTTPS 协议（明文传输）",
                            category: DangerCategory::High,
                        });
                    }
                }
            }
            // curl piped into bash/sh
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

    // netcat reverse-shell detection
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

    // --- 7. Code execution: eval / exec / source ---
    {
        let exec_cmds = ["eval", "exec", "source"];
        if lower_tokens.iter().any(|t| exec_cmds.contains(&t.as_str())) {
            return Some(DangerInfo {
                risk: "危险：使用 eval/exec/source 动态执行代码",
                category: DangerCategory::Medium,
            });
        }
        // detect the `. script.sh` pattern (dot-space: bash source syntax)
        if lower_tokens.len() >= 2
            && lower_tokens[0] == "."
            && !lower_tokens[1].starts_with('/')
            && !lower_tokens[1].starts_with("./")
        {
            // `.` followed by a non-path token is likely the source command
            if lower_tokens[1].contains(".sh") || lower_tokens[1].contains(".bash") {
                return Some(DangerInfo {
                    risk: "危险：使用 source/dot 命令执行脚本",
                    category: DangerCategory::Medium,
                });
            }
        }
    }

    // --- 8. Environment injection: KEY=VALUE cmd ---
    {
        let env_inject_regex = regex::Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*=").ok();
        if let Some(re) = &env_inject_regex {
            let first = tokens.first().map(|t| t.as_str()).unwrap_or("");
            if re.is_match(first)
                && !first.contains('/')  // exclude '=' inside paths
                && tokens.len() > 1
            {
                // check for tampering with critical vars like PATH/LD_PRELOAD/LD_LIBRARY_PATH
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

    // --- 9. Dangerous git operations ---
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

    // --- 10. chmod 777 / chown on critical paths ---
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

    // --- 11. Historical bypass vectors ---
    {
        // 11a. Interpreter -c quoted payloads: after tokenization the payload is a
        //      single token, bypassing the lexical checks above (e.g. bash -c "rm -rf ~").
        //      Strip the quotes and recursively check the payload itself.
        //      SEC-07: PowerShell 的单横线长项 `-Command`（小写化后 "-command"）也必须
        //      视为载荷旗标——缺了它，`powershell -Command "rm -rf ~"` 逃过递归检查。
        let shells = ["bash", "sh", "zsh", "dash", "ksh", "pwsh", "powershell"];
        for i in 0..lower_tokens.len() {
            if shells.contains(&lower_tokens[i].as_str())
                && lower_tokens
                    .get(i + 1)
                    .is_some_and(|a| a == "-c" || a == "--command" || a == "-command")
            {
                if let Some(payload) = tokens.get(i + 2) {
                    let stripped = payload.trim_matches(|c| c == '"' || c == '\'');
                    if let Some(inner) = check_dangerous_command(stripped) {
                        return Some(inner);
                    }
                }
            }
        }

        // 11b. Any producer piped into a shell: applies to every producer
        //      (echo/ cat/ base64, ...), not only curl/wget.
        let is_shell_token = |t: &str| {
            let base = t.rsplit('/').next().unwrap_or(t);
            matches!(base, "sh" | "bash" | "zsh" | "dash" | "ksh")
        };
        for i in 0..tokens.len() {
            let t = tokens[i].as_str();
            // shape 1: `|` as a standalone token, next token is a shell
            if t == "|" && tokens.get(i + 1).is_some_and(|n| is_shell_token(n)) {
                return Some(DangerInfo {
                    risk: "危险：命令管道到 shell 执行（任意代码执行）",
                    category: DangerCategory::Critical,
                });
            }
            // shape 2: `|sh` written as a single token
            if let Some(rest) = t.strip_prefix('|') {
                if is_shell_token(rest) {
                    return Some(DangerInfo {
                        risk: "危险：命令管道到 shell 执行（任意代码执行）",
                        category: DangerCategory::Critical,
                    });
                }
            }
        }

        // 11c. find -delete / find -exec rm: common recursive-deletion bypass shapes
        if lower_tokens.iter().any(|t| t == "find" || t == "find.exe") {
            if lower_tokens.iter().any(|t| t == "-delete") {
                return Some(DangerInfo {
                    risk: "危险：find -delete 递归删除",
                    category: DangerCategory::Critical,
                });
            }
            if let Some(exec_idx) = lower_tokens.iter().position(|t| t == "-exec") {
                if lower_tokens
                    .get(exec_idx + 1)
                    .is_some_and(|n| is_rm_command(n))
                {
                    return Some(DangerInfo {
                        risk: "危险：find -exec rm 递归删除",
                        category: DangerCategory::Critical,
                    });
                }
            }
        }

        // 11d. xargs rm: bulk deletion
        if let Some(xidx) = lower_tokens
            .iter()
            .position(|t| t == "xargs" || t == "xargs.exe")
        {
            if lower_tokens.get(xidx + 1).is_some_and(|n| is_rm_command(n)) {
                return Some(DangerInfo {
                    risk: "危险：xargs rm 批量删除",
                    category: DangerCategory::Critical,
                });
            }
        }

        // 11e. git clean -f*: wipes all untracked files (-x includes ignored files)
        if let Some(idx) = lower_tokens
            .iter()
            .position(|t| t == "git" || t == "git.exe")
        {
            if lower_tokens.get(idx + 1).is_some_and(|s| s == "clean") {
                let has_f = lower_tokens
                    .get(idx + 2)
                    .is_some_and(|a| a.starts_with('-') && a.contains('f'));
                if has_f {
                    return Some(DangerInfo {
                        risk: "危险：git clean -f 删除未跟踪文件",
                        category: DangerCategory::Critical,
                    });
                }
            }
        }
    }

    None
}

// ====== Tool definitions ======

/// Returns the ToolSpec for execute_command.
pub fn get_command_tool_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".into(),
        function: ToolFunctionSpec {
            name: tool_names::EXECUTE_COMMAND.into(),
            description: "在项目目录中执行系统命令，返回 stdout/stderr 和退出码。沙箱内默认**无网络**（Linux/macOS OS 级隔离）：命令需要联网（下载依赖、API 调用、访问 localhost 服务）时必须显式设 network=true（审批时向用户标注）。用于运行构建、测试、git 命令等。".into(),
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
                    },
                    "network": {
                        "type": "boolean",
                        "description": "此命令是否需要网络访问。默认 false（沙箱内无网络，OS 级阻断外部出口）。npm install / cargo build 首次拉取依赖、curl API 等必须设为 true。"
                    }
                },
                "required": ["command"]
            }),
        },
    }
}

// ====== Command execution ======

/// Script-interpreter names eligible for host-PATH forwarding. Harness-session
/// finding (round 2, issue 2): the sandboxed shell sees a stripped environment —
/// `python`/`node`/`py` probes all exit 9009 — while the host AIO process itself
/// has them on PATH; forwarding bridges the two views.
const FORWARDED_INTERPRETERS: &[&str] = &["python", "python3", "py", "pip", "node", "npm", "npx"];

/// Rewrites a command whose FIRST token is a bare known-interpreter name into the
/// host-process-resolved absolute path, returning the rewritten command plus the
/// interpreter's directory (for best-effort R+X granting inside the AppContainer
/// sandbox — the interpreter tree is normally invisible to the AC token, e.g. a
/// D:\Anaconda3 without an ALL APPLICATION PACKAGES ACE).
///
/// Fires only on a literally bare first token (no quotes, path separators, or
/// drive colons): `python tests/x.py`, `node -e "..."`, `npm install`. Anything
/// else (`cd x && python y`, absolute paths, `cmd /c python ...`) passes through
/// untouched. Resolution is PATH-only via [`crate::utils::exec_resolve`] (SEC-06:
/// the project cwd is never consulted, so a planted interpreter is not picked up).
/// When resolution fails the command passes through unchanged. Pure — unit-tested.
pub(crate) fn forward_script_interpreter(command: &str) -> (String, Option<std::path::PathBuf>) {
    let trimmed = command.trim_start();
    let indent_len = command.len() - trimmed.len();
    let first_end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    let first = &trimmed[..first_end];
    let is_bare_known = !first.is_empty()
        && !first.contains(['/', '\\', '"', ':'])
        && FORWARDED_INTERPRETERS.contains(&first.to_ascii_lowercase().as_str());
    if !is_bare_known {
        return (command.to_string(), None);
    }
    let Some(resolved) = crate::utils::exec_resolve::resolve_executable(first) else {
        return (command.to_string(), None);
    };
    let rest = &trimmed[first_end..];
    let rewritten = format!(
        "{}\"{}\"{}",
        &command[..indent_len],
        resolved.display(),
        rest
    );
    (rewritten, resolved.parent().map(|p| p.to_path_buf()))
}

/// Checks the command for dangerous shell metacharacter sequences.
fn check_shell_metacharacters(command: &str) -> Option<&'static str> {
    // 1. backtick command substitution
    if command.contains('`') {
        return Some("禁止使用反引号 (`) 进行命令替换");
    }

    // 2. $() command substitution (excluding ones inside single quotes)
    // simplified: any $( outside single quotes is rejected
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

    // 3. `> /dev/` device-file redirection
    // already covered by check_dangerous_command; not rechecked here

    None
}

/// Reads a pipe into a bounded buffer: keeps the first cap bytes, **keeps draining**
/// (discarding the rest) and counts total bytes read. Continuous draining guarantees
/// the producer never blocks writing to the pipe (paired with reader threads started
/// before wait(), this avoids the deadlock when output exceeds the ~64KB OS pipe buffer).
///
/// Returns (kept bytes, total bytes); the total feeds the truncation note in the result.
fn drain_pipe_capped<R: std::io::Read>(mut pipe: R, cap: usize) -> (Vec<u8>, usize) {
    let mut kept = Vec::new();
    let mut total = 0usize;
    let mut buf = [0u8; 16384];
    loop {
        match pipe.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                total += n;
                if kept.len() < cap {
                    let take = (cap - kept.len()).min(n);
                    kept.extend_from_slice(&buf[..take]);
                }
            }
        }
    }
    (kept, total)
}

/// Executes a system command in the project root directory.
///
/// # Arguments
/// - `command` - the command string to execute
/// - `project_root` - absolute path of the project root (working directory)
/// - `timeout_ms` - optional timeout in milliseconds, default 60s
/// - `allow_network` - whether network egress is allowed (false = Linux --unshare-net /
///   macOS SBPL deny network*; declared explicitly by the model via the `network` tool
///   parameter, default false)
/// - `ac_dirs` - Windows AppContainer config (Some = enabled + toolchain R+X dirs;
///   resolved by the caller from the app config; always None on non-Windows)
///
/// # Returns
/// A ToolResult carrying stdout, stderr, exit_code and timed_out.
pub fn execute_command(
    command: &str,
    project_root: &str,
    timeout_ms: Option<u64>,
    allow_network: bool,
    ac_dirs: Option<Vec<String>>,
) -> ToolResult {
    if command.trim().is_empty() {
        return tool_err("命令为空");
    }

    // length limit
    if command.len() > MAX_COMMAND_LEN {
        return tool_err(&format!(
            "命令过长 ({:.1}KB)，最大允许 {:.1}KB",
            command.len() as f64 / 1024.0,
            MAX_COMMAND_LEN as f64 / 1024.0
        ));
    }

    // Harness follow-up (round 2, issue 2): bare interpreter names resolve against
    // the HOST PATH (the sandbox shell's stripped PATH can't see them).
    let (command, interpreter_dir) = forward_script_interpreter(command);
    let command = command.as_str();
    #[cfg(not(windows))]
    let _ = &interpreter_dir; // only the AppContainer branch consumes the dir

    // dangerous shell metacharacter check
    if let Some(reason) = check_shell_metacharacters(command) {
        return tool_err(&format!("安全拦截: {reason}"));
    }

    // Dangerous-command detection: Critical findings (system destruction / privilege
    // escalation / fork bomb) are **rejected in every agent mode** - in Auto/Workflow
    // mode the execute_command permission is Allow and skips the approval prompt, so
    // without this interception there would be no protection at all. High/Medium
    // findings only log a warning (in Ask mode the approval prompt shows the risk
    // description to the user).
    if let Some(danger) = check_dangerous_command(command) {
        if danger.category == DangerCategory::Critical {
            return tool_err(&format!("已拒绝执行高危命令: {}", danger.risk));
        }
        tracing::warn!("检测到潜在风险命令（{}）: {}", danger.risk, command);
    }

    let timeout = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS).min(MAX_TIMEOUT_MS);

    // Windows AppContainer OS-level sandbox: when enabled via config
    // (sandboxWindows=appcontainer) take a dedicated path - file-write control +
    // network blocked by default; any failure fails closed with an error.
    #[cfg(windows)]
    if let Some(dirs) = ac_dirs {
        // The forwarded interpreter's tree needs R+X inside the AC (its directory
        // usually has no ALL APPLICATION PACKAGES ACE, e.g. D:\Anaconda3). Granted
        // best-effort: a system location (C:\Windows py.exe) may deny the ACL write
        // while already being readable — that must not kill the command.
        let dirs_lower: Vec<String> = dirs.iter().map(|d| d.to_ascii_lowercase()).collect();
        let interpreter_extra: Vec<String> = interpreter_dir
            .filter(|d| {
                let dl = d.to_string_lossy().to_ascii_lowercase();
                !dirs_lower.iter().any(|x| x.eq_ignore_ascii_case(&dl))
            })
            .map(|d| vec![d.to_string_lossy().into_owned()])
            .unwrap_or_default();
        return execute_command_in_appcontainer(
            command,
            project_root,
            timeout,
            allow_network,
            &dirs,
            &interpreter_extra,
        );
    }

    // pick the shell per platform
    let shell_name = if cfg!(windows) { "cmd" } else { "sh" };
    let shell_arg = if cfg!(windows) { "/C" } else { "-c" };
    // SEC-06: resolve the shell against PATH only — CreateProcess searches the cwd
    // (the project root, model-writable) BEFORE PATH, so a planted `cmd.exe`/`sh`
    // there must not be executed.
    let Some(shell) = crate::utils::exec_resolve::resolve_executable(shell_name) else {
        return tool_err(&format!(
            "无法定位系统 shell（PATH 中未找到 {shell_name}），拒绝执行"
        ));
    };
    let shell = shell.to_string_lossy().into_owned();

    // Sandbox wrapping (Linux/macOS best-effort): replace the launch argv when available; otherwise run natively
    let sandbox_argv = crate::utils::sandbox::sandbox_command_prefix(
        &shell,
        shell_arg,
        command,
        project_root,
        allow_network,
    );
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
    // Non-Windows: run the command in its own process group so timeout/cleanup kills the whole subtree
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return tool_err(&format!("启动命令失败: {e}")),
    };

    // Windows: assign the child to a per-execution Job Object sandbox (the guard's
    // Drop tears down the entire process tree, grandchildren included, on timeout).
    // No-op on non-Windows.
    let mut job_guard = match crate::utils::sandbox::assign_to_job(&child) {
        Ok(g) => Some(g),
        Err(e) => {
            // SEC-15: fail-closed. The Job Object is the only process-tree containment
            // on the native Windows path — without it, a timeout kill would take down
            // only the direct child and leak every grandchild. Running unsandboxed
            // after a failed assignment (the historic fail-open) silently dropped the
            // tree-teardown guarantee. Kill the just-spawned child and error instead.
            let _ = child.kill();
            let _ = child.wait();
            return tool_err(&format!(
                "无法分配 Job Object 沙箱（fail-closed，命令未执行）: {e}"
            ));
        }
    };

    // Reader threads must start before wait(): reading only after wait() deadlocks
    // once output exceeds the OS pipe buffer (~64KB) - the child blocks writing to
    // the pipe and never exits, then gets killed by the timeout even if the command
    // itself would finish in 2 seconds. Readers keep the first MAX_OUTPUT_BYTES bytes
    // and keep draining the rest so the child never blocks on pipe writes.
    use std::sync::mpsc;
    let (stdout_tx, stdout_rx) = mpsc::channel();
    let (stderr_tx, stderr_rx) = mpsc::channel();
    if let Some(pipe) = child.stdout.take() {
        std::thread::spawn(move || {
            let _ = stdout_tx.send(drain_pipe_capped(pipe, MAX_OUTPUT_BYTES));
        });
    }
    if let Some(pipe) = child.stderr.take() {
        std::thread::spawn(move || {
            let _ = stderr_tx.send(drain_pipe_capped(pipe, MAX_OUTPUT_BYTES));
        });
    }

    // wait with timeout via a channel, avoiding a dependency on the unstable wait_timeout
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
            // timeout -> tear down the whole process tree immediately:
            // - Windows: dropping the Job guard triggers KILL_ON_JOB_CLOSE (all grandchildren included)
            // - non-Windows: kill the process group (process_group(0) put the subtree in its own group)
            drop(job_guard.take());
            if let Ok(mut c) = child_arc.lock() {
                #[cfg(unix)]
                {
                    let pid = c.id();
                    if pid > 0 {
                        // PATH-only resolution (SEC-06): never spawn a bare `kill`
                        if let Some(kill) = crate::utils::exec_resolve::resolve_executable("kill") {
                            let _ = std::process::Command::new(kill)
                                .arg("-KILL")
                                .arg(format!("-{pid}"))
                                .status();
                        }
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

    // collect reader results: the process has exited or been torn down, so the pipes
    // close and the readers finish shortly; in the extreme case (a leftover process
    // still holds the pipe write end) give up after 10s to avoid unbounded waiting.
    let (stdout, stdout_total) = stdout_rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or_default();
    let (stderr, stderr_total) = stderr_rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or_default();

    build_command_result(
        command,
        exit_code,
        timed_out,
        timeout,
        stdout,
        stdout_total,
        stderr,
        stderr_total,
    )
}

/// Full command lifecycle for the Windows AppContainer path.
///
/// Differences from the native path: the process is created with the
/// SECURITY_CAPABILITY attribute (low-privilege token, network off by default);
/// file writes are limited to the project root / dedicated temp dir / toolchain
/// R+X dirs; stdout/stderr pipes are created by the AC module (File read ends);
/// timeouts tear down the whole process tree via Job guard Drop + TerminateProcess.
/// All other semantics (drain-to-avoid-deadlock, truncation, result building)
/// match the native path.
#[cfg(windows)]
fn execute_command_in_appcontainer(
    command: &str,
    project_root: &str,
    timeout: u64,
    allow_network: bool,
    toolchain_dirs: &[String],
    best_effort_rx_dirs: &[String],
) -> ToolResult {
    // Same command-line shape as the native path: cmd /C <command> (cwd is set to
    // the project root by the AC module). With the AppContainer attribute present,
    // executable resolution runs under the low-privilege token - the full path
    // (ComSpec) must be used; a bare "cmd" fails with ERROR_FILE_NOT_FOUND.
    // SEC-06: if ComSpec is unset, fall back to PATH-only resolution (never a bare
    // name, which would consult the project-root cwd).
    let cmd_path = std::env::var("ComSpec")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            crate::utils::exec_resolve::resolve_executable("cmd")
                .map(|p| p.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "cmd.exe".to_string());
    let cmdline = format!("{cmd_path} /C {command}");
    let (mut child, job) = match crate::utils::sandbox::spawn_in_appcontainer(
        &cmdline,
        project_root,
        allow_network,
        toolchain_dirs,
        best_effort_rx_dirs,
    ) {
        Ok(v) => v,
        Err(e) => {
            return tool_err(&format!(
                "AppContainer 沙箱启动失败（fail-closed，不回退无沙箱执行）: {e}"
            ))
        }
    };
    let mut job = Some(job);
    tracing::debug!("[appcontainer] 命令已启动 (pid={}): {command}", child.pid);

    // reader threads (same drain semantics as the native path; avoids the >64KB output deadlock)
    use std::sync::mpsc;
    let (stdout_tx, stdout_rx) = mpsc::channel();
    let (stderr_tx, stderr_rx) = mpsc::channel();
    if let Some(pipe) = child.stdout_read.take() {
        std::thread::spawn(move || {
            let _ = stdout_tx.send(drain_pipe_capped(pipe, MAX_OUTPUT_BYTES));
        });
    }
    if let Some(pipe) = child.stderr_read.take() {
        std::thread::spawn(move || {
            let _ = stderr_tx.send(drain_pipe_capped(pipe, MAX_OUTPUT_BYTES));
        });
    }

    // wait (blocking; this function runs on a spawn_blocking thread). Timeout -> Job kills the whole tree + terminate as backstop
    let (exit_code, timed_out) = {
        let (code, to) = crate::utils::sandbox::wait_with_timeout(&child, timeout as u32);
        if to {
            drop(job.take());
            crate::utils::sandbox::terminate(&child);
            (None, true)
        } else {
            (code, false)
        }
    };

    let (stdout, stdout_total) = stdout_rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or_default();
    let (stderr, stderr_total) = stderr_rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or_default();
    // also clean up potentially leftover grandchildren (daemons etc.) on the normal path
    drop(job.take());

    build_command_result(
        command,
        exit_code,
        timed_out,
        timeout,
        stdout,
        stdout_total,
        stderr,
        stderr_total,
    )
}

/// Shared result construction for the native and AppContainer paths.
#[allow(clippy::too_many_arguments)]
fn build_command_result(
    command: &str,
    exit_code: Option<i32>,
    timed_out: bool,
    timeout: u64,
    stdout: Vec<u8>,
    stdout_total: usize,
    stderr: Vec<u8>,
    stderr_total: usize,
) -> ToolResult {
    let mut result = String::new();
    result.push_str(&format!("$ {command}\n"));

    if timed_out {
        result.push_str(&format!("\n⏱ 命令超时 (>{timeout}ms)，已强制终止\n"));
    }

    if let Some(code) = exit_code {
        result.push_str(&format!("\nexit code: {code}\n"));
    }

    if !stdout.is_empty() || stdout_total > 0 {
        result.push_str(&format!(
            "\n[stdout]:\n{}\n",
            truncate_output(&stdout, MAX_OUTPUT_BYTES)
        ));
        if stdout_total > stdout.len() {
            result.push_str(&format!(
                "[!] stdout 输出超长，仅保留前 {}B / 共 {}B\n",
                stdout.len(),
                stdout_total
            ));
        }
    }
    if !stderr.is_empty() || stderr_total > 0 {
        result.push_str(&format!(
            "\n[stderr]:\n{}\n",
            truncate_output(&stderr, MAX_OUTPUT_BYTES)
        ));
        if stderr_total > stderr.len() {
            result.push_str(&format!(
                "[!] stderr 输出超长，仅保留前 {}B / 共 {}B\n",
                stderr.len(),
                stderr_total
            ));
        }
    }

    if (exit_code != Some(0)) || timed_out {
        tool_err(&result)
    } else {
        tool_ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(r: &ToolResult) -> String {
        r.content
            .first()
            .and_then(|c| c.data.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }

    /// Critical-level dangerous commands must be rejected directly inside
    /// execute_command, without relying on the approval prompt. Covers the
    /// protection gap of Auto/Workflow mode (execute_command permission is
    /// Allow, no Ask prompt). Note: these samples are all lexically matched -
    /// detection returns before anything actually runs.
    #[test]
    fn critical_dangerous_commands_are_blocked() {
        let root = std::env::temp_dir().to_string_lossy().to_string();
        for cmd in [
            "sudo echo aio_test",
            // targets a nonexistent device name: harmless even if the guard fails
            "dd if=/dev/zero of=/dev/aio_test_nonexistent_device",
        ] {
            let r = execute_command(cmd, &root, Some(5_000), false, None);
            assert!(r.is_error, "命令应被拒绝: {cmd}");
            assert!(
                text_of(&r).contains("高危命令"),
                "拒绝原因缺失: {}",
                text_of(&r)
            );
        }
    }

    /// Known bypass vectors listed in the security review must all be classified Critical.
    #[test]
    fn known_bypass_vectors_are_blocked() {
        for cmd in [
            // interpreter quoted payload (a single token after tokenization)
            "bash -c \"rm -rf ~\"",
            "sh -c 'sudo reboot'",
            // any producer piped into a shell
            "echo payload | sh",
            "cat evil.sh | bash",
            "base64 -d payload | sh",
            "xxd -r x | bash",
            "|bash echo x",
            // find / xargs recursive deletion
            "find / -delete",
            "find . -exec rm {} ;",
            "xargs rm -rf .",
            // git clean
            "git clean -fdx",
        ] {
            let info = check_dangerous_command(cmd);
            assert!(info.is_some(), "绕过向量未命中: {cmd}");
            assert_eq!(
                info.unwrap().category,
                DangerCategory::Critical,
                "应为 Critical: {cmd}"
            );
        }
    }

    /// New checks must not introduce false positives (benign pipe/find/git usage stays unaffected).
    #[test]
    fn benign_pipe_and_find_still_run() {
        for cmd in [
            "ls | grep foo",
            "cargo build | tee log.txt",
            "find . -name '*.rs'",
            "find . -type f -exec grep x {} ;",
            "echo rm | xargs echo",
            "git clean -n",
            "bash -c 'cargo test'",
            "powershell -Command Get-Date",
            "powershell -Command Get-ChildItem",
        ] {
            assert!(check_dangerous_command(cmd).is_none(), "假阳性: {cmd}");
        }
    }

    /// Regression (SEC-07): `powershell -Command "<payload>"` is a quoted payload the
    /// §11a recursion must inspect — the flag set used to miss the single-dash long
    /// form, letting `rm -rf ~` slip through as one quoted token.
    #[test]
    fn powershell_single_dash_command_flag_is_inspected() {
        for cmd in [
            "powershell -Command \"Remove-Item -Recurse -Force ~\"",
            "pwsh -command 'rm -rf ~'",
        ] {
            let danger = check_dangerous_command(cmd);
            assert!(
                danger.is_some_and(|d| d.category == DangerCategory::Critical),
                "应识别载荷危险: {cmd}"
            );
        }
    }

    /// Benign commands are unaffected (false-positive regression guard).
    #[test]
    fn benign_command_still_runs() {
        let root = std::env::temp_dir().to_string_lossy().to_string();
        let r = execute_command("echo aio_benign_check", &root, Some(10_000), false, None);
        assert!(!r.is_error, "普通命令不应被拦截: {}", text_of(&r));
        assert!(text_of(&r).contains("aio_benign_check"));
    }

    /// SEC-10: user absolute paths under /home, /tmp, ~ are NOT system destruction —
    /// the old prefix list hard-blocked them as Critical in every mode (Critical
    /// bypasses approvals entirely), breaking legitimate `rm -rf` cleanup.
    #[test]
    fn rm_rf_user_subpaths_are_not_critical() {
        for cmd in [
            "rm -rf /home/user/project/build",
            "rm -rf /tmp/cache-dir",
            "rm -rf ~/build",
            "rm -rf /opt/myapp/cache",
            "rm -rf C:\\Users\\me\\proj\\target",
            "Remove-Item -Recurse C:\\Users\\me\\proj\\node_modules",
        ] {
            assert!(
                check_dangerous_command(cmd).is_none(),
                "用户路径误判系统销毁: {cmd}"
            );
        }
    }

    /// SEC-10: the system directories themselves and OS-critical trees stay Critical.
    #[test]
    fn rm_rf_system_targets_stay_critical() {
        for cmd in [
            "rm -rf /",
            "rm -rf /*",
            "rm -rf /etc",
            "rm -rf /etc/",
            "rm -rf /etc/nginx",
            "rm -rf /usr",
            "rm -rf /usr/share/data",
            "rm -rf /home",
            "rm -rf /tmp",
            "rm -rf /var",
            "rm -rf ~",
            "rm -rf ~/",
            "rm -rf /",
            "Remove-Item -Recurse C:\\Windows\\Temp",
            "Remove-Item -Recurse ~",
        ] {
            let info = check_dangerous_command(cmd);
            assert!(
                info.is_some_and(|d| d.category == DangerCategory::Critical),
                "系统路径应保持 Critical: {cmd}"
            );
        }
    }

    /// Large-output commands must not deadlock: stdout/stderr must be read before
    /// wait(); reading only after wait() deadlocks once output exceeds the OS pipe
    /// buffer (~64KB) - the child blocks writing to the pipe, never exits, and gets
    /// killed by the timeout even if it would finish in 2 seconds. Readers start
    /// before wait and drain continuously.
    #[test]
    fn large_output_does_not_deadlock() {
        let root = std::env::temp_dir().to_string_lossy().to_string();
        // ~340KB output (Windows: 20000 lines x 17B; Unix: head -n 20000)
        let cmd = if cfg!(windows) {
            "for /L %i in (1,1,20000) do @echo 0123456789ABCDEF"
        } else {
            "yes 0123456789ABCDEF | head -n 20000"
        };
        let r = execute_command(cmd, &root, Some(30_000), false, None);
        let text = text_of(&r);
        assert!(!r.is_error, "大输出命令应正常完成: {text}");
        assert!(!text.contains("超时"), "不应被超时误杀: {text}");
        assert!(text.contains("0123456789ABCDEF"), "输出内容缺失");
    }

    // ====== forward_script_interpreter (harness round 2, issue 2) ======

    /// Commands that must pass through untouched: operators, absolute/quoted
    /// first tokens, unknown names, sub-commands after another command.
    #[test]
    fn forward_interpreter_passthrough_cases() {
        for cmd in [
            "cargo build",
            "cd tests && python run.py",
            "cmd /c python run.py",
            "\"python\" run.py",
            "python.exe run.py",
            "D:\\Anaconda3\\python.exe run.py",
            "./scripts/python.sh",
            "echo python",
            "",
            "   ",
        ] {
            let (rewritten, dir) = forward_script_interpreter(cmd);
            assert_eq!(rewritten, cmd, "不应改写: {cmd}");
            assert!(dir.is_none(), "不应产生授权目录: {cmd}");
        }
    }

    /// A bare known interpreter first token is rewritten to the quoted
    /// host-resolved absolute path, preserving the rest and leading whitespace.
    #[test]
    fn forward_interpreter_rewrites_bare_names() {
        // python is on the test machine's PATH (the AIO dev environment guarantees it)
        if crate::utils::exec_resolve::resolve_executable("python").is_none() {
            return; // PATH-less CI runner: nothing to forward, skip
        }
        let (rewritten, dir) = forward_script_interpreter("python tests/run.py");
        assert!(
            rewritten.starts_with('"'),
            "应改写为带引号绝对路径: {rewritten}"
        );
        assert!(
            rewritten.ends_with("tests/run.py"),
            "参数应保留: {rewritten}"
        );
        assert!(
            !rewritten.contains(" python"),
            "首 token 应被整体替换: {rewritten}"
        );
        assert!(dir.is_some(), "应返回解释器目录用于 R+X 授权");
        let (rewritten2, _) = forward_script_interpreter("  npm  install --no-audit");
        assert!(
            rewritten2.starts_with("  \""),
            "前导空白应保留: {rewritten2}"
        );
        assert!(
            rewritten2.ends_with(" install --no-audit"),
            "参数与中间空白应保留: {rewritten2}"
        );
    }

    /// Case-insensitive on the interpreter name (Windows shells are).
    #[test]
    fn forward_interpreter_matches_case_insensitive() {
        if crate::utils::exec_resolve::resolve_executable("python").is_none() {
            return;
        }
        let (rewritten, _) = forward_script_interpreter("PYTHON -V");
        assert!(rewritten.starts_with('"'), "大写名也应转发: {rewritten}");
    }

    /// E2E (harness round 2, issue 2): a bare `python` command must run inside
    /// the Windows AppContainer sandbox — forwarding rewrites it to the host-PATH
    /// absolute path and the best-effort R+X grant makes the interpreter tree
    /// (normally invisible: no ALL APPLICATION PACKAGES ACE) readable/executable.
    ///
    /// Ignored by default: on machines with path-level security filtering beyond
    /// the DACL (the dev box's EDR-layer denies exec/dir-list on its Anaconda
    /// install even WITH the R+X grant in place — reads pass, exec is refused),
    /// the assertion fails despite a correct grant. Rerun manually on a clean
    /// machine: `cargo test ac_python_forwarding -- --ignored`. Skipped silently
    /// when the host has no python on PATH.
    #[test]
    #[cfg(windows)]
    #[ignore = "interpreter exec is subject to machine-level path filtering; run manually on clean machines"]
    fn ac_python_forwarding_end_to_end() {
        if crate::utils::exec_resolve::resolve_executable("python").is_none() {
            return; // host has no python — nothing to forward
        }
        let project = std::env::temp_dir().join("aio-ac-test-python-fwd");
        let _ = std::fs::remove_dir_all(&project);
        std::fs::create_dir_all(&project).unwrap();
        let r = execute_command(
            "python -c \"print('aio_py_ok')\"",
            project.to_str().unwrap(),
            Some(30_000),
            false,
            Some(vec![]), // AppContainer enabled, empty toolchain list
        );
        let text = text_of(&r);
        assert!(
            !r.is_error && text.contains("aio_py_ok"),
            "AC 内 python 转发应成功: {text}"
        );
        let _ = std::fs::remove_dir_all(&project);
    }
}

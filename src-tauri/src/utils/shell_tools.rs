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
use crate::utils::exec_resolve::HiddenConsole;
use serde_json::json;
use std::process::Command;
use std::time::Duration;

// ====== Constants ======

const DEFAULT_TIMEOUT_MS: u64 = 60_000;
const MAX_TIMEOUT_MS: u64 = 300_000;
/// Output budget for one command stream: the drain layer keeps the head AND the
/// tail (compilers, test runners, and failure stack traces put the signal at the
/// END; a head-only cap hid `cargo test` failure details for outputs past 50KB —
/// harness feedback round 1, P1-1). Total per stream stays 50KB.
const MAX_OUTPUT_HEAD_BYTES: usize = 30_000;
const MAX_OUTPUT_TAIL_BYTES: usize = 20_000;
const MAX_COMMAND_LEN: usize = 4096;

// ====== Helpers ======

// tool_ok / tool_err come from utils::tool_result (single source of truth repo-wide)
pub use crate::utils::tool_result::{tool_err, tool_ok};

// Truncation is provided by utils::truncate (name matches semantics)
use crate::core::tool_names;

// ====== Console output decoding (harness round 3, hidden P0) =====
//
// cmd.exe on zh-CN Windows emits localized messages in GBK (code page 936); the
// previous lossy-UTF-8 conversion rendered "拒绝访问。" as mojibake, which
// silently KILLED every text-signature feature built on command output:
// sandbox-expansion detection (is_sandbox_access_denied), not-found detection,
// and env-class failure classification. Decode UTF-8-first (most modern tools
// speak UTF-8), falling back to the system ANSI code page.

/// Converts raw command output bytes to text: strict UTF-8 first, then the
/// Windows ANSI code page (CP_ACP → GBK on zh-CN), then lossy UTF-8 as the
/// last resort. Streams that carry PowerShell's CLIXML serialization marker
/// (`#< CLIXML`, emitted when a redirected pwsh serializes error records —
/// Windows PowerShell 5.1 ignores `-OutputFormat Text`) get a mechanical
/// CLIXML→text pass. Pure given the injected decoder — unit-tested.
pub(crate) fn decode_console_output(bytes: &[u8]) -> String {
    let text = if std::str::from_utf8(bytes).is_ok() {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        match decode_ansi_codepage(bytes) {
            Some(s) => s,
            None => String::from_utf8_lossy(bytes).into_owned(),
        }
    };
    if text.trim_start().starts_with("#< CLIXML") {
        decode_clixml(&text)
    } else {
        text
    }
}

/// Mechanical CLIXML → readable text (harness feedback round 2, P0-3): PowerShell
/// serializes error records as `<Objs><S S="Error">…</S>…</Objs>` when stderr is
/// redirected and `-OutputFormat Text` is unavailable (Windows PowerShell 5.1
/// ignores the flag). The wire form is unreadable (`_x001B_[31;1m…_x000D__x000A_`)
/// and breaks every text-signature detector. Decoding is mechanical:
/// record boundaries `</S>` → newlines, strip tags, unescape `_xHHHH_` entities,
/// strip ANSI color sequences, trim empty lines.
fn decode_clixml(text: &str) -> String {
    use std::sync::LazyLock;
    static ENTITY: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"_x([0-9A-Fa-f]{4})_").expect("固定 CLIXML 实体正则必然编译")
    });
    static TAG: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"<[^>]*>").expect("固定标签正则必然编译"));
    static ANSI: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"\x1b\[[0-9;]*[A-Za-z]").expect("固定 ANSI 正则必然编译")
    });
    // Each <S …> node is one record; drop the "#< CLIXML" preamble line along the way.
    let newline_separated = text.replace("</S>", "\n");
    let stripped = TAG.replace_all(&newline_separated, "");
    let unescaped = ENTITY.replace_all(&stripped, |caps: &regex::Captures| {
        let code = u32::from_str_radix(&caps[1], 16).unwrap_or(0xFFFD);
        char::from_u32(code).unwrap_or('\u{FFFD}').to_string()
    });
    let no_ansi = ANSI.replace_all(&unescaped, "");
    no_ansi
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// ANSI-code-page decode via kernel32 `MultiByteToWideChar(CP_ACP)` (Windows
/// only). Returns None on conversion failure so the caller can fall back.
#[cfg(windows)]
fn decode_ansi_codepage(bytes: &[u8]) -> Option<String> {
    const CP_ACP: u32 = 0;
    extern "system" {
        fn MultiByteToWideChar(
            code_page: u32,
            dw_flags: u32,
            lp_multi_byte_str: *const u8,
            cb_multi_byte: i32,
            lp_wide_char_str: *mut u16,
            cch_wide_char: i32,
        ) -> i32;
    }
    if bytes.is_empty() {
        return Some(String::new());
    }
    let len = bytes.len().min(i32::MAX as usize) as i32;
    let wide_len =
        unsafe { MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), len, std::ptr::null_mut(), 0) };
    if wide_len <= 0 {
        return None;
    }
    let mut wide = vec![0u16; wide_len as usize];
    let written =
        unsafe { MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), len, wide.as_mut_ptr(), wide_len) };
    if written <= 0 {
        return None;
    }
    String::from_utf16(&wide[..written as usize]).ok()
}

#[cfg(not(windows))]
fn decode_ansi_codepage(_bytes: &[u8]) -> Option<String> {
    None // no ANSI code page outside Windows; lossy UTF-8 is the fallback
}

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
            description: "在项目目录中执行系统命令，返回 stdout/stderr 和退出码。沙箱内默认**无网络**（Linux/macOS OS 级隔离）：命令需要联网（下载依赖、API 调用、访问 localhost 服务）时必须显式设 network=true（审批时向用户标注）。用于运行构建、测试、git 命令等。注意（Windows）：沙箱的 Job UI 限制使窗口枚举/查找 API（EnumWindows/FindWindow）在本命令的子进程内看不到任何桌面对象——窗口探测恒为空是沙箱所致而非目标无窗口；需要观测/验证 GUI 窗口行为的任务应在本机全权模式下运行（无沙箱）。".into(),
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

/// Executable names eligible for host-PATH forwarding. Harness findings:
/// round 2 — the sandboxed shell sees a stripped environment (`python`/`node`
/// probes exit 9009) while the host AIO process has them on PATH; round 3 —
/// the same holds for `git` (report: shell has no git either). `git`/`cargo`
/// installs usually live under Program Files, whose default ALL APPLICATION
/// PACKAGES ACE makes the forwarded absolute path genuinely executable inside
/// the AppContainer (unlike user-dir interpreters, which the expansion
/// dialog then mediates).
const FORWARDED_INTERPRETERS: &[&str] = &[
    "python", "python3", "py", "pip", "node", "npm", "npx", "git", "cargo",
];

/// Whether a command result text carries the Windows sandbox (AppContainer)
/// access-denied signature: WinError 5 surfaces as "Access is denied" (en) or
/// "拒绝访问" (zh) from cmd, PowerShell (UnauthorizedAccessException maps here),
/// and Python ([WinError 5]). Harness round 3: this signature triggers the
/// sandbox-expansion request (one-click grant / unsandboxed-once / ignore).
/// Depends on [`decode_console_output`] — GBK mojibake previously made the
/// Chinese half of this matcher dead on zh-CN systems.
pub(crate) fn is_sandbox_access_denied(result_text: &str) -> bool {
    result_text.contains("Access is denied")
        || result_text.contains("拒绝访问")
        || result_text.contains("Access to the path is denied")
}

/// Whether a command result text says the command was NOT FOUND (as opposed to
/// found-and-failed): cmd's "'x' 不是内部或外部命令" / "'x' is not recognized",
/// plus the 9009 exit marker. Environment-class failure signature (harness
/// round 3: the breaker must distinguish env-blocked from business failures).
pub(crate) fn is_command_not_found_result(result_text: &str) -> bool {
    result_text.contains("不是内部或外部命令")
        || result_text.contains("is not recognized")
        || result_text.contains("无法将")
        || result_text.contains("exit code: 9009")
}

/// Whether a command result text is an ENVIRONMENT-class failure (missing
/// command / missing dependency / permission denied) rather than a business
/// failure (command ran and returned nonzero). Used by the tool failure
/// breaker: only env-class consecutive failures trip the disable.
pub(crate) fn is_env_blocked_failure(result_text: &str) -> bool {
    is_sandbox_access_denied(result_text)
        || is_command_not_found_result(result_text)
        || result_text.contains("系统找不到指定的文件")
        || result_text.contains("The system cannot find the file specified")
        || result_text.contains("ModuleNotFoundError")
        || result_text.contains("command not found")
}

/// Splits the command into whitespace tokens, returning (byte offset of the
/// operation token, token text). The operation token is the first token after
/// unwrapping `cmd`/`cmd.exe` + its `/x` flags — i.e. what actually runs:
/// `cmd /c python -V` → `python`; `cmd /c del x` → `del`; `cargo build` → `cargo`.
/// Bails (None) when a quote appears before the operation token (conservative).
fn operation_token_span(command: &str) -> Option<(usize, &str)> {
    let trimmed = command.trim_start();
    let offset = command.len() - trimmed.len();
    let mut pos = 0usize;
    let mut first = true;
    let mut in_cmd_header = false;
    for tok in trimmed.split_whitespace() {
        let tok_start = trimmed[pos..].find(tok).unwrap_or(0) + pos;
        let lower = tok.to_ascii_lowercase();
        if first {
            first = false;
            if lower == "cmd" || lower == "cmd.exe" {
                in_cmd_header = true;
                pos = tok_start + tok.len();
                continue;
            }
        }
        if in_cmd_header && tok.starts_with('/') {
            pos = tok_start + tok.len();
            continue; // cmd flags: /c /k /s ...
        }
        if tok.contains('"') {
            return None; // quoted payload — do not guess inside quotes
        }
        return Some((offset + tok_start, tok));
    }
    None
}

/// The forwarded-executable name this command targets, if any: `python -V` →
/// "python", `cmd /c npm install` → "npm". None for non-forwarded operations
/// (`cmd /c del x` → None; `del` is not in the list). Quoted/absolute/relative
/// operation tokens are never forwarded.
pub(crate) fn forwarded_interpreter_of(command: &str) -> Option<String> {
    let (_, tok) = operation_token_span(command)?;
    let lower = tok.to_ascii_lowercase();
    let base = lower.strip_suffix(".exe").unwrap_or(&lower);
    let bare = !base.is_empty()
        && !base.contains(['/', '\\', ':'])
        && !base.contains('"')
        && FORWARDED_INTERPRETERS.contains(&base);
    bare.then(|| base.to_string())
}

/// Dedup key for the sandbox-expansion ask-once window: the OPERATION the
/// command performs (post cmd-unwrap), not the raw first token — `cmd /c del x`
/// and `del x` share the key "del"; `cmd /c python -V` and `python -V` share
/// "python"; unrelated cmd invocations do not collide.
// Windows-only at runtime (only the AppContainer expansion consumes it); the
// unit test keeps running on the Windows CI runner.
#[cfg(windows)]
pub(crate) fn expansion_key(command: &str) -> String {
    operation_token_span(command)
        .map(|(_, tok)| tok.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Rewrites a command whose operation token (first token, or the token after
/// `cmd /c`-style flags) is a bare known-executable name into the host-process-
/// resolved absolute path, returning the rewritten command plus the executable's
/// directory (for best-effort R+X granting inside the AppContainer sandbox —
/// the tree is normally invisible to the AC token, e.g. a D:\Anaconda3 without
/// an ALL APPLICATION PACKAGES ACE).
///
/// Fires only on a literally bare operation token (no quotes, path separators,
/// or drive colons): `python tests/x.py`, `node -e "..."`, `npm install`, and —
/// harness round 3, the gap the LLM testers actually hit — `cmd /c python -V`.
/// Anything else (`cd x && python y`, absolute paths) passes through untouched.
/// Resolution is PATH-only via [`crate::utils::exec_resolve`] (SEC-06: the
/// project cwd is never consulted, so a planted interpreter is not picked up).
/// When resolution fails the command passes through unchanged. Pure — unit-tested.
pub(crate) fn forward_script_interpreter(command: &str) -> (String, Option<std::path::PathBuf>) {
    let Some((tok_start, tok)) = operation_token_span(command) else {
        return (command.to_string(), None);
    };
    if forwarded_interpreter_of(command).is_none() {
        return (command.to_string(), None);
    }
    let Some(resolved) = crate::utils::exec_resolve::resolve_executable(tok) else {
        return (command.to_string(), None);
    };
    let mut rewritten = String::with_capacity(command.len() + 16);
    rewritten.push_str(&command[..tok_start]);
    rewritten.push('"');
    rewritten.push_str(&resolved.display().to_string());
    rewritten.push('"');
    rewritten.push_str(&command[tok_start + tok.len()..]);
    (rewritten, resolved.parent().map(|p| p.to_path_buf()))
}

/// Checks the command for dangerous shell metacharacter sequences.
///
/// Blanket-block on purpose (harness feedback round 2, P1-1 asked to relax `$()`
/// for read-only commands — declined: `$()` is a genuine command-substitution
/// vector regardless of the statement's read/write intent, and the scan runs
/// BEFORE any approval/sandbox trust decision, so it cannot rely on that intent).
/// The mitigation is the message itself: teach the equivalent PowerShell
/// rewrites so the caller does not burn turns rediscovering them.
fn check_shell_metacharacters(command: &str) -> Option<&'static str> {
    // 1. backtick (PowerShell escape / line-continuation — an obfuscation vector;
    //    not bash command substitution, but the blanket-block rationale is the same)
    if command.contains('`') {
        return Some(
            "反引号已被拦截（PowerShell 转义/续行字符，可用于绕过扫描）。改写方式：长命令拆为多语句（';' 或管道）；多行脚本写入临时 .ps1 后以 pwsh -File 执行",
        );
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
                return Some(
                    "$() 子表达式已被拦截（具命令替换语义，安全扫描一刀切）。改写方式：取字段用管道 Select-Object -ExpandProperty；拼接用属性访问 + '+'；复杂内插写入临时 .ps1 后以 pwsh -File 执行",
                );
            }
            _ => {}
        }
    }

    // 3. `> /dev/` device-file redirection
    // already covered by check_dangerous_command; not rechecked here

    None
}

/// One drained pipe stream: the kept head and tail byte slices plus the total
/// bytes read. `gap()` tells whether a middle section was dropped.
#[derive(Default)]
pub(crate) struct DrainedOutput {
    pub(crate) head: Vec<u8>,
    pub(crate) tail: Vec<u8>,
    pub(crate) total: usize,
}

impl DrainedOutput {
    /// Bytes kept in total (head + tail).
    fn kept(&self) -> usize {
        self.head.len() + self.tail.len()
    }
    /// True when a middle section of the stream was discarded while draining.
    fn gap(&self) -> bool {
        self.total > self.kept()
    }
    /// The full kept stream as one contiguous buffer (only meaningful when the
    /// stream fit within head+tail — no gap — so no seam splits a code point).
    fn joined(&self) -> Vec<u8> {
        let mut all = Vec::with_capacity(self.kept());
        all.extend_from_slice(&self.head);
        all.extend_from_slice(&self.tail);
        all
    }
}

/// Reads a pipe into bounded head+tail buffers, **keeps draining** (discarding
/// the middle) and counts total bytes read. Continuous draining guarantees the
/// producer never blocks writing to the pipe (paired with reader threads
/// started before wait(), this avoids the deadlock when output exceeds the
/// ~64KB OS pipe buffer).
///
/// Tail semantics: the LAST `tail_cap` bytes are retained via a draining ring —
/// compiler/test-runner failures land at the end of the stream, so the tail
/// carries the actionable lines (harness feedback round 1, P1-1: a head-only
/// cap repeatedly hid `cargo test` failure details behind "前 50000B / 共…").
fn drain_pipe_capped<R: std::io::Read>(
    mut pipe: R,
    head_cap: usize,
    tail_cap: usize,
) -> DrainedOutput {
    let mut out = DrainedOutput::default();
    let mut tail: std::collections::VecDeque<u8> = std::collections::VecDeque::new();
    let mut buf = [0u8; 16384];
    loop {
        match pipe.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                out.total += n;
                if out.head.len() < head_cap {
                    let take = (head_cap - out.head.len()).min(n);
                    out.head.extend_from_slice(&buf[..take]);
                    if take < n {
                        tail.extend(buf[take..n].iter().copied());
                    }
                } else {
                    tail.extend(buf[..n].iter().copied());
                }
                // Trim the ring to the tail cap (amortized O(total)).
                if tail.len() > tail_cap {
                    tail.drain(0..tail.len() - tail_cap);
                }
            }
        }
    }
    out.tail = tail.into();
    out
}

/// Windows shell selection (config `shellWindows`): `"powershell"` (default —
/// pwsh 7 with a `powershell.exe` 5.1 fallback) or `"cmd"`.
/// `Cmd` stays constructible on all platforms (the non-Windows execute path
/// passes it as a placeholder argument); `PowerShell` is Windows-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsShell {
    Cmd,
    #[cfg(windows)]
    PowerShell,
}

/// Parses the config value; unknown values fall back to PowerShell (the default).
#[cfg(windows)]
pub fn parse_windows_shell(value: &str) -> WindowsShell {
    match value {
        "cmd" => WindowsShell::Cmd,
        _ => WindowsShell::PowerShell,
    }
}

/// Resolved shell executable.
#[cfg(windows)]
struct ShellInvocation {
    program: String,
    is_powershell: bool,
}

/// Resolves the Windows shell binary: cmd via ComSpec/PATH; PowerShell prefers
/// pwsh 7 (App Execution Alias on PATH) and falls back to `powershell.exe` 5.1
/// (System32, always present).
#[cfg(windows)]
fn resolve_windows_shell(kind: WindowsShell) -> Result<ShellInvocation, String> {
    match kind {
        WindowsShell::Cmd => {
            let program = std::env::var("ComSpec")
                .ok()
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    crate::utils::exec_resolve::resolve_executable("cmd")
                        .map(|p| p.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "cmd.exe".to_string());
            Ok(ShellInvocation {
                program,
                is_powershell: false,
            })
        }
        WindowsShell::PowerShell => {
            if let Some(p) = crate::utils::exec_resolve::resolve_executable("pwsh") {
                return Ok(ShellInvocation {
                    program: p.to_string_lossy().into_owned(),
                    is_powershell: true,
                });
            }
            if let Some(p) = crate::utils::exec_resolve::resolve_executable("powershell") {
                return Ok(ShellInvocation {
                    program: p.to_string_lossy().into_owned(),
                    is_powershell: true,
                });
            }
            Err(
                "未找到 PowerShell（pwsh / powershell 均不可解析）/ PowerShell not found"
                    .to_string(),
            )
        }
    }
}

/// PowerShell prelude that keeps every `Start-Process` child off-screen.
///
/// AIO runs under the Windows GUI subsystem, so its own process owns no console:
/// a grandchild created by `Start-Process` (the classic way to launch a dev
/// server: `Start-Process npm.cmd -ArgumentList run,dev -PassThru`) is created by
/// powershell.exe with default flags and therefore allocates a brand-new visible
/// console. `CREATE_NO_WINDOW` on AIO's direct child cannot reach it — that is
/// exactly the "frontend and backend each pop a terminal window" defect.
///
/// The shim shadows the cmdlet for this shell session and defaults it to
/// `-WindowStyle Hidden` unless the caller already decided (explicit
/// `-WindowStyle` or `-NoNewWindow`). Forwarding uses **hash splatting of
/// `$PSBoundParameters`**, which is why the full common parameter set is
/// declared: array splatting (`@args`) binds entries POSITIONALLY, so appending
/// `'-WindowStyle' 'Hidden'` would be taken as a positional value and fail with
/// "no positional parameter accepts -WindowStyle" — every Start-Process call
/// would then silently do nothing. `start` and `saps` are aliases of the cmdlet
/// and also resolve to this function (verified on PowerShell 5.1 and 7).
///
/// `$Verb`/`-Verb RunAs` still works (the elevation prompt is a separate,
/// user-initiated consent UI, not an unrequested console).
///
/// Review round 1 additions: `[CmdletBinding(SupportsShouldProcess)]` (the real
/// cmdlet accepts `-WhatIf`/`-Confirm`; without the declaration those fail
/// binding on the shim) and `[hashtable]$Environment` (pwsh 7.4+; callers on
/// 5.1 never pass it, so declaring it is harmless there). A missing parameter
/// declaration is not cosmetic: PowerShell binding rejects unknown parameters,
/// so every call using it would hard-fail.
#[cfg(windows)]
const START_PROCESS_HIDDEN_SHIM: &str = concat!(
    "function global:Start-Process {",
    "[CmdletBinding(SupportsShouldProcess)]",
    "param(",
    "[Parameter(Position=0)][string]$FilePath,",
    "[Parameter(Position=1)][string[]]$ArgumentList,",
    "[pscredential]$Credential,",
    "[hashtable]$Environment,",
    "[switch]$LoadUserProfile,",
    "[switch]$NoNewWindow,",
    "[switch]$PassThru,",
    "[string]$RedirectStandardError,",
    "[string]$RedirectStandardInput,",
    "[string]$RedirectStandardOutput,",
    "[switch]$UseNewEnvironment,",
    "[string]$Verb,",
    "[switch]$Wait,",
    "[System.Diagnostics.ProcessWindowStyle]$WindowStyle,",
    "[string]$WorkingDirectory)",
    ";",
    "if (-not $PSBoundParameters.ContainsKey('WindowStyle') -and -not $NoNewWindow) {",
    "$PSBoundParameters['WindowStyle'] = [System.Diagnostics.ProcessWindowStyle]::Hidden",
    "};",
    "Microsoft.PowerShell.Management\\Start-Process @PSBoundParameters",
    "};"
);

/// Builds the pwsh script for one command: a leading quoted string needs the `&`
/// call operator (pwsh treats a bare quoted string as output, not invocation), and
/// the trailing lines propagate the exit code — `pwsh -Command` otherwise always
/// exits 0, which would break error detection / the breaker / sandbox expansion.
///
/// The script is `UTF-8 prelude` → [`START_PROCESS_HIDDEN_SHIM`] → body → exit
/// propagation. The shim runs before the body so a `Start-Process` in the very
/// first pipeline stage is already covered.
///
/// Exit-tail shape (harness feedback round 2, P0-1/P0-2): the body runs inside a
/// scriptblock piped through `Out-Default`, and `$?` is captured INSIDE the block
/// (the pipe resets it outside). Two defects of the historic tail
/// (`if ($null -ne $LASTEXITCODE) { exit $LASTEXITCODE } elseif (-not $?) { exit 1 }`)
/// both fix here:
///
/// 1. **Formatter output lost on `exit`**: PowerShell's default formatter for
///    object output (`Get-ChildItem | Select Name`, `Get-Process`, …) commits
///    lazily at runspace close; a script-level `exit` skips that flush, so the
///    stdout was literally 2 bytes (one newline) whenever `$LASTEXITCODE` had
///    been set by ANY earlier native command — "ran git, then printed a table"
///    dropped the table 100% of the time. `Out-Default` renders the objects
///    eagerly inside the pipeline, before any exit runs.
/// 2. **`$LASTEXITCODE=0` masked later cmdlet errors**: the first branch was
///    already true with value 0, so a PowerShell-level error AFTER a successful
///    native command exited 0 — every cmdlet failure became "success" for
///    `tool_ok` and the failure-signature detectors. The new tail only exits on
///    a NONZERO native code, falling through to the captured `$?` otherwise.
///
/// Verified by probe on pwsh 7.6 and 5.1: pure-table (rc0+bytes), git-then-table
/// (rc0+bytes), cmdlet-error-after-git (rc1), `cmd /c exit 7` + table (rc7 + bytes
/// preserved), pure-string (rc0).
#[cfg(windows)]
fn pwsh_script(command: &str) -> String {
    let body = if command.trim_start().starts_with('"') {
        format!("& {command}")
    } else {
        command.to_string()
    };
    // UTF-8 console prelude (harness environment report item 3): zh-CN Windows
    // defaults the console to code page 936, and PowerShell encodes its own
    // output — as well as what it relays from native children — through
    // [Console]::OutputEncoding. GBK system messages therefore mixed with
    // UTF-8 (node/cargo output) into one stream, which failed UTF-8
    // validation and reached us decoded wholly as GBK mojibake. Forcing UTF-8
    // makes the whole stream consistent; decode_console_output keeps its GBK
    // fallback for anything emitted by raw GBK writers (direct git spawns).
    //
    // Best-effort (try/catch) so an exotic console-less host still runs.
    let prelude = "try { [Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false); $OutputEncoding=[Console]::OutputEncoding } catch { }";
    format!(
        "{prelude}\n{START_PROCESS_HIDDEN_SHIM}\n$global:__aio_ok = $true\n& {{\n{body}\n$global:__aio_ok = $?\n}} | Out-Default\nif ($null -ne $LASTEXITCODE -and $LASTEXITCODE -ne 0) {{ exit $LASTEXITCODE }} elseif (-not $global:__aio_ok) {{ exit 1 }}"
    )
}

/// Rewrites the cmd builtin `start` to `start /b` so the launched program shares
/// the shell's (hidden) console instead of allocating a brand-new visible one.
///
/// `start` is the cmd-native way to launch a long-running program (a dev server)
/// without blocking the tool call, and it defaults to CREATE_NEW_CONSOLE: under
/// the GUI-subsystem build — where AIO's own `CREATE_NO_WINDOW` shell owns no
/// console at all — that window is immediately visible and stays on screen for
/// the whole session. `/b` starts the program in the same console, so there is
/// nothing to show; the child still inherits the stdout/stderr pipes, so the
/// tool call keeps collecting its output.
///
/// Only a COMMAND-INITIAL `start` is touched: line start, or right after
/// `&` `&&` `||` `|` `(`. `net start mysql`, `npm start`, `sc start x` and
/// `echo start` therefore pass through untouched (they are arguments of another
/// program, not the builtin). A caller-chosen window mode (/b, /min, /max,
/// /wait, /i) is preserved as-is — /min is already invisible enough and must not
/// be silently overridden, and the function is idempotent (a second pass adds
/// nothing). Pure — unit-tested.
#[cfg(windows)]
pub(crate) fn rewrite_cmd_start_hidden(command: &str) -> String {
    use std::sync::OnceLock;
    // `start` at a command-initial position, capturing the separating whitespace
    // so the replacement can put `/b` in exactly the right place.
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    let re = PATTERN.get_or_init(|| {
        // The trailing separator is CAPTURED rather than asserted with `(?=...)`:
        // the regex crate has no look-around, and an `Err` here would silently
        // disable the whole rewrite.
        regex::Regex::new(r"(?i)(^|&&|\|\||[&|()\r\n])(\s*)start(\s|$)")
            .expect("cmd start pattern is a literal and always compiles")
    });
    let mut out = String::with_capacity(command.len() + 8);
    let mut last = 0usize;
    for caps in re.captures_iter(command) {
        let whole = caps.get(0).expect("group 0 always present");
        if whole.start() < last {
            continue; // overlapping match from a previous replacement
        }
        // Up to and including the `start` token, leaving the trailing separator
        // out so `/b` can be inserted before it.
        let trail = caps.get(3).expect("group 3 always present");
        out.push_str(&command[last..trail.start()]);
        // The next token decides: if the caller already picked a window mode,
        // leave the command exactly as written.
        let rest = &command[whole.end()..];
        // `split_whitespace` already skips leading whitespace.
        let already_chosen = rest.split_whitespace().next().is_some_and(|tok| {
            let t = tok.to_ascii_lowercase();
            t == "/b" || t == "/min" || t == "/max" || t == "/wait" || t == "/i"
        });
        if !already_chosen {
            out.push_str(" /b");
        }
        out.push_str(trail.as_str());
        last = whole.end();
    }
    out.push_str(&command[last..]);
    out
}

/// cmd path: flip the hidden console to UTF-8 before the user command. cmd's
/// own messages and console-API-aware children (git, ver, echo) then emit
/// UTF-8 on the pipes instead of GBK, keeping mixed streams UTF-8-consistent.
/// The `&` sequence leaves errorlevel handling to the user command; `>nul`
/// keeps "Active code page: 65001" out of the output. Added AFTER the
/// metacharacter/danger scans run, so it never trips them.
#[cfg(windows)]
fn cmd_utf8_prelude(command: &str) -> String {
    // `start /b` first: the rewrite preserves any window mode the caller
    // already chose, and running it on the user's own command text keeps the
    // anchors (`&`, `&&`, line start) meaningful — prefixing with `chcp` would
    // otherwise turn the first `start` into a non-command-initial token.
    format!("chcp 65001 >nul & {}", rewrite_cmd_start_hidden(command))
}

/// `-EncodedCommand` payload: UTF-16LE base64 — immune to every quote-escaping
/// problem on both the argv and raw-command-line invocation paths.
#[cfg(windows)]
fn pwsh_encoded(script: &str) -> String {
    use base64::Engine as _;
    let utf16: Vec<u16> = script.encode_utf16().collect();
    let bytes: Vec<u8> = utf16.iter().flat_map(|w| w.to_le_bytes()).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
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
/// - `protect_git_ro` - Codex-style `<root>/.git` deny-write inside whichever sandbox
///   runs (Windows AppContainer deny ACE / Linux ro-bind / macOS SBPL deny); the
///   builtin git_* tools run in the host process and are unaffected
/// - `shell` - Windows shell selection (`shellWindows` config: powershell default / cmd)
///
/// - `cancel` — user-stop token (agent loop). The wait loop polls it in 500ms slices;
///   on cancellation the process tree is torn down exactly like the timeout path and
///   the call returns a "stopped by user" ToolResult. `None` = uncancellable (tests,
///   non-agent callers).
///
/// # Returns
/// A ToolResult carrying stdout, stderr, exit_code and timed_out.
#[allow(
    clippy::too_many_arguments,
    reason = "explicit execution context (sandbox, shell kind, cancellation token)"
)]
pub fn execute_command(
    command: &str,
    project_root: &str,
    timeout_ms: Option<u64>,
    allow_network: bool,
    ac_dirs: Option<Vec<String>>,
    protect_git_ro: bool,
    shell: WindowsShell,
    cancel: Option<tokio_util::sync::CancellationToken>,
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
                              // Windows-only sandbox inputs have no role on Unix (ac_dirs = AppContainer
                              // dir list, shell = Windows shell selection): consume them so the
                              // cross-platform signature stays unchanged, mirroring interpreter_dir above.
    #[cfg(not(windows))]
    let _ = (ac_dirs, shell);

    // dangerous shell metacharacter check
    if let Some(reason) = check_shell_metacharacters(command) {
        return tool_err(&format!("安全拦截: {reason}"));
    }

    // Dangerous-command detection: Critical findings (system destruction / privilege
    // escalation / fork bomb) are **rejected in every agent mode** - in Auto
    // (完全权限) mode the execute_command permission is Allow and skips the approval
    // prompt, so without this interception there would be no protection at all.
    // High/Medium findings only log a warning (in Ask mode the approval prompt shows
    // the risk description to the user).
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
            protect_git_ro,
            shell,
            cancel,
        );
    }

    // Shell resolution per platform. Windows: config-selected (powershell default /
    // cmd); Unix: sh, wrapped by bwrap/SBPL when available.
    #[cfg(windows)]
    let (sandbox_argv, shell_program, shell_is_powershell) = {
        let inv = match resolve_windows_shell(shell) {
            Ok(i) => i,
            Err(e) => return tool_err(&e),
        };
        (None::<Vec<String>>, inv.program, inv.is_powershell)
    };
    #[cfg(not(windows))]
    let (sandbox_argv, shell_program, _shell_is_powershell) = {
        let Some(shell) = crate::utils::exec_resolve::resolve_executable("sh") else {
            return tool_err("无法定位系统 shell（PATH 中未找到 sh），拒绝执行");
        };
        let shell = shell.to_string_lossy().into_owned();
        let argv = crate::utils::sandbox::sandbox_command_prefix(
            &shell,
            "-c",
            command,
            project_root,
            allow_network,
            protect_git_ro,
        );
        let program = match &argv {
            Some(a) => a[0].clone(),
            None => shell,
        };
        (argv, program, false)
    };
    let mut cmd = if let Some(argv) = sandbox_argv {
        let mut c = Command::new(&argv[0]);
        c.args(&argv[1..]);
        c
    } else {
        let mut c = Command::new(&shell_program);
        #[cfg(windows)]
        {
            if shell_is_powershell {
                // pwsh：-EncodedCommand（UTF-16LE base64）免疫一切引号转义问题；
                // 脚本尾部传播退出码（pwsh -Command 默认退出 0，不做此层则所有
                // 失败命令都会被当成成功，错误检测/熔断/扩权全失效）。
                // -OutputFormat Text（harness 反馈第二轮 P0-3）：stderr 被重定向时
                // PowerShell 把错误记录序列化成 CLIXML（#< CLIXML <Objs><S …>），
                // 报错文本完全不可读且打断所有文本签名判定；pwsh 7 认这个参数并
                // 保持纯文本，5.1 静默忽略它（残留由 decode_console_output 的
                // CLIXML 解码兜底）。实测：加参数后 stderr 229B CLIXML → 52B 纯文本。
                c.args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-NoLogo",
                    "-OutputFormat",
                    "Text",
                    "-EncodedCommand",
                    &pwsh_encoded(&pwsh_script(command)),
                ]);
            } else {
                // cmd：/S /C "<命令>" 规范形态——/S 下只剥最外层包裹引号，内部
                // 引号（转发解释器的带引号路径 + 参数内引号）原样保留。
                use std::os::windows::process::CommandExt;
                c.raw_arg("/S")
                    .raw_arg("/C")
                    .raw_arg(format!("\"{}\"", cmd_utf8_prelude(command)));
            }
        }
        #[cfg(not(windows))]
        c.arg("-c").arg(command);
        c
    };
    cmd.current_dir(project_root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // GUI-subsystem release build owns no console: without this the shell child
        // allocates one and the user sees a terminal flash on every command.
        // SEC-8: the suspended variant — the child spawns frozen, joins its Job
        // Object below, and only then executes (assign-then-resume). Without the
        // suspension a fast-spawning child can put grandchildren outside the job
        // in the CreateProcess→AssignProcessToJobObject window, and those would
        // survive the timeout's whole-tree teardown.
        .hidden_console_suspended();
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

    // SEC-8: the child spawned frozen (hidden_console_suspended) and has now joined
    // the Job Object — let it execute. A failed resume would leave it frozen forever,
    // so that failure is fatal for this execution (kill + error, fail-closed).
    #[cfg(windows)]
    if let Err(e) = crate::utils::sandbox::resume_suspended_child(child.id()) {
        let _ = child.kill();
        let _ = child.wait();
        return tool_err(&format!(
            "恢复挂起子进程失败（fail-closed，命令未执行）: {e}"
        ));
    }

    // Reader threads must start before wait(): reading only after wait() deadlocks
    // once output exceeds the OS pipe buffer (~64KB) - the child blocks writing to
    // the pipe and never exits, then gets killed by the timeout even if the command
    // itself would finish in 2 seconds. Readers keep the head+tail budget
    // (MAX_OUTPUT_HEAD/TAIL_BYTES) and keep draining the middle so the child
    // never blocks on pipe writes.
    use std::sync::mpsc;
    let (stdout_tx, stdout_rx) = mpsc::channel();
    let (stderr_tx, stderr_rx) = mpsc::channel();
    if let Some(pipe) = child.stdout.take() {
        std::thread::spawn(move || {
            let _ = stdout_tx.send(drain_pipe_capped(
                pipe,
                MAX_OUTPUT_HEAD_BYTES,
                MAX_OUTPUT_TAIL_BYTES,
            ));
        });
    }
    if let Some(pipe) = child.stderr.take() {
        std::thread::spawn(move || {
            let _ = stderr_tx.send(drain_pipe_capped(
                pipe,
                MAX_OUTPUT_HEAD_BYTES,
                MAX_OUTPUT_TAIL_BYTES,
            ));
        });
    }

    // wait with timeout via a channel, avoiding a dependency on the unstable wait_timeout.
    // The wait is sliced (500ms) so user cancellation (stop button -> CancellationToken)
    // interrupts a running command mid-flight instead of waiting out its full timeout —
    // mainstream-parity behavior (Claude Code's Esc kills the running process tree).
    // Cancel teardown is identical to the timeout path.
    use std::sync::{Arc, Mutex};
    // unix-only: the process-group kill in the teardown path runs BEFORE the lock
    // (the waiter thread holds it until exit) — captured here so the kill needs no lock.
    #[cfg(unix)]
    let child_pid = child.id();
    let child_arc = Arc::new(Mutex::new(child));
    let child_clone = Arc::clone(&child_arc);

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut c = child_clone.lock().unwrap();
        let status = c.wait();
        let _ = tx.send(status);
    });

    const WAIT_SLICE: Duration = Duration::from_millis(500);
    let budget = Duration::from_millis(timeout);
    let wait_started = std::time::Instant::now();
    let mut exit_status: Option<std::process::ExitStatus> = None;
    let mut timed_out = false;
    let mut cancelled = false;
    loop {
        if cancel.as_ref().is_some_and(|t| t.is_cancelled()) {
            cancelled = true;
            break;
        }
        let elapsed = wait_started.elapsed();
        if elapsed >= budget {
            timed_out = true;
            break;
        }
        match rx.recv_timeout(WAIT_SLICE.min(budget - elapsed)) {
            Ok(Ok(status)) => {
                exit_status = Some(status);
                break;
            }
            Ok(Err(_)) => return tool_err("等待命令进程失败"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return tool_err("命令进程意外终止");
            }
        }
    }

    if timed_out || cancelled {
        // timeout/cancel -> tear down the whole process tree immediately:
        // - Windows: dropping the Job guard triggers KILL_ON_JOB_CLOSE (all grandchildren included)
        // - non-Windows: kill the process group (process_group(0) put the subtree in its own group).
        //   The group kill must precede the lock: the waiter thread holds it until the child exits,
        //   so acquiring it before killing would deadlock on Unix.
        // `let _ =` instead of drop(): the non-Windows JobGuard has no Drop impl
        // (drop_non_drop lint); the value still releases at the statement end.
        let _ = job_guard.take();
        #[cfg(unix)]
        {
            if child_pid > 0 {
                // PATH-only resolution (SEC-06): never spawn a bare `kill`
                if let Some(kill) = crate::utils::exec_resolve::resolve_executable("kill") {
                    let _ = std::process::Command::new(kill)
                        .arg("-KILL")
                        .arg(format!("-{child_pid}"))
                        .hidden_console()
                        .status();
                }
            }
        }
        if let Ok(mut c) = child_arc.lock() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
    if cancelled {
        return tool_err("命令已被用户停止");
    }
    let exit_code = exit_status.and_then(|s| s.code());

    // collect reader results: the process has exited or been torn down, so the pipes
    // close and the readers finish shortly; in the extreme case (a leftover process
    // still holds the pipe write end) give up after 10s to avoid unbounded waiting.
    let stdout = stdout_rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or_default();
    let stderr = stderr_rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or_default();

    build_command_result(
        command,
        project_root,
        exit_code,
        timed_out,
        timeout,
        stdout,
        stderr,
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
#[allow(
    clippy::too_many_arguments,
    reason = "explicit sandbox execution context (shell kind, git protection, grant lists)"
)]
fn execute_command_in_appcontainer(
    command: &str,
    project_root: &str,
    timeout: u64,
    allow_network: bool,
    toolchain_dirs: &[String],
    best_effort_rx_dirs: &[String],
    protect_git_ro: bool,
    shell: WindowsShell,
    cancel: Option<tokio_util::sync::CancellationToken>,
) -> ToolResult {
    // Shell resolution for the AppContainer: pwsh 7 的 Store App Execution Alias
    //（WindowsApps 下的 reparse point）在 AppContainer 内不可执行——解析结果落在
    // WindowsApps 下时回退 System32 的 powershell.exe 5.1（ALL APPLICATION PACKAGES
    // ACE 覆盖，恒可用）；pwsh 的 MSI 安装（Program Files）无此问题。
    let invocation = match resolve_windows_shell(shell) {
        Ok(i) => i,
        Err(e) => return tool_err(&e),
    };
    let program = if invocation.is_powershell
        && invocation
            .program
            .to_ascii_lowercase()
            .contains("windowsapps")
    {
        crate::utils::exec_resolve::resolve_executable("powershell")
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| {
                r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe".to_string()
            })
    } else {
        invocation.program
    };
    let cmdline = if invocation.is_powershell {
        // -EncodedCommand：UTF-16LE base64，免疫命令行引号转义问题
        format!(
            "\"{program}\" -NoProfile -NonInteractive -NoLogo -EncodedCommand {}",
            pwsh_encoded(&pwsh_script(command))
        )
    } else {
        // /S + 包裹引号：cmd 只剥最外层包裹引号，多引号命令不触发「剥首尾引号」规则
        format!("\"{program}\" /S /C \"{}\"", cmd_utf8_prelude(command))
    };
    let (mut child, job) = match crate::utils::sandbox::spawn_in_appcontainer(
        &cmdline,
        project_root,
        allow_network,
        toolchain_dirs,
        best_effort_rx_dirs,
        protect_git_ro,
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
            let _ = stdout_tx.send(drain_pipe_capped(
                pipe,
                MAX_OUTPUT_HEAD_BYTES,
                MAX_OUTPUT_TAIL_BYTES,
            ));
        });
    }
    if let Some(pipe) = child.stderr_read.take() {
        std::thread::spawn(move || {
            let _ = stderr_tx.send(drain_pipe_capped(
                pipe,
                MAX_OUTPUT_HEAD_BYTES,
                MAX_OUTPUT_TAIL_BYTES,
            ));
        });
    }

    // wait (blocking; this function runs on a spawn_blocking thread), sliced so user
    // cancellation interrupts a running command mid-flight. Timeout/cancel -> Job kills
    // the whole tree + terminate as backstop.
    let (exit_code, timed_out, cancelled) = {
        const WAIT_SLICE_MS: u64 = 500;
        let budget = timeout;
        let wait_started = std::time::Instant::now();
        let outcome: (Option<i32>, bool, bool) = loop {
            if cancel.as_ref().is_some_and(|t| t.is_cancelled()) {
                break (None, false, true);
            }
            let elapsed = wait_started.elapsed().as_millis() as u64;
            if elapsed >= budget {
                break (None, true, false);
            }
            let slice = (budget - elapsed).min(WAIT_SLICE_MS) as u32;
            let (code, to) = crate::utils::sandbox::wait_with_timeout(&child, slice);
            if !to {
                break (code, false, false);
            }
        };
        if outcome.1 || outcome.2 {
            drop(job.take());
            crate::utils::sandbox::terminate(&child);
        }
        outcome
    };
    if cancelled {
        return tool_err("命令已被用户停止");
    }

    let stdout = stdout_rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or_default();
    let stderr = stderr_rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or_default();
    // also clean up potentially leftover grandchildren (daemons etc.) on the normal path
    drop(job.take());

    build_command_result(
        command,
        project_root,
        exit_code,
        timed_out,
        timeout,
        stdout,
        stderr,
    )
}

/// Shared result construction for the native and AppContainer paths.
fn build_command_result(
    command: &str,
    project_root: &str,
    exit_code: Option<i32>,
    timed_out: bool,
    timeout: u64,
    stdout: DrainedOutput,
    stderr: DrainedOutput,
) -> ToolResult {
    // A command that could have created/removed a repository invalidates the memoized
    // `is_git_repo` answer, so the git tools appear (or disappear) on the next turn
    // instead of after the cache TTL. git init / clone / rm -rf .git must not appear
    // to have had no effect on the tool set.
    if crate::utils::git_tools::command_may_change_repo(command) {
        crate::utils::git_tools::invalidate_git_repo_cache(project_root);
    }
    let mut result = String::new();
    result.push_str(&format!("$ {command}\n"));

    if timed_out {
        result.push_str(&format!("\n⏱ 命令超时 (>{timeout}ms)，已强制终止\n"));
    }

    if let Some(code) = exit_code {
        result.push_str(&format!("\nexit code: {code}\n"));
    }

    // Decode BEFORE truncation: GBK/ANSI console messages must reach the
    // model (and the signature detectors) as readable text (harness round 3).
    // With no gap, head+tail joined first so a code point never splits at the
    // seam; with a gap, the marker line separates the two lossy decodes.
    let render_stream = |label: &str, s: &DrainedOutput, out: &mut String| {
        if s.head.is_empty() && s.total == 0 {
            return;
        }
        if s.gap() {
            let head = decode_console_output(&s.head);
            let tail = decode_console_output(&s.tail);
            out.push_str(&format!(
                "\n[{label}]:\n{}\n\n[!] …中段输出已丢弃（共 {}B，保留头 {}B + 尾 {}B）…\n\n{}\n",
                head,
                s.total,
                s.head.len(),
                s.tail.len(),
                tail
            ));
        } else {
            let body = decode_console_output(&s.joined());
            out.push_str(&format!("\n[{label}]:\n{body}\n"));
        }
    };
    render_stream("stdout", &stdout, &mut result);
    render_stream("stderr", &stderr, &mut result);

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
    /// protection gap of Auto (完全权限) mode (execute_command permission is
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
            let r = execute_command(
                cmd,
                &root,
                Some(5_000),
                false,
                None,
                false,
                WindowsShell::Cmd,
                None,
            );
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
        let r = execute_command(
            "echo aio_benign_check",
            &root,
            Some(10_000),
            false,
            None,
            false,
            WindowsShell::Cmd,
            None,
        );
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
        let r = execute_command(
            cmd,
            &root,
            Some(30_000),
            false,
            None,
            false,
            WindowsShell::Cmd,
            None,
        );
        let text = text_of(&r);
        assert!(!r.is_error, "大输出命令应正常完成: {text}");
        assert!(!text.contains("超时"), "不应被超时误杀: {text}");
        assert!(text.contains("0123456789ABCDEF"), "输出内容缺失");
    }

    /// Access-denied signature detection (harness round 3): WinError 5 renders as
    /// either locale string, from cmd, PowerShell, or Python tracebacks.
    #[test]
    fn sandbox_access_denied_signature() {
        for text in [
            "[Error] $ cmd\nexit code: 1\n[stderr]:\nAccess is denied.\n",
            "Remove-Item : 拒绝访问",
            "PermissionError: [WinError 5] 拒绝访问",
        ] {
            assert!(is_sandbox_access_denied(text), "应识别: {text}");
        }
        for text in [
            "exit code: 1\n[stderr]:\n不是内部或外部命令", // 9009: missing, not denied
            "cargo build finished",
            "",
            "错误: 文件不存在",
        ] {
            assert!(!is_sandbox_access_denied(text), "不应误报: {text}");
        }
    }

    // ====== forward_script_interpreter (harness round 2 + round 3) ======

    /// Commands that must pass through untouched: operators, absolute/quoted
    /// first tokens, unknown names, sub-commands after another command.
    /// (Round 3: `cmd /c python run.py` MOVED OUT of this list — it is now
    /// forwarded, which is exactly the gap the LLM testers hit; `cargo` and
    /// `python.exe` are also forwarded now, so they are rewrite cases.)
    #[test]
    fn forward_interpreter_passthrough_cases() {
        for cmd in [
            "cd tests && python run.py",
            "cmd /c del abc.txt", // del is not a forwarded executable
            "cmd /c nosuchcmd_r3_1",
            "\"python\" run.py",
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

    /// Harness round 3 (the actual tester form): `cmd /c <interp> …` unwraps
    /// the cmd header and its flags, then forwards the operation token. The
    /// `python.exe` suffix form forwards too.
    // Windows-resolution semantics (PATHEXT suffix probing of `python.exe`);
    // the Linux resolver matches literal names only, so this runs on Windows.
    #[test]
    #[cfg(windows)]
    fn forward_interpreter_rewrites_cmd_wrapped() {
        if crate::utils::exec_resolve::resolve_executable("python").is_none() {
            return;
        }
        let (rewritten, dir) = forward_script_interpreter("cmd /c python --version");
        assert!(
            rewritten.starts_with("cmd /c \"") && rewritten.ends_with("--version"),
            "应改写 cmd 包裹内的解释器: {rewritten}"
        );
        assert!(
            !rewritten.contains(" python "),
            "内部 token 应被整体替换: {rewritten}"
        );
        assert!(dir.is_some(), "cmd 包裹也应返回解释器目录");

        // Uppercase CMD.EXE header + multiple flags
        let (rewritten2, _) = forward_script_interpreter("CMD.EXE /C /F python -V");
        assert!(
            rewritten2.starts_with("CMD.EXE /C /F \""),
            "大小写与多 flag 都应解包: {rewritten2}"
        );

        // .exe-suffixed operation token
        let (rewritten3, dir3) = forward_script_interpreter("cmd /c python.exe tests/run.py");
        assert!(
            rewritten3.starts_with("cmd /c \"") && dir3.is_some(),
            "python.exe 形态也应转发: {rewritten3}"
        );
    }

    /// forwarded_interpreter_of: what the command targets, for expansion triggering.
    #[test]
    fn forwarded_interpreter_of_forms() {
        assert_eq!(
            forwarded_interpreter_of("cmd /c python --version").as_deref(),
            Some("python"),
            "cmd 包裹应识别"
        );
        assert_eq!(
            forwarded_interpreter_of("python -V").as_deref(),
            Some("python")
        );
        assert_eq!(
            forwarded_interpreter_of("PYTHON.EXE -V").as_deref(),
            Some("python"),
            "大小写与 .exe 后缀应归一"
        );
        assert_eq!(
            forwarded_interpreter_of("cmd /c npm install").as_deref(),
            Some("npm")
        );
        assert_eq!(
            forwarded_interpreter_of("git status").as_deref(),
            Some("git")
        );
        assert_eq!(
            forwarded_interpreter_of("cmd /c del abc.txt"),
            None,
            "del 不在转发列表"
        );
        assert_eq!(forwarded_interpreter_of("cd x && python y"), None);
        assert_eq!(forwarded_interpreter_of("\"python\" x"), None);
    }

    /// expansion_key: operation-level dedup for the sandbox-expansion window —
    /// `cmd /c X` and bare `X` share one key; unrelated operations do not.
    #[test]
    #[cfg(windows)]
    fn expansion_key_is_operation_level() {
        assert_eq!(expansion_key("cmd /c python --version"), "python");
        assert_eq!(expansion_key("python -V"), "python");
        assert_eq!(expansion_key("cmd /c del x"), "del");
        assert_eq!(expansion_key("del x"), "del");
        assert_eq!(expansion_key("  cmd /c ver"), "ver");
        assert_eq!(expansion_key(""), "");
    }

    /// Case-insensitive on the interpreter name (Windows shells are).
    // Windows-resolution semantics: PATHEXT probing is case-insensitive; the
    // Linux resolver is case-sensitive, so an uppercase PYTHON stays unresolved.
    #[test]
    #[cfg(windows)]
    fn forward_interpreter_matches_case_insensitive() {
        if crate::utils::exec_resolve::resolve_executable("python").is_none() {
            return;
        }
        let (rewritten, _) = forward_script_interpreter("PYTHON -V");
        assert!(rewritten.starts_with('"'), "大写名也应转发: {rewritten}");
    }

    // ====== console output decoding (harness round 3, hidden P0) ======

    /// UTF-8 output passes through untouched; invalid-UTF-8 GBK bytes decode to
    /// readable Chinese via the ANSI code page (zh-CN CP936) — the case that
    /// previously turned "拒绝访问。" into mojibake and killed signature matching.
    #[test]
    fn decode_console_output_utf8_and_gbk() {
        assert_eq!(decode_console_output(b"plain ascii"), "plain ascii");
        assert_eq!(decode_console_output("中文 utf-8".as_bytes()), "中文 utf-8");
        // GBK bytes of "拒绝访问。" (BE DC BE F8 B7 C3 CE CA A1 A3)
        let gbk_denied = [0xBEu8, 0xDC, 0xBE, 0xF8, 0xB7, 0xC3, 0xCE, 0xCA, 0xA1, 0xA3];
        // CP_ACP fallback is ACP-based BY DESIGN: the GBK decode (and the
        // signature matching built on it) targets zh-CN systems (ACP=936).
        // On other locales (en-US CI runner, ACP=1252) these bytes legitimately
        // take the lossy path — assert the full decode only when ACP is 936;
        // otherwise assert the no-panic contract (先运行后判定 same spirit as
        // the AppContainer tests: run it, decide by environment).
        #[cfg(windows)]
        {
            extern "system" {
                fn GetACP() -> u32;
            }
            let acp = unsafe { GetACP() };
            if acp == 936 {
                let decoded = decode_console_output(&gbk_denied);
                assert!(
                    decoded.contains("拒绝访问"),
                    "GBK 拒绝访问应正确解码（ACP=936 系统），got: {decoded:?}"
                );
                // The signature matcher must now see the real text
                assert!(is_sandbox_access_denied(&decoded), "解码后签名应可识别");
            } else {
                let _ = decode_console_output(&gbk_denied);
            }
        }
        // On non-Windows there is no ANSI fallback: lossy UTF-8, no panic
        let _ = decode_console_output(&gbk_denied);
    }

    /// Not-found + env-class signatures (harness round 3): the breaker's
    /// env/business distinction and the 9009-expansion trigger depend on these.
    #[test]
    fn not_found_and_env_class_signatures() {
        for text in [
            "'python' 不是内部或外部命令，也不是可运行的程序或批处理文件",
            "'x' is not recognized as an internal or external command",
            "exit code: 9009",
        ] {
            assert!(is_command_not_found_result(text), "应识别未找到: {text}");
            assert!(is_env_blocked_failure(text), "未找到属环境类: {text}");
        }
        for text in [
            "拒绝访问。",
            "Access is denied.",
            "ModuleNotFoundError: No module named 'numpy'",
            "/bin/sh: 1: git: command not found",
        ] {
            assert!(is_env_blocked_failure(text), "应识别环境类: {text}");
        }
        // Business failures: command RAN and failed — must NOT be env-class
        for text in [
            "exit code: 1\n[stderr]:\n1 test failed",
            "assert False: assertion failed",
            "error: could not compile `aio`",
        ] {
            assert!(
                !is_env_blocked_failure(text),
                "业务失败不应判为环境类: {text}"
            );
            assert!(!is_command_not_found_result(text));
        }
    }

    /// E2E decode verification (harness round 3 + environment-report fix): a real
    /// cmd.exe run whose hidden console starts at the zh-CN default (code page
    /// 936) must surface readable Chinese in the tool result. The UTF-8 prelude
    /// (`chcp 65001`) now flips the console before the user command, so this
    /// exercises the UTF-8 path end-to-end; the decoder's GBK fallback remains
    /// covered by `decode_console_output_utf8_and_gbk` (raw GBK writers like
    /// direct git spawns still exist).
    #[test]
    #[cfg(windows)]
    fn gbk_console_output_decodes_e2e() {
        let root = std::env::temp_dir().to_string_lossy().to_string();
        let r = execute_command(
            "cmd /c echo 拒绝访问测试OK",
            &root,
            Some(15_000),
            false,
            None,
            false,
            WindowsShell::Cmd,
            None,
        );
        let text = text_of(&r);
        assert!(
            !r.is_error && text.contains("拒绝访问测试OK"),
            "控制台输出应正确解码: {text}"
        );
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
            false,
            WindowsShell::Cmd,
            None,
        );
        let text = text_of(&r);
        assert!(
            !r.is_error && text.contains("aio_py_ok"),
            "AC 内 python 转发应成功: {text}"
        );
        let _ = std::fs::remove_dir_all(&project);
    }

    /// 本机全权/native 路径引号保真 e2e：整条命令文本必须**原样**到达 cmd——参数内
    /// 双引号不被剥、不转义。回归背景：native 路径曾用 arg() 把整条命令按 MSVCRT
    /// 规则包成单参数（含空格加引号、内部引号转义 \"），cmd 收到双重引号包裹的
    /// 文本 → 引号丢失、含空格参数被拆散（本机全权档实测）。
    #[test]
    #[cfg(windows)]
    fn native_path_preserves_quotes() {
        let root = std::env::temp_dir().to_string_lossy().to_string();
        let r = execute_command(
            "echo \"A B\" end",
            &root,
            Some(15_000),
            false,
            None,
            false,
            WindowsShell::Cmd,
            None,
        );
        let text = text_of(&r);
        assert!(
            !r.is_error && text.contains("\"A B\""),
            "引号应原样保留: {text}"
        );
    }

    /// native 路径的转发解释器 e2e：裸名 python 改写为绝对路径后经 raw 命令行执行，
    /// cmd 的 /C 引号规则应正确剥离路径引号（与沙箱路径同语义）。
    #[test]
    #[cfg(windows)]
    fn native_path_forwarded_interpreter_runs() {
        if crate::utils::exec_resolve::resolve_executable("python").is_none() {
            return; // host has no python on PATH
        }
        let root = std::env::temp_dir().to_string_lossy().to_string();
        let r = execute_command(
            "python -c \"print('aio_native_ok')\"",
            &root,
            Some(30_000),
            false,
            None,
            false,
            WindowsShell::Cmd,
            None,
        );
        let text = text_of(&r);
        assert!(
            !r.is_error && text.contains("aio_native_ok"),
            "native 转发解释器应成功: {text}"
        );
    }

    /// PowerShell 默认 shell e2e（native 路径）：基本执行 + 退出码传播——pwsh
    /// -Command 默认退出 0，脚本尾部的 `exit $LASTEXITCODE` 包装必须把 native
    /// 命令的失败码透传出来（错误检测/熔断/扩权都依赖它）。
    #[test]
    #[cfg(windows)]
    fn powershell_native_runs_and_propagates_exit_code() {
        let root = std::env::temp_dir().to_string_lossy().to_string();
        let ok = execute_command(
            "Write-Output 'aio_pwsh_ok'",
            &root,
            Some(30_000),
            false,
            None,
            false,
            WindowsShell::PowerShell,
            None,
        );
        let ok_text = text_of(&ok);
        assert!(
            !ok.is_error && ok_text.contains("aio_pwsh_ok"),
            "pwsh 基本执行应成功: {ok_text}"
        );
        let fail = execute_command(
            "cmd /c exit 7",
            &root,
            Some(30_000),
            false,
            None,
            false,
            WindowsShell::PowerShell,
            None,
        );
        assert!(
            fail.is_error,
            "退出码 7 应传播为错误 (exit={:?}, text={})",
            text_of(&fail),
            text_of(&fail)
        );
    }

    /// PowerShell 默认 shell e2e（AppContainer 路径）：pwsh 的 Store 别名在 AC 内
    /// 不可执行 → 自动回退 System32 的 powershell.exe 5.1；只读命令 + 引号参数
    /// 在沙箱内均应可用。
    #[test]
    #[cfg(windows)]
    fn appcontainer_powershell_runs() {
        let project = std::env::temp_dir().join("aio-ac-test-pwsh");
        let _ = std::fs::remove_dir_all(&project);
        std::fs::create_dir_all(&project).unwrap();
        let r = execute_command(
            "Write-Output ('ac_pwsh_' + (6*7))",
            project.to_str().unwrap(),
            Some(30_000),
            false,
            Some(vec![]), // AppContainer enabled
            false,
            WindowsShell::PowerShell,
            None,
        );
        let text = text_of(&r);
        assert!(
            !r.is_error && text.contains("ac_pwsh_42"),
            "AC 内 pwsh 应可用: {text}"
        );
        let _ = std::fs::remove_dir_all(&project);
    }

    /// 取消令牌（用户点停止）必须在命令执行中途打断它并拆毁进程树——主流对齐
    /// （Claude Code 的 Esc 会杀掉正在运行的命令）；协作取消只查 select! 分支时，
    /// 长命令期间停止按钮形同虚设。30s 睡眠命令 + 300ms 后取消 → 秒级返回。
    #[test]
    #[cfg(windows)]
    fn cancel_token_interrupts_running_command_promptly() {
        let root = std::env::temp_dir().to_string_lossy().to_string();
        let token = tokio_util::sync::CancellationToken::new();
        let cancel = token.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            cancel.cancel();
        });
        let started = std::time::Instant::now();
        let r = execute_command(
            "ping 127.0.0.1 -n 30 >nul",
            &root,
            Some(60_000),
            false,
            None,
            false,
            WindowsShell::Cmd,
            Some(token),
        );
        let elapsed = started.elapsed();
        assert!(r.is_error, "被取消的命令应返回错误结果");
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "取消应秒级生效（实际 {:?}）",
            elapsed
        );
    }

    /// cmd 的 `start` 内建默认 CREATE_NEW_CONSOLE：在 GUI 子系统的发行版里，AIO 自己
    /// 的隐藏 shell 没有控制台，`start` 于是新建一个**可见**窗口并常驻——就是"前端和
    /// 后端各弹一个终端窗口"的成因。重写为 `start /b` 后子进程共享父 shell（不可见）
    /// 的控制台，控制台一切不可见，而 stdout/stderr 管道照旧继承，输出不丢。
    #[test]
    #[cfg(windows)]
    fn cmd_start_is_rewritten_to_background() {
        assert_eq!(
            rewrite_cmd_start_hidden("start \"\" npm.cmd run dev"),
            "start /b \"\" npm.cmd run dev"
        );
        // 命令起始位置由 `&&` / `&` / `||` / `(` / 行首引入，都要覆盖
        assert_eq!(
            rewrite_cmd_start_hidden("cd /d D:\\proj && start \"\" npm.cmd run dev"),
            "cd /d D:\\proj && start /b \"\" npm.cmd run dev"
        );
        assert_eq!(
            rewrite_cmd_start_hidden("echo hi & start \"\" mover-v.exe"),
            "echo hi & start /b \"\" mover-v.exe"
        );
        assert_eq!(rewrite_cmd_start_hidden("a || start b"), "a || start /b b");
        assert_eq!(rewrite_cmd_start_hidden("(start b)"), "(start /b b)");
        // 大小写不敏感
        assert_eq!(rewrite_cmd_start_hidden("START b"), "START /b b");
    }

    /// 别把 `start` 当参数用的情况改坏：`net start`、`npm start`、`sc start`、
    /// `echo start` 都是**别的程序**的参数，改了就破坏语义；引号内的文本同理。
    #[test]
    #[cfg(windows)]
    fn cmd_start_rewrite_leaves_non_builtin_untouched() {
        for keep in [
            "net start mysql",
            "npm start",
            "sc start wuauserv",
            "echo start here",
            "echo \"start the server\"",
            "start /b \"\" npm.cmd run dev",
            "start /min \"\" app.exe",
            "start /wait setup.exe",
            "start /i app.exe",
        ] {
            assert_eq!(rewrite_cmd_start_hidden(keep), keep, "不应改写: {keep}");
        }
    }

    /// 幂等：重写结果再重写一次必须不变（cmd_utf8_prelude 与其它调用点可能叠加）。
    #[test]
    #[cfg(windows)]
    fn cmd_start_rewrite_is_idempotent() {
        for c in [
            "start \"\" npm.cmd run dev",
            "cd /d D:\\p && start /min app.exe",
            "npm start",
            "net start mysql",
        ] {
            let once = rewrite_cmd_start_hidden(c);
            assert_eq!(rewrite_cmd_start_hidden(&once), once, "非幂等: {c}");
        }
    }

    /// 端到端：真实 spawn 一个 `cmd /S /C`，`start /b` 起的孙进程输出仍必须经管道
    /// 回到调用方——`/b` 只改控制台归属，不能把输出弄丢（这是"隐藏窗口"与"拿得到
    /// 输出"两个目标同时成立的前提）。
    #[test]
    #[cfg(windows)]
    fn cmd_start_rewrite_keeps_child_output() {
        use std::io::Read as _;
        use std::os::windows::process::CommandExt as _;
        let mut cmd = std::process::Command::new("cmd.exe");
        // Same argv shape as the production cmd path (raw_arg + one outer quote
        // pair); plain arg() would add a second quoting layer and `start /b ""`
        // would no longer parse.
        cmd.raw_arg("/S")
            .raw_arg("/C")
            .raw_arg("\"start /b \"\" cmd /c echo aio_start_ok\"")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::null());
        cmd.hidden_console();
        let mut child = cmd.spawn().expect("spawn cmd");
        let mut out = String::new();
        if let Some(mut so) = child.stdout.take() {
            let _ = so.read_to_string(&mut out);
        }
        let _ = child.wait();
        assert!(
            out.contains("aio_start_ok"),
            "start /b 的子进程输出应经管道返回: {out:?}"
        );
    }

    /// PowerShell 路径的 shim 必须真的在生成的脚本里，且脚本语法有效：shim 遮蔽的是
    /// 真实 cmdlet，一处拼错就会让所有 `Start-Process` 静默失效——比弹窗更糟。
    /// 顺带守住 `-PassThru` 与退出码传播不被 shim 破坏。
    #[test]
    #[cfg(windows)]
    fn pwsh_script_carries_working_start_process_shim() {
        let script = pwsh_script("Start-Process cmd -ArgumentList '/c','exit 3' -Wait");
        // 结构性：shim 定义 + 默认隐藏 + 哈希 splatting 转发
        assert!(
            script.contains("function global:Start-Process"),
            "缺少 shim"
        );
        assert!(
            script.contains("ProcessWindowStyle]::Hidden"),
            "缺少默认隐藏"
        );
        assert!(
            script.contains("@PSBoundParameters"),
            "必须用 $PSBoundParameters 哈希 splatting（数组 splatting 会按位置绑定而失败）"
        );
        // Review round 1: the parameter surface must cover what the real cmdlet
        // accepts, or calls using those switches fail binding before forwarding.
        assert!(
            script.contains("[CmdletBinding(SupportsShouldProcess)]"),
            "必须声明 SupportsShouldProcess（-WhatIf/-Confirm 否则无法绑定）"
        );
        assert!(
            script.contains("[hashtable]$Environment"),
            "必须声明 -Environment（pwsh 7.4+ 调用方会用到）"
        );
        // shim 必须在 body 之前，否则 body 首个 Start-Process 命中不到
        let shim_at = script.find("function global:Start-Process").unwrap();
        let body_at = script.find("Start-Process cmd").unwrap();
        assert!(shim_at < body_at, "shim 必须先于用户命令");

        // 行为：脚本能被 pwsh 执行且 -PassThru 仍返回进程对象；退出码 3 透传
        let out = std::process::Command::new("cmd")
            .args(["/C", "where pwsh >nul 2>nul && echo yes"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        if !out.contains("yes") {
            return; // 无 pwsh（Windows PowerShell 5.1 由其它测试覆盖）
        }
        let probe = "Start-Process cmd -ArgumentList '/c','exit 3' -Wait -PassThru | ".to_string()
            + "ForEach-Object { Write-Output ('pid_' + ($null -ne $_.Id)) }; exit 3";
        let mut c = std::process::Command::new("pwsh");
        c.args([
            "-NoProfile",
            "-EncodedCommand",
            &pwsh_encoded(&pwsh_script(&probe)),
        ]);
        crate::utils::exec_resolve::HiddenConsole::hidden_console(&mut c);
        let r = c.output().expect("run pwsh");
        let text = String::from_utf8_lossy(&r.stdout).to_string();
        assert!(
            text.contains("pid_True"),
            "shim 下 -PassThru 应仍返回进程对象: {text:?} / {r:?}"
        );
        assert_eq!(r.status.code(), Some(3), "退出码应透传");
    }

    /// P1-1（harness 反馈第一轮）：超长输出必须**头尾都保留**——编译器/测试器的
    /// 失败详情几乎总在末尾，只留头部会让 `cargo test` 的失败信息反复拿不到。
    /// 排水环保尾部、结果体在中段丢弃处带标记。
    #[test]
    fn long_output_keeps_head_and_tail() {
        // Markers at both ends of a >head+tail stream so the assertion is unambiguous.
        let mut data = vec![b'a'; 100_000];
        data[..4].copy_from_slice(b"HEAD");
        let tail_len = MAX_OUTPUT_TAIL_BYTES;
        let tail_marker_at = data.len() - 10;
        data[tail_marker_at..].copy_from_slice(b"TAILMARKER");
        let d = drain_pipe_capped(&data[..], MAX_OUTPUT_HEAD_BYTES, tail_len);
        assert_eq!(d.total, data.len());
        assert!(d.gap(), "中段应被丢弃");
        assert_eq!(d.head.len(), MAX_OUTPUT_HEAD_BYTES);
        assert_eq!(d.tail.len(), MAX_OUTPUT_TAIL_BYTES);
        assert_eq!(&d.head[..4], b"HEAD");
        assert_eq!(&d.tail[d.tail.len() - 10..], b"TAILMARKER");
        let r = build_command_result(
            "x",
            "root",
            Some(0),
            false,
            1000,
            d,
            DrainedOutput::default(),
        );
        let text = text_of(&r);
        assert!(text.contains("[stdout]:"), "应含 stdout 段: {text}");
        assert!(text.contains("中段输出已丢弃"), "应含中段丢弃标记: {text}");
        assert!(text.contains("HEAD"), "头部标记应保留: {text}");
        assert!(text.contains("TAILMARKER"), "尾部标记应保留: {text}");
    }

    /// P1-1 companion: a stream that fits the budget stays contiguous (no gap
    /// marker, no seam — a code point must never split at a head/tail boundary).
    #[test]
    fn short_output_stays_contiguous_without_gap_marker() {
        let data = "短输出 short output".repeat(50); // UTF-8 multi-byte, well under the caps
        let d = drain_pipe_capped(
            data.as_bytes(),
            MAX_OUTPUT_HEAD_BYTES,
            MAX_OUTPUT_TAIL_BYTES,
        );
        assert!(!d.gap());
        assert_eq!(d.joined(), data.as_bytes());
        let r = build_command_result(
            "x",
            "root",
            Some(0),
            false,
            1000,
            d,
            DrainedOutput::default(),
        );
        let text = text_of(&r);
        assert!(!text.contains("中段输出已丢弃"), "无中段不应有标记: {text}");
        assert!(text.contains("短输出"), "内容应完整: {text}");
    }

    /// P0-3（harness 反馈第二轮）：CLIXML 流（重定向 stderr 下 PowerShell 5.1
    /// 序列化错误记录）解码回可读文本——记录边界成行、剥标签、还原 `_xHHHH_`
    /// 实体、去 ANSI 色码。
    #[test]
    fn decode_console_output_unwraps_clixml() {
        let raw = "#< CLIXML\r\n<Objs Version=\"1.1.0.1\" xmlns=\"http://schemas.microsoft.com/powershell/2004/04\"><S S=\"Error\">_x001B_[31;1mGet-Content: _x001B_[0m_x000D__x000A_找不到路径\"nope\"。_x000D__x000A_</S><S S=\"Error\">_x001B_[31;1msecond line_x001B_[0m_x000D__x000A_</S></Objs>";
        let out = decode_console_output(raw.as_bytes());
        assert!(out.contains("Get-Content:"), "首条错误应可读: {out}");
        assert!(out.contains("找不到路径"), "正文应可读: {out}");
        assert!(out.contains("second line"), "第二条记录独立成行: {out}");
        assert!(!out.contains("CLIXML"), "外壳标记应剥除: {out}");
        assert!(!out.contains("_x001B_"), "实体应还原: {out}");
        assert!(!out.contains('\u{1b}'), "ANSI 色码应清除: {out:?}");
        // 非 CLIXML 流原样通过
        assert_eq!(decode_console_output("普通文本".as_bytes()), "普通文本");
    }

    /// P0-1/P0-2（harness 反馈第二轮）：pwsh 尾部结构——scriptblock + Out-Default
    /// + 块内捕获 `$?` + 仅非零 native 码才 exit。
    ///
    /// 旧尾部（`$null -ne $LASTEXITCODE` 即 exit）会在「跑过一次 git 后输出表格」时
    /// 100% 丢表格、并把后续 cmdlet 报错上报为成功。
    #[test]
    #[cfg(windows)]
    fn pwsh_script_uses_out_default_tail() {
        let script = pwsh_script("Write-Output hello");
        assert!(script.contains("& {"), "body 应在 scriptblock 内: {script}");
        assert!(
            script.contains("| Out-Default"),
            "应经 Out-Default 立即渲染格式化输出: {script}"
        );
        assert!(
            script.contains("$global:__aio_ok = $?"),
            "应在块内捕获 $（管道外会重置）: {script}"
        );
        assert!(
            script.contains("$LASTEXITCODE -ne 0"),
            "仅非零 native 码才 exit: {script}"
        );
        // body 在 scriptblock 内、在尾部之前
        let block_at = script.find("& {").unwrap();
        let body_at = script.find("Write-Output hello").unwrap();
        let tail_at = script.rfind("Out-Default").unwrap();
        assert!(block_at < body_at && body_at < tail_at);
    }

    /// P0-1/P0-2 行为矩阵（真 pwsh，带 -OutputFormat Text 的完整生产形态）。
    /// 三个用例在旧尾部下全挂：表格丢失（2B stdout）、cmdlet 报错被 exit 0 掩盖。
    #[test]
    #[cfg(windows)]
    fn pwsh_tail_preserves_formatter_output_and_reports_cmdlet_errors() {
        let has_pwsh = std::process::Command::new("cmd")
            .args(["/C", "where pwsh >nul 2>nul && echo yes"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        if !has_pwsh.contains("yes") {
            return; // 无 pwsh：5.1 路径的等价行为由结构测试与探针报告覆盖
        }
        use crate::utils::exec_resolve::HiddenConsole as _;
        // cargo test runs the binary with the CRATE root as cwd — use a file that
        // exists there (Cargo.toml). package.json only exists at the workspace root
        // (the first version of this test asserted against it and false-failed).
        let workdir = std::env::current_dir().expect("cwd");
        let probe_file = if workdir.join("Cargo.toml").exists() {
            "Cargo.toml"
        } else {
            "package.json"
        };
        let table_cmd = format!(
            "git rev-parse --short HEAD | Out-Null; Get-ChildItem -Filter {probe_file} | Select-Object Name"
        );
        let run = |cmd: &str| {
            let mut c = std::process::Command::new("pwsh");
            c.current_dir(&workdir).args([
                "-NoProfile",
                "-NonInteractive",
                "-NoLogo",
                "-OutputFormat",
                "Text",
                "-EncodedCommand",
                &pwsh_encoded(&pwsh_script(cmd)),
            ]);
            c.hidden_console();
            let r = c.output().expect("run pwsh");
            (
                r.status.code(),
                String::from_utf8_lossy(&r.stdout).into_owned(),
                String::from_utf8_lossy(&r.stderr).into_owned(),
            )
        };

        // 1) native 成功后输出表格：旧尾部丢表格（延迟格式化器被 exit 跳过）
        let (rc1, out1, _) = run(&table_cmd);
        assert!(
            out1.contains(probe_file),
            "native 后的表格输出必须保留（P0-1）: {out1:?}"
        );
        assert_eq!(rc1, Some(0), "git 成功 + 表格 → rc0");

        // 2) native 成功后 cmdlet 报错：旧尾部 exit 0 掩盖（P0-2）
        let (rc2, out2, err2) =
            run("git rev-parse --short HEAD | Out-Null; Get-Content nope_xyz_aio");
        assert_eq!(
            rc2,
            Some(1),
            "cmdlet 报错必须上报 rc1，而不是被 $LASTEXITCODE=0 掩盖"
        );
        assert!(
            !err2.contains("CLIXML"),
            "stderr 应为纯文本（-OutputFormat Text）: {err2:?}"
        );
        assert!(
            out2.is_empty() || out2.len() < 8,
            "该用例无对象输出: {out2:?}"
        );

        // 3) native 非零 + 表格：退出码与输出双保留
        let (rc3, out3, _) = run(&format!(
            "cmd /c exit 7; Get-ChildItem -Filter {probe_file} | Select-Object Name"
        ));
        assert_eq!(rc3, Some(7), "native 非零退出码应透传");
        assert!(
            out3.contains(probe_file),
            "native 非零时的表格也应保留: {out3:?}"
        );
    }
}

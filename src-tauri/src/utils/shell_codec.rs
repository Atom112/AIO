//! Shell codec: the encode/decode pair around agent shell commands (Windows).
//!
//! Split out of `utils::shell_tools` (harness round 2 grew it past the file-size
//! gate; this layer is a self-contained concern). Pure functions only — no
//! process spawning, no state:
//!
//! **Encode** — shaping a command string into a shell invocation script:
//! [`pwsh_script`] (UTF-8 prelude + [`START_PROCESS_HIDDEN_SHIM`] + scriptblock
//! body + exit-tail), [`cmd_utf8_prelude`], [`rewrite_cmd_start_hidden`],
//! [`pwsh_encoded`].
//!
//! **Decode** — turning captured console bytes back into readable text:
//! [`decode_console_output`] (strict UTF-8 → ANSI code page → lossy), including
//! the mechanical CLIXML unwrap for redirected PowerShell error records.
//!
//! The command-execution lifecycle (specs, sandbox wiring, drain, teardown)
//! stays in `shell_tools`, which re-exports this module's items for its own
//! call sites and tests.

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
/// CLIXML→text pass.
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

// ====== Windows shell script shaping =====

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
pub(crate) const START_PROCESS_HIDDEN_SHIM: &str = concat!(
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
pub(crate) fn pwsh_script(command: &str) -> String {
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
pub(crate) fn cmd_utf8_prelude(command: &str) -> String {
    // `start /b` first: the rewrite preserves any window mode the caller
    // already chose, and running it on the user's own command text keeps the
    // anchors (`&`, `&&`, line start) meaningful — prefixing with `chcp` would
    // otherwise turn the first `start` into a non-command-initial token.
    format!("chcp 65001 >nul & {}", rewrite_cmd_start_hidden(command))
}

/// `-EncodedCommand` payload: UTF-16LE base64 — immune to every quote-escaping
/// problem on both the argv and raw-command-line invocation paths.
#[cfg(windows)]
pub(crate) fn pwsh_encoded(script: &str) -> String {
    use base64::Engine as _;
    let utf16: Vec<u16> = script.encode_utf16().collect();
    let bytes: Vec<u8> = utf16.iter().flat_map(|w| w.to_le_bytes()).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

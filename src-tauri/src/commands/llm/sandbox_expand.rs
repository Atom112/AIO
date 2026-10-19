//! Sandbox expansion request (harness round 3, Gemini-CLI-shaped UX).
//!
//! When a command fails inside the Windows AppContainer sandbox with the
//! access-denied signature (WinError 5), the raw error alone leaves the model
//! blind-retrying (round 1: 25 rounds burned) and the user unaware. This module
//! converts the failure into a one-click remediation dialog by reusing the
//! ask_user plumbing (the frontend's AgentQuestionBubble renders it; the
//! listener routes by request_id only, so no frontend changes are needed):
//!
//! 1. 授权 <目录> 并重试 — session R+X grant (process-lifetime), retry once
//! 2. 授权 <目录> 并永久记住 — additionally persisted to sandboxWindowsToolchainDirs
//! 3. 跳出沙箱执行（仅本次） — retry once with the AppContainer disabled
//! 4. 忽略 — the original error stands; the model/breaker handles it
//!
//! Custom free-form input is interpreted as a directory to grant (session).
//! Ask-once dedup: per first command token, a denial signature asks at most once
//! per [`REASK_AFTER`] window — unattended runs degrade to today's behavior
//! (timeout = Ignore) instead of stalling the turn repeatedly.

// Windows-only module: every runtime caller (the AppContainer execution path in
// commands/llm) is `#[cfg(windows)]`. Gating the whole module keeps clippy's
// dead-code analysis honest on the Linux/macOS CI matrix; the pure-logic tests
// below keep running on the Windows CI runner.
#![cfg(windows)]

use crate::commands::llm::tasks::{ask_user_questions, QuestionOptionView, QuestionView};
use crate::core::i18n::pick;
use crate::core::models::QuestionResponse;
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tokio_util::sync::CancellationToken;

/// Expansion wait budget, aligned with the tool-approval and ask_user waits
/// (600s): the user may be away from the screen, and a denial that degrades to
/// Ignore after 60s loses the whole remediation path. The 30-min re-ask window
/// (REASK_AFTER) still throttles repeat prompts for the same denial signature.
const EXPANSION_TIMEOUT: Duration = Duration::from_secs(600);

/// A denial signature re-asks at most once per this window (per first token).
const REASK_AFTER: Duration = Duration::from_secs(30 * 60);

/// The user's remediation choice for a sandbox-blocked command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExpansionDecision {
    /// Keep the original error; the model (and the failure breaker) handles it
    Ignore,
    /// Re-run this one command with the AppContainer disabled
    RunUnsandboxedOnce,
    /// Grant `dir` R+X for the process lifetime, then retry
    GrantSessionRetry { dir: String },
    /// Grant `dir` R+X for the process lifetime AND persist it to the config
    GrantPermanentRetry { dir: String },
}

// ---- Auto-mode self-heal policy (security review P0, PR #151) ----
//
// The escalation surface is NARROWED to one class: an interpreter/toolchain that is
// INVISIBLE inside the sandbox (not-found signature + a derivable directory). That
// class self-heals with a session grant and — only when the user explicitly opted in
// via `sandboxUnsandboxedFallback` — one unsandboxed retry if the grant could not
// land (EDR-blocked ACL write). **Access-denied class denials (write targets, .git
// protection, out-of-project paths) NEVER escalate to unsandboxed execution**, with
// or without the switch: the denial text comes from command OUTPUT the model can
// influence, so it must never gate a boundary break. Denied commands stay denied —
// the model routes around them (builtin git_* tools, different paths) or the user
// widens the boundary via static config (`sandboxWindowsToolchainDirs`).

/// Per-turn budget for unsandboxed retries (all escalation sources combined).
/// Reset at every agent-turn start; exceeded → the original error stands, with a
/// note in the result text.
const UNSANDBOXED_BUDGET_PER_TURN: usize = 8;

fn unsandboxed_budget() -> &'static std::sync::atomic::AtomicUsize {
    static BUDGET: std::sync::OnceLock<std::sync::atomic::AtomicUsize> = std::sync::OnceLock::new();
    BUDGET.get_or_init(|| std::sync::atomic::AtomicUsize::new(0))
}

/// Resets the per-turn unsandboxed budget (called at every agent-turn start).
pub(crate) fn reset_unsandboxed_budget() {
    unsandboxed_budget().store(0, std::sync::atomic::Ordering::Relaxed);
}

/// Consumes one unit of the budget; false = exhausted (escalation refused).
fn take_unsandboxed_budget() -> bool {
    let budget = unsandboxed_budget();
    let used = budget.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    used < UNSANDBOXED_BUDGET_PER_TURN
}

/// The Auto self-heal decision for a fresh denial, per the narrowed policy above.
/// Pure — unit-tested without a UI. The second decision point (one unsandboxed
/// retry after a failed grant, not-found class only) lives in
/// [`escalate_after_failed_grant`].
pub(crate) fn auto_remediation(hint_dir: Option<&str>) -> ExpansionDecision {
    match hint_dir {
        // 解释器/工具链不可见（not-found 蕴含 hint）或有派生目录的拒绝类：
        // 会话授权后重试——授权把目录带进沙箱视野，边界不破
        Some(dir) => ExpansionDecision::GrantSessionRetry {
            dir: dir.to_string(),
        },
        // 无目录可授权：access-denied 类（写目标被拒、.git 保护、项目外路径……）
        // —— 永不跳出沙箱，保持错误让模型绕路
        None => ExpansionDecision::Ignore,
    }
}

/// Whether a SECOND escalation — one unsandboxed retry — is permitted after a
/// session grant failed to fix a not-found denial (e.g. the EDR blocked the ACL
/// write so the directory stayed invisible). Only the not-found class qualifies,
/// and only when the user opted in via `sandboxUnsandboxedFallback`. Pure — tested.
pub(crate) fn escalate_after_failed_grant(not_found: bool, fallback_enabled: bool) -> bool {
    not_found && fallback_enabled && take_unsandboxed_budget()
}

/// Appends a visibility note to a tool result's first text part (security review:
/// unsandboxed retries and their side effects must be visible to the user AND the
/// model — previously only a tracing::warn line existed).
pub(crate) fn annotate_result(tr: &mut crate::core::models::ToolResult, note: &str) {
    if let Some(c) = tr.content.iter_mut().find(|c| c.kind == "text") {
        let existing = c.data.get("text").and_then(|v| v.as_str()).unwrap_or("");
        c.data["text"] = serde_json::json!(format!("{existing}\n{note}"));
    }
}

// ---- ask-once dedup (per first command token) ----

fn recently_asked() -> &'static std::sync::Mutex<HashMap<String, Instant>> {
    static MAP: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Instant>>> =
        std::sync::OnceLock::new();
    MAP.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn first_token(command: &str) -> String {
    // Operation-level key (harness round 3): `cmd /c python -V` and `python -V`
    // share "python"; `cmd /c del x` and `del x` share "del" — the ask-once window
    // dedups by what the command DOES, not by the raw first token.
    crate::utils::shell_tools::expansion_key(command)
}

/// Whether this command's operation token is eligible for a new expansion request
/// (not asked within the [`REASK_AFTER`] window).
pub(crate) fn should_ask(command: &str) -> bool {
    let key = first_token(command);
    if key.is_empty() {
        return false;
    }
    let map = recently_asked().lock().unwrap();
    match map.get(&key) {
        Some(at) => at.elapsed() >= REASK_AFTER,
        None => true,
    }
}

/// Records that an expansion request fired for this command's first token.
pub(crate) fn mark_asked(command: &str) {
    let key = first_token(command);
    if key.is_empty() {
        return;
    }
    recently_asked().lock().unwrap().insert(key, Instant::now());
}

/// Test-only: backdate the entry so `should_ask` is fully coverable.
#[cfg(test)]
fn mark_asked_at(command: &str, at: Instant) {
    let key = first_token(command);
    recently_asked().lock().unwrap().insert(key, at);
}

// ---- directory merge (pure, unit-tested) ----

/// Merges configured toolchain dirs with session grants, deduplicating
/// case-insensitively and preserving order (config first).
pub(crate) fn merge_rx_dirs(config_dirs: &[String], session_dirs: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for d in config_dirs.iter().chain(session_dirs.iter()) {
        let t = d.trim();
        if !t.is_empty() && !out.iter().any(|x| x.eq_ignore_ascii_case(t)) {
            out.push(t.to_string());
        }
    }
    out
}

/// AppContainer R+X dir list for spawning: the configured toolchain dirs plus
/// the process-lifetime session grants (from earlier expansion approvals).
/// None when the sandbox is disabled — the caller passes it as `ac_dirs`.
#[cfg(windows)]
pub(crate) fn appcontainer_dirs_with_session_grants(app: &AppHandle) -> Option<Vec<String>> {
    let cfg = crate::core::config::load_app_config(app).unwrap_or_default();
    if cfg.sandbox_windows == "appcontainer" {
        let session = crate::utils::sandbox::session_rx_grants();
        Some(merge_rx_dirs(&cfg.sandbox_windows_toolchain_dirs, &session))
    } else {
        None
    }
}

/// Best-effort persist of a user-approved toolchain dir into
/// `sandboxWindowsToolchainDirs` (the "永久记住" option). Failures only log —
/// the session grant already applies for this process.
#[cfg(windows)]
pub(crate) fn persist_toolchain_dir(app: &AppHandle, dir: &str) {
    let dir = dir.trim();
    if dir.is_empty() {
        return;
    }
    match crate::core::config::load_app_config(app) {
        Ok(mut cfg) => {
            if cfg
                .sandbox_windows_toolchain_dirs
                .iter()
                .any(|d| d.eq_ignore_ascii_case(dir))
            {
                return;
            }
            cfg.sandbox_windows_toolchain_dirs.push(dir.to_string());
            if let Err(e) = crate::core::config::save_app_config(app, &cfg) {
                tracing::warn!("[sandbox-expand] 永久授权写入配置失败（仅本次会话生效）: {e}");
            }
        }
        Err(e) => tracing::warn!("[sandbox-expand] 读取配置失败（仅本次会话生效）: {e}"),
    }
}

// ---- decision request (synthetic ask_user) ----

/// Fires the expansion dialog and maps the user's answer to a decision.
///
/// `hint_dir` is the auto-derived grant candidate (the forwarded interpreter's
/// directory); when absent the dialog offers only the unsandboxed/ignore pair
/// plus a free-form directory input.
pub(crate) async fn request_sandbox_expansion(
    app: &AppHandle,
    command: &str,
    err_snippet: &str,
    hint_dir: Option<String>,
    token: &CancellationToken,
) -> Result<ExpansionDecision, String> {
    let snippet: String = err_snippet.lines().take(12).collect::<Vec<_>>().join("\n");
    let question = if hint_dir.is_some() {
        pick(
            &format!("命令被 Windows 沙箱拒绝：\n$ {command}\n\n{snippet}\n\n是否为沙箱补充授权？"),
            &format!("The command was denied by the Windows sandbox:\n$ {command}\n\n{snippet}\n\nExpand sandbox access?"),
        )
    } else {
        pick(
            &format!("命令被 Windows 沙箱拒绝：\n$ {command}\n\n{snippet}\n\n（可输入要授权的目录路径，或选择以下操作）"),
            &format!("The command was denied by the Windows sandbox:\n$ {command}\n\n{snippet}\n\n(Type a directory to grant, or pick an action below)"),
        )
    };

    let grant_session_label = hint_dir.as_deref().map(|d| {
        pick(
            &format!("授权 {d} 并重试（本次会话）"),
            &format!("Grant {d} and retry (this session)"),
        )
    });
    let grant_permanent_label = hint_dir.as_deref().map(|d| {
        pick(
            &format!("授权 {d} 并永久记住"),
            &format!("Grant {d} and remember permanently"),
        )
    });
    let unsandboxed_label = pick("跳出沙箱执行（仅本次）", "Run without sandbox (this once)");
    let ignore_label = pick("忽略，让智能体自行处理", "Ignore — let the agent handle it");

    let mut options: Vec<QuestionOptionView> = Vec::new();
    if let (Some(session_l), Some(perm_l)) = (&grant_session_label, &grant_permanent_label) {
        options.push(QuestionOptionView {
            label: session_l.clone(),
            description: Some(pick(
                "在当前应用会话内授予该目录读取/执行权限，然后重试这条命令",
                "Grant read/execute on that directory for this app session, then retry the command",
            )),
            recommended: true,
        });
        options.push(QuestionOptionView {
            label: perm_l.clone(),
            description: Some(pick(
                "写入应用配置，之后同类命令不再被拦截",
                "Persist to the app config so future commands of this kind are no longer blocked",
            )),
            recommended: false,
        });
    }
    let unsandboxed_recommended = options.is_empty();
    options.push(QuestionOptionView {
        label: unsandboxed_label.clone(),
        description: Some(pick(
            "这条命令将脱离沙箱运行一次（其余命令仍受沙箱约束）",
            "This single command runs outside the sandbox (all others stay sandboxed)",
        )),
        recommended: unsandboxed_recommended,
    });
    options.push(QuestionOptionView {
        label: ignore_label.clone(),
        description: Some(pick("保持原错误结果", "Keep the original error result")),
        recommended: false,
    });

    let questions = vec![QuestionView {
        question_id: "q1".into(),
        question,
        options,
        allow_custom: true,
    }];

    let response: QuestionResponse =
        ask_user_questions(app, "", "", questions, EXPANSION_TIMEOUT, token).await?;
    Ok(map_response(
        &response,
        hint_dir.as_deref(),
        grant_session_label.as_deref(),
        grant_permanent_label.as_deref(),
        &unsandboxed_label,
        &ignore_label,
    ))
}

/// Maps the dialog answer to a decision. Pure — unit-tested without a UI.
pub(crate) fn map_response(
    response: &QuestionResponse,
    hint_dir: Option<&str>,
    grant_session_label: Option<&str>,
    grant_permanent_label: Option<&str>,
    unsandboxed_label: &str,
    _ignore_label: &str,
) -> ExpansionDecision {
    if response.skipped {
        return ExpansionDecision::Ignore;
    }
    let Some(ans) = response.answers.iter().find(|a| a.question_id == "q1") else {
        return ExpansionDecision::Ignore;
    };
    let answer = ans.answer.trim();
    if answer.is_empty() {
        return ExpansionDecision::Ignore;
    }
    if ans.is_custom {
        // Free-form input = a directory to grant (session-lifetime)
        let p = Path::new(answer);
        if p.is_absolute() {
            return ExpansionDecision::GrantSessionRetry {
                dir: answer.to_string(),
            };
        }
        return ExpansionDecision::Ignore;
    }
    if Some(answer) == grant_session_label {
        return hint_dir
            .map(|d| ExpansionDecision::GrantSessionRetry { dir: d.to_string() })
            .unwrap_or(ExpansionDecision::Ignore);
    }
    if Some(answer) == grant_permanent_label {
        return hint_dir
            .map(|d| ExpansionDecision::GrantPermanentRetry { dir: d.to_string() })
            .unwrap_or(ExpansionDecision::Ignore);
    }
    if answer == unsandboxed_label {
        return ExpansionDecision::RunUnsandboxedOnce;
    }
    ExpansionDecision::Ignore
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::QuestionAnswer;

    fn answer(text: &str, is_custom: bool) -> QuestionResponse {
        QuestionResponse {
            skipped: false,
            answers: vec![QuestionAnswer {
                question_id: "q1".into(),
                answer: text.into(),
                is_custom,
            }],
        }
    }

    const HINT: Option<&str> = Some("D:\\Anaconda3");
    const GRANT_S: Option<&str> = Some("授权 D:\\Anaconda3 并重试（本次会话）");
    const GRANT_P: Option<&str> = Some("授权 D:\\Anaconda3 并永久记住");
    const UNSANDBOXED: &str = "跳出沙箱执行（仅本次）";
    const IGNORE: &str = "忽略，让智能体自行处理";

    /// Option answers map to their decisions.
    #[test]
    fn maps_option_answers() {
        let d = map_response(
            &answer(GRANT_S.unwrap(), false),
            HINT,
            GRANT_S,
            GRANT_P,
            UNSANDBOXED,
            IGNORE,
        );
        assert_eq!(
            d,
            ExpansionDecision::GrantSessionRetry {
                dir: "D:\\Anaconda3".into()
            }
        );
        let d = map_response(
            &answer(GRANT_P.unwrap(), false),
            HINT,
            GRANT_S,
            GRANT_P,
            UNSANDBOXED,
            IGNORE,
        );
        assert_eq!(
            d,
            ExpansionDecision::GrantPermanentRetry {
                dir: "D:\\Anaconda3".into()
            }
        );
        let d = map_response(
            &answer(UNSANDBOXED, false),
            HINT,
            GRANT_S,
            GRANT_P,
            UNSANDBOXED,
            IGNORE,
        );
        assert_eq!(d, ExpansionDecision::RunUnsandboxedOnce);
        let d = map_response(
            &answer(IGNORE, false),
            HINT,
            GRANT_S,
            GRANT_P,
            UNSANDBOXED,
            IGNORE,
        );
        assert_eq!(d, ExpansionDecision::Ignore);
    }

    /// Skipped / missing / empty answers degrade to Ignore (agent decides).
    #[test]
    fn skipped_missing_empty_degrade_to_ignore() {
        let skipped = QuestionResponse {
            skipped: true,
            answers: vec![],
        };
        assert_eq!(
            map_response(&skipped, HINT, GRANT_S, GRANT_P, UNSANDBOXED, IGNORE),
            ExpansionDecision::Ignore
        );
        let no_q1 = QuestionResponse {
            skipped: false,
            answers: vec![],
        };
        assert_eq!(
            map_response(&no_q1, HINT, GRANT_S, GRANT_P, UNSANDBOXED, IGNORE),
            ExpansionDecision::Ignore
        );
        let empty = answer("  ", false);
        assert_eq!(
            map_response(&empty, HINT, GRANT_S, GRANT_P, UNSANDBOXED, IGNORE),
            ExpansionDecision::Ignore
        );
    }

    /// Custom input: an absolute path becomes a session grant; junk degrades to Ignore.
    #[test]
    fn custom_input_maps_to_dir_grant() {
        let d = map_response(
            &answer("C:\\Tools\\cargo", true),
            HINT,
            GRANT_S,
            GRANT_P,
            UNSANDBOXED,
            IGNORE,
        );
        assert_eq!(
            d,
            ExpansionDecision::GrantSessionRetry {
                dir: "C:\\Tools\\cargo".into()
            }
        );
        let d = map_response(
            &answer("just some words", true),
            HINT,
            GRANT_S,
            GRANT_P,
            UNSANDBOXED,
            IGNORE,
        );
        assert_eq!(d, ExpansionDecision::Ignore);
        let d = map_response(
            &answer("relative/path", true),
            HINT,
            GRANT_S,
            GRANT_P,
            UNSANDBOXED,
            IGNORE,
        );
        assert_eq!(d, ExpansionDecision::Ignore);
    }

    /// Without a hint dir the grant labels are absent: grant answers degrade to Ignore.
    #[test]
    fn no_hint_grant_labels_degrade() {
        let d = map_response(
            &answer("whatever", false),
            None,
            None,
            None,
            UNSANDBOXED,
            IGNORE,
        );
        assert_eq!(d, ExpansionDecision::Ignore);
        let d = map_response(
            &answer(UNSANDBOXED, false),
            None,
            None,
            None,
            UNSANDBOXED,
            IGNORE,
        );
        assert_eq!(d, ExpansionDecision::RunUnsandboxedOnce);
    }

    /// Dedup: fresh token asks; after mark_asked it doesn't; after the window it does again.
    #[test]
    fn ask_once_dedup_window() {
        let cmd = "python run.py";
        recently_asked().lock().unwrap().clear();
        assert!(should_ask(cmd), "首次应询问");
        mark_asked(cmd);
        assert!(!should_ask(cmd), "窗口内不应重复询问");
        // Same first token, different command, still deduped (keyed by token)
        assert!(!should_ask("python -V"), "同首 token 去重");
        // Backdate beyond the window → asks again
        mark_asked_at(cmd, Instant::now() - REASK_AFTER - Duration::from_secs(1));
        assert!(should_ask(cmd), "窗口过后应恢复询问");
        recently_asked().lock().unwrap().clear();
    }

    /// merge_rx_dirs: order preserved (config first), case-insensitive dedup, trims.
    #[test]
    fn merge_rx_dirs_dedup_and_order() {
        let merged = merge_rx_dirs(
            &["D:\\Anaconda3".into(), "C:\\cargo".into()],
            &["d:\\anaconda3".into(), "  E:\\go  ".into(), "".into()],
        );
        assert_eq!(
            merged,
            vec![
                "D:\\Anaconda3".to_string(),
                "C:\\cargo".to_string(),
                "E:\\go".to_string(),
            ]
        );
    }
}

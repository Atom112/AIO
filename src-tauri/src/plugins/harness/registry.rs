//! Harness tool registry: the built-in ACP entries plus availability detection.
//!
//! Every entry maps a stable tool id to its spawn spec. Two spawn shapes cover the
//! ACP family in this PR: `Bare` (a CLI on PATH that speaks ACP natively, e.g.
//! `opencode acp` / `omp acp` / `dsh --profile acp`) and `Npx` (an npm-packaged
//! ACP adapter run through `npx -y <package>`, e.g. the official Claude / Codex
//! adapters). Claude Code and Codex gain deeper *native* adapters in later PRs;
//! until then their ACP entries keep them usable end-to-end.

use serde::{Deserialize, Serialize};

/// How an ACP-speaking tool is launched.
#[derive(Clone, Debug)]
pub enum AcpCommand {
    /// A directly resolvable CLI (PATH-only resolution via exec_resolve).
    Bare { command: String, args: Vec<String> },
    /// An npm package run via `npx -y <package> <args...>` (fetched on first use).
    Npx { package: String, args: Vec<String> },
}

/// The adapter family a registered tool dispatches to.
#[derive(Clone, Debug)]
pub enum HarnessToolKind {
    Acp { command: AcpCommand },
    // (PR2) ClaudeNative, (PR4) CodexExec — reserved.
}

/// One registered harness tool.
#[derive(Clone, Debug)]
pub struct HarnessToolSpec {
    /// Stable tool id (persisted in workspaces/sessions; never rename).
    pub id: &'static str,
    /// Display name (brand names are not localized).
    pub name: &'static str,
    pub kind: HarnessToolKind,
}

/// Availability detection result for the frontend tool list.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HarnessToolStatus {
    pub id: String,
    /// ready | installable | missing
    pub state: String,
}

/// Built-in registry (order = sidebar order).
pub fn builtin_tools() -> Vec<HarnessToolSpec> {
    vec![
        HarnessToolSpec {
            id: "claude",
            name: "Claude Code",
            kind: HarnessToolKind::Acp {
                command: AcpCommand::Npx {
                    package: "@agentclientprotocol/claude-agent-acp".to_string(),
                    args: vec![],
                },
            },
        },
        HarnessToolSpec {
            id: "codex",
            name: "Codex",
            kind: HarnessToolKind::Acp {
                command: AcpCommand::Npx {
                    package: "@agentclientprotocol/codex-acp".to_string(),
                    args: vec![],
                },
            },
        },
        HarnessToolSpec {
            id: "opencode",
            name: "OpenCode",
            kind: HarnessToolKind::Acp {
                command: AcpCommand::Bare {
                    command: "opencode".to_string(),
                    args: vec!["acp".to_string()],
                },
            },
        },
        HarnessToolSpec {
            id: "omp",
            name: "Oh My Pi",
            kind: HarnessToolKind::Acp {
                command: AcpCommand::Bare {
                    command: "omp".to_string(),
                    args: vec!["acp".to_string()],
                },
            },
        },
        HarnessToolSpec {
            id: "gemini",
            name: "Gemini CLI",
            kind: HarnessToolKind::Acp {
                command: AcpCommand::Bare {
                    command: "gemini".to_string(),
                    args: vec!["--acp".to_string()],
                },
            },
        },
        HarnessToolSpec {
            id: "qwen",
            name: "Qwen Code",
            kind: HarnessToolKind::Acp {
                command: AcpCommand::Bare {
                    command: "qwen".to_string(),
                    args: vec!["--acp".to_string()],
                },
            },
        },
        HarnessToolSpec {
            id: "goose",
            name: "goose",
            kind: HarnessToolKind::Acp {
                command: AcpCommand::Bare {
                    command: "goose".to_string(),
                    args: vec!["acp".to_string()],
                },
            },
        },
        HarnessToolSpec {
            id: "dsh",
            name: "DSH",
            kind: HarnessToolKind::Acp {
                command: AcpCommand::Bare {
                    command: "dsh".to_string(),
                    args: vec!["--profile".to_string(), "acp".to_string()],
                },
            },
        },
    ]
}

/// Look up a tool spec by id.
pub fn lookup(tool_id: &str) -> Option<HarnessToolSpec> {
    builtin_tools().into_iter().find(|t| t.id == tool_id)
}

/// Detect availability for every built-in entry:
/// - `Bare`: the CLI resolves on PATH → ready, else missing.
/// - `Npx`: `npx` itself resolves → installable (the adapter fetches on first
///   spawn, so presence of the package does not change the UX); npx missing → missing.
pub fn detect_tools() -> Vec<HarnessToolStatus> {
    let npx_available = crate::utils::exec_resolve::resolve_executable("npx").is_some();
    builtin_tools()
        .into_iter()
        .map(|spec| {
            let state = match &spec.kind {
                HarnessToolKind::Acp { command } => match command {
                    AcpCommand::Bare { command, .. } => {
                        if crate::utils::exec_resolve::resolve_executable(command).is_some() {
                            "ready"
                        } else {
                            "missing"
                        }
                    }
                    AcpCommand::Npx { .. } => {
                        if npx_available {
                            "installable"
                        } else {
                            "missing"
                        }
                    }
                },
            };
            HarnessToolStatus {
                id: spec.id.to_string(),
                state: state.to_string(),
            }
        })
        .collect()
}

//! Harness tool registry: built-in ACP entries + user-registered custom ACP
//! agents plus availability detection.
//!
//! Every entry maps a stable tool id to its spawn spec. Three spawn shapes cover
//! the family: `Bare` (a CLI on PATH that speaks ACP natively, e.g. `opencode
//! acp` / `dsh --profile acp` / user-registered custom agents), `Npx` (an
//! npm-packaged ACP adapter run through `npx -y <package>`), and `ClaudeNative`
//! (Claude Code's own stream-json protocol with an ACP fallback).

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
    Acp {
        command: AcpCommand,
    },
    /// Claude Code native stream-json driver when a real `claude.exe`/`claude`
    /// binary exists; falls back to the bundled ACP command otherwise.
    ClaudeNative {
        fallback: AcpCommand,
    },
    /// OpenAI Codex native `exec --json` driver when a `codex` CLI exists;
    /// falls back to the bundled ACP command otherwise.
    CodexExec {
        fallback: AcpCommand,
    },
}

/// One value of a spawn-time option (CLI-flag-backed).
#[derive(Clone, Debug)]
pub struct SpawnOptionValueSpec {
    /// Value id shown to the GUI / persisted as the preference key.
    pub id: String,
    /// Display label (registry-declared, English; the GUI may i18n known ids).
    pub name: String,
    /// false = the tool's default behavior — inject nothing for this value.
    pub inject: bool,
}

/// A CLI-flag-backed option a tool supports but does NOT expose over its wire
/// protocol (e.g. omp's `--approval-mode`: its ACP config options cover
/// mode/model/thinking, but the approval modes always-ask|write|yolo are
/// TUI-only flags — verified against omp v17.2.1). Spawn-time only: the
/// driver injects `{arg}={value}` at process start; changing it respawns the
/// session (the GUI persists the preference and stops the runtime).
#[derive(Clone, Debug)]
pub struct SpawnOptionSpec {
    /// Config-option id shown to the GUI (e.g. "approval-mode").
    pub option_id: String,
    /// Display label (registry-declared, English; the GUI may i18n known ids).
    pub name: String,
    /// CLI flag; the chosen value is injected as `{arg}={id}` when that
    /// value's `inject` is true.
    pub arg: String,
    pub values: Vec<SpawnOptionValueSpec>,
}

/// One registered harness tool.
#[derive(Clone, Debug)]
pub struct HarnessToolSpec {
    /// Stable tool id (persisted in workspaces/sessions; never rename a
    /// builtin). Custom agents carry their generated uuid here.
    pub id: String,
    /// Display name (brand names are not localized).
    pub name: String,
    pub kind: HarnessToolKind,
    /// CLI-flag-backed options the tool supports outside its wire protocol
    /// (empty for most tools).
    pub spawn_options: Vec<SpawnOptionSpec>,
}

/// Availability detection result for the frontend tool list.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HarnessToolStatus {
    pub id: String,
    /// Display name (brand names for built-ins; user-chosen for customs).
    pub name: String,
    /// ready | installable | missing
    pub state: String,
    /// Which adapter serves this tool as detected: claude-native | acp |
    /// acp-fallback (drives UI affordances like the permission-mode selector).
    pub adapter: String,
    /// True for user-registered custom ACP agents (drives the delete action).
    #[serde(default)]
    pub custom: bool,
}

/// Built-in registry (order = sidebar order).
pub fn builtin_tools() -> Vec<HarnessToolSpec> {
    vec![
        HarnessToolSpec {
            id: "claude".to_string(),
            name: "Claude Code".to_string(),
            spawn_options: vec![],
            kind: HarnessToolKind::ClaudeNative {
                fallback: AcpCommand::Npx {
                    package: "@agentclientprotocol/claude-agent-acp".to_string(),
                    args: vec![],
                },
            },
        },
        HarnessToolSpec {
            id: "codex".to_string(),
            name: "Codex".to_string(),
            spawn_options: vec![],
            kind: HarnessToolKind::CodexExec {
                fallback: AcpCommand::Npx {
                    package: "@agentclientprotocol/codex-acp".to_string(),
                    args: vec![],
                },
            },
        },
        HarnessToolSpec {
            id: "opencode".to_string(),
            name: "OpenCode".to_string(),
            spawn_options: vec![],
            kind: HarnessToolKind::Acp {
                command: AcpCommand::Bare {
                    command: "opencode".to_string(),
                    args: vec!["acp".to_string()],
                },
            },
        },
        HarnessToolSpec {
            id: "omp".to_string(),
            name: "Oh My Pi".to_string(),
            // omp's ACP surface covers mode/model/thinking, but the approval
            // modes (always-ask|write|yolo) are TUI-only flags — inject them
            // at spawn (probe-verified: `omp acp --approval-mode=yolo` serves
            // ACP normally, oclif global flag).
            spawn_options: vec![SpawnOptionSpec {
                option_id: "approval-mode".to_string(),
                name: "Approval Mode".to_string(),
                arg: "--approval-mode".to_string(),
                values: vec![
                    SpawnOptionValueSpec {
                        id: "always-ask".to_string(),
                        name: "Always ask".to_string(),
                        inject: false,
                    },
                    SpawnOptionValueSpec {
                        id: "write".to_string(),
                        name: "Write".to_string(),
                        inject: true,
                    },
                    SpawnOptionValueSpec {
                        id: "yolo".to_string(),
                        name: "YOLO".to_string(),
                        inject: true,
                    },
                ],
            }],
            kind: HarnessToolKind::Acp {
                command: AcpCommand::Bare {
                    command: "omp".to_string(),
                    args: vec!["acp".to_string()],
                },
            },
        },
        HarnessToolSpec {
            id: "gemini".to_string(),
            name: "Gemini CLI".to_string(),
            spawn_options: vec![],
            kind: HarnessToolKind::Acp {
                command: AcpCommand::Bare {
                    command: "gemini".to_string(),
                    args: vec!["--acp".to_string()],
                },
            },
        },
        HarnessToolSpec {
            id: "qwen".to_string(),
            name: "Qwen Code".to_string(),
            spawn_options: vec![],
            kind: HarnessToolKind::Acp {
                command: AcpCommand::Bare {
                    command: "qwen".to_string(),
                    args: vec!["--acp".to_string()],
                },
            },
        },
        HarnessToolSpec {
            id: "goose".to_string(),
            name: "goose".to_string(),
            spawn_options: vec![],
            kind: HarnessToolKind::Acp {
                command: AcpCommand::Bare {
                    command: "goose".to_string(),
                    args: vec!["acp".to_string()],
                },
            },
        },
        HarnessToolSpec {
            id: "dsh".to_string(),
            name: "DSH".to_string(),
            spawn_options: vec![],
            kind: HarnessToolKind::Acp {
                command: AcpCommand::Bare {
                    command: "dsh".to_string(),
                    args: vec!["--profile".to_string(), "acp".to_string()],
                },
            },
        },
    ]
}

/// Look up a built-in tool spec by id (Claude/Codex/opencode/... — no customs).
pub fn lookup(tool_id: &str) -> Option<HarnessToolSpec> {
    builtin_tools().into_iter().find(|t| t.id == tool_id)
}

/// Look up any tool spec — built-ins first, then user-registered custom ACP
/// agents from the app-data registry. This is the dispatch path for sessions.
pub fn lookup_any(app: &tauri::AppHandle, tool_id: &str) -> Option<HarnessToolSpec> {
    if let Some(spec) = lookup(tool_id) {
        return Some(spec);
    }
    let agents = crate::plugins::harness::store::list_custom_agents(app).ok()?;
    let agent = agents.into_iter().find(|a| a.id == tool_id)?;
    Some(HarnessToolSpec {
        id: agent.id,
        name: agent.name,
        spawn_options: vec![],
        kind: HarnessToolKind::Acp {
            command: AcpCommand::Bare {
                command: agent.command,
                args: agent.args,
            },
        },
    })
}

/// Detect availability for every entry (built-ins then custom agents):
/// - `Bare`: the CLI resolves on PATH → ready, else missing.
/// - `Npx`: `npx` itself resolves → installable (the adapter fetches on first
///   spawn, so presence of the package does not change the UX); npx missing → missing.
/// - `ClaudeNative`: a real native binary (never a .cmd/.ps1 shim) → ready with
///   the claude-native adapter; otherwise npx → installable over the ACP fallback.
pub fn detect_tools(app: &tauri::AppHandle) -> Vec<HarnessToolStatus> {
    let npx_available = crate::utils::exec_resolve::resolve_executable("npx").is_some();
    let mut statuses: Vec<HarnessToolStatus> = builtin_tools()
        .into_iter()
        .map(|spec| detect_status(spec, npx_available, false))
        .collect();
    if let Ok(agents) = crate::plugins::harness::store::list_custom_agents(app) {
        for agent in agents {
            let spec = HarnessToolSpec {
                id: agent.id,
                name: agent.name,
                spawn_options: vec![],
                kind: HarnessToolKind::Acp {
                    command: AcpCommand::Bare {
                        command: agent.command,
                        args: agent.args,
                    },
                },
            };
            statuses.push(detect_status(spec, npx_available, true));
        }
    }
    statuses
}

/// Availability for one spec (shared by built-ins and custom agents).
fn detect_status(spec: HarnessToolSpec, npx_available: bool, custom: bool) -> HarnessToolStatus {
    let (state, adapter): (&str, &str) = match &spec.kind {
        HarnessToolKind::Acp { command } => {
            let adapter = "acp";
            let state = match command {
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
            };
            (state, adapter)
        }
        HarnessToolKind::ClaudeNative { .. } => {
            if crate::plugins::harness::claude::detect_native_binary().is_some() {
                ("ready", "claude-native")
            } else if npx_available {
                ("installable", "acp-fallback")
            } else {
                ("missing", "acp-fallback")
            }
        }
        HarnessToolKind::CodexExec { .. } => {
            if crate::utils::exec_resolve::resolve_executable("codex").is_some() {
                ("ready", "codex-native")
            } else if npx_available {
                ("installable", "acp-fallback")
            } else {
                ("missing", "acp-fallback")
            }
        }
    };
    HarnessToolStatus {
        id: spec.id,
        name: spec.name,
        state: state.to_string(),
        adapter: adapter.to_string(),
        custom,
    }
}

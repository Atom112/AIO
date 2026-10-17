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

/// One value of a spawn-time option (CLI-flag/env-backed).
#[derive(Clone, Debug)]
pub struct SpawnOptionValueSpec {
    /// Value id shown to the GUI / persisted as the preference key.
    pub id: String,
    /// Display label (registry-declared, English; the GUI i18n's known ids).
    pub name: String,
    /// false = the tool's default behavior — inject nothing for this value.
    pub inject: bool,
}

/// How a chosen spawn-option value reaches the child process.
#[derive(Clone, Debug)]
pub enum SpawnOptionInject {
    /// CLI argv: injected as two entries (`--approval-mode`, `yolo`) — the
    /// space-separated form every getopt/clap/oclif parser accepts.
    Flag { arg: String },
    /// Environment variable set to the value id (e.g. goose GOOSE_MODE).
    Env { var: String },
}

/// A process-spawn-level option a tool supports but does NOT expose over its
/// wire protocol. Families verified so far:
/// - omp `--approval-mode` (always-ask|write|yolo) — its ACP config options
///   cover mode/model/thinking, but the approval modes are TUI-only flags
///   (verified against omp v17.2.1);
/// - gemini/qwen `--approval-mode` (default|auto_edit|yolo|plan),
///   codex `-s/--sandbox` (read-only|workspace-write|danger-full-access),
///   goose `GOOSE_MODE` env (auto|approve|smart_approve|chat).
///
/// Spawn-time only: the driver injects at process start; changing the value
/// respawns the session (the GUI persists the preference and stops the
/// runtime). Values whose `inject` is false are the tool default.
#[derive(Clone, Debug)]
pub struct SpawnOptionSpec {
    /// Config-option id shown to the GUI (e.g. "approval-mode").
    pub option_id: String,
    /// Display label (registry-declared, English; the GUI i18n's known ids).
    pub name: String,
    /// How the chosen value reaches the child process.
    pub inject: SpawnOptionInject,
    pub values: Vec<SpawnOptionValueSpec>,
}

/// One user-level config file the tool owns (the CLI's own source of truth —
/// AIO reads/writes it directly for near-native parity).
#[derive(Clone, Debug)]
pub struct ConfigFileSlotSpec {
    /// Stable slot id (UI routing key; also the read/write command's slot id).
    pub id: String,
    /// Display label (English; the GUI i18n's known ids).
    pub label: String,
    /// Path template: `~/…` → home dir, `%APPDATA%/…` → roaming app-data
    /// (Windows), `$DSH_HOME/…` → DSH home (falls back to `~/.dsh`).
    pub path: String,
    /// json | jsonc | toml | yaml (validation + editor behavior driver).
    pub format: String,
}

/// Setup/auth surface of a tool: how the user authenticates it, which env var
/// receives an AIO-stored API key at spawn, the workspace instruction file
/// convention, and the home-relative probe paths that indicate a completed
/// setup (drives the tool-settings modal).
#[derive(Clone, Debug, Default)]
pub struct ToolSetupSpec {
    /// Env var that receives the AIO-stored API key at spawn (None = the tool
    /// uses only its own auth flow / config file).
    pub api_key_env: Option<String>,
    /// Official auth/setup docs URL (opened from the tool settings modal).
    pub auth_docs_url: String,
    /// Workspace instruction file convention (CLAUDE.md / AGENTS.md / ...).
    pub instruction_file: String,
    /// Probe paths indicating a completed auth setup: `~/…` resolves to the
    /// home directory, `%APPDATA%/…` to the roaming app-data dir (Windows).
    pub auth_probe_paths: Vec<String>,
    /// User-level config files owned by the tool (settings pages).
    pub config_files: Vec<ConfigFileSlotSpec>,
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
    /// Auth/provider/instruction setup surface (tool settings modal).
    pub setup: ToolSetupSpec,
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

/// Shared `--approval-mode` flag option (gemini / qwen family).
fn approval_mode_flag(values: &[(&str, &str, bool)]) -> SpawnOptionSpec {
    SpawnOptionSpec {
        option_id: "approval-mode".to_string(),
        name: "Approval Mode".to_string(),
        inject: SpawnOptionInject::Flag {
            arg: "--approval-mode".to_string(),
        },
        values: values
            .iter()
            .map(|(id, name, inject)| SpawnOptionValueSpec {
                id: id.to_string(),
                name: name.to_string(),
                inject: *inject,
            })
            .collect(),
    }
}

/// Built-in registry (order = sidebar order).
pub fn builtin_tools() -> Vec<HarnessToolSpec> {
    vec![
        HarnessToolSpec {
            id: "claude".to_string(),
            setup: ToolSetupSpec {
                api_key_env: Some("ANTHROPIC_API_KEY".to_string()),
                auth_docs_url: "https://code.claude.com/docs/en/setup".to_string(),
                instruction_file: "CLAUDE.md".to_string(),
                auth_probe_paths: vec!["~/.claude/.credentials.json".to_string()],
                config_files: vec![ConfigFileSlotSpec {
                    id: "settings".to_string(),
                    label: "User settings".to_string(),
                    path: "~/.claude/settings.json".to_string(),
                    format: "json".to_string(),
                }],
            },
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
            setup: ToolSetupSpec {
                api_key_env: Some("OPENAI_API_KEY".to_string()),
                auth_docs_url: "https://developers.openai.com/codex/cli/".to_string(),
                instruction_file: "AGENTS.md".to_string(),
                auth_probe_paths: vec!["~/.codex/auth.json".to_string()],
                config_files: vec![ConfigFileSlotSpec {
                    id: "config".to_string(),
                    label: "User config".to_string(),
                    path: "~/.codex/config.toml".to_string(),
                    format: "toml".to_string(),
                }],
            },
            name: "Codex".to_string(),
            // Codex exec: `-s/--sandbox` picks the sandbox policy per turn
            // (read-only|workspace-write|danger-full-access — codex exec
            // --help verified). "default" leaves the tool's own config.toml.
            spawn_options: vec![SpawnOptionSpec {
                option_id: "sandbox".to_string(),
                name: "Sandbox".to_string(),
                inject: SpawnOptionInject::Flag {
                    arg: "-s".to_string(),
                },
                values: vec![
                    SpawnOptionValueSpec {
                        id: "default".to_string(),
                        name: "Default".to_string(),
                        inject: false,
                    },
                    SpawnOptionValueSpec {
                        id: "read-only".to_string(),
                        name: "Read-only".to_string(),
                        inject: true,
                    },
                    SpawnOptionValueSpec {
                        id: "workspace-write".to_string(),
                        name: "Workspace write".to_string(),
                        inject: true,
                    },
                    SpawnOptionValueSpec {
                        id: "danger-full-access".to_string(),
                        name: "Full access".to_string(),
                        inject: true,
                    },
                ],
            }],
            kind: HarnessToolKind::CodexExec {
                fallback: AcpCommand::Npx {
                    package: "@agentclientprotocol/codex-acp".to_string(),
                    args: vec![],
                },
            },
        },
        HarnessToolSpec {
            id: "opencode".to_string(),
            setup: ToolSetupSpec {
                api_key_env: None,
                auth_docs_url: "https://opencode.ai/docs/".to_string(),
                instruction_file: "AGENTS.md".to_string(),
                auth_probe_paths: vec!["~/.local/share/opencode/auth.json".to_string()],
                config_files: vec![
                    ConfigFileSlotSpec {
                        id: "global".to_string(),
                        label: "Global config".to_string(),
                        path: "~/.config/opencode/opencode.json".to_string(),
                        format: "json".to_string(),
                    },
                    ConfigFileSlotSpec {
                        id: "global-jsonc".to_string(),
                        label: "Global config (JSONC)".to_string(),
                        path: "~/.config/opencode/opencode.jsonc".to_string(),
                        format: "jsonc".to_string(),
                    },
                ],
            },
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
            setup: ToolSetupSpec {
                api_key_env: None,
                auth_docs_url: "https://omp.sh".to_string(),
                instruction_file: "AGENTS.md".to_string(),
                auth_probe_paths: vec!["~/.omp/agent/config.yml".to_string()],
                config_files: vec![ConfigFileSlotSpec {
                    id: "config".to_string(),
                    label: "Agent config".to_string(),
                    path: "~/.omp/agent/config.yml".to_string(),
                    format: "yaml".to_string(),
                }],
            },
            name: "Oh My Pi".to_string(),
            // omp's ACP surface covers mode/model/thinking, but the approval
            // modes (always-ask|write|yolo) are TUI-only flags — inject them
            // at spawn (probe-verified: `omp acp --approval-mode=yolo` serves
            // ACP normally, oclif global flag).
            spawn_options: vec![SpawnOptionSpec {
                option_id: "approval-mode".to_string(),
                name: "Approval Mode".to_string(),
                inject: SpawnOptionInject::Flag {
                    arg: "--approval-mode".to_string(),
                },
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
            setup: ToolSetupSpec {
                api_key_env: Some("GEMINI_API_KEY".to_string()),
                auth_docs_url: "https://geminicli.com/docs/".to_string(),
                instruction_file: "GEMINI.md".to_string(),
                auth_probe_paths: vec!["~/.gemini/oauth_creds.json".to_string()],
                config_files: vec![ConfigFileSlotSpec {
                    id: "settings".to_string(),
                    label: "User settings".to_string(),
                    path: "~/.gemini/settings.json".to_string(),
                    format: "json".to_string(),
                }],
            },
            name: "Gemini CLI".to_string(),
            // gemini's ACP surface doesn't expose approval modes; the CLI flag
            // does (default|auto_edit|yolo|plan, per the official cli docs).
            spawn_options: vec![approval_mode_flag(&[
                ("default", "Default", false),
                ("auto_edit", "Auto edit", true),
                ("yolo", "YOLO", true),
                ("plan", "Plan", true),
            ])],
            kind: HarnessToolKind::Acp {
                command: AcpCommand::Bare {
                    command: "gemini".to_string(),
                    args: vec!["--acp".to_string()],
                },
            },
        },
        HarnessToolSpec {
            id: "qwen".to_string(),
            setup: ToolSetupSpec {
                api_key_env: Some("DASHSCOPE_API_KEY".to_string()),
                auth_docs_url:
                    "https://qwenlm.github.io/qwen-code-docs/en/users/configuration/auth/"
                        .to_string(),
                instruction_file: "QWEN.md".to_string(),
                auth_probe_paths: vec!["~/.qwen/oauth_creds.json".to_string()],
                config_files: vec![ConfigFileSlotSpec {
                    id: "settings".to_string(),
                    label: "User settings".to_string(),
                    path: "~/.qwen/settings.json".to_string(),
                    format: "json".to_string(),
                }],
            },
            name: "Qwen Code".to_string(),
            // Qwen Code is a gemini-cli fork — same --approval-mode surface.
            spawn_options: vec![approval_mode_flag(&[
                ("default", "Default", false),
                ("auto_edit", "Auto edit", true),
                ("yolo", "YOLO", true),
                ("plan", "Plan", true),
            ])],
            kind: HarnessToolKind::Acp {
                command: AcpCommand::Bare {
                    command: "qwen".to_string(),
                    args: vec!["--acp".to_string()],
                },
            },
        },
        HarnessToolSpec {
            id: "goose".to_string(),
            setup: ToolSetupSpec {
                api_key_env: None,
                auth_docs_url: "https://block.github.io/goose/docs/getting-started/providers"
                    .to_string(),
                instruction_file: ".goosehints".to_string(),
                auth_probe_paths: vec![
                    "~/.config/goose/config.yaml".to_string(),
                    "%APPDATA%/Block/goose/config/config.yaml".to_string(),
                ],
                config_files: {
                    let dir = if cfg!(windows) {
                        "%APPDATA%/Block/goose/config"
                    } else {
                        "~/.config/goose"
                    };
                    vec![
                        ConfigFileSlotSpec {
                            id: "config".to_string(),
                            label: "Config".to_string(),
                            path: format!("{dir}/config.yaml"),
                            format: "yaml".to_string(),
                        },
                        ConfigFileSlotSpec {
                            id: "permission".to_string(),
                            label: "Permissions".to_string(),
                            path: format!("{dir}/permission.yaml"),
                            format: "yaml".to_string(),
                        },
                    ]
                },
            },
            name: "goose".to_string(),
            // goose modes are configured via the GOOSE_MODE env var (no CLI
            // flag): auto|approve|smart_approve|chat (per the goose docs).
            spawn_options: vec![SpawnOptionSpec {
                option_id: "goose-mode".to_string(),
                name: "Permission Mode".to_string(),
                inject: SpawnOptionInject::Env {
                    var: "GOOSE_MODE".to_string(),
                },
                values: vec![
                    SpawnOptionValueSpec {
                        id: "auto".to_string(),
                        name: "Autonomous".to_string(),
                        inject: false,
                    },
                    SpawnOptionValueSpec {
                        id: "approve".to_string(),
                        name: "Manual approval".to_string(),
                        inject: true,
                    },
                    SpawnOptionValueSpec {
                        id: "smart_approve".to_string(),
                        name: "Smart approval".to_string(),
                        inject: true,
                    },
                    SpawnOptionValueSpec {
                        id: "chat".to_string(),
                        name: "Chat only".to_string(),
                        inject: true,
                    },
                ],
            }],
            kind: HarnessToolKind::Acp {
                command: AcpCommand::Bare {
                    command: "goose".to_string(),
                    args: vec!["acp".to_string()],
                },
            },
        },
        HarnessToolSpec {
            id: "dsh".to_string(),
            setup: ToolSetupSpec {
                api_key_env: None,
                auth_docs_url: "https://deepseek-ai.github.io/dsh/".to_string(),
                instruction_file: "AGENTS.md".to_string(),
                auth_probe_paths: vec!["~/.dsh".to_string()],
                config_files: vec![ConfigFileSlotSpec {
                    id: "settings".to_string(),
                    label: "User settings".to_string(),
                    path: "$DSH_HOME/settings.yaml".to_string(),
                    format: "yaml".to_string(),
                }],
            },
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
        setup: ToolSetupSpec {
            instruction_file: "AGENTS.md".to_string(),
            ..Default::default()
        },
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
                setup: ToolSetupSpec {
                    instruction_file: "AGENTS.md".to_string(),
                    ..Default::default()
                },
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

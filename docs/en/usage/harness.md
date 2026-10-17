# Harness Workbench (CLI Tool Compatibility Layer)

The harness workbench turns AIO into a GUI compatibility layer for CLI coding agents (Claude Code, Codex, OpenCode, Oh My Pi, Gemini CLI, Qwen Code, goose, DSH, ...): AIO drives those CLI tools and renders their message stream, chain of thought, tool calls and file changes with its own native UI.

## Boundary: what AIO does here

- **AIO provides**: launching and supervising the CLI tool processes, mapping their event streams to the native UI, forwarding user input and approval decisions, and persisting a display copy of the conversation locally.
- **AIO does NOT provide**: in this scenario AIO's own tools, sandbox, memory, skills and subagents are never invoked. Tool execution and permission enforcement belong entirely to the selected CLI tool (its own sandbox modes, permission prompts or `--yolo`-style config).
- AIO injects no system prompt and rewrites nothing — the conversation passes through verbatim.

## Page structure

The page mirrors the chat page; enter via the terminal icon in the navigation bar:

- **Left two-level sidebar**: level 1 lists the detected CLI tools (each with its brand logo and a state dot: green = ready, yellow = auto-installable, grey = not installed; hover a tool row to fold/unfold its workspace list, add a workspace, or delete a custom tool); level 2 lists the workspace directories registered under each tool — every tool's workspace list folds/unfolds independently like the chat page's child topics, the fold state survives restarts, and adding a workspace auto-unfolds its tool.
- **Center column**: the status bar shows tool / workspace, run state (connecting / running / idle / error) and token readouts; message bubbles reuse the chat page components (Markdown, thinking block, agent process block, diff view).
- **Right sidebar**: the topic list of the current (tool × workspace) with create / rename / delete.

Topics start as "New topic" and auto-rename to the head of the first message; manual rename is always available.

## Supported CLI tools and installation

Node.js 20+ (`npx` on PATH) is required for everything except DSH. Tools marked "auto-installable" fetch their adapter via `npx -y` inside AIO on first use — the first launch may take tens of seconds to download.

| Tool        | Integration                                                                                         | Install command (run in a terminal)                                                                                                                                                                                                                                                                                                        |
| ----------- | --------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Claude Code | native driver first; falls back to the official ACP adapter (npm) when no native binary is detected | Recommended: `irm https://claude.ai/install.ps1 \| iex` (official installer, auto-updating); or `npm i -g @anthropic-ai/claude-code` (**also served by the native driver** — AIO locates the real binary inside the package). Both need `claude` login, done by yourself; the login state (`~/.claude`) is shared between the two installs |
| Codex       | native driver first (`codex` CLI); falls back to the official ACP adapter (npm)                     | `npm i -g @openai/codex` (or winget `OpenAI.Codex`), then complete `codex` login in a terminal; or `npm i -g @agentclientprotocol/codex-acp` (login still required)                                                                                                                                                                        |
| OpenCode    | native ACP (`opencode acp`)                                                                         | `npm i -g opencode-ai` or `curl -fsSL https://opencode.ai/install \| bash`                                                                                                                                                                                                                                                                 |
| Oh My Pi    | native ACP (`omp acp`)                                                                              | `irm https://omp.sh/install.ps1 \| iex`                                                                                                                                                                                                                                                                                                    |
| Gemini CLI  | native ACP (`gemini --acp`)                                                                         | `npm i -g @google/gemini-cli`                                                                                                                                                                                                                                                                                                              |
| Qwen Code   | native ACP (`qwen --acp`)                                                                           | `npm i -g @qwen-code/qwen-code`                                                                                                                                                                                                                                                                                                            |
| goose       | native ACP (`goose acp`)                                                                            | see the goose install docs                                                                                                                                                                                                                                                                                                                 |
| DSH         | native ACP (`dsh --profile acp`)                                                                    | `npm i -g @deepseek-ai/dsh`                                                                                                                                                                                                                                                                                                                |

crush (no ACP, no machine-readable output) and aider (text-only, no session resume, stalled upstream) are not supported.

### Custom ACP tools

Any ACP-speaking CLI can be registered as a harness tool: click "Add custom ACP tool" at the bottom of the left sidebar and fill in a display name, the command, and its arguments (one per line, e.g. `my-agent` + `acp`). Custom tools are full peers of the built-ins (workspaces / topics / approvals / resume) and can be deleted any time. You author the command line yourself — the same trust boundary as MCP stdio servers.

## Workflow

1. Select a tool whose state is "Ready" (or "Auto-installable") on the left.
2. Click "Add workspace" and register a directory via the folder picker or the AIO project list.
3. Create a topic — selecting it connects to the CLI tool immediately (process spawn + protocol handshake, no message needed), and its config controls appear in the input bar right away. Send a message. The tool's progress (text, thinking, tool calls, file diffs) streams into the message area; when the tool pushes an execution plan, a collapsible task list appears under the status bar; while you are on other pages, running tools show up in the chat status-bar background-task menu.
4. When the turn finishes, the reply is persisted locally in AIO; reopen the topic any time to review the full record.

### Permission approvals

When the selected tool asks to run a sensitive action, an approval bubble appears above the input: tool name, argument preview, a diff preview of the affected file, and the options the tool offered (typically "Allow once / Always allow / Reject / Always reject"). Approvals have no timeout — the tool waits for your decision; navigating away does not interrupt the wait.

Claude Code (native integration) offers "Allow once / Reject" (its protocol has no always tier); the status bar additionally provides a permission-mode dropdown: Default (ask) / Accept edits / Plan / Don't ask / Full auto. The mode switches live for a running session and becomes the default for the next new session.

Codex (native integration) has no approval interaction — headless exec mode never asks; sandbox and approval behavior come from `~/.codex/config.toml` (`approval_policy` / `sandbox_mode`). No approval bubbles appear for Codex on this page.

### Model and thinking level

The input bar's bottom-left shows the selected tool's config controls (only those the tool actually exposes are rendered; a tool you have used before shows its last reported options immediately, native Claude Code / Codex show presets even on first use, and an ACP tool that has never run on this machine reveals its options after its first session):

- **ACP tools** (OpenCode / Oh My Pi / Gemini CLI / Qwen Code / goose / DSH / custom ACP tools, plus Claude Code / Codex in their ACP-fallback mode): the dropdowns come from the options the tool itself reports at session setup; picking one dispatches to the tool immediately — per the protocol, changes take effect from the next turn. Probe-verified for Oh My Pi: it reports **Mode** (default / plan — read-only planning), **Model** (hundreds of catalog entries, labeled `provider · model` in the dropdown so the source is visible), and **Thinking** (off / auto / high).
- **Claude Code (native)**: the model dropdown switches live (the `set_model` control frame); the thinking level is a spawn flag (`--effort`) — changing it restarts the session process and resumes the conversation on the next send.
- **Codex (native)**: model and thinking level apply per turn (every send is a fresh `codex exec` process with `-m` / `-c model_reasoning_effort=`); the model list is a preset of common models.

Beyond protocol options, some tools carry CLI-flag-only switches — AIO maps them as "spawn-time option" dropdowns (changing one restarts the session to apply):

- **Oh My Pi: Approval mode** — Always ask (default) / Write-gated (writes need confirmation) / Full access (auto-approve everything), backed by omp's `--approval-mode` flag (always-ask | write | yolo). The ACP channel does not expose this surface; AIO injects the flag at session start.
- **Gemini CLI / Qwen Code: Approval mode** — Default (always ask) / Auto-edit / Full auto / Plan, backed by `--approval-mode` (default | auto_edit | yolo | plan).
- **goose: Permission mode** — Autonomous (default) / Manual approval / Smart approval / Chat only, backed by the `GOOSE_MODE` environment variable (auto | approve | smart_approve | chat).
- **Codex: Sandbox** — Tool default / Read-only / Workspace write / Full access, backed by `codex exec -s` (read-only | workspace-write | danger-full-access), applied per turn.

Preferences (native-adapter model / thinking level, spawn-time options) persist per tool across AIO restarts.

### Tool setup (auth and API keys)

Hover a tool row's gear icon to open its settings modal:

- **Auth status**: AIO probes the tool's own config directory (e.g. `~/.claude/.credentials.json`, `~/.codex/auth.json`) and shows whether the official login/setup is done.
- **API key** (Claude Code / Codex / Gemini CLI / Qwen Code): set the provider's API key — stored in the OS keychain (never written into the tool's own files) and injected as the matching environment variable (`ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `GEMINI_API_KEY` / `DASHSCOPE_API_KEY`) when AIO spawns a session for that tool; applies to new sessions.
- **Official docs**: one click opens the tool's auth/configuration guide.

Oh My Pi / OpenCode / goose / DSH use their own login or configuration systems (no API-key injection); the modal shows detection status and the docs link.

### Workspace instructions (CLAUDE.md / AGENTS.md / ...)

Each CLI tool reads a convention instruction file at the workspace root. Hover a workspace row's edit icon to view/edit that file without leaving AIO (saving creates it when absent):

| Tool                                             | Instruction file |
| ------------------------------------------------ | ---------------- |
| Claude Code                                      | `CLAUDE.md`      |
| Codex / OpenCode / Oh My Pi / DSH / custom tools | `AGENTS.md`      |
| Gemini CLI                                       | `GEMINI.md`      |
| Qwen Code                                        | `QWEN.md`        |
| goose                                            | `.goosehints`    |

### Stop and resume

- While running, "Stop" (status bar or input area) issues a protocol-level cancel, escalating to a hard process-tree kill after a grace window.
- Each CLI tool keeps its own session; AIO records its session id. Sending again after a stop resumes the same session (when the tool supports resume); the same applies across app restarts. Tools without resume start a fresh session on the next turn — AIO's local history remains visible either way.
- Slash commands pass through verbatim to the tool's own command system. Some tools' interactive commands may hang in headless mode — use "Stop" when that happens.

## Tool settings pages (Settings → CLI tools)

A "CLI tools" section is added to the settings page (list + a second-level page per tool). Tool configuration uses **the tool's own config files as the single source of truth** — AIO reads and writes the very files the CLI reads (`~/.claude/settings.json`, `~/.codex/config.toml`, `~/.config/opencode/opencode.json(c)`, `~/.omp/agent/config.yml`, `~/.gemini/settings.json`, `~/.qwen/settings.json`, goose's `config.yaml`/`permission.yaml`, DSH's `settings.yaml`).

Each tool page contains:

- **Overview**: install/auth state, config-file probes (path/exists/backup mark), DSH's read-only profile stack;
- **Common config** (all 8 tools): a structured form over the high-frequency keys, dispatched by the tool's config format —
  - **JSON tools** (Claude Code / Gemini CLI / Qwen Code / OpenCode): default model, permission mode (claude), approval mode (gemini/qwen), always-thinking, permission rules (allow/ask/deny chips), env pairs; every change writes settings.json directly (other keys preserved; clearing removes the key → default). Disabled automatically when OpenCode runs on JSONC;
  - **Codex** (TOML via toml_edit, comments preserved): default model, approval policy, sandbox mode, context window, notify command — comments and table layout stay byte-faithful;
  - **goose** (YAML line surgery): provider, default model, tool execution mode, temperature, max tokens/turns, toolshim — only the target `KEY: value` line is touched; every other line (comments included) stays byte-identical;
  - **Oh My Pi** (the **official channel**): `omp config list --json` drives the **full settings catalog** (448 keys with types and descriptions) rendered as collapsible groups + a filter box; writes go through `omp config set` (omp's own parser validates — enum errors, valid values included, surface verbatim) and resets through `omp config reset` — perfectly in sync with the CLI, zero schema reimplementation. The section hides when omp is not installed (raw editor still serves);
- **Credentials**: auth detection + the API key (OS keychain, injected at spawn) + official docs;
- **Advanced config**: a validated raw editor per config file — content is re-parsed by format before saving (JSON/JSONC/TOML/YAML; parse failures are refused), the pre-AIO state is **backed up once** (`<file>.aio-bak`, one-click restore), and a staleness guard rejects writes when the file changed externally while editing.

Trust boundary: only user-level files are written (never managed/enterprise layers or credential files); secrets never land in config files.

Each tool page also provides:

- **MCP servers**: manage the MCP servers in the tool's own config file — claude `~/.claude.json` `mcpServers` (new slot), gemini/qwen `settings.json` `mcpServers`, opencode `mcp` (local/remote), codex `config.toml` `[mcp_servers.<name>]`, goose `extensions` — list (stdio command or HTTP URL preview) / add / edit (stdio: command + args per line + env vars; HTTP: URL) / delete, written in each tool's native shape (name whitelist blocks path escapes; codex rides toml_edit comment preservation; goose rewrites with a backup notice). omp/dsh MCP goes through the raw editors;
- **Instructions & skills**: the tool's own global instruction files (claude `~/.claude/CLAUDE.md`, codex `~/.codex/AGENTS.md`, gemini `~/.gemini/GEMINI.md`, qwen `~/.qwen/QWEN.md`, opencode `~/.config/opencode/AGENTS.md`, omp `~/.omp/AGENTS.md` — served by the backed-up advanced editor) plus **markdown collection management**: claude skills (`~/.claude/skills/<name>/SKILL.md`, directory-shaped), goose prompt templates (`prompts/*.md`), opencode custom commands (`commands/*.md`) — list/create/edit/delete with name validation (letters/digits/hyphens, no path escapes), atomic writes and one-time pre-overwrite backups. Workspace-level instruction files stay editable from the harness page's workspace rows.

## Data and privacy

- The display record is stored in AIO's local database (`harness_*` tables) purely for review and topic management; nothing is sent to any remote.
- The CLI tools' own session files, credentials and configs live in their default directories (`~/.claude`, `~/.codex`, `~/.omp`, ...); AIO neither reads nor stores those credentials.
- Child-process environment variables pass through a whitelist (PATH, HOME/USERPROFILE, proxies, ...); secrets stored in AIO are never injected.

## Troubleshooting

- **"Node.js not found"**: install Node 20+, restart AIO, retry.
- **First launch is slow**: `npx` is downloading the adapter package (Claude Code / Codex) — expected; subsequent launches use the cache.
- **Stuck on "Connecting"**: verify the CLI runs and is logged in from a terminal (`claude` / `codex login`); check the `[harness:...]` lines in the AIO log.
- **Windows "command not found"**: ensure the install directory is on PATH and restart AIO (the GUI process inherits a minimal environment at launch).

# Harness Workbench (CLI Tool Compatibility Layer)

The harness workbench turns AIO into a GUI compatibility layer for CLI coding agents (Claude Code, Codex, OpenCode, Oh My Pi, Gemini CLI, Qwen Code, goose, DSH, ...): AIO drives those CLI tools and renders their message stream, chain of thought, tool calls and file changes with its own native UI.

## Boundary: what AIO does here

- **AIO provides**: launching and supervising the CLI tool processes, mapping their event streams to the native UI, forwarding user input and approval decisions, and persisting a display copy of the conversation locally.
- **AIO does NOT provide**: in this scenario AIO's own tools, sandbox, memory, skills and subagents are never invoked. Tool execution and permission enforcement belong entirely to the selected CLI tool (its own sandbox modes, permission prompts or `--yolo`-style config).
- AIO injects no system prompt and rewrites nothing — the conversation passes through verbatim.

## Page structure

The page mirrors the chat page; enter via the terminal icon in the navigation bar:

- **Left two-level sidebar**: level 1 lists the detected CLI tools (state dot: green = ready, yellow = auto-installable, grey = not installed); level 2 lists the workspace directories registered under the selected tool.
- **Center column**: the status bar shows tool / workspace, run state (connecting / running / idle / error) and token readouts; message bubbles reuse the chat page components (Markdown, thinking block, agent process block, diff view).
- **Right sidebar**: the topic list of the current (tool × workspace) with create / rename / delete.

Topics start as "New topic" and auto-rename to the head of the first message; manual rename is always available.

## Supported CLI tools and installation

Node.js 20+ (`npx` on PATH) is required for everything except DSH. Tools marked "auto-installable" fetch their adapter via `npx -y` inside AIO on first use — the first launch may take tens of seconds to download.

| Tool        | Integration                                                                       | Install command (run in a terminal)                                                                                                                                                                                                              |
| ----------- | --------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Claude Code | native driver first (native binary); falls back to the official ACP adapter (npm) | Recommended: `irm https://claude.ai/install.ps1 \| iex` (native build, fullest features); or `npm i -g @agentclientprotocol/claude-agent-acp` (Node 22+, feature subset). Both need `claude` login or `ANTHROPIC_API_KEY` configured by yourself |
| Codex       | official ACP adapter (npm)                                                        | `npm i -g @agentclientprotocol/codex-acp` (complete `codex` login yourself)                                                                                                                                                                      |
| OpenCode    | native ACP (`opencode acp`)                                                       | `npm i -g opencode-ai` or `curl -fsSL https://opencode.ai/install \| bash`                                                                                                                                                                       |
| Oh My Pi    | native ACP (`omp acp`)                                                            | `irm https://omp.sh/install.ps1 \| iex`                                                                                                                                                                                                          |
| Gemini CLI  | native ACP (`gemini --acp`)                                                       | `npm i -g @google/gemini-cli`                                                                                                                                                                                                                    |
| Qwen Code   | native ACP (`qwen --acp`)                                                         | `npm i -g @qwen-code/qwen-code`                                                                                                                                                                                                                  |
| goose       | native ACP (`goose acp`)                                                          | see the goose install docs                                                                                                                                                                                                                       |
| DSH         | native ACP (`dsh --profile acp`)                                                  | `npm i -g @deepseek-ai/dsh`                                                                                                                                                                                                                      |

crush (no ACP, no machine-readable output) and aider (text-only, no session resume, stalled upstream) are not supported.

## Workflow

1. Select a tool whose state is "Ready" (or "Auto-installable") on the left.
2. Click "Add workspace" and register a directory via the folder picker or the AIO project list.
3. Create a topic and send a message. The tool's progress (text, thinking, tool calls, file diffs) streams into the message area.
4. When the turn finishes, the reply is persisted locally in AIO; reopen the topic any time to review the full record.

### Permission approvals

When the selected tool asks to run a sensitive action, an approval bubble appears above the input: tool name, argument preview, a diff preview of the affected file, and the options the tool offered (typically "Allow once / Always allow / Reject / Always reject"). Approvals have no timeout — the tool waits for your decision; navigating away does not interrupt the wait.

Claude Code (native integration) offers "Allow once / Reject" (its protocol has no always tier); the status bar additionally provides a permission-mode dropdown: Default (ask) / Accept edits / Plan / Don't ask / Full auto. The mode switches live for a running session and becomes the default for the next new session.

### Stop and resume

- While running, "Stop" (status bar or input area) issues a protocol-level cancel, escalating to a hard process-tree kill after a grace window.
- Each CLI tool keeps its own session; AIO records its session id. Sending again after a stop resumes the same session (when the tool supports resume); the same applies across app restarts. Tools without resume start a fresh session on the next turn — AIO's local history remains visible either way.
- Slash commands pass through verbatim to the tool's own command system. Some tools' interactive commands may hang in headless mode — use "Stop" when that happens.

## Data and privacy

- The display record is stored in AIO's local database (`harness_*` tables) purely for review and topic management; nothing is sent to any remote.
- The CLI tools' own session files, credentials and configs live in their default directories (`~/.claude`, `~/.codex`, `~/.omp`, ...); AIO neither reads nor stores those credentials.
- Child-process environment variables pass through a whitelist (PATH, HOME/USERPROFILE, proxies, ...); secrets stored in AIO are never injected.

## Troubleshooting

- **"Node.js not found"**: install Node 20+, restart AIO, retry.
- **First launch is slow**: `npx` is downloading the adapter package (Claude Code / Codex) — expected; subsequent launches use the cache.
- **Stuck on "Connecting"**: verify the CLI runs and is logged in from a terminal (`claude` / `codex login`); check the `[harness:...]` lines in the AIO log.
- **Windows "command not found"**: ensure the install directory is on PATH and restart AIO (the GUI process inherits a minimal environment at launch).

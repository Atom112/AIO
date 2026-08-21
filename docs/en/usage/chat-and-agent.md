# Chat and Agent

## Assistants, Topics, and Projects

- Assistants store a name, system prompt, preferred model, and enabled MCPs/Skills.
- Topics store messages, auto-generated titles, summaries, token data, Agent steps, and message branching relationships.
- Projects bind to a local directory and create a project assistant along with `.aio/` configuration.

A project is not a copy of the directory. The Agent's file, Shell, Git, LSP, and built-in file tools operate directly on the bound directory; before switching projects or Git branches, verify that you have no unsaved changes.

### Creating a Project

Use the "+" button at the bottom of the left sidebar to open the `ProjectCreateModal`, which contains a name input and a directory browser. The path is validated on input to ensure the directory is usable. The modal has enter/exit animations.

### Editing a Project

The `ProjectSettingsModal` supports changing the project name, binding a preferred model, and enabling/disabling MCP Servers and Skills. Once changes are confirmed, the corresponding configuration files in the project's `.aio/` directory are updated accordingly.

## Conversation Controls

The chat input area provides the following controls:

- **Model**: Select a remote or local model for the current assistant.
- **Reasoning Intensity**: Off, Low, Medium, High. AIO injects corresponding prompts, and when the Provider returns native `reasoning_content`, it is displayed separately.
- **Web Search**: Allows the Agent to use `web_search` and `web_fetch`. Normal chat only receives a web search prompt and does not automatically get project tools.
- **Agent Mode**: Switch between Chat, Normal, Auto, Plan, and Workflow.

`Ctrl+Shift+R` cycles through reasoning intensity levels, `Ctrl+Shift+S` toggles web search, and `Esc` stops the current generation. Settings are persisted in the local WebView.

When the model catalog provides context window information, the interface shows several token statistics components:

- **SessionStats**: Total session input tokens, output tokens, tool call count, and context window usage percentage.
- **TokenBar**: Context window progress bar with color coding (green -> yellow -> red), showing used tokens / max tokens in real time.
- **TokenStatsBar**: Summary bar showing total tokens, request count, and estimated cost.

When usage reaches approximately 75% and there are more than 10 messages, AIO summarizes older messages while keeping the most recent messages within a token budget (about 20k tokens, at least one full turn); the new summary fully replaces the old one to avoid bloat. You can also use `/compact` to compress manually.

Components are located in `src/features/chat/components/` (`SessionStats.tsx`, `TokenBar.tsx`, `TokenStatsBar.tsx`).

## Messages and Topics

- **Edit**: Editing a user message deletes it and the immediately following assistant reply, places the original text back in the input box, and re-sends it on confirmation.
- **Delete**: Deleting a single message also updates the corresponding record in SQLite.
- **Branch**: Create a new topic from a specific message; the original topic remains unchanged.
- **Auto-title**: A title is generated after the first round of replies completes; if the request fails, a locally truncated version of the first user message is used as the fallback title.
- **Stop**: Terminates the current stream and Agent loop. Already produced text, steps, and file changes are retained.

### Sidebar Collapse

Both sidebars support collapsing to expand the message area:

- **Left Sidebar** (`ProjectSidebar`): Collapses to a 48px icon bar. Hovering reveals the project list and conversation entries. Toggle with the collapse button or the `Ctrl+B` shortcut. The `isCollapsed` property controls the collapsed state.
- **Right Sidebar** (`TopicSidebar`): `isCollapsed` controls a transition animation from `0%` to `${width}%`. Toggle with the collapse button or the `Ctrl+Alt+B` shortcut.

## Agent Modes

| Mode     | Reading and Analysis                                                      | Write File / Git Changes                                              | Shell and Other Tools                                                                 |
| -------- | ------------------------------------------------------------------------- | --------------------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| Chat     | No Agent tools available                                                  | Denied                                                                | Denied                                                                                |
| Normal   | File search, reading, Git status, etc. allowed by default                 | Writing, deletion, `git add`, `git commit`, etc. require confirmation | Tools not explicitly allowed require confirmation                                     |
| Auto     | Regular read/write, Git changes, and command execution allowed by default | File deletions still require confirmation                             | Still constrained by path sandbox, dangerous command detection, and custom deny rules |
| Plan     | Read, search, Git status/Diff/Log allowed                                 | Writing, directory creation, deletion, and Git changes denied         | Web tools denied; other MCP tools evaluated per rules                                 |
| Workflow | Reads and executes by split steps                                         | Regular writes execute automatically, deletions require confirmation  | Suitable for well-defined multi-step tasks with automated verification                |

### Workflow Mode

Workflow mode splits a task into independent steps and executes them serially or in parallel. The interface shows a horizontal pipeline diagram (`WorkflowVisualization` component) displaying step names, status icons (pending -> running -> completed -> failed), and elapsed time. Suitable for well-defined multi-step tasks with automated acceptance criteria.

#### Differences from Plan Mode

- **Plan Mode**: Researches and outputs a plan but does not execute it.
- **Workflow Mode**: Automatically splits steps and executes them; regular writes execute automatically, deletions require confirmation.

Auto and Workflow modes significantly reduce interactive confirmation but are not a bypass for security boundaries. `deny` rules in `.aio/permissions.json` take precedence over `allow` rules, and file paths remain constrained to the project root directory.

![Agent File Diff and Approval](../../assets/screenshots/agent-diff-approval.webp)

## Agent Built-in Tools

Agent mode provides several built-in tools. In addition to file, Git, and search tools, it also supports a command execution tool:

### execute_command

- **Purpose**: Execute system commands in the project directory, returning stdout, stderr, and the exit code.
- **Permission Model**: Requires approval in `Normal` mode, auto-allowed in `Auto`/`Workflow` mode, denied in `Chat`/`Plan` mode.
- **Security**: Child processes are isolated via a Windows Job Object sandbox (`src-tauri/src/utils/sandbox.rs`); dangerous command detection (`shell_tools::check_dangerous_command`) catches destructive, installation, and network operations.
- **Dangerous Command Categories**:
  - Destructive: `rm`/`del`/`format`;
  - Installation: `curl`/`wget`/`pip`/`npm`;
  - Network: `ssh`/`nc`, etc.
- **Sub-agent Permissions**: Different roles have different allowances for `execute_command` -- `general`/`debugger`/`tester` are allowed, others are denied.

> See the full tool list in [Agent Tools Reference](agent-tools.md), including parameter descriptions, return values, and default permissions for each tool.

## Agent Process and File Changes

Agent replies display steps including reasoning, tool calls, sub-agents, and elapsed time. Steps, tool results, and final text are written to the message record and persist after app restart; overly long results are truncated in the UI to avoid bloating the message area.

Before file tool execution, a unified Diff can be shown in the approval bubble. After execution, a summary below the reply shows added/deleted lines per file and allows expanding the Diff.

In Git projects, you can revert changes for individual files or the entire batch. Revert works by restoring the target files to `HEAD`:

> [!CAUTION]
> Revert discards any uncommitted changes in the same file that were not part of the Agent's modification. Check `git status` and Diff before reverting; non-Git projects do not show the full revert option.

When a tool fails, the app configuration enables automatic retries by default: up to 2 retries with a 500 ms interval. Permission denials, user cancellations, and errors that should not be retried are not treated as successful results.

### DiffView Component

`DiffView` (`src/shared/components/DiffView.tsx`) parses Diff content with unified formatting and color-coded hunk headers. Supports expanding/collapsing change blocks, shows added/deleted line counts per file, and includes a file-level revert button.

## Git and LSP

- Git branches can be viewed, refreshed, and switched at the top of the project view.
- The Agent can use status, Diff, log, stage, and commit tools; approval depends on the mode and permission rules.
- LSP automatically detects available language servers based on project files and displays diagnostics in the problems panel.

The `ProblemsPanel` displays diagnostics grouped by severity (Error / Warning / Info / Hint), with color-coded icons per group. Supports filtering by severity and grouping by file. Clicking a diagnostic entry navigates to the corresponding file and line number. Panel visibility is controlled by the `problemsPanelVisible` state signal.

- LSP startup failures do not block normal file reading, searching, or Git operations.

## Cross-Session Project Knowledge

When cross-session memory is enabled in "Settings -> App Settings", the Agent gains:

- `remember`: Save `decision`, `pattern`, `convention`, or `note`;
- `recall`: Search by keywords and optional category.

Entries are stored in `.aio/knowledge.json`, with a maximum of 50 entries, and are injected into subsequent project conversations. They are not shared across projects and are not automatically deleted when the toggle is turned off. See [App Settings and Shortcuts](app-settings-and-shortcuts.md#cross-session-memory) for detailed settings.

## Sub-agents

AIO includes built-in roles for exploration, implementation, general tasks, architecture, diagnosis, review, documentation, testing, and requirements analysis. Each role has its own system prompt and tool allow/deny list, and cannot create further sub-agents.

The main Agent has two delegation methods:

- `delegate_task`: Runs a single sub-task;
- `delegate_tasks`: Submits 1-10 mutually independent sub-tasks at once, running in parallel.

Batch delegation produces only one approval in non-auto modes. The current effective concurrency limit is 5; tasks exceeding this limit queue up. The backend type reserves `maxConcurrentSubagents`, but the settings page and disk configuration do not yet support persistent modification.

In "Settings -> Sub-agent Model", you can:

- Set cloud model overrides for built-in or custom roles;
- Create, modify, and delete custom roles;
- Restrict which tools a role can use.

When no override is set, sub-agents inherit the main Agent's model. Local models are only accessible through inheritance and do not appear in the sub-agent cloud model override list.

## Attachments

The chat input supports:

- **Images**: PNG, JPG, JPEG, WebP; single file under 10 MiB;
- **Documents**: PDF, DOCX, PPTX; single file under 30 MiB;
- **Text**: TXT, MD, JSON, CSV, LOG, XML, YAML, YML, INI, TSV; single file under 5 MiB.

Images are converted to data URIs; PDF, Office, and text files have their content extracted. Attachments are deduplicated by SHA-256 and stored in the app data directory. They are cleaned up when the associated message, topic, or assistant is deleted.

## Slash Commands

Typing `/` opens the `SlashCommandMenu` component, which shows a fuzzy-search menu. Arrow keys move the highlight, `Enter` inserts only the command name into the input box (press `Enter` again to send), `Tab` inserts and selects, and `Esc` closes the menu. The menu uses Portal positioning to ensure correct placement during scrolling or complex layouts.

### Command Categories

**Prompt Class** (`/review`, `/explain`, `/fix`, `/optimize`, `/translate`, `/summarize`): Each command is associated with a `promptBody` template containing a `$ARGUMENTS` placeholder. After selection, the full Agent pipeline is invoked and the result is written to the conversation history. Prompt class commands accept optional arguments; when no arguments are provided, the current project path is automatically injected.

**Action Class** (`/clear`, `/compact`, `/search`, `/settings`, `/help`): Execute JavaScript handlers directly, without template invocation.

**Special** (`/btw`): Uses an independent streaming request that does not invoke tools, write to topic history, or interrupt the currently running main task.

### Detailed Behavior

| Command        | Detailed Behavior                                                                                                    |
| -------------- | -------------------------------------------------------------------------------------------------------------------- |
| `/clear`       | Displays a confirmation dialog before clearing the current topic history                                             |
| `/compact`     | Automatically triggered at 75% token usage + more than 10 messages; also supports manual invocation                  |
| `/review`      | Reviews code changes and generates a review report                                                                   |
| `/explain`     | Explains code logic in the current context                                                                           |
| `/fix`         | Analyzes and fixes specified issues                                                                                  |
| `/optimize`    | Optimizes code performance or readability                                                                            |
| `/translate`   | Target language depends on UI locale: `en` -> English, `zh` -> Chinese, not hardcoded                                |
| `/summarize`   | Summarizes the current topic content                                                                                 |
| `/search`      | Toggles the web search switch                                                                                        |
| `/settings`    | Opens the settings page                                                                                              |
| `/help`        | Dynamically generates a command list from the command registry                                                       |
| `/btw <query>` | 60s connection timeout / 120s stream timeout; uses a dedicated system prompt that prohibits continuing the main task |

## Usage Statistics

View LLM call data under "Settings -> Usage".

- **Usage Summary** (`UsageSummaryCards`): Total tokens, total requests, estimated cost, active days;
- **Heatmap** (`UsageHeatmap`): 365-day GitHub-style contribution heatmap showing daily request volume;
- **Model Breakdown** (`ModelBreakdown`): Bar chart of token usage and request counts per model;
- **Time Range**: 7 days / 30 days / 90 days / All;
- **Data Source**: The backend reads from the `usage_log` table via `get_usage_summary` and `get_usage_summary_by_model` Tauri commands;
- **i18n**: All UI labels are under the `usage.*` namespace.

## Markdown Rendering

Code blocks in chat display language icons:

- **Source**: Catppuccin VSCode Icons (MIT license), supporting 40+ language SVG icons;
- **CSS Variables**: Defined in `src/index.css` (`:root`-level `--vscode-ctp-*` variables);
- **Icon Mapping**: The language-to-icon mapping table is located in `src/shared/components/Markdown.tsx` (line 25+).

### Reasoning Content Display

When the Provider returns native `reasoning_content`, the reasoning is displayed via the `ThinkBlock` component (`src/shared/components/ThinkBlock.tsx`):

- Auto-expands during streaming, showing reasoning content in real time;
- Collapses to a summary line after generation completes, showing reasoning duration;
- Users can manually expand/collapse the reasoning content.

## Share and Export

### Entry Points

- **Current Topic**: The share icon button on the right side of the ChatInterface header; clicking opens the export flow.
- **Inactive Topics**: The "Export" button in the TopicSidebar right-click context menu; clicking goes directly to the export flow.

### Message Selection Mode

After entering the export flow, a checkbox appears to the left of each message. "Select All"/"Deselect" buttons are provided at the top, and the number of selected messages is shown in real time. After confirming the selection, proceed to the export format and options page.

### Export Formats

- **Screenshot (default)**: Select PNG or JPEG format, with narrow or wide width options. Uses `html-to-image` to capture the message area; supports download or copy to clipboard.
- **Markdown**: Exported content includes topic name, timestamp, and role labels. Reasoning content is displayed in blockquote style, with an optional `tool_calls` inclusion. Supports copy to clipboard or download as `.md` file.
- **JSON**: Two modes available -- `full` retains all fields (role, content, modelId, reasoning, toolCalls, agentSteps, tokens), `simple` retains only role and content. Supports copy or download as `.json` file.
- **PDF**: Messages are first rendered as HTML, converted to a canvas via `toCanvas`, then rendered into a multi-page A4 layout using jsPDF. Download only.

### Filter Options

The following filters can be toggled before export:

- **Include Reasoning Content** (default on)
- **System Messages** (default off)
- **Tool Calls** (default off)

### Architecture

The export functionality is purely frontend-based and does not depend on the Rust backend. Core export functions are located in `src/core/utils/exportConversation.ts`:

- `exportAsMarkdown`: Generates Markdown text
- `exportAsJSON`: Generates JSON data
- `exportAsHtml`: Generates HTML for rendering

Screenshot and PDF exports depend on the `html-to-image` and `jsPDF` libraries respectively.

See [MCP and Skills](mcp-and-skills.md) for MCP and Skill installation, binding, and approval rules.

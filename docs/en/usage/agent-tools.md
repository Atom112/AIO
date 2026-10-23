# Agent Tools Reference

## Overview

Agent mode provides 30+ built-in tools across 7 categories. The following table lists the **core tools** with their basic permission characteristics (the six memory tools and orchestration tools have full parameter docs in their sections; subagent-management tools like `get_subagent_result` are covered under orchestration):

> **Execution semantics**: read-only tools within the same response block (reads/search/web queries/git read/LSP/memory recall) run concurrently; mutating tools (file writes, command execution, git write) and MCP tools run serially in original order. Parallel execution is on by default; it can be turned off by setting `parallelToolExecution: false` in `config.json` (no settings UI).
>
> **Subject scope**: the table and permission matrix above describe the **main agent**. Subagent tool sets are assembled per role (`SubagentProfile.allowed_tools/denied_tools`) and differ from the main agent — e.g. the read-only roles explorer/architect explicitly deny `git_*` (9 read-only tools, no Git tools); only coder/general and similar hold Git reads. A subagent calling a tool that was not injected gets a fail-closed "tool not available" error.
>
> **MCP tools**: tools from MCP servers enabled for the assistant are injected into the agent loop (the `general` subagent profile also gets them); name collisions are auto-renamed to `mcp__<server>__<tool>`. Tools from untrusted servers still prompt for each call in full-permission mode (the "Trusted" toggle in MCP settings, or "Always allow" in the approval bubble, which writes the tool into the project permission config and stops the prompts).

| Tool Name             | Category      | Normal Default | Auto Default | Plan Default |
| --------------------- | ------------- | -------------- | ------------ | ------------ |
| `read_file`           | File          | Allow          | Allow        | Allow        |
| `write_file`          | File          | Ask            | Allow        | Deny         |
| `list_directory`      | File          | Allow          | Allow        | Allow        |
| `search_files`        | File          | Allow          | Allow        | Allow        |
| `search_content`      | File          | Allow          | Allow        | Allow        |
| `replace_in_file`     | File          | Ask            | Allow        | Deny         |
| `delete_file`         | File          | Ask            | Allow        | Deny         |
| `make_directory`      | File          | Ask            | Allow        | Deny         |
| `git_status`          | Git           | Allow          | Allow        | Allow        |
| `git_diff`            | Git           | Allow          | Allow        | Allow        |
| `git_log`             | Git           | Allow          | Allow        | Allow        |
| `git_add`             | Git           | Ask            | Allow        | Deny         |
| `git_commit`          | Git           | Ask            | Allow        | Deny         |
| `read_lints`          | LSP           | Ask            | Allow        | Ask          |
| `lsp_definition`      | LSP           | Ask            | Allow        | Ask          |
| `lsp_references`      | LSP           | Ask            | Allow        | Ask          |
| `lsp_hover`           | LSP           | Ask            | Allow        | Ask          |
| `lsp_symbols`         | LSP           | Ask            | Allow        | Ask          |
| `web_fetch`           | Web           | Ask            | Allow        | Deny         |
| `web_search`          | Web           | Ask            | Allow        | Deny         |
| `execute_command`     | Shell         | Ask            | Allow        | Deny         |
| `think`               | Orchestration | Allow          | Allow        | Allow        |
| `project_map`         | Orchestration | Allow          | Allow        | Allow        |
| `delegate_task`       | Orchestration | Ask            | Allow        | Ask          |
| `delegate_tasks`      | Orchestration | Ask            | Allow        | Ask          |
| `create_tasks`        | Orchestration | Allow          | Allow        | Allow        |
| `update_task`         | Orchestration | Allow          | Allow        | Allow        |
| `list_tasks`          | Orchestration | Allow          | Allow        | Allow        |
| `ask_user`            | Orchestration | Allow          | Allow        | Allow        |
| `read_skill`          | Orchestration | Allow          | Allow        | Allow        |
| `read_artifact`       | Orchestration | Allow          | Allow        | Allow        |
| `get_subagent_result` | Orchestration | Ask            | Allow        | Ask          |
| `remember`            | Knowledge     | Allow          | Allow        | Deny         |
| `recall`              | Knowledge     | Allow          | Allow        | Allow        |
| `search_memory`       | Knowledge     | Allow          | Allow        | Allow        |
| `update_memory`       | Knowledge     | Allow          | Allow        | Deny         |
| `forget_memory`       | Knowledge     | Allow          | Allow        | Deny         |
| `search_code`         | Knowledge     | Ask            | Allow        | Ask          |

> Permissions are affected by project `.aio/permissions.json` custom rules. Deny rules always take precedence over Allow rules. See the [Permission System Reference](../reference/permissions.md) for details.

---

## File Tools

Source: `src-tauri/src/utils/file_tools.rs`

All file tool paths are restricted by the project root directory sandbox (`safe_path`) and cannot access files outside the project.

### `read_file`

Read file content.

| Item            | Value                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Description** | Read the content of a specified file within the project directory, with 1-based line numbers (format ` 1                                                                                                                                                                                                                                                                                                                                                              | content`, directly usable for `replace_in_file` `start_line`/`end_line`). Binary files are rejected. Default max 300 lines / 32KB; when exceeded, returns head+tail with the omitted line range annotated. |
| **Parameters**  | `path` (string, required) -- file path; `start_line` (integer, optional) -- start reading at this 1-based line: pass it when reading a MIDDLE segment, it returns exactly `max_lines` real-numbered lines with no head+tail fallback; `max_lines` (integer, optional) -- max lines, default 300, pass -1 for unlimited; `max_bytes` (integer, optional) -- max bytes, default 32KB; `with_hash` (boolean, optional) -- return file SHA-256 digest (hashline protocol) |
| **Return**      | Line-numbered text content of the file; when limited, returns head+tail with the omitted line range annotated                                                                                                                                                                                                                                                                                                                                                         |
| **Limits**      | Default max 300 lines / 32KB; hard cap 1MB                                                                                                                                                                                                                                                                                                                                                                                                                            |
| **Permission**  | Read operation: Normal/Allow, Auto/Allow, Plan/Allow                                                                                                                                                                                                                                                                                                                                                                                                                  |

### `write_file`

Create or overwrite a file.

| Item            | Value                                                                                                                       |
| --------------- | --------------------------------------------------------------------------------------------------------------------------- |
| **Description** | Write content to a file. Creates if it does not exist, overwrites if it does. Parent directories are created automatically. |
| **Parameters**  | `path` (string, required) -- file path; `content` (string, required) -- file content                                        |
| **Return**      | Write confirmation and byte count                                                                                           |
| **Limits**      | Maximum 5MB                                                                                                                 |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Deny                                                                                           |

### `list_directory`

List directory contents.

| Item            | Value                                                                                            |
| --------------- | ------------------------------------------------------------------------------------------------ |
| **Description** | List entries (files and subdirectories) in a specified directory, including size and type icons. |
| **Parameters**  | `path` (string, optional) -- directory path, defaults to project root                            |
| **Return**      | List of directory entries (max 10,000 items)                                                     |
| **Limits**      | Maximum 10,000 items                                                                             |
| **Permission**  | Read operation: Normal/Allow, Auto/Allow, Plan/Allow                                             |

### `search_files`

Search for files by glob pattern.

| Item            | Value                                                                                                                   |
| --------------- | ----------------------------------------------------------------------------------------------------------------------- |
| **Description** | Recursively search for files matching a glob pattern.                                                                   |
| **Parameters**  | `pattern` (string, required) -- glob pattern, e.g. `"src/**/*.ts"`; `basePath` (string, optional) -- starting directory |
| **Return**      | List of matching files (max 10,000 items)                                                                               |
| **Limits**      | Maximum 10,000 items                                                                                                    |
| **Permission**  | Read operation: Normal/Allow, Auto/Allow, Plan/Allow                                                                    |

### `search_content`

Search file contents with a regular expression.

| Item            | Value                                                                                                     |
| --------------- | --------------------------------------------------------------------------------------------------------- |
| **Description** | Search file contents by regular expression. Shows up to the first 10 matching lines per file.             |
| **Parameters**  | `pattern` (string, required) -- regular expression; `path` (string, optional) -- target file or directory |
| **Return**      | Match results: file name, line number, line content                                                       |
| **Limits**      | --                                                                                                        |
| **Permission**  | Read operation: Normal/Allow, Auto/Allow, Plan/Allow                                                      |

### `replace_in_file`

Perform a replacement in a file (exact match or line-range anchored).

| Item            | Value                                                                                                                                                                                                                                                                                                                                                                                     |
| --------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Description** | Two modes: 1) exact replacement of `old_string` (must appear exactly once); 2) line-range anchored -- leave `old_string` empty and pass `start_line`/`end_line` (1-based); the tool reads the current line range and replaces it with `new_string`, so large edits do not require resending the old code (token-efficient). If line numbers are stale, it errors and asks you to re-read. |
| **Parameters**  | `path` (string, required) -- file path; `old_string` (string, optional) -- original string to replace (must match exactly); `new_string` (string, required) -- replacement string; `start_line`/`end_line` (integer, optional) -- line-range mode boundaries; `anchor_hash` (string, optional) -- file SHA-256 digest (hashline drift protection)                                         |
| **Return**      | Replacement confirmation and matched line number                                                                                                                                                                                                                                                                                                                                          |
| **Limits**      | Content after replacement must not exceed 5MB; binary files cannot be replaced                                                                                                                                                                                                                                                                                                            |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Deny                                                                                                                                                                                                                                                                                                                                                         |

### `delete_file`

Delete a file.

| Item            | Value                                                                       |
| --------------- | --------------------------------------------------------------------------- |
| **Description** | Delete the specified file. Cannot delete directories (use system commands). |
| **Parameters**  | `path` (string, required) -- file path                                      |
| **Return**      | Deletion confirmation                                                       |
| **Limits**      | --                                                                          |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Deny                                           |

### `make_directory`

Create a directory.

| Item            | Value                                                                                      |
| --------------- | ------------------------------------------------------------------------------------------ |
| **Description** | Create a new directory. Parent directories are created automatically if they do not exist. |
| **Parameters**  | `path` (string, required) -- directory path                                                |
| **Return**      | Creation confirmation                                                                      |
| **Limits**      | --                                                                                         |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Deny                                                          |

---

## Git Tools

Source: `src-tauri/src/utils/git_tools.rs`

Executed via the system `git` CLI. All calls use argument arrays (not shell strings), eliminating injection risk. Tools return a friendly error when used outside a Git directory.

### `git_status`

View working tree and staging area status.

| Item            | Value                                                                                                           |
| --------------- | --------------------------------------------------------------------------------------------------------------- |
| **Description** | Display modified, staged, untracked files and the current branch in machine-readable format (`--porcelain=v2`). |
| **Parameters**  | None                                                                                                            |
| **Return**      | git status output (max 80KB)                                                                                    |
| **Permission**  | Read operation: Allow across all modes                                                                          |

### `git_diff`

View differences.

| Item            | Value                                                                                                                   |
| --------------- | ----------------------------------------------------------------------------------------------------------------------- |
| **Description** | Show uncommitted changes. Default shows working tree changes; set `staged=true` to show staged changes.                 |
| **Parameters**  | `staged` (boolean, optional) -- whether to show staged changes; `path` (string, optional) -- limit to file or directory |
| **Return**      | unified diff output (max 80KB)                                                                                          |
| **Permission**  | Read operation: Allow across all modes                                                                                  |

### `git_log`

View commit history.

| Item            | Value                                                                                                                                                                                                  |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Description** | Default shows the last 10 commits.                                                                                                                                                                     |
| **Parameters**  | `count` (integer, optional) -- number of entries, default 10, max 50; `path` (string, optional) -- limit to file or directory; `oneline` (boolean, optional) -- compact single-line mode, default true |
| **Return**      | git log output (max 80KB)                                                                                                                                                                              |
| **Permission**  | Read operation: Allow across all modes                                                                                                                                                                 |

### `git_add`

Stage files.

| Item            | Value                                                                                                                                     |
| --------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| **Description** | Add files to the staging area. Can specify a list of files or use `all=true` to stage all changes.                                        |
| **Parameters**  | `files` (string[], optional) -- list of file paths to stage; `all` (boolean, optional) -- stage all changes (`git add -A`), default false |
| **Return**      | Stage confirmation                                                                                                                        |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Deny                                                                                                         |

### `git_commit`

Create a commit.

| Item            | Value                                                        |
| --------------- | ------------------------------------------------------------ |
| **Description** | Create a Git commit. Refuses when the staging area is empty. |
| **Parameters**  | `message` (string, required) -- commit message, max 10KB     |
| **Return**      | Commit confirmation                                          |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Deny                            |

---

## LSP Tools

Source: `src-tauri/src/utils/lsp_tools.rs`, `src-tauri/src/utils/lsp_agent_tools.rs`

If the language server for the file type is not running, it is started on demand when an LSP tool is called (the server must be installed locally, e.g. `pyright-langserver`, `rust-analyzer`, `typescript-language-server`, `gopls`). Before a query, the target file's current content is synced into the server (didOpen/didChange) so symbols and diagnostics are available promptly; read_lints briefly waits and retries while diagnostics have not been published yet. LSP startup failure does not affect other tools.

### `read_lints`

Read LSP diagnostics.

| Item            | Value                                                                                                                                                                                                       |
| --------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Description** | Read LSP diagnostics (compilation errors, type errors, code warnings, etc.) in the project.                                                                                                                 |
| **Parameters**  | `paths` (string[], optional) -- list of file paths, empty returns diagnostics for all files; `severity` (enum, optional) -- filter level: `"error"` / `"warning"` / `"info"` / `"hint"` / `"all"` (default) |
| **Return**      | Categorized diagnostic list                                                                                                                                                                                 |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Ask                                                                                                                                                                            |

### `lsp_definition`

Jump to symbol definition.

| Item            | Value                                                                                                                                                                                |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Description** | Jump to the definition at the specified position, returning the file path, line number, and column number of the definition.                                                         |
| **Parameters**  | `path` (string, required) -- file path (relative to project root); `line` (number, required) -- line number (0-indexed); `character` (number, required) -- column number (0-indexed) |
| **Return**      | Definition location (file:line:column)                                                                                                                                               |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Ask                                                                                                                                                     |

### `lsp_references`

Find symbol references.

| Item            | Value                                                                                                                                                     |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Description** | Find all references to a specified symbol, returning a list of reference locations.                                                                       |
| **Parameters**  | `path` (string, required) -- file path; `line` (number, required) -- line number (0-indexed); `character` (number, required) -- column number (0-indexed) |
| **Return**      | List of reference locations                                                                                                                               |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Ask                                                                                                                          |

### `lsp_hover`

View type/documentation information.

| Item            | Value                                                                                                                                                     |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Description** | View type information and documentation comments at a specified position.                                                                                 |
| **Parameters**  | `path` (string, required) -- file path; `line` (number, required) -- line number (0-indexed); `character` (number, required) -- column number (0-indexed) |
| **Return**      | Hover tooltip content                                                                                                                                     |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Ask                                                                                                                          |

### `lsp_symbols`

List file symbols.

| Item            | Value                                                                                 |
| --------------- | ------------------------------------------------------------------------------------- |
| **Description** | List all symbols (functions, classes, variables, etc.) in a file and their positions. |
| **Parameters**  | `path` (string, required) -- file path to query (relative to project root)            |
| **Return**      | List of symbols (hierarchically expanded)                                             |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Ask                                                      |

---

## Web Tools

Source: `src-tauri/src/utils/web_tools.rs`

Security measures: HTTPS-only, internal IP blocking (SSRF protection), response body size limits, rate limiting.

### `web_fetch`

Fetch web page content.

| Item            | Value                                                                                                                             |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| **Description** | Fetch content from a specified URL, returning plain text (HTML tags stripped). HTTPS only.                                        |
| **Parameters**  | `url` (string, required) -- web URL (must be `https://`); `max_bytes` (integer, optional) -- maximum bytes to return, default 1MB |
| **Return**      | Plain text content of the page                                                                                                    |
| **Limits**      | Connection timeout 5s, total timeout 30s; HTTPS only; internal IPs blocked                                                        |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Deny                                                                                                 |

### `web_search`

Search the web.

| Item            | Value                                                                                                             |
| --------------- | ----------------------------------------------------------------------------------------------------------------- |
| **Description** | Search the web (Bing first, DuckDuckGo fallback), returning result snippets and links.                            |
| **Parameters**  | `query` (string, required) -- search query; `count` (integer, optional) -- number of results to return, default 5 |
| **Return**      | Result snippets + link list                                                                                       |
| **Limits**      | 3-second rate limit interval (same process); HTTPS only                                                           |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Deny                                                                                 |

---

## Shell Execution Tool

Source: `src-tauri/src/utils/shell_tools.rs`

### `execute_command`

Execute a system command in the project directory.

| Item            | Value                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Description** | Execute a system command, returning stdout, stderr, and exit code.                                                                                                                                                                                                                                                                                                                                                                                                        |
| **Parameters**  | `command` (string, required) -- command string, e.g. `"cargo build"`; `description` (string, optional) -- command description (audit log only); `timeout` (integer, optional) -- timeout in milliseconds, default 60,000ms, max 300,000ms                                                                                                                                                                                                                                 |
| **Return**      | stdout + stderr output (50KB each maximum)                                                                                                                                                                                                                                                                                                                                                                                                                                |
| **Limits**      | Command length max 4096 characters; backtick/`$()` command substitution prohibited; dangerous commands blocked (`sudo`, `rm -rf /`, `format`, etc.); Windows Job Object sandbox; bare script interpreters as the first token (`python`/`py`/`node`/`npm`, etc.) are forwarded to the host-PATH-resolved absolute path (these names are otherwise invisible inside the sandbox shell), with a best-effort R+X grant on the interpreter directory under the Windows sandbox |
| **Security**    | Dangerous command detection (lexical token analysis), rejecting destructive/install/network exfiltration/env injection/fork bomb patterns                                                                                                                                                                                                                                                                                                                                 |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Deny; in sub-agents: general/debugger/tester roles allow, others deny                                                                                                                                                                                                                                                                                                                                                                        |

---

## Orchestration Tools

Source: `src-tauri/src/core/subagent.rs`, `src-tauri/src/utils/think.rs`, `src-tauri/src/utils/project_map.rs`

### `delegate_task`

Create a single sub-agent to execute a subtask.

| Item            | Value                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Description** | Create a sub-agent to execute an independent subtask. The sub-agent has its own LLM context window.                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| **Parameters**  | `profile` (string, required) -- sub-agent type: `explorer` / `coder` / `general` / `architect` / `debugger` / `reviewer` / `writer` / `tester` / `requirements` or a custom role ID; `task` (string, required) -- complete task description; `context_files` (string[], optional) -- initial file path list; `wait` (boolean, optional) -- whether to wait for completion, default true; `isolation` (string, optional) -- file isolation mode `worktree` / `shared`, see [Multi-agent parallel write coordination](#multi-agent-parallel-write-coordination) |
| **Return**      | Sub-agent work summary (with a merge outcome note when worktree-isolated)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Ask                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |

### `delegate_tasks`

Batch create multiple sub-agents to execute in parallel.

| Item            | Value                                                                                                                                                                                                                                                                                |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Description** | Create multiple sub-agents (1-10) in parallel in a single call. Total time is approximately the slowest subtask. On git projects write-capable tasks are isolated in worktrees by default (see [Multi-agent parallel write coordination](#multi-agent-parallel-write-coordination)). |
| **Parameters**  | `context` (string, optional) -- shared context for all subtasks; `tasks` (array, required, 1-10 items) -- list of subtasks, each with `profile`, `task`, `context_files`, `isolation` (optional, `worktree` / `shared`)                                                              |
| **Return**      | Work summary for each sub-agent (with per-task merge outcome notes when worktree-isolated)                                                                                                                                                                                           |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Ask                                                                                                                                                                                                                                                     |

### `create_tasks`

Create the agent task list (main-agent task tracking).

| Item            | Value                                                                                                                                                                                 |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Description** | Decompose multi-step work into a tracked task list; the user sees a live task panel above the chat area. Keep statuses current with `update_task` as work proceeds.                   |
| **Parameters**  | `tasks` (array, required, 1-10 items) -- each with `title` (one-line task title); `clear_existing` (boolean, optional) -- clear the existing list (replanning), default false appends |
| **Return**      | Numbered list with task IDs and statuses                                                                                                                                              |
| **Permission**  | Allow in all modes (pure metadata; never touches files)                                                                                                                               |

### `update_task`

Update a task's status or rewrite its title.

| Item            | Value                                                                                                                                                           |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Description** | Update as each task starts/completes so the user sees real progress.                                                                                            |
| **Parameters**  | `task_id` (string, required) -- target task ID; `status` (string, optional) -- `pending` / `in_progress` / `completed`; `title` (string, optional) -- new title |
| **Return**      | The updated full list (errors list the valid IDs when the task is not found)                                                                                    |
| **Permission**  | Allow in all modes                                                                                                                                              |

### `list_tasks`

View the current topic's task list.

| Item            | Value                                                                        |
| --------------- | ---------------------------------------------------------------------------- |
| **Description** | Check it when resuming work in a new turn, to avoid duplicate task creation. |
| **Parameters**  | None                                                                         |
| **Return**      | Current list (IDs + statuses)                                                |
| **Permission**  | Allow in all modes                                                           |

### `ask_user`

Ask the user about decision points (recommended options + custom input).

| Item            | Value                                                                                                                                                                                                                                                                                                                             |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Description** | Call for genuine decision points (ambiguous requirements, multiple viable approaches, risky trade-offs); the loop pauses, the user answers in a question bubble in the chat flow (pick an option, type a custom answer, or skip and let the agent decide), and the answers return as the tool result. Ask at most once per point. |
| **Parameters**  | `questions` (array, required, 1-4 items) -- each with `question`; `options` (0-4 items, each with `label`, optional `description`, optional `recommended` flag); `allow_custom` (custom answers allowed, default true)                                                                                                            |
| **Return**      | `[User answers]` with the choice per question; when skipped, "decide on your own and continue"                                                                                                                                                                                                                                    |
| **Permission**  | Allow in all modes (the user's answer IS the confirmation)                                                                                                                                                                                                                                                                        |

### Multi-agent parallel write coordination

When multiple agents modify files concurrently, AIO combines two layers of protection: physical worktree isolation plus per-file locks for the shared mode.

- **Isolation mode** (fixed auto policy, no setting): on git projects, awaited write-capable subtasks (`delegate_task` with `wait=true` and `delegate_tasks` tasks) automatically get an isolated git worktree (sibling directory `<project>.aio-wt/`, branch `aio/sub-<id>`, based on the current HEAD). On completion the changes are committed to the task branch and merged back into the current branch; non-git projects, network paths, or creation failures fall back to the shared mode (protected by file locks). Pass `isolation: "shared"` per task for a single-task override.
- **Merging**: Auto mode merges automatically; Normal mode asks one batch approval (listing branches, changed files and diff summaries). On a **conflict** the merge aborts safely, the worktree is kept, and the result reports the conflicted files — redo that task serially on the latest code or resolve manually.
- **Shared-mode protection**: `write_file` / `replace_in_file` / `delete_file` / `make_directory` take a per-file exclusive lock (120-second wait timeout reporting the holder); after another agent modified a file, `replace_in_file` must carry `anchor_hash` and an overwriting `write_file` must carry `expect_hash` — a mismatch errors out and requires a re-read, turning lost updates into recoverable re-read errors.
- **Scope**: `wait=false` background subagents always run shared (file locks still apply). Worktrees are based on the main checkout's HEAD and do NOT include its uncommitted changes — pass `isolation: "shared"` when a task must build on uncommitted work.
- **Recommendations**: partition parallel write tasks by file set; files written by shell commands and git write operations (`git_add` / `git_commit` / `checkout_branch`) bypass the file locks (consistent with the industry) — prefer file tools for critical files; avoid switching branches in the main checkout while worktree tasks run — the task merges land on whatever branch is current at merge time.

### `read_skill`

Read the full instructions of an enabled Skill on demand (token efficiency: in Agent mode only a skill directory is injected; full content is read on demand).

| Item            | Value                                                                                              |
| --------------- | -------------------------------------------------------------------------------------------------- |
| **Description** | Read the full instructions of an enabled Skill. Call this tool first when a task involves a skill. |
| **Parameters**  | `name` (string, required) -- skill name (e.g. `docx`, `xlsx`)                                      |
| **Return**      | Full skill instruction text; when not found, returns the list of available skills                  |
| **Permission**  | Allow in all modes                                                                                 |

### `read_artifact`

Read a locally archived history artifact (snapcompact-style: when summarization fails, early messages are archived locally and a `[History archive #id]` placeholder is inserted; read details on demand).

| Item            | Value                                                                    |
| --------------- | ------------------------------------------------------------------------ |
| **Description** | Read the archived content of early conversation messages by artifact id. |
| **Parameters**  | `artifact_id` (string, required) -- history archive ID                   |
| **Return**      | Archived content (truncated to ~30k chars)                               |
| **Permission**  | Allow in all modes                                                       |

### `think`

Record structured thinking (no-op).

| Item            | Value                                                                                                                           |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| **Description** | Record a structured thought process. No side effects, returns only confirmation. Used in Plan mode and orchestration scenarios. |
| **Parameters**  | `thought` (string, required) -- thought content                                                                                 |
| **Return**      | Confirmation text (with first 100 characters preview)                                                                           |
| **Permission**  | Allow across all modes                                                                                                          |

### `project_map`

Generate a project structure overview.

| Item            | Value                                                                                                                             |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| **Description** | Generate a tree diagram of the project directory structure. Used to quickly understand project layout.                            |
| **Parameters**  | None                                                                                                                              |
| **Return**      | Directory tree (max depth 4 levels, max 50 entries per directory), skipping hidden directories and `node_modules` / `target` etc. |
| **Permission**  | Allow across all modes                                                                                                            |

---

## Memory Tools (Project-level RAG)

Source: `src-tauri/src/services/memory/tools.rs`

When project memory is enabled (`memoryEnabled` + per-project toggle), the Agent gets six memory tools: `remember` / `recall` / `search_memory` / `update_memory` / `forget_memory` / `search_code`. Facts are stored in `{project_root}/.aio/memory/memory.sqlite` (semantic vectors + FTS5 keyword hybrid retrieval, optionally refined by two-stage reranking); a legacy `{project_root}/.aio/knowledge.json` is auto-migrated into facts on first open (idempotent). Retrieved facts are injected into the stable system-prompt prefix (index=1) under a token budget.

### `remember`

Save a project-level fact (vector dedup before write; near-duplicates are arbitrated by the LLM into merge / update / supersede).

| Item            | Value                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Description** | Remember a project-level fact (architecture decisions, code conventions, common patterns, key findings), persisted across sessions with semantic retrieval.                                                                                                                                                                                                                                                                                                                                   |
| **Parameters**  | `key` (string, optional) -- deduplication key. **A second write with the same key OVERWRITES** (in-place update, stable id; the old content is archived to the version history, viewable in the memory panel, and `recall` returns only the latest) -- do not reuse a key when version evolution must be kept; `content` (string, required) -- fact content; `category` (enum, required) -- `"decision"` / `"pattern"` / `"convention"` / `"note"` / `"entity"` / `"architecture"` / `"task"` |
| **Return**      | Save confirmation                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| **Limits**      | Default cap of 2000 active facts per project; excess facts are archived by importance x access decay, lowest first (pinned facts are kept)                                                                                                                                                                                                                                                                                                                                                    |
| **Permission**  | Normal/Allow, Auto/Allow, Plan/Deny                                                                                                                                                                                                                                                                                                                                                                                                                                                           |

### `recall`

Hybrid semantic + keyword retrieval over project memory.

| Item            | Value                                                                                              |
| --------------- | -------------------------------------------------------------------------------------------------- |
| **Description** | Retrieve project-level memory (vector + FTS5 keyword hybrid; returns facts with relevance scores). |
| **Parameters**  | `query` (string, required) -- retrieval intent; `category` (string, optional) -- category filter   |
| **Return**      | List of facts with relevance scores                                                                |
| **Permission**  | Normal/Allow, Auto/Allow, Plan/Allow                                                               |

Other memory tools: `search_memory` (fine-grained search with `k` and a minimum relevance `min_score`), `update_memory` (edit a fact by id), `forget_memory` (archive / soft-delete), `search_code` (codebase semantic search; index the codebase first from the memory panel). Sub-agents get the read-only `recall` / `search_code` tools. Everything is managed from project settings -> Memory panel, which also supports exporting/importing memory JSON.

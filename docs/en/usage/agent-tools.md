# Agent Tools Reference

## Overview

Agent mode provides 20+ built-in tools across 6 categories. The following table lists all tools and their basic permission characteristics:

| Tool Name         | Category      | Normal Default | Auto Default | Plan Default |
| ----------------- | ------------- | -------------- | ------------ | ------------ |
| `read_file`       | File          | Allow          | Allow        | Allow        |
| `write_file`      | File          | Ask            | Allow        | Deny         |
| `list_directory`  | File          | Allow          | Allow        | Allow        |
| `search_files`    | File          | Allow          | Allow        | Allow        |
| `search_content`  | File          | Allow          | Allow        | Allow        |
| `replace_in_file` | File          | Ask            | Allow        | Deny         |
| `delete_file`     | File          | Ask            | Ask          | Deny         |
| `make_directory`  | File          | Ask            | Allow        | Deny         |
| `git_status`      | Git           | Allow          | Allow        | Allow        |
| `git_diff`        | Git           | Allow          | Allow        | Allow        |
| `git_log`         | Git           | Allow          | Allow        | Allow        |
| `git_add`         | Git           | Ask            | Allow        | Deny         |
| `git_commit`      | Git           | Ask            | Allow        | Deny         |
| `read_lints`      | LSP           | Ask            | Allow        | Ask          |
| `lsp_definition`  | LSP           | Ask            | Allow        | Ask          |
| `lsp_references`  | LSP           | Ask            | Allow        | Ask          |
| `lsp_hover`       | LSP           | Ask            | Allow        | Ask          |
| `lsp_symbols`     | LSP           | Ask            | Allow        | Ask          |
| `web_fetch`       | Web           | Ask            | Allow        | Deny         |
| `web_search`      | Web           | Ask            | Allow        | Deny         |
| `execute_command` | Shell         | Ask            | Allow        | Deny         |
| `think`           | Orchestration | Allow          | Allow        | Allow        |
| `project_map`     | Orchestration | Allow          | Allow        | Allow        |
| `delegate_task`   | Orchestration | Ask            | Allow        | Ask          |
| `delegate_tasks`  | Orchestration | Ask            | Allow        | Ask          |
| `create_workflow` | Orchestration | Ask            | Allow        | Ask          |
| `remember`        | Knowledge     | Ask            | Allow        | Ask          |
| `recall`          | Knowledge     | Ask            | Allow        | Ask          |

> Permissions are affected by project `.aio/permissions.json` custom rules. Deny rules always take precedence over Allow rules. See the [Permission System Reference](../reference/permissions.md) for details.

---

## File Tools

Source: `src-tauri/src/utils/file_tools.rs`

All file tool paths are restricted by the project root directory sandbox (`safe_path`) and cannot access files outside the project.

### `read_file`

Read file content.

| Item            | Value                                                                                         |
| --------------- | --------------------------------------------------------------------------------------------- |
| **Description** | Read the content of a specified file within the project directory. Binary files are rejected. |
| **Parameters**  | `path` (string, required) -- file path                                                        |
| **Return**      | Text content of the file                                                                      |
| **Limits**      | Maximum 1MB; truncated with a notice if exceeded                                              |
| **Permission**  | Read operation: Normal/Allow, Auto/Allow, Plan/Allow                                          |

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

Perform an exact string replacement in a file.

| Item            | Value                                                                                                                                                                             |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Description** | Replace `old_string` with `new_string`. `old_string` must appear exactly once in the file, otherwise an error is raised requesting more context.                                  |
| **Parameters**  | `path` (string, required) -- file path; `old_string` (string, required) -- original string to replace (must match exactly); `new_string` (string, required) -- replacement string |
| **Return**      | Replacement confirmation and matched line number                                                                                                                                  |
| **Limits**      | Content after replacement must not exceed 5MB; binary files cannot be replaced                                                                                                    |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Deny                                                                                                                                                 |

### `delete_file`

Delete a file.

| Item            | Value                                                                       |
| --------------- | --------------------------------------------------------------------------- |
| **Description** | Delete the specified file. Cannot delete directories (use system commands). |
| **Parameters**  | `path` (string, required) -- file path                                      |
| **Return**      | Deletion confirmation                                                       |
| **Limits**      | --                                                                          |
| **Permission**  | Normal/Ask, Auto/Ask, Plan/Deny                                             |

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

Requires a language server for the corresponding file type to be running in the project. LSP startup failure does not affect other tools.

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
| **Description** | Search the web via DuckDuckGo, returning result snippets and links.                                               |
| **Parameters**  | `query` (string, required) -- search query; `count` (integer, optional) -- number of results to return, default 5 |
| **Return**      | Result snippets + link list                                                                                       |
| **Limits**      | 3-second rate limit interval (same process); HTTPS only                                                           |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Deny                                                                                 |

---

## Shell Execution Tool

Source: `src-tauri/src/utils/shell_tools.rs`

### `execute_command`

Execute a system command in the project directory.

| Item            | Value                                                                                                                                                                                                                                     |
| --------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Description** | Execute a system command, returning stdout, stderr, and exit code.                                                                                                                                                                        |
| **Parameters**  | `command` (string, required) -- command string, e.g. `"cargo build"`; `description` (string, optional) -- command description (audit log only); `timeout` (integer, optional) -- timeout in milliseconds, default 60,000ms, max 300,000ms |
| **Return**      | stdout + stderr output (50KB each maximum)                                                                                                                                                                                                |
| **Limits**      | Command length max 4096 characters; backtick/`$()` command substitution prohibited; dangerous commands blocked (`sudo`, `rm -rf /`, `format`, etc.); Windows Job Object sandbox                                                           |
| **Security**    | Dangerous command detection (lexical token analysis), rejecting destructive/install/network exfiltration/env injection/fork bomb patterns                                                                                                 |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Deny; in sub-agents: general/debugger/tester roles allow, others deny                                                                                                                                        |

---

## Orchestration Tools

Source: `src-tauri/src/core/subagent.rs`, `src-tauri/src/utils/think.rs`, `src-tauri/src/utils/project_map.rs`

### `delegate_task`

Create a single sub-agent to execute a subtask.

| Item            | Value                                                                                                                                                                                                                                                                                                                                                                                   |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Description** | Create a sub-agent to execute an independent subtask. The sub-agent has its own LLM context window.                                                                                                                                                                                                                                                                                     |
| **Parameters**  | `profile` (string, required) -- sub-agent type: `explorer` / `coder` / `general` / `architect` / `debugger` / `reviewer` / `writer` / `tester` / `requirements` or a custom role ID; `task` (string, required) -- complete task description; `context_files` (string[], optional) -- initial file path list; `wait` (boolean, optional) -- whether to wait for completion, default true |
| **Return**      | Sub-agent work summary                                                                                                                                                                                                                                                                                                                                                                  |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Ask                                                                                                                                                                                                                                                                                                                                                        |

### `delegate_tasks`

Batch create multiple sub-agents to execute in parallel.

| Item            | Value                                                                                                                                                                    |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Description** | Create multiple sub-agents (1-10) in parallel in a single call. Total time is approximately the slowest subtask.                                                         |
| **Parameters**  | `context` (string, optional) -- shared context for all subtasks; `tasks` (array, required, 1-10 items) -- list of subtasks, each with `profile`, `task`, `context_files` |
| **Return**      | Work summary for each sub-agent                                                                                                                                          |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Ask                                                                                                                                         |

### `create_workflow`

Create a sequentially-executing sub-agent workflow.

| Item            | Value                                                                                                                                    |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| **Description** | Create a sequentially-executing sub-agent workflow. Each step's output is automatically passed to the next. Requires at least 2 steps.   |
| **Parameters**  | `title` (string, required) -- workflow title; `steps` (array, required, >=2 items) -- list of steps, each with `profile`, `name`, `task` |
| **Return**      | Work summary for each step                                                                                                               |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Ask                                                                                                         |

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

## Knowledge Tools

Source: `src-tauri/src/utils/knowledge.rs`

Knowledge data is persisted to `{project_root}/.aio/knowledge.json` and automatically injected into the system prompt when the Agent starts.

### `remember`

Save project-level knowledge.

| Item            | Value                                                                                                                                                                                                                         |
| --------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Description** | Remember a piece of project-level knowledge (architecture decisions, code conventions, common patterns), persisted across sessions.                                                                                           |
| **Parameters**  | `key` (string, required) -- short identifier (used for deduplication and retrieval); `content` (string, required) -- knowledge content; `category` (enum, required) -- `"decision"` / `"pattern"` / `"convention"` / `"note"` |
| **Return**      | Save confirmation                                                                                                                                                                                                             |
| **Limits**      | Maximum 50 entries; when exceeded, the oldest entry is removed                                                                                                                                                                |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Ask                                                                                                                                                                                              |

### `recall`

Retrieve project-level knowledge.

| Item            | Value                                                                                                                                                                    |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Description** | Fuzzy match `key` and `content` by keywords, optionally filter by category.                                                                                              |
| **Parameters**  | `query` (string, required) -- search keywords (case-insensitive); `category` (enum, optional) -- category filter: `"decision"` / `"pattern"` / `"convention"` / `"note"` |
| **Return**      | List of matching knowledge entries                                                                                                                                       |
| **Permission**  | Normal/Ask, Auto/Allow, Plan/Ask                                                                                                                                         |

# System Architecture

## Overview

AIO is a Tauri 2 desktop application:

```text
SolidJS UI
  ├── features/chat
  ├── features/settings
  ├── core/store + types + utils
  └── shared/components
          | invoke / event
Rust commands
  ├── LLM and Agent
  ├── Configuration, Attachments, Projects and Updates
  ├── Provider, Engine, MCP, Skill, Git and LSP
  └── SQLite / Secure Store / Process State
```

The frontend handles interaction, navigation, and reactive state; Rust handles persistence, network requests, file and process access, and tool execution that requires a trust boundary.

## Frontend

- `src/features/chat`: Chat page, project sidebar, Agent process, message branching, sharing and issue panels.
- `src/features/settings`: Provider, MCP, Skill, usage, sub-agent and app settings.
- `src/core/store`: Global SolidJS signals/store and backend sync operations.
- `src/core/shortcuts.ts`: Keyboard shortcut and slash command registration.
- `src/shared/components`: Markdown, icons, navigation and common interaction components.

Routing centers on the chat page and nested settings pages. Provider details within settings use separate sub-routes.

## Rust Backend

`src-tauri/src/lib.rs` initializes SQLite, Tauri plugins and global state, and registers all invoke commands. Main modules:

- `commands/llm/`: Normal streaming chat, Agent loop, sub-agents, usage, summaries, titles and `/btw`;
- `commands/provider_config.rs`: Provider v2 configuration, connection test and model fetching;
- `commands/engine.rs`: Local engine installation, start, stop and status;
- `commands/mcp.rs`: MCP lifecycle, tools, Resources, Prompts and approval;
- `commands/skill.rs`: Global/project Skills, marketplace and NPX import;
- `commands/project.rs`: Project CRUD and `.aio/` initialization;
- `commands/lsp.rs`, `commands/git.rs`: Project language service and Git operations.
- `commands/memory.rs`, `commands/embedding.rs`: project memory (RAG) fact CRUD / retrieval and embedding provisioning;
- `commands/model_download.rs`: on-demand model downloader (embed manifest + install-dir/enable management, settings page "Embedding models");
- `services/memory/`: per-project memory store (facts + FTS5 + vector scoring), hybrid retrieval and memory tools;
- `plugins/embed/`: embedding (vectorization) plugin (local ONNX only: bundled model + downloaded models).

## Plugin Boundary

- Provider: Google, Anthropic, Ollama and OpenAI-compatible.
- Local Engine: llama.cpp, Ollama and vLLM (auto-discovered via engine scanning, see [providers-and-models.md -> Engine Scanning](../usage/providers-and-models.md#engine-scanning-and-auto-discovery)).
- MCP Transport: stdio, HTTP and Streamable HTTP.
- Embed: local ONNX text embeddings (vectorization) powering semantic project memory — bundled all-MiniLM-L6-v2 out of the box, plus mainstream open-source models downloaded via Settings → Embedding models; online embedding is no longer supported.
- LSP: Manages startup, requests, diagnostics and shutdown for different language servers.

The Manager registers plugins at startup; commands look up implementations by identifier. New implementations should reuse existing traits rather than adding parallel dispatch layers.

## State and Persistence

Long-term data:

- SQLite: Chats, attachment relations, projects and usage;
- JSON: Provider, MCP, Skill, model catalog and app settings;
- System credential store: API Keys and MCP secrets;
- Project `.aio/`: Project-level MCP, Skill and permissions.
- Project `.aio/knowledge.json`: Optional cross-session project knowledge;
- Project `.aio/memory/memory.sqlite`: project-level semantic memory (RAG) store with facts, version audit, FTS5 full-text and vector indexes.

Runtime state:

- LLM stream and cancellation tokens;
- Current local engine subprocesses;
- MCP connections, invocation tasks and pending approval requests;
- LSP clients;
- Sub-agent execution handles.

On window destruction, stream tasks are cancelled, local engines are terminated, MCP connections are drained and in-flight tool calls are aborted.

### Recent Performance Changes

- **Blocking I/O migration**: Shell command execution (`shell_tools::execute_command`), Git operations (`git_tools::execute_git_tool`), file operations (`file_tools::execute_file_tool`) all run on the tokio `spawn_blocking` pool, avoiding main runtime thread blockage.
- **Image lazy loading**: `ChatInterface` uses `ResizeObserver` for streaming auto-scroll, with scroll position correction after image loading completes.
- **Code block header**: Reduced header height, added sticky copy button to minimize scrolling distance.

## Normal Chat Data Flow

1. The frontend converts messages and attachments into Provider messages.
2. Rust selects the Provider based on the API URL.
3. The Provider initiates a streaming request.
4. Rust pushes text, reasoning content, usage and completion status via Tauri events.
5. The frontend updates the thread; Rust writes messages and attachment relations to SQLite.

## Agent Data Flow

1. The project and assistant determine Agent mode, model, Skill and MCP Server.
2. System prompt injects the project path and available tools.
3. The model returns tool calls.
4. The permission module returns allow, ask or deny.
5. Built-in tools or the MCP Server execute and feed results back to the model.
6. The Agent continues iterating; the frontend displays steps, sub-agents and usage throughout.

Plan mode only researches and outputs a plan; read-only mode keeps just the read/search/web toolset and never modifies anything. Sub-agents use a role-constrained toolset and cannot delegate further.

### Tool Execution Semantics

- **Parallel dispatch**: read-only built-in tools within the same response block execute concurrently (`FuturesUnordered`); mutating tools and MCP tools run serially in original order; `role:tool` backfill preserves index order. Toggle: `parallelToolExecution`.
- **Multi-agent write coordination**: mutating file tools all funnel through the single `execute_builtin_tool` choke point — a per-file exclusive lock (`core/file_locks.rs`, 120-second wait timeout reporting the holder; lock keys are case-folded on Windows, long waits are traced) plus last-writer tracking; after another agent modified a file, `replace_in_file` must carry `anchor_hash` and an overwriting `write_file` must carry `expect_hash` (lost updates become recoverable re-read errors); `read_file` returns the digest of the complete file; writes go through an atomic tmp+rename (preserving the target's permission bits, falling back to a direct write with a trace). The built-in filesystem MCP server's tool variants are filtered from agent tool sets per `FsServerPolicy` (mutating ones dropped outright, read ones deduped), so no file write can bypass the choke point; git write operations (`git_add`/`git_commit`/`checkout_branch`) and files written by shell commands still bypass the locks, and a worktree merge refreshes last-writer for every changed file to force re-anchoring on the next write.
- **Main agent tool set** (`mainAgentToolAccess`): `hybrid` by default — read-only tools called directly, mutating work delegated; `orchestrator` is the legacy strict mode; `full` gives the main agent all built-in tools.
- **MCP tool injection**: tools from servers enabled for the assistant are injected into the main agent and the `general` subagent (`allow_mcp_tools`); name collisions are rewritten to `mcp__<server>__<tool>`; untrusted servers force per-call approval even in full-permission mode (`trusted` field).
- **Model call retry**: the POST + status-check phase retries transport errors and 429/5xx with 1s/2s/4s backoff (`modelCallRetries`, default 3); mid-stream errors are not retried.
- **Background subagents**: `delegate_task(wait=false)` handles and cancellation tokens are registered in `SubagentHandles` and can be stopped individually via the `stop_subagent` command (parent cancel cascades).

### Permissions and Retries

The permission module first loads mode default rules, then overlays `.aio/permissions.json`. Matching considers tool name, Server, mode and path; deny rules take precedence, then priority selection. When a decision requires asking, the backend creates a pending approval request and dispatches it to the frontend via events; pending calls resume once allow or deny is received.

File-write type tools compute a Diff before approval. Upon successful execution, Git tools capture the actual changes again and write them into the message's `fileChanges`, used for expanding Diffs and for single-file/batch revert.

Tool failures retry up to 2 times by default, with a 500ms interval. The retry wrapper lives in the unified tool execution path, avoiding different strategies across tools.

### Sub-agents and Result Preservation

`delegate_task` and `delegate_tasks` reuse a single task execution function. Batch delegation executes 1-10 tasks concurrently via Tokio, with a Semaphore limiting actual concurrency to 5 by default; results are collected per sub-task and fed back to the main Agent in one pass.

**Worktree isolation** (fixed auto policy, `utils/worktree.rs`): on git projects, awaited write-capable subtasks get an isolated git worktree + branch in the sibling `<project>.aio-wt/` directory (based on the main checkout's current HEAD, `git worktree lock` held while running, stale entries swept, the creation base commit recorded); file/shell/git/LSP/project_map tools are all confined to the worktree root (commands and edits cannot reach the main checkout), while permission rules and project memory stay bound to the main project root. On completion the task's changes (a dirty tree OR commits ahead of the base — a subagent's own commits are never misjudged as empty work) land on the task branch: Auto mode merges back into the current branch directly, Normal mode asks one batch approval; while the main checkout has a merge/rebase/cherry-pick in progress the merge is refused and the branch kept (the user's in-progress state is never aborted); a conflict aborts the merge safely, keeps the worktree, and reports the conflicted files in the result. Non-git projects, network paths, background (`wait=false`) subagents and sequential workflow steps always run shared (file locks still apply).

When an Agent is aborted or a streaming event fails, already-produced steps, tool outputs and file changes are still merged into the current assistant message and persisted, avoiding a blank final reply.

### Project Memory (RAG)

With project memory enabled (`memoryEnabled`, overridable per project in project settings), the Agent gets five tools (`remember` / `recall` / `search_memory` / `update_memory` / `forget_memory`); facts are written to a per-project `.aio/memory/memory.sqlite` (semantic vectors + FTS5 keyword hybrid retrieval) and auto-injected into the stable system-prompt prefix (index=1) under a token budget. The embedding model is configured per project (stored in the project memory store meta, switched from the Memory panel; defaults to the bundled all-MiniLM-L6-v2 that works out of the box and offline, or any other local model downloaded and enabled in Settings → Embedding models; switching clears that project's vectors and re-embeds automatically); keyword retrieval is the automatic fallback when embeddings are not configured or the selected model is disabled.

Self-evolution landed in P2: after each agent turn, facts are extracted asynchronously in the background (sleep-time compute, debounced, per-project opt-in); near-duplicate high-similarity facts are arbitrated by the LLM (merge / update / supersede / keep_separate, fail-safe on errors), with a full audit trail in `fact_versions`; when active facts exceed the per-project cap, lowest-scoring non-pinned facts are archived by importance x access decay. A memory panel (search / list / edit / pin / archive / delete / version history / reindex / prune / clear) is available in project settings.

P3 integrates sqlite-vec (vec0 virtual tables) for in-SQLite vector search (auto-registered via sqlite3_auto_extension, falling back to brute-force cosine when unavailable); memory can be exported/imported as JSON (no vectors; reindex after import) and reindexing emits progress events. P4 adds code-level RAG: parallel `code_chunks` + `fts_code` + `vec_code` stores, incremental `memory_code_index` (skips unchanged files by mtime/size and ignores .git/node_modules/target etc.), a `search_code` tool plus `memory_code_search` command doing vector + keyword fusion; subagents (explorer/architect, etc.) get read-only `recall` / `search_code` tools.

## MCP Data Flow

1. Load global configuration and overlay project configuration.
2. stdio/HTTP plugins establish connections and complete initialization.
3. Fetch Tools, Resources and Prompts.
4. Only expose tools enabled for the current assistant to the model.
5. Tool calls go through permission checks, approval and timeout control.
6. Results enter the subsequent model request as tool messages.

## Update Data Flow

1. The Tauri updater requests `latest.json` from GitHub Releases via the endpoint in `tauri.conf.json`.
2. The backend converts "newest available", "updatable", "service not ready", "network or other failure" into structured results.
3. The frontend displays the version and Release notes; upon user confirmation it downloads the signed artifact and shows progress.
4. After download completes, the updater installs; the app restarts via restart command.

The public signing key is baked into the Tauri configuration; the signing private key exists only in the release environment. Local normal builds cannot produce update artifacts that could replace official Releases.

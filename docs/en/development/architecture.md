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
- `services/memory/`: per-project memory store (facts + FTS5 + vector scoring), hybrid retrieval and memory tools;
- `plugins/embed/`: embedding (vectorization) plugins (bundled local / online OpenAI-compatible).

## Plugin Boundary

- Provider: Google, Anthropic, Ollama and OpenAI-compatible.
- Local Engine: llama.cpp, Ollama and vLLM (auto-discovered via engine scanning, see [providers-and-models.md -> Engine Scanning](../usage/providers-and-models.md#engine-scanning-and-auto-discovery)).
- MCP Transport: stdio, HTTP and Streamable HTTP.
- Embed: bundled-local and online OpenAI-compatible text embeddings (vectorization) powering semantic project memory.
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

Plan mode only researches and outputs a plan; workflow mode splits and auto-executes tasks. Sub-agents use a role-constrained toolset and cannot delegate further.

### Permissions and Retries

The permission module first loads mode default rules, then overlays `.aio/permissions.json`. Matching considers tool name, Server, mode and path; deny rules take precedence, then priority selection. When a decision requires asking, the backend creates a pending approval request and dispatches it to the frontend via events; pending calls resume once allow or deny is received.

File-write type tools compute a Diff before approval. Upon successful execution, Git tools capture the actual changes again and write them into the message's `fileChanges`, used for expanding Diffs and for single-file/batch revert.

Tool failures retry up to 2 times by default, with a 500ms interval. The retry wrapper lives in the unified tool execution path, avoiding different strategies across tools.

### Sub-agents and Result Preservation

`delegate_task` and `delegate_tasks` reuse a single task execution function. Batch delegation executes 1-10 tasks concurrently via Tokio, with a Semaphore limiting actual concurrency to 5 by default; results are collected per sub-task and fed back to the main Agent in one pass.

When an Agent is aborted or a streaming event fails, already-produced steps, tool outputs and file changes are still merged into the current assistant message and persisted, avoiding a blank final reply.

### Project Memory (RAG)

With project memory enabled (`memoryEnabled`, overridable per project in project settings), the Agent gets five tools (`remember` / `recall` / `search_memory` / `update_memory` / `forget_memory`); facts are written to a per-project `.aio/memory/memory.sqlite` (semantic vectors + FTS5 keyword hybrid retrieval) and auto-injected into the stable system-prompt prefix (index=1) under a token budget. Embedding is provided by `plugins/embed` (a bundled all-MiniLM-L6-v2 model that works out of the box and offline, or a swappable online OpenAI-compatible API); keyword retrieval is the automatic fallback when embeddings are not configured. The local model ships with the app - no download or config needed.

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

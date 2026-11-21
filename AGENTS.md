# AIO (All-In-One AI) — Agent Coding Guide

**English** | [简体中文](AGENTS.zh-CN.md)

## Language Policy

English is the primary language of this repository. Code comments, commit messages, PR and issue descriptions, and the root governance documents (`README.md`, `CONTRIBUTING.md`, `AGENTS.md`, `SECURITY.md`) are written in English first. Chinese is maintained as a full second-language mirror: `README.zh-CN.md`, `CONTRIBUTING.zh-CN.md`, `AGENTS.zh-CN.md`, and `docs/zh/`.

- When a comment or document carries both languages, English comes first and Chinese follows (see "Code Comments").
- Commit messages use an English subject and body; an optional Chinese translation follows the English body (see [CONTRIBUTING.md](CONTRIBUTING.md) §3).
- User-facing strings are still localized through `t()` / `ierr!` with `zh-CN` and `en-US` keys; this policy does not replace i18n.

## Project Overview

AIO is a lightweight, cross-platform AI assistant desktop client built with **Tauri 2.x** (Rust backend) and **SolidJS** (frontend). It supports remote API providers (OpenAI-compatible, Anthropic, Google Gemini, Ollama), local inference engines (llama.cpp, Ollama, vLLM — multi-engine, one active at a time), and an **Agent mode** that runs tool-augmented loops (file, git, shell, web, LSP, knowledge tools via MCP), spawns subagents, and enforces a three-state permission model (Allow/Ask/Deny) across five agent modes (read-only / workspace-edit / full-permission / machine-full-trust / plan). Other subsystems: a **Harness workbench** (`/harness` page) that drives third-party CLI coding agents (Claude Code, Codex, OpenCode, Oh My Pi, Gemini CLI, Qwen Code, goose, DSH) over the Agent Client Protocol and renders their streams natively (AIO's own tools stay out of the loop there), project workspaces with Git integration, MCP server management (stdio/HTTP transports, catalog, tool approvals), skills (built-in, market, npx import), LSP diagnostics, usage tracking, app self-update, and full zh-CN/en-US localization. User docs live in `docs/` (mirrored: `docs/zh/`, `docs/en/`); development guides are in `docs/zh|en/development/`.

**Document responsibilities (multi-contributor)**: this file (`AGENTS.md`) is the single source of truth for coding standards and applies equally to humans and AI agents; [CONTRIBUTING.md](CONTRIBUTING.md) covers process (issues / branches / commit messages / PRs / releases); [SECURITY.md](SECURITY.md) is the vulnerability-reporting channel. This file follows the [agents.md open format](https://agents.md/) — any agent working on any file in the repository should read it first (nested AGENTS.md files take precedence locally).

## Tech Stack & Versions

| Layer             | Technology                                                                                                                           | Version                   |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------ | ------------------------- |
| Frontend          | SolidJS                                                                                                                              | ^1.9.13                   |
| Routing           | @solidjs/router                                                                                                                      | ^0.16.1                   |
| Styling           | Tailwind CSS (v4, via `@tailwindcss/postcss`, no tailwind.config)                                                                    | ^4.0.0                    |
| Build Tool        | Vite                                                                                                                                 | ^8.0.16                   |
| Desktop Framework | Tauri                                                                                                                                | 2.11.x                    |
| Tauri Plugins     | dialog, opener, updater (fs/shell removed — zero frontend usage; all file/command operations go through the app's own command layer) | 2.x                       |
| Backend Language  | Rust (pinned in `src-tauri/rust-toolchain.toml`; use `cargo +1.97.1`)                                                                | 1.97.1                    |
| Database          | SQLite (rusqlite)                                                                                                                    | bundled                   |
| i18n              | @solid-primitives/i18n (locales: zh-CN, en-US)                                                                                       | ^2.2.1                    |
| State Management  | SolidJS Store + Signals                                                                                                              | built-in                  |
| Models Catalog    | @aio/models-data (sibling local package; JSON bundled as tauri resource)                                                             | `file:../aio-models-data` |
| Provider Icons    | @lobehub/icons-static-svg                                                                                                            | ^1.91.0                   |

## Directory Structure

### Frontend (`src/`)

```
src/
├── index.tsx            # App entry: Router + lazy routes + startup inits (projects, MCP, skills, engine scan)
├── Layout.tsx           # Root layout with NavBar
├── index.css            # Tailwind v4 CSS + theme CSS variables (light/dark mode)
├── core/
│   ├── store/           # Domain modules: types/models/chat/engine/project/profiles/ui-prefs/update/harness/stream/layout/task-board/subagent-view/background-tasks (store.ts is a barrel re-export only; interfaces live in types.ts); diagnostics.ts
│   ├── api/             # Tauri command wrapper layer (chat/mcp/skills/config/update/engine/project/chat/attachment/lsp/usage/providers/memory/catalog/dialog) + events.ts (event-name constants, guarded by a two-sided test) + streamPayloads.ts (stream event payload types) — components and utils must never call invoke directly
│   ├── types/           # Backend mirror types: mcp.ts, skill.ts
│   ├── utils/           # modelLogo.ts (provider logos), models.ts (ProviderConfig/Catalog cache + queries, invoke moved down to api/catalog), mcp.ts, exportConversation.ts — no direct invoke (layering-utils gate)
│   ├── i18n/            # index.ts (t(), locale) + locales/zh-CN.json + locales/en-US.json
│   ├── shortcuts.ts     # Keyboard shortcut + slash command registry
│   ├── agent-prompts.ts # Agent system-prompt presets
│   └── assets/
├── features/
│   ├── chat/            # ChatPage.tsx + hooks/ (chatEventListeners.ts: 18 stream listeners registered globally in the root Layout + 3 drag-and-drop listeners at page level; chatTurnEffects.ts: turn-finalization effects) + constants.ts + components/ (ChatInterface, TopicSidebar, ProjectSidebar, AgentProcessBlock, SubagentBlock, AgentModeSelector, ReasoningButton, TokenBar, TokenStatsBar, ShareModal, ProjectSettingsModal, ProjectSelector, TaskBoardPanel, AgentQuestionBubble, ...)
│   ├── harness/         # HarnessPage.tsx (/harness: GUI compatibility layer for CLI tools) + hooks/harnessEventListeners.ts (global registration) + components/ (HarnessToolSidebar two-level tool/workspace sidebar, HarnessTopicSidebar, HarnessChatInterface, HarnessMessageList, HarnessApprovalBubble, HarnessAddWorkspaceModal) — reuses chat leaf components for rendering
│   └── settings/        # SettingsPage.tsx, ProviderDetailPage.tsx + components/ (ProviderList, AppSettings, McpServerList, McpServerDetail, SkillList, SubagentModelSettings, EmbeddingModelSettings, LocalModelMarket, UsageSettings + usage chart components)
└── shared/
    └── components/      # Icon, Markdown, NavBar, CommandPalette, SlashCommandMenu, Dropdown, Switch, ModelRow, GlobalKeyboardHandler, DiffView, ThinkBlock, UpdateNotification
```

### Backend (`src-tauri/src/`)

```
src-tauri/src/
├── main.rs
├── lib.rs               # Bootstrap: tracing init, managed state, invoke handler registration, engine cleanup on window destroy
├── mcp_fs_server.rs     # Built-in stdio filesystem MCP server (invoked with --fs-server)
├── core/
│   ├── models/          # Shared data structures (split by domain: chat/config/mcp/skill/project/agent/engine)
│   ├── db.rs            # SQLite initialization (versioned migrations via core/migrate.rs)
│   ├── state.rs         # Tauri managed state (StreamManager, DbState, LocalEngineState, SubagentHandles)
│   ├── config.rs        # AppConfig persistence (save_app_config/load_app_config; command layer keeps thin wrappers only)
│   ├── events.rs        # Backend→frontend event-name constant registry (kebab-case guard test; new events must register here)
│   ├── file_locks.rs    # Multi-agent write coordination: per-file mutex + last-writer tracking + cross-writer hashline gating
│   ├── fs_util.rs       # Atomic write (temp+rename) utilities — every JSON/config persistence path must go through here
│   ├── http.rs          # Outbound HTTP conventions: default User-Agent (reqwest sends none by default → ModelScope CDN 403) and client_builder starter
│   ├── ids.rs           # Built-in ID constants (BUILTIN_SERVER_ID / BUILTIN_FS_SERVER_ID)
│   ├── mcp_trust.rs     # Project-level .aio/mcp-servers.json trust store (fingerprint-keyed, outside the repo)
│   ├── migrate.rs       # Versioned SQLite migration runner (PRAGMA user_version; shared by chat and memory DBs)
│   ├── model_registry.rs # Embedding/rerank model registry (MANIFEST) and directory resolution
│   ├── path_safety.rs   # IPC path-argument validation from the frontend (safe_join: rejects absolute paths/`..`/reserved characters)
│   ├── paths.rs         # App data directory / package ID
│   ├── permission.rs    # Agent tool permission model (allowed/denied rules per agent mode)
│   ├── secure_store.rs  # OS keychain-backed secret storage (API keys, MCP server secrets)
│   └── subagent.rs      # Subagent profile definitions (10 built-in, incl. toolless vision) + tool specs
├── commands/
│   ├── mod.rs
│   ├── config.rs        # Assistants, app config, profile model overrides, custom subagent profiles, auto-start, topic branching
│   ├── llm/             # mod.rs (thin IPC wrappers + run_agent_turn_inner main loop): streaming/agent loop/budget/handoff/messages/subagents/tasks (task list + ask_user)/titles/tool_specs/usage/locked_tools (file-lock wiring); btw.rs: "By The Way" side questions
│   ├── engine.rs        # Local engine lifecycle: start/stop/status, engine scan, model listing
│   ├── attachment.rs    # Chat attachments + file content extraction command
│   ├── provider_config.rs # Provider configs (LobeHub-shaped), connection tests, model fetching, API key management
│   ├── project.rs       # Project CRUD, path validation
│   ├── git.rs           # Git branch ops, file revert
│   ├── mcp.rs           # MCP server lifecycle, tools, resources, prompts, tool approvals
│   ├── mcp_catalog.rs   # MCP server marketplace catalog
│   ├── skill.rs         # Skills CRUD, market browsing/download, npx import
│   ├── lsp.rs           # LSP server lifecycle, diagnostics, auto-detect
│   ├── catalog.rs       # Models catalog (bundled @aio/models-data / models.dev)
│   ├── local_model_market.rs  # Local GGUF model market: HF/ModelScope search merge + hardware profile + HF token
│   ├── local_model_download.rs # Local GGUF downloader: multi-source fallback/resume/space preflight/directory migration
│   ├── embedding.rs     # Embedding models: test / list / download / start-stop / directory
│   ├── rerank.rs        # Rerank models: test / save API key
│   ├── memory.rs        # Project memory commands (memory_*: CRUD/hybrid retrieval/stats/toggles)
│   ├── harness.rs       # Harness workbench commands (harness_*: tool detection/workspaces/topics/messages/send/stop/approval)
│   ├── export.rs        # Conversation export (Markdown/HTML/PDF source material)
│   ├── image_gen.rs     # Image generation commands
│   └── update.rs        # App update check/install/restart
├── services/            # Domain service layer (called by commands; must not depend on commands)
│   └── memory/          # Project-level memory/RAG service
│       ├── mod.rs       # Domain entry + collect_rows_warn (row-level error visibility) + migration/read split (migrate_* writes, resolve_* pure reads)
│       ├── manager.rs   # Per-project SQLite connection LRU cache
│       ├── store/       # Memory store layer (mod/search/config/graph/stats): facts + version audit + FTS5 full text + vector scoring + code symbol index
│       ├── code.rs      # Codebase indexing and vector retrieval (async + spawn_blocking)
│       ├── retrieve.rs  # Retrieval post-processing: two-stage rerank + query rewriting + multi-query RRF fusion (fail-open)
│       ├── extract.rs   # LLM background fact extraction
│       ├── judge.rs     # Fact deduplication/arbitration (cosine similarity threshold)
│       ├── consolidate.rs # Sleep-time consolidation (daily automatic + manual)
│       ├── graph.rs     # Relationship graph extraction
│       ├── llm.rs       # Thin LLM-call wrapper dedicated to the memory pipeline
│       ├── tools.rs     # Agent memory tools (remember/recall/search_memory)
│       ├── vec0.rs      # SQLite vec0 vector-extension adapter
│       ├── vector.rs    # Vector utilities (bytes_to_f32/brute_force_topk)
│       └── eval.rs      # RAG evaluation (dataset in src-tauri/tests/rag_eval)
├── plugins/
│   ├── engine/          # LocalEnginePlugin trait + llama_cpp, ollama, vllm, installer
│   ├── provider/        # ProviderPlugin trait + openai_compat (fallback, registered last), anthropic, google, ollama
│   ├── harness/         # Harness workbench (CLI tool compatibility layer): model (normalized events)/adapter (DriverCtx+trait+spawn)/registry (built-in ACP tool registry+detection)/acp (ACP JSON-RPC driver)/store (harness_* three tables)/manager (session runtime+approval resolution)
│   ├── llm.rs           # LLM HTTP client (post_chat_completion/chat_text_content, etc.; command layer now delegates here)
│   ├── embed/           # Embedding provider plugins (local_onnx + cache)
│   ├── rerank/          # Rerank provider plugins (local_onnx + openai_compat)
│   ├── mcp/             # MCP transports (stdio, http), connection lifecycle, convert (single protocol-conversion implementation), error types
│   └── lsp/             # LSP client, transport, error types
└── utils/
    ├── file_parser.rs   # Attachment content extraction (PDF, Office, images, text) + model path validation
    ├── file_tools.rs    # Agent file tools
    ├── git_tools.rs     # Agent git tools
    ├── shell_tools.rs   # Agent shell tools (sandboxed)
    ├── web_tools.rs     # Agent web tools
    ├── lsp_tools.rs     # Agent LSP tools
    ├── lsp_agent_tools.rs # LSP tool adapters for the agent loop
    ├── knowledge.rs     # Project knowledge (.aio/knowledge.json)
    ├── project_map.rs   # Project structure mapping
    ├── sandbox.rs       # Shell sandboxing (path allow/deny validation)
    ├── exec_resolve.rs  # PATH-only executable resolution (the single entry point for all bare-name spawns; Windows PATHEXT order normalization) + HiddenConsole (hide console windows for all child processes)
    ├── think.rs         # Reasoning tool
    ├── url_validation.rs # SSRF/URL safety checks
    ├── token_counter.rs # Token counting
    ├── tool_result.rs   # Unified tool-result envelope (tool_ok/tool_err/tool_ok_mcp/tool_err_mcp)
    ├── time.rs          # Time helpers (now_iso/now_unix_secs/days_from_civil, etc.)
    ├── truncate.rs      # Truncation helpers (character/byte head-tail truncation, truncate_tool_result)
    ├── diff.rs          # Diff preview (compute_diff_preview/simple_diff, char-boundary safe)
    ├── jsonrpc.rs       # JSON-RPC pending request map (PendingMap; shared by MCP and LSP clients)
    ├── project_index.rs # Project index helpers (projects file parsing)
    └── worktree.rs      # Subagent worktree isolation (create/commit/merge back to main branch/conflict abort/expired cleanup)
```

## Coding Conventions

### Rust

- Use `snake_case` for functions, variables, modules, and file names.
- Use `PascalCase` for structs, enums, and traits.
- Error handling: prefer `map_err(|e| e.to_string())?` in command functions. Avoid panicking in user-facing paths.
- All `#[tauri::command]` functions MUST have a doc comment describing params and behavior.
- Keep commands thin; delegate business logic to modules in `plugins/` or `utils/`.
- When adding a new Tauri command:
  1. Implement in the appropriate `commands/*.rs` file.
  2. Export it in `commands/mod.rs`.
  3. Register it in `lib.rs` via `tauri::generate_handler![...]`.

### Code Comments

- **Explain "why", not "what"**: self-explanatory lines need no comment; comments are for decisions, constraints, and pitfalls (security boundaries, compatibility constraints, platform quirks, performance trade-offs). Numbered `SEC-*`/`MCP-*`/`CHAT-*` references to review findings are the model to follow.
- **Language**: write comments in English first. Chinese may follow immediately as a second-language translation on the next line or paragraph; never place Chinese before English. New or edited code must not introduce Chinese-only comments. Existing Chinese-only comments are migrated when the surrounding code is touched — do not mass-translate unrelated files in a feature PR.
- **Module headers**: every `mod.rs`/complex module starts with a paragraph describing its responsibilities and boundaries (see `core/migrate.rs`, `commands/feedback.rs`).
- **Document public items**: pub fn / pub struct must state parameters, return values, side effects, and error cases (`///` doc comments, English first; an optional Chinese translation follows the English paragraphs). User-facing messages go through `ierr!` and stay bilingual.
- **unsafe / security-sensitive code** (sandbox, path validation, secret storage, IPC boundary): explain invariants and attack surface paragraph by paragraph; bare `unsafe` without a comment is forbidden.
- **No tombstone comments** (stale, commented-out dead code): delete it — git remembers everything.
- **Temporary switches**: debugging-only code must be marked `TODO(<name>)` or removed before merge; `TODO` must carry an issue number or an owner.

### Interface Contract (Tauri IPC / events / plugin traits)

- **Command naming**: `snake_case` starting with a verb (`start_local_server`, `export_diagnostic_package`); group by domain prefix (`memory_*`, `harness_*`). **Command names are the public frontend API and must never be renamed after release** — when evolving, keep the old name as a thin wrapper or deprecation marker (`#[deprecated]` documenting the replacement).
- **Payload serialization**: Rust uses `#[serde(rename_all = "camelCase")]` uniformly (legacy snake_case fields are mirrored faithfully on the TS side with a comment; see the `AppConfig` precedent); TS mirror types live in `core/api/streamPayloads.ts` or `core/types/` and **map one-to-one to the Rust structs**.
- **Commands must be async or return fast**: commands with real I/O (process/file/network/system-wide enumeration) always use `#[tauri::command(async)]` to run on the blocking pool — synchronous commands run on the main thread and freeze the UI (the `get_feedback_snapshot` pitfall after 0.8.3). Treat any new synchronous command as suspect during review.
- **Event contract**: backend→frontend event names are kebab-case and **registered on both sides** (`core/events.rs` and `core/api/events.ts`, enforced by a guard test); payload shapes are defined in `streamPayloads.ts` with a comment naming the corresponding backend struct. Events are broadcasts — payloads must not carry sensitive data (keys, unmasked paths).
- **Plugin traits**: registration order for `ProviderPlugin` / `LocalEnginePlugin` / harness adapters is dispatch order (`openai_compat` is always the fallback and registers last); trait method changes count as breaking interface changes and require a `BREAKING CHANGE` marker in the CHANGELOG.
- **Security boundary**: every command that accepts a frontend path argument must pass through `core/path_safety.rs`'s `safe_join`; error messages returned to the frontend must not include sensitive details beyond local absolute paths.

### SolidJS / TypeScript

- Components: `PascalCase` files and export names.
- Signals: `camelCase`, use `createSignal` for local component state.
- Store updates: use functional path syntax (`setDatas('assistants', a => a.id === id, 'name', value)`).
- Avoid `any` in new code. Define interfaces in `core/types/`.
- Utility functions go to `core/utils/`, never duplicate logic across components.
- **Styles use Tailwind CSS utility classes**: avoid inline styles or custom CSS files unless truly necessary. Tailwind v4 has no `tailwind.config`; theme tokens are the CSS variables in `src/index.css` (`--primary-rgb`, `--surface-bg`, `--text-base-rgb`, `--border-dim`, etc.), and dark mode is driven by the `isDarkMode` signal. New colors must go through CSS variables; hardcoded `rgba(...)` literals are forbidden.
- **Emoji characters are forbidden in code** (including JSX text, template strings, and config objects). Use the `<Icon name="..." />` component instead. In string contexts (`confirm()`, `alert()`), use plain-text equivalents (`✓` / `✗` / `[!]`).
- **Modal backgrounds are standardized**: every modal container uses the acrylic style `background: var(--dialog-bg)` + `backdrop-filter: var(--popup-blur)` + `border: 1px solid var(--border-dim)` (variables defined in `src/index.css`); the overlay is always `bg-black/25` (no blur). Reference: `src/features/chat/components/ProjectSettingsModal.tsx`.
- **Dropdowns are standardized**: every dropdown uses `src/shared/components/Dropdown.tsx`; settings/form rows use `variant="ghost"` (transparent trigger + `--border-dim` border; see the language selector in `src/features/settings/components/AppSettings.tsx`), while toolbars use the default `solid` variant. Native `<select>` elements are forbidden (`npm run check:gates` enforces it).
- **Checkboxes are standardized**: every checkbox uses `src/shared/components/Checkbox.tsx` (primary-colored outlined square + check icon; plain-text labels use the `label` prop, rich labels use children). Native `<input type="checkbox">` is forbidden (`npm run check:gates` enforces it).
- **Icons must not be replaced by characters**: graphical meaning in buttons/controls (close cross, arrows, checkmarks, etc.) must use `<Icon name="..." />` SVG icons, never characters (e.g., `x`/`×`/`✕` as a close icon or `→` as an arrow); JSX text nodes must not contain standalone `× ✕ ✓ ✗ → ← ▲ ▼` symbols (gate-enforced). The only exception is plain-text string contexts (`confirm()`/`alert()`/logs) where `✓`/`✗`/`[!]` are allowed. Add missing icons to the `PATHS` table in `src/shared/components/Icon.tsx` (factory-function form); never fall back to characters.
- **Equal heights within a row**: controls in the same flex row (buttons, inputs, dropdowns, labels) must use the same height tier — prefer constants from `shared/components/buttonStyles.ts` at the same tier (inline actions use the `px-3 py-1.5 text-xs` tier: `btnSecondarySm`/`btnDangerOutline`/`btnSuccessOutline` and other Sm/Outline variants). Mixing padding/font-size tiers in one row is forbidden (e.g., `btnBadge` at `px-2 py-1 text-[11px]` next to `btnDangerOutline` at `px-3 py-1.5 text-xs`); input radius and vertical padding in form rows must match the modal's dropdown trigger (`Dropdown` ghost is `rounded-lg py-1.5`, inputs use `rounded-lg`).
- **Semantic-colored buttons are always outline**: text-action buttons with semantic color (confirm/save/create, delete/stop/close, test connection/start/approve, save key, etc.) may only use the Outline family from `buttonStyles.ts` — the border and text carry the semantic color (`border-pri-50`+`text-pri`, `border-danger/50`+`text-danger`, `border-success/50`+`text-success`, `border-warn/50`+`text-warn`), while the button body stays neutral (`bg-white/[0.06]`, same as secondary; the light semantic tint appears only on hover). Solid semantic buttons (`bg-pri`) and tinted backgrounds (`bg-danger/10`/`bg-green-400/10`, etc.) are forbidden; neutral actions use `btnSecondary`/`btnSecondarySm`. The send/stop composite icon control in the composer and selected tabs are exempt.
- **i18n**: every user-facing string must come from `t()` (`core/i18n`); every key must exist in both `locales/zh-CN.json` and `locales/en-US.json` with an identical `{{placeholder}}` set (`npm run check:i18n` enforces it). Hardcoding UI copy in JSX/template strings is forbidden.
- **Child processes must hide the console**: release builds use the GUI subsystem (`#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`) and have no console of their own; any console-subsystem child process (cmd/powershell/git/npx/node/reg/taskkill…) without `CREATE_NO_WINDOW` makes Windows **create** a console window — users see a terminal flash. In debug mode children inherit the debug console, so this only reproduces in the installed build. Rule: every `std::process::Command` / `tokio::process::Command` must chain `.hidden_console()` (`utils::exec_resolve::HiddenConsole`, a cross-platform no-op); raw `CreateProcessW` paths (`utils/sandbox/win_appcontainer.rs`) must set `CREATE_NO_WINDOW` and `STARTF_USESHOWWINDOW`+`SW_HIDE` themselves. Piped stdio does not prevent console allocation (`npm run check:gates` enforces this).
- **UI copy restraint**: the UI only carries functional text (labels, states, operation results, error messages, one-line placeholders). Explanatory descriptions from development/design — feature intros, rationale, tutorials, long config-field explanations — never go into the UI; they belong in `docs/`. Before adding UI copy, ask: would the user be unable to use this feature without this sentence? Keep it only if yes. Prefer expressing operation status through the control's own state change (e.g., button text briefly switches to "Cancelled/Downloaded" for a second, a list row disappears, layout changes) over adding persistent status lines; error messages (including causes) are exempt.

### Naming & Strings

- **Do NOT hardcode engine names** (e.g., `"Local-Llama.cpp"`) anywhere. Use `model.engine_type` or URL-based heuristics (`isLocalUrl`).
- Frontend event names should be descriptive and kebab-case (e.g., `engine-progress`, `llm-chunk`).

## Extensibility Guide

- **Adding a New API Provider**: OpenAI-compatible providers usually need NO Rust changes — add the provider + model metadata to the sibling `aio-models-data` repo (consumed via `@aio/models-data`), add the logo mapping in `core/utils/modelLogo.ts`, and verify via Settings → Providers custom URL + "test connection". Only providers whose request/stream format differs (auth header, response JSON shape) get a dedicated plugin: implement `ProviderPlugin` in `src-tauri/src/plugins/provider/`, register it in `ProviderManager::new()` in `plugins/provider/mod.rs` BEFORE `openai_compat` (registration order is the dispatch order; `openai_compat` must stay last as fallback). UI lives in `features/settings/ProviderDetailPage.tsx` (shared provider card; the engine card reuses the same page).

- **Adding a New Local Inference Engine**: implement `LocalEnginePlugin` in a new `plugins/engine/<id>.rs` (methods: `name`, `identifier`, `supported_extensions`, `is_platform_supported`, `install_path`, `is_installed`, `start`, `build_command`, `parse_progress_from_log`, `detect_installation`, `default_port`, `progress_event_name`), register in `EngineManager::new()` in `plugins/engine/mod.rs`. Engine binaries are **user-installed by convention** — neither bundled with the app nor auto-downloaded: `detect_installation` probes PATH and common install locations (winget/conda/homebrew/nix), and the settings page links to the engine's official builds when missing. `plugins/engine/installer.rs` only provides install-dir and detection helpers (no download path — keep it that way unless the product decision changes). The frontend picks the engine up automatically via `engine_type` on `ActivatedModel` and the startup engine scan (`scan_installed_engines`).

- **Adding a New File Parser**: add the extension to the whitelist match in `utils/file_parser.rs` (`process_file_content`, plus `attachment_mime_type`/`validate_attachment_path` if the type is new), implement the parser as a private helper in the same module, add the extension to the `ALLOWED_DOC` / `ALLOWED_IMG` lists in `src/features/chat/ChatPage.tsx` (attachment picker), and update `docs/zh|en/reference/configuration.md` (attachment whitelist). The Rust whitelist is the trust boundary — never only change the picker.

More extension paths (MCP transports, new Tauri commands, agent tools, permission defaults) with test guidance: see `docs/zh|en/development/extensions.md` and `docs/zh|en/development/architecture.md`.

## State Management Rules

### Global State (`core/store/`)

Global state is split into domain modules (`store.ts` is only a barrel re-export, external import paths stay unchanged; interfaces live in `types.ts`):

- `chat.ts`: `datas` (assistants/topics/messages) + app config + MCP state + Skills + title renaming
- `models.ts`: selected model / model catalog / provider configs / modelKey / isLocalModel
- `engine.ts`: local engine start / progress / connection / scan cache
- `project.ts` / `profiles.ts` / `ui-prefs.ts` / `update.ts` / `task-board.ts` / `stream.ts` / `layout.ts` / `background-tasks.ts`: their own domains
- `harness.ts`: Harness workbench (tool detection/workspaces/topics/messages/drafts/approvals/run state; selection persisted to localStorage)

Key signals: `datas` (Store: assistants/activatedModels), `config` (apiUrl/apiKey), `selectedModel`, `currentAssistantId`, `currentTopicId`, `projects`/`currentProjectId` (+ `gitBranch`/`gitBranches`), `mcpServers`/`mcpServerStatus`/`mcpToolsCache`, `skills`, `providerConfigs`, `modelsCatalog*`, `profileModelOverrides`, `customSubagentProfiles`, `engineScanResults`, `appUpdate*`, harness domain signals (`harnessTools`/`harnessWorkspaces`/`harnessTopics`/`harnessMessages`/`harnessDrafts`/`harnessApprovals`/`harnessRunStates`); UI preferences (`themeColor`/`isDarkMode`/`reasoningLevel`/`webSearchEnabled`) persist to localStorage.

### Backend State (`core/state.rs` + `lib.rs`)

- `StreamManager`: active LLM stream tasks keyed `"{assistant_id}-{topic_id}"`; stop via `CancellationToken::cancel()` so the task exits gracefully and emits its epilogue — never `abort()`.
- `DbState`: SQLite connection.
- `LocalEngineState`: running local engine processes, HashMap keyed by `engine_type`; one active engine at a time enforced by `start_local_server`.
- `SubagentHandles`: background subagent join handles.
- Plus managers registered in `lib.rs`: `EngineManager`, `McpServerManager`/`McpServerState`/`McpRequestManager`/`PendingApprovals`, `LspManager`, `FileLockRegistry` (multi-agent write coordination, see `core/file_locks.rs`), `HarnessManager` (harness session runtime: child process + command channel + cancellation token + approval oneshot map; two-stage stop = protocol cancel → taskkill process tree).

## Important Compatibility Notes

- **Command Stability**: Public Tauri command names (`start_local_server`, `stop_local_server`, `is_local_server_running`, `call_llm_stream`, etc.) must remain stable to preserve frontend compatibility.
- **Config Migration**: The `ActivatedModel` struct supports an optional `engine_type` field. Old configs without this field deserialize safely (defaults to `None`), and logic treats `None` as legacy llama.cpp behavior.
- **Event Names**: `engine-progress` is the generic progress event (all engines default to it); the llama.cpp plugin still emits `llama-progress` for backward compatibility. Other frontend events: `llm-chunk`, `llm-round-start`, `llm-compression`, `llm-compression-failed`, `subagent-start` / `subagent-step` / `subagent-done` / `subagent-error`, `mcp-server-status`, `mcp-server-stderr`, `app-update-progress`, and the harness event family (`harness-event` / `harness-state` / `harness-approval-requested` / `harness-approval-expired`). New events must be kebab-case.

## Branch & Release Workflow

Multi-contributor conventions (process details and commit-message rules live in [CONTRIBUTING.md](CONTRIBUTING.md)):

- **Day-to-day development**: cut a feature branch from the latest `dev`, named `dev-<type>-<topic>` (e.g. `dev-feat-memory-panel`; Git cannot create a `dev/<feature>` namespace while the `dev` branch exists, hence hyphens); one branch does one thing, and changes are committed to the feature branch.
- **Feature completion**: PR the feature branch to `dev`; merge after CI (ci.yml, same `npm run verify` baseline) is green and a CODEOWNERS review approves; delete the feature branch afterwards (local + remote). External contributors use fork PRs.
- **Releases**: when releasing, PR `dev` to `master` and merge; leave `dev` untouched. Then bump versions (CHANGELOG + three version locations), and push a `vX.Y.Z` tag to trigger `release.yml`.
- **master** contains released content only; the only long-lived branches are `dev` and `master`; force-pushing either is forbidden.
- **Prereleases**: tags such as `v1.0.0-alpha` (a hyphen marks a prerelease) — release.yml uses the version number to mark the GitHub Release as a prerelease automatically (`releases/latest` keeps pointing at the stable line, so stable users are never pushed an alpha); alpha testers download manually from that Release page.
- **Local exe rebuild (quick acceptance, no installer)**: `cargo +1.97.1 build --release --manifest-path src-tauri/Cargo.toml --features tauri/custom-protocol` — **`--features tauri/custom-protocol` is mandatory**: Tauri 2's `generate_context!` uses it to distinguish dev/prod (without it the context is baked for dev: dist is not embedded and the webview loads devUrl, so a bare cargo build opens with "localhost refused to connect" — `tauri dev` and CI's `tauri build` each handle it implicitly, but bare cargo does not). Window usability must be judged by webview content (CDP screenshot or PrintWindow), not by the process reporting "responding".

## Build & Dev Commands

```bash
# Install locked dependencies
npm ci

# Dev mode (frontend + Tauri)
npm run tauri dev

# Production build
npm run tauri build

# Format (prettier + cargo fmt)
npm run format

# Full frontend and Rust quality baseline:
#   check:frontend = check:i18n (locale key parity) + check:docs (markdown links) + check:gates (architecture/style gates) + prettier --check + eslint --max-warnings 0 + tsc --noEmit + vite build
#   check:rust     = cargo fmt --check + clippy --all-targets -D warnings + cargo test
npm run verify
```

## Review Checklist for Agents

Before submitting changes, verify:

- [ ] New Rust modules are declared in their parent `mod.rs`.
- [ ] New Tauri commands are registered in `lib.rs`.
- [ ] No hardcoded engine/provider names in logic; use constants, `engine_type` fields, or `core/utils/modelLogo.ts` mappings.
- [ ] `npm run verify` passes without warnings or errors.
- [ ] New UI strings are i18n'd: keys exist in BOTH `locales/zh-CN.json` and `locales/en-US.json` with matching `{{placeholders}}`.
- [ ] No hardcoded color literals; theme colors go through CSS variables in `src/index.css`.
- [ ] New backend→frontend event names are registered on both sides (`src-tauri/src/core/events.rs` and `src/core/api/events.ts`, kebab-case).
- [ ] New Tauri commands are called through `core/api/<domain>.ts` wrappers; components never call `invoke` directly.
- [ ] `npm run check:gates` passes (bare-Chinese JSX / colors / `any` ratchet / layering / file-size / command registration / hidden-console for child processes).
- [ ] Commands with I/O are async (blocking pool); new commands have doc comments with parameter descriptions (see Interface Contract).
- [ ] New or changed comments are English-first (Chinese may follow as the second language); no new Chinese-only comments.
- [ ] Commit messages use an English subject and body; an optional Chinese translation follows the English body.
- [ ] Doc changes are synced on both sides (`docs/zh/` and `docs/en/`) and pass `check:docs`.

## Agent Collaboration Rules (AI-assisted development)

This repository welcomes AI agents and much of the work is already agent-produced. Agents follow the same standards as humans (coding standards in this file, process in CONTRIBUTING.md), plus these red lines:

- **Verification honesty (most important)**: any claim of "verified / all green / tests pass" must include the commands actually run and the real output. Never claim unexecuted tests pass; clearly state what could not run locally (e.g., platform-specific tests) and leave it to CI.
- **No gate bypass**: do not loosen `check-gates.mjs` thresholds, use `--no-verify`, special-case code to pass checks, or skip cases. Gate changes are separate PRs and require maintainer approval.
- **Read before asserting**: read the source and docs before drawing conclusions about uncertain behavior/interfaces; when citing a rule, name the source file and section — never paraphrase from memory.
- **Minimal diffs**: do only what the task requires — no drive-by refactors, no mass formatting of unrelated files, no unrelated dependency bumps; record adjacent problems as issues instead of fixing them in passing.
- **PRs mandatory**: agent output always goes through the PR flow (template, CI, review) and is never pushed directly to `dev` / `master`; commit messages follow Conventional Commits; never impersonate human work or invent review status.
- **Secret hygiene**: never commit keys/tokens/personal data; sanitize logs and screenshots; use fabricated content for examples and conversations (the repo once had real data slip into a screenshot, and re-shooting is expensive).
- **Report blockers**: when stuck, rejected by the environment, or facing a conflict between the request and the standards, stop and report with evidence — never silently work around it or lower the completion bar.

### Multi-Agent Concurrent Writing Coordination

When multiple agents (or agents and humans) modify the repository at the same time: `git pull` before editing the same file; `core/file_locks.rs` is the product's runtime file lock (unrelated to git write coordination); resolve git-level conflicts with "small PRs + single-topic branches" — align on an issue before starting large refactors.

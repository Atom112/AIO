# AIO (All-In-One AI) — Agent Coding Guide

## Project Overview

AIO is a lightweight, cross-platform AI assistant desktop client built with **Tauri 2.x** (Rust backend) and **SolidJS** (frontend). It supports remote API providers (OpenAI-compatible, Anthropic, Google Gemini, Ollama), local inference engines (llama.cpp, Ollama, vLLM — multi-engine, one active at a time), and an **Agent mode** that runs tool-augmented loops (file, git, shell, web, LSP, knowledge tools via MCP), spawns subagents, and enforces a three-tier permission model. Other subsystems: project workspaces with Git integration, MCP server management (stdio/HTTP transports, catalog, tool approvals), skills (built-in, market, npx import), LSP diagnostics, usage tracking, app self-update, and full zh-CN/en-US localization. User docs live in `docs/` (mirrored: `docs/zh/`, `docs/en/`); development guides are in `docs/zh|en/development/`.

## Tech Stack & Versions

| Layer             | Technology                                                               | Version                   |
| ----------------- | ------------------------------------------------------------------------ | ------------------------- |
| Frontend          | SolidJS                                                                  | ^1.9.13                   |
| Routing           | @solidjs/router                                                          | ^0.16.1                   |
| Styling           | Tailwind CSS (v4, via `@tailwindcss/postcss`, no tailwind.config)        | ^4.0.0                    |
| Build Tool        | Vite                                                                     | ^8.0.16                   |
| Desktop Framework | Tauri                                                                    | 2.11.x                    |
| Tauri Plugins     | dialog, fs, shell, opener, updater                                       | 2.x                       |
| Backend Language  | Rust (pinned in `src-tauri/rust-toolchain.toml`; use `cargo +1.97.1`)    | 1.97.1                    |
| Database          | SQLite (rusqlite)                                                        | bundled                   |
| i18n              | @solid-primitives/i18n (locales: zh-CN, en-US)                           | ^2.2.1                    |
| State Management  | SolidJS Store + Signals                                                  | built-in                  |
| Models Catalog    | @aio/models-data (sibling local package; JSON bundled as tauri resource) | `file:../aio-models-data` |
| Provider Icons    | @lobehub/icons-static-svg                                                | ^1.91.0                   |

## Directory Structure

### Frontend (`src/`)

```
src/
├── index.tsx            # App entry: Router + lazy routes + startup inits (projects, MCP, skills, engine scan)
├── Layout.tsx           # Root layout with NavBar
├── index.css            # Tailwind v4 CSS + theme CSS variables (light/dark mode)
├── core/
│   ├── store/           # store.ts: global reactive state AND core interfaces (Message, Topic, Assistant, ActivatedModel, AgentStep...); diagnostics.ts
│   ├── types/           # Backend mirror types: mcp.ts, skill.ts
│   ├── utils/           # modelLogo.ts (provider logos), models.ts (ProviderConfig/Catalog), mcp.ts, exportConversation.ts
│   ├── i18n/            # index.ts (t(), locale) + locales/zh-CN.json + locales/en-US.json
│   ├── shortcuts.ts     # Keyboard shortcut + slash command registry
│   ├── agent-prompts.ts # Agent system-prompt presets
│   └── assets/
├── features/
│   ├── chat/            # ChatPage.tsx + components/ (ChatInterface, TopicSidebar, ProjectSidebar, AgentProcessBlock, SubagentBlock, AgentModeSelector, ReasoningButton, TokenBar, TokenStatsBar, ShareModal, ProjectSettingsModal, ProjectSelector, ...) + hooks/
│   └── settings/        # SettingsPage.tsx, ProviderDetailPage.tsx + components/ (ProviderList, AppSettings, McpServerList, McpServerDetail, SkillList, SubagentModelSettings, UsageSettings + usage chart components)
└── shared/
    └── components/      # Icon, Markdown, NavBar, CommandPalette, SlashCommandMenu, Dropdown, UserDropdown, Switch, ModelRow, GlobalKeyboardHandler, DiffView, ThinkBlock, UpdateNotification, AvatarCropModel
```

### Backend (`src-tauri/src/`)

```
src-tauri/src/
├── main.rs
├── lib.rs               # Bootstrap: tracing init, managed state, invoke handler registration, engine cleanup on window destroy
├── mcp_fs_server.rs     # Built-in stdio filesystem MCP server (invoked with --fs-server)
├── core/
│   ├── models.rs        # Shared data structures (ActivatedModel, StreamPayload, ToolResultPayload, ...)
│   ├── db.rs            # SQLite initialization and schema
│   ├── state.rs         # Tauri managed state (StreamManager, DbState, LocalEngineState, SubagentHandles)
│   ├── permission.rs    # Agent tool permission model (allowed/denied rules per agent mode)
│   ├── secure_store.rs  # OS keychain-backed secret storage (API keys, MCP server secrets)
│   └── subagent.rs      # Subagent profile definitions (9 built-in) + tool specs
├── commands/
│   ├── mod.rs
│   ├── config.rs        # Assistants, app config, avatars, profile model overrides, custom subagent profiles, auto-start, topic branching
│   ├── llm/             # mod.rs: streaming, agent loop, subagents, summarization, usage, token counting; btw.rs: "By The Way" Q&A
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
│   └── update.rs        # App update check/install/restart
├── plugins/
│   ├── engine/          # LocalEnginePlugin trait + llama_cpp, ollama, vllm, installer
│   ├── provider/        # ProviderPlugin trait + openai_compat (fallback, registered last), anthropic, google, ollama
│   ├── mcp/             # MCP transports (stdio, http), connection lifecycle, error types
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
    ├── sandbox.rs       # Shell sandboxing (H-rule path checks)
    ├── think.rs         # Reasoning tool
    ├── url_validation.rs # SSRF/URL safety checks
    └── token_counter.rs # Token counting
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

### SolidJS / TypeScript

- Components: `PascalCase` files and export names.
- Signals: `camelCase`, use `createSignal` for local component state.
- Store updates: use functional path syntax (`setDatas('assistants', a => a.id === id, 'name', value)`).
- Avoid `any` in new code. Define interfaces in `core/types/`.
- Utility functions go to `core/utils/`, never duplicate logic across components.
- **样式统一使用 Tailwind CSS 工具类编写**，避免内联样式或自定义 CSS 文件，除非确有必要。Tailwind v4 无 `tailwind.config`；主题令牌是 `src/index.css` 中的 CSS 变量（`--primary-rgb`、`--surface-bg`、`--text-base-rgb`、`--border-dim` 等），暗色模式由 `isDarkMode` 信号切换，新颜色必须走 CSS 变量，禁止硬编码 `rgba(...)` 字面量。
- **严禁在代码中使用 Emoji 字符（包括 JSX 文本、模板字符串、配置对象）。一律使用 `<Icon name="..." />` 组件替换。对于字符串上下文（`confirm()`、`alert()`），使用纯文本等价符号（`✓` / `✗` / `[!]`）。**
- **i18n**：所有面向用户的字符串必须通过 `t()`（`core/i18n`）取词；每个 key 必须同时存在于 `locales/zh-CN.json` 与 `locales/en-US.json`，且 `{{placeholder}}` 集合一致（`npm run check:i18n` 强制校验）。禁止在 JSX/模板字符串中硬编码 UI 文案。

### Naming & Strings

- **Do NOT hardcode engine names** (e.g., `"Local-Llama.cpp"`) anywhere. Use `model.engine_type` or URL-based heuristics (`isLocalUrl`).
- Frontend event names should be descriptive and kebab-case (e.g., `engine-progress`, `llm-chunk`).

## Extensibility Guide

- **Adding a New API Provider**: OpenAI-compatible providers usually need NO Rust changes — add the provider + model metadata to the sibling `aio-models-data` repo (consumed via `@aio/models-data`), add the logo mapping in `core/utils/modelLogo.ts`, and verify via Settings → Providers custom URL + "test connection". Only providers whose request/stream format differs (auth header, response JSON shape) get a dedicated plugin: implement `ProviderPlugin` in `src-tauri/src/plugins/provider/`, register it in `ProviderManager::new()` in `plugins/provider/mod.rs` BEFORE `openai_compat` (registration order is the dispatch order; `openai_compat` must stay last as fallback). UI lives in `features/settings/ProviderDetailPage.tsx` (shared provider card; engine card reuses the same page).

- **Adding a New Local Inference Engine**: implement `LocalEnginePlugin` in a new `plugins/engine/<id>.rs` (methods: `name`, `identifier`, `supported_extensions`, `is_platform_supported`, `install_path`, `is_installed`, `start`, `build_command`, `parse_progress_from_log`, `detect_installation`, `default_port`, `progress_event_name`), register in `EngineManager::new()` in `plugins/engine/mod.rs`, and hook auto-install into `plugins/engine/installer.rs` if the engine ships binaries. Engine binaries are NOT bundled — `tauri.conf.json` `bundle.resources` only ships the models catalog; engines install into app data at `install_path()`. The frontend picks the engine up automatically via `engine_type` on `ActivatedModel` and the startup engine scan (`scan_installed_engines`).

- **Adding a New File Parser**: add the extension to the whitelist match in `utils/file_parser.rs` (`process_file_content`, plus `attachment_mime_type`/`validate_attachment_path` if the type is new), implement the parser as a private helper in the same module, add the extension to the `ALLOWED_DOC` / `ALLOWED_IMG` lists in `src/features/chat/ChatPage.tsx` (attachment picker), and update `docs/zh|en/reference/configuration.md` (attachment whitelist). The Rust whitelist is the trust boundary — never only change the picker.

More extension paths (MCP transports, new Tauri commands, agent tools, permission defaults) with test guidance: see `docs/zh|en/development/extensions.md` and `docs/zh|en/development/architecture.md`.

## State Management Rules

### Global State (`core/store/store.ts`)

- `datas` (Store): `assistants`, `activatedModels`.
- Signals: `config` (apiUrl/apiKey), `selectedModel`, `currentAssistantId`, `currentTopicId`, `projects`/`currentProjectId` (+ Git branch signals: `gitBranch`, `gitBranches`), `mcpServers`/`mcpServerStatus`/`mcpToolsCache`, `skills`, `providerConfigs`, `modelsCatalog` (+ status/source), `profileModelOverrides`, `customSubagentProfiles`, `engineScanResults`, `workflowState`, `appUpdate*`; UI-preference signals (`themeColor`, `isDarkMode`, `reasoningLevel`, `webSearchEnabled`) persist to localStorage.

### Backend State (`core/state.rs` + `lib.rs`)

- `StreamManager`: active LLM stream tasks keyed `"{assistant_id}-{topic_id}"`; stop via `CancellationToken::cancel()` so the task exits gracefully and emits its epilogue — never `abort()`.
- `DbState`: SQLite connection.
- `LocalEngineState`: running local engine processes, HashMap keyed by `engine_type`; one active engine at a time enforced by `start_local_server`.
- `SubagentHandles`: background subagent join handles.
- Plus managers registered in `lib.rs`: `EngineManager`, `McpServerManager`/`McpServerState`/`McpRequestManager`/`PendingApprovals`, `LspManager`.

## Important Compatibility Notes

- **Command Stability**: Public Tauri command names (`start_local_server`, `stop_local_server`, `is_local_server_running`, `call_llm_stream`, etc.) must remain stable to preserve frontend compatibility.
- **Config Migration**: The `ActivatedModel` struct supports an optional `engine_type` field. Old configs without this field deserialize safely (defaults to `None`), and logic treats `None` as legacy llama.cpp behavior.
- **Event Names**: `engine-progress` is the generic progress event (all engines default to it); the llama.cpp plugin still emits `llama-progress` for backward compatibility. Other frontend events: `llm-chunk`, `llm-round-start`, `llm-compression`, `llm-compression-failed`, `subagent-start` / `subagent-step` / `subagent-done` / `subagent-error`, `mcp-server-status`, `mcp-server-stderr`, `app-update-progress`. New events must be kebab-case.

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
#   check:frontend = check:i18n (locale key parity) + check:docs (markdown links) + prettier --check + eslint --max-warnings 0 + tsc --noEmit + vite build
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

# AIO (All-In-One AI) — Agent Coding Guide

## Project Overview

AIO is a lightweight, cross-platform AI assistant desktop client built with **Tauri 2.x** (Rust backend) and **SolidJS** (frontend). It supports remote API providers (OpenAI-compatible, Anthropic, Google Gemini, Ollama), local inference engines (llama.cpp, Ollama, vLLM — multi-engine, one active at a time), and an **Agent mode** that runs tool-augmented loops (file, git, shell, web, LSP, knowledge tools via MCP), spawns subagents, and enforces a three-tier permission model. Other subsystems: a **Harness workbench** (`/harness` page) that drives third-party CLI coding agents (Claude Code, Codex, OpenCode, Oh My Pi, Gemini CLI, Qwen Code, goose, DSH) over the Agent Client Protocol and renders their streams natively (AIO's own tools stay out of the loop there), project workspaces with Git integration, MCP server management (stdio/HTTP transports, catalog, tool approvals), skills (built-in, market, npx import), LSP diagnostics, usage tracking, app self-update, and full zh-CN/en-US localization. User docs live in `docs/` (mirrored: `docs/zh/`, `docs/en/`); development guides are in `docs/zh|en/development/`.

## Tech Stack & Versions

| Layer             | Technology                                                                            | Version                   |
| ----------------- | ------------------------------------------------------------------------------------- | ------------------------- |
| Frontend          | SolidJS                                                                               | ^1.9.13                   |
| Routing           | @solidjs/router                                                                       | ^0.16.1                   |
| Styling           | Tailwind CSS (v4, via `@tailwindcss/postcss`, no tailwind.config)                     | ^4.0.0                    |
| Build Tool        | Vite                                                                                  | ^8.0.16                   |
| Desktop Framework | Tauri                                                                                 | 2.11.x                    |
| Tauri Plugins     | dialog, opener, updater（fs/shell 已移除——前端零使用，全部文件/命令操作走自有命令层） | 2.x                       |
| Backend Language  | Rust (pinned in `src-tauri/rust-toolchain.toml`; use `cargo +1.97.1`)                 | 1.97.1                    |
| Database          | SQLite (rusqlite)                                                                     | bundled                   |
| i18n              | @solid-primitives/i18n (locales: zh-CN, en-US)                                        | ^2.2.1                    |
| State Management  | SolidJS Store + Signals                                                               | built-in                  |
| Models Catalog    | @aio/models-data (sibling local package; JSON bundled as tauri resource)              | `file:../aio-models-data` |
| Provider Icons    | @lobehub/icons-static-svg                                                             | ^1.91.0                   |

## Directory Structure

### Frontend (`src/`)

```
src/
├── index.tsx            # App entry: Router + lazy routes + startup inits (projects, MCP, skills, engine scan)
├── Layout.tsx           # Root layout with NavBar
├── index.css            # Tailwind v4 CSS + theme CSS variables (light/dark mode)
├── core/
│   ├── store/           # 域模块：types/models/chat/engine/project/profiles/ui-prefs/update/workflow（store.ts 仅作 barrel 再导出；接口在 types.ts）；diagnostics.ts
│   ├── api/             # Tauri 命令封装层（chat/mcp/skills/config/update/engine/project/chat/attachment/lsp/usage/providers/memory/catalog/dialog）+ events.ts（事件名常量，双端守门测试）+ streamPayloads.ts（流事件载荷类型）—— 组件与 utils 均不得裸 invoke
│   ├── types/           # Backend mirror types: mcp.ts, skill.ts
│   ├── utils/           # modelLogo.ts (provider logos), models.ts (ProviderConfig/Catalog 缓存+查询，invoke 已下沉 api/catalog), mcp.ts, exportConversation.ts —— 禁止裸 invoke（门禁 layering-utils）
│   ├── i18n/            # index.ts (t(), locale) + locales/zh-CN.json + locales/en-US.json
│   ├── shortcuts.ts     # Keyboard shortcut + slash command registry
│   ├── agent-prompts.ts # Agent system-prompt presets
│   └── assets/
├── features/
│   ├── chat/            # ChatPage.tsx + hooks/（chatEventListeners.ts：18 个流监听器在根 Layout 全局注册 + 3 个拖放监听器页面级；chatTurnEffects.ts：回合收尾效果）+ constants.ts + components/ (ChatInterface, TopicSidebar, ProjectSidebar, AgentProcessBlock, SubagentBlock, AgentModeSelector, ReasoningButton, TokenBar, TokenStatsBar, ShareModal, ProjectSettingsModal, ProjectSelector, TaskBoardPanel, AgentQuestionBubble, ...)
│   ├── harness/         # HarnessPage.tsx（/harness：CLI 工具图形化兼容层）+ hooks/harnessEventListeners.ts（全局注册）+ components/（HarnessToolSidebar 两级工具/工作区栏, HarnessTopicSidebar, HarnessChatInterface, HarnessMessageList, HarnessApprovalBubble, HarnessAddWorkspaceModal）——复用 chat 叶子组件渲染
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
│   ├── models/          # Shared data structures (chat/config/mcp/skill/project/agent/engine 域拆分)
│   ├── db.rs            # SQLite initialization（走 core/migrate.rs 版本化迁移）
│   ├── state.rs         # Tauri managed state (StreamManager, DbState, LocalEngineState, SubagentHandles)
│   ├── config.rs        # AppConfig 持久化（save_app_config/load_app_config；命令层仅存薄包装）
│   ├── events.rs        # 后端→前端事件名常量注册表（kebab-case 守门测试；新事件须登记）
│   ├── file_locks.rs    # 多智能体写协调：per-file 互斥锁 + last-writer 追踪 + 跨写者 hashline 门控
│   ├── fs_util.rs       # 原子写（temp+rename）公共工具——所有 JSON/配置持久化必须走这里
│   ├── ids.rs           # 内置 ID 常量（BUILTIN_SERVER_ID / BUILTIN_FS_SERVER_ID）
│   ├── mcp_trust.rs     # 项目级 .aio/mcp-servers.json 信任存储（指纹键控，仓库之外）
│   ├── migrate.rs       # 版本化 SQLite 迁移运行器（PRAGMA user_version；chat 库与记忆库共用）
│   ├── model_registry.rs # 嵌入/重排模型注册表（MANIFEST）与目录解析
│   ├── path_safety.rs   # IPC 前端路径参数校验（safe_join：拒绝绝对路径/`..`/保留字符）
│   ├── paths.rs         # 应用数据目录 / 包 ID
│   ├── permission.rs    # Agent tool permission model (allowed/denied rules per agent mode)
│   ├── secure_store.rs  # OS keychain-backed secret storage (API keys, MCP server secrets)
│   └── subagent.rs      # Subagent profile definitions (10 built-in, incl. toolless vision) + tool specs
├── commands/
│   ├── mod.rs
│   ├── config.rs        # Assistants, app config, profile model overrides, custom subagent profiles, auto-start, topic branching
│   ├── llm/             # mod.rs（薄 IPC 包装 + run_agent_turn_inner 主循环）: streaming/agent loop/budget/handoff/messages/subagents/tasks（任务清单 + ask_user）/titles/tool_specs/usage/locked_tools（文件锁接线）；btw.rs: "By The Way" 侧问
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
│   ├── local_model_market.rs  # 本地 GGUF 模型市场：HF/ModelScope 搜索合并 + 硬件档案 + HF Token
│   ├── local_model_download.rs # 本地 GGUF 下载器：多源回退/断点续传/空间预检/目录迁移
│   ├── embedding.rs     # 嵌入模型：测试 / 列表 / 下载 / 启停 / 目录
│   ├── rerank.rs        # 重排模型：测试 / 保存 API Key
│   ├── memory.rs        # 项目记忆命令（memory_*：CRUD/混合检索/统计/开关）
│   ├── harness.rs       # Harness 工作台命令（harness_*：工具检测/工作区/话题/消息/发送/停止/审批）
│   ├── export.rs        # 会话导出（Markdown/HTML/PDF 素材）
│   ├── image_gen.rs     # 图像生成命令
│   └── update.rs        # App update check/install/restart
├── services/            # 领域服务层（被命令层调用；不得反向依赖 commands）
│   └── memory/          # 项目级记忆/RAG 服务
│       ├── mod.rs       # 域入口 + collect_rows_warn（行级错误可见化）+ 迁移/读取拆分（migrate_* 显式写、resolve_* 纯读）
│       ├── manager.rs   # 每项目 SQLite 连接 LRU 缓存
│       ├── store/       # 记忆库存储层（mod/search/config/graph/stats）：facts + 版本审计 + FTS5 全文 + 向量打分 + 代码符号索引
│       ├── code.rs      # 代码库索引与向量检索（async + spawn_blocking）
│       ├── retrieve.rs  # 检索后处理：二阶段精排（rerank）+ 查询改写 + 多查询 RRF 融合（fail-open）
│       ├── extract.rs   # LLM 后台事实抽取
│       ├── judge.rs     # 事实判重/仲裁（余弦相似度阈值）
│       ├── consolidate.rs # sleep-time 合并整理（每日自动 + 手动）
│       ├── graph.rs     # 关系图谱抽取
│       ├── llm.rs       # 记忆管线专用的 LLM 调用薄封装
│       ├── tools.rs     # Agent 记忆工具（remember/recall/search_memory）
│       ├── vec0.rs      # SQLite vec0 向量扩展适配
│       ├── vector.rs    # 向量工具（bytes_to_f32/brute_force_topk）
│       └── eval.rs      # RAG 评测（数据集 src-tauri/tests/rag_eval）
├── plugins/
│   ├── engine/          # LocalEnginePlugin trait + llama_cpp, ollama, vllm, installer
│   ├── provider/        # ProviderPlugin trait + openai_compat (fallback, registered last), anthropic, google, ollama
│   ├── harness/         # Harness 工作台（CLI 工具兼容层）：model（归一化事件）/adapter（DriverCtx+trait+spawn）/registry（内置 ACP 工具注册表+检测）/acp（ACP JSON-RPC 驱动）/store（harness_* 三表）/manager（会话运行时+审批解析）
│   ├── llm.rs           # LLM HTTP 客户端（post_chat_completion/chat_text_content 等，命令层已下沉至此）
│   ├── embed/           # 嵌入 provider 插件（local_onnx + cache）
│   ├── rerank/          # 重排 provider 插件（local_onnx + openai_compat）
│   ├── mcp/             # MCP transports (stdio, http), connection lifecycle, convert（协议转换单实现）, error types
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
    ├── sandbox.rs       # Shell sandboxing（路径白名单/黑名单校验）
    ├── exec_resolve.rs  # PATH-only 可执行文件解析（全部裸名 spawn 的唯一入口；Windows PATHEXT 顺序归一）
    ├── think.rs         # Reasoning tool
    ├── url_validation.rs # SSRF/URL safety checks
    ├── token_counter.rs # Token counting
    ├── tool_result.rs   # 工具结果统一封装（tool_ok/tool_err/tool_ok_mcp/tool_err_mcp）
    ├── time.rs          # 时间辅助（now_iso/now_unix_secs/days_from_civil 等）
    ├── truncate.rs      # 截断辅助（字符/字节头尾截断、truncate_tool_result）
    ├── diff.rs          # diff 预览（compute_diff_preview/simple_diff，含 char-boundary 安全）
    ├── jsonrpc.rs       # JSON-RPC pending 请求表（PendingMap；MCP 与 LSP 客户端共用）
    ├── project_index.rs # 项目索引辅助（projects 文件解析）
    └── worktree.rs      # 子智能体 worktree 隔离（创建/提交/合并回主分支/冲突中止/过期清理）
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
- **弹窗（模态）背景统一**：所有模态弹窗的容器必须使用亚克力样式 `background: var(--dialog-bg)` + `backdrop-filter: var(--popup-blur)` + `border: 1px solid var(--border-dim)`（变量定义在 `src/index.css`），遮罩层固定为 `bg-black/25`（遮罩不加模糊）。参照实现：`src/features/chat/components/ProjectSettingsModal.tsx`。
- **下拉菜单统一**：所有下拉选择一律使用 `src/shared/components/Dropdown.tsx`；设置页/表单行内使用 `variant="ghost"`（透明触发器 + `--border-dim` 边框，参照 `src/features/settings/components/AppSettings.tsx` 的语言选择器），工具栏使用默认 `solid` 变体。禁止原生 `<select>` 元素（`npm run check:gates` 门禁强制执行）。
- **勾选框统一**：所有勾选框一律使用 `src/shared/components/Checkbox.tsx`（主色描边方块 + check 图标；纯文本标签用 `label` 属性，富文本标签用 children）。禁止原生 `<input type="checkbox">`（`npm run check:gates` 门禁强制执行）。
- **图标禁用字符代替**：按钮/控件中表达图形语义（关闭叉号、箭头、对勾等）一律使用 `<Icon name="..." />` SVG 图标，禁止用字符代替（如用 `x`/`×`/`✕` 当叉号、`→` 当箭头）；JSX 文本节点中禁止出现独占一个元素内容的 `× ✕ ✓ ✗ → ← ▲ ▼` 等符号字符（门禁强制执行）。仅有的例外是字符串上下文（`confirm()`/`alert()`/日志等纯文本）中的 `✓`/`✗`/`[!]`。图标缺失时在 `src/shared/components/Icon.tsx` 的 `PATHS` 表中新增（工厂函数形式），不得退用字符。
- **同行元素高度相等**：同一行（同一 flex 行容器）内的按钮/输入框/下拉/标签等控件必须使用相同的高度档位——优先复用 `shared/components/buttonStyles.ts` 中同一档位的常量（行内操作统一 `px-3 py-1.5 text-xs` 档：`btnSecondarySm`/`btnDangerOutline`/`btnSuccessOutline` 等 Sm/Outline 系列），禁止同一行混用不同 padding/字号档位（如 `btnBadge` 的 `px-2 py-1 text-[11px]` 与 `btnDangerOutline` 的 `px-3 py-1.5 text-xs` 并排）；表单行内输入框的圆角与垂直内边距也需与同弹窗的下拉触发器一致（`Dropdown` ghost 变体为 `rounded-lg py-1.5`，输入框用 `rounded-lg` 匹配）。
- **语义色按钮一律 outline**：带语义颜色的文本操作按钮（确定/保存/创建、删除/停止/关闭、测试连接/启动/批准、存密钥等）只允许 `buttonStyles.ts` 的 Outline 家族——**边框与文字携带语义色**（`border-pri-50`+`text-pri`、`border-danger/50`+`text-danger`、`border-success/50`+`text-success`、`border-warn/50`+`text-warn`），按钮主体保持中性底（`bg-white/[0.06]`，与 secondary 一致，hover 才浮现浅语义色底）。禁止实底语义按钮（`bg-pri` 实色）与色块底（`bg-danger/10`/`bg-green-400/10` 等）；中性操作用 `btnSecondary`/`btnSecondarySm`。发送/停止复合图标控件（输入区主控件）与 tab 选中态不在此规则约束内。
- **i18n**：所有面向用户的字符串必须通过 `t()`（`core/i18n`）取词；每个 key 必须同时存在于 `locales/zh-CN.json` 与 `locales/en-US.json`，且 `{{placeholder}}` 集合一致（`npm run check:i18n` 强制校验）。禁止在 JSX/模板字符串中硬编码 UI 文案。
- **UI 文案克制**：界面上只放功能性文字（标签、状态、操作结果、错误提示、一句话占位符）。开发/设计阶段的解释性描述——功能介绍、原理说明、使用教程、配置字段的长说明——一律不写入 UI，它们属于 `docs/`。新增 UI 文案时先问：删掉这句话用户会不会用不了这个功能？会才留。操作状态反馈优先用控件自身的状态变化表达（如按钮文字短暂切换为「已取消/已下载」一秒后恢复、列表项消失、布局切换），而不是新增常驻状态文字行；错误信息（含失败原因）除外。

### Naming & Strings

- **Do NOT hardcode engine names** (e.g., `"Local-Llama.cpp"`) anywhere. Use `model.engine_type` or URL-based heuristics (`isLocalUrl`).
- Frontend event names should be descriptive and kebab-case (e.g., `engine-progress`, `llm-chunk`).

## Extensibility Guide

- **Adding a New API Provider**: OpenAI-compatible providers usually need NO Rust changes — add the provider + model metadata to the sibling `aio-models-data` repo (consumed via `@aio/models-data`), add the logo mapping in `core/utils/modelLogo.ts`, and verify via Settings → Providers custom URL + "test connection". Only providers whose request/stream format differs (auth header, response JSON shape) get a dedicated plugin: implement `ProviderPlugin` in `src-tauri/src/plugins/provider/`, register it in `ProviderManager::new()` in `plugins/provider/mod.rs` BEFORE `openai_compat` (registration order is the dispatch order; `openai_compat` must stay last as fallback). UI lives in `features/settings/ProviderDetailPage.tsx` (shared provider card; engine card reuses the same page).

- **Adding a New Local Inference Engine**: implement `LocalEnginePlugin` in a new `plugins/engine/<id>.rs` (methods: `name`, `identifier`, `supported_extensions`, `is_platform_supported`, `install_path`, `is_installed`, `start`, `build_command`, `parse_progress_from_log`, `detect_installation`, `default_port`, `progress_event_name`), register in `EngineManager::new()` in `plugins/engine/mod.rs`, and hook auto-install into `plugins/engine/installer.rs` if the engine ships binaries. Engine binaries are NOT bundled — `tauri.conf.json` `bundle.resources` only ships the models catalog; engines install into app data at `install_path()`. The frontend picks the engine up automatically via `engine_type` on `ActivatedModel` and the startup engine scan (`scan_installed_engines`).

- **Adding a New File Parser**: add the extension to the whitelist match in `utils/file_parser.rs` (`process_file_content`, plus `attachment_mime_type`/`validate_attachment_path` if the type is new), implement the parser as a private helper in the same module, add the extension to the `ALLOWED_DOC` / `ALLOWED_IMG` lists in `src/features/chat/ChatPage.tsx` (attachment picker), and update `docs/zh|en/reference/configuration.md` (attachment whitelist). The Rust whitelist is the trust boundary — never only change the picker.

More extension paths (MCP transports, new Tauri commands, agent tools, permission defaults) with test guidance: see `docs/zh|en/development/extensions.md` and `docs/zh|en/development/architecture.md`.

## State Management Rules

### Global State (`core/store/`)

全局状态拆分为域模块（`store.ts` 仅作 barrel 再导出，外部导入路径不变；接口在 `types.ts`）：

- `chat.ts`：`datas`（助手/话题/消息）+ 应用配置 + MCP 状态 + Skills + 标题重命名
- `models.ts`：选中模型 / 模型目录 / 供应商配置 / modelKey / isLocalModel
- `engine.ts`：本地引擎启动 / 进度 / 连接 / 扫描缓存
- `project.ts` / `profiles.ts` / `ui-prefs.ts` / `update.ts` / `workflow.ts`：各自域
- `harness.ts`：Harness 工作台（工具检测/工作区/话题/消息/草稿/审批/运行态，选择持久化到 localStorage）

关键信号：`datas`（Store: assistants/activatedModels）、`config`（apiUrl/apiKey）、`selectedModel`、`currentAssistantId`、`currentTopicId`、`projects`/`currentProjectId`（+ `gitBranch`/`gitBranches`）、`mcpServers`/`mcpServerStatus`/`mcpToolsCache`、`skills`、`providerConfigs`、`modelsCatalog*`、`profileModelOverrides`、`customSubagentProfiles`、`engineScanResults`、`workflowState`、`appUpdate*`、harness 域信号（`harnessTools`/`harnessWorkspaces`/`harnessTopics`/`harnessMessages`/`harnessDrafts`/`harnessApprovals`/`harnessRunStates`）；UI 偏好（`themeColor`/`isDarkMode`/`reasoningLevel`/`webSearchEnabled`）持久化到 localStorage。

### Backend State (`core/state.rs` + `lib.rs`)

- `StreamManager`: active LLM stream tasks keyed `"{assistant_id}-{topic_id}"`; stop via `CancellationToken::cancel()` so the task exits gracefully and emits its epilogue — never `abort()`.
- `DbState`: SQLite connection.
- `LocalEngineState`: running local engine processes, HashMap keyed by `engine_type`; one active engine at a time enforced by `start_local_server`.
- `SubagentHandles`: background subagent join handles.
- Plus managers registered in `lib.rs`: `EngineManager`, `McpServerManager`/`McpServerState`/`McpRequestManager`/`PendingApprovals`, `LspManager`, `FileLockRegistry`（多智能体写协调，见 `core/file_locks.rs`）, `HarnessManager`（harness 会话运行时：子进程 + 命令通道 + 取消令牌 + 审批 oneshot 表；停止两段式=协议取消→taskkill 进程树）。

## Important Compatibility Notes

- **Command Stability**: Public Tauri command names (`start_local_server`, `stop_local_server`, `is_local_server_running`, `call_llm_stream`, etc.) must remain stable to preserve frontend compatibility.
- **Config Migration**: The `ActivatedModel` struct supports an optional `engine_type` field. Old configs without this field deserialize safely (defaults to `None`), and logic treats `None` as legacy llama.cpp behavior.
- **Event Names**: `engine-progress` is the generic progress event (all engines default to it); the llama.cpp plugin still emits `llama-progress` for backward compatibility. Other frontend events: `llm-chunk`, `llm-round-start`, `llm-compression`, `llm-compression-failed`, `subagent-start` / `subagent-step` / `subagent-done` / `subagent-error`, `mcp-server-status`, `mcp-server-stderr`, `app-update-progress`, harness 事件族（`harness-event` / `harness-state` / `harness-approval-requested` / `harness-approval-expired`）. New events must be kebab-case.

## Branch & Release Workflow

单人开发约定（分支名规范 + PR 流程）：

- **日常开发**：从 `dev` 切出功能分支，命名 `dev-<功能>`（Git 不允许在 `dev` 分支存在时创建 `dev/<功能>` 命名空间，故用连字符前缀）；更改提交到功能分支。
- **功能完成**：功能分支 PR 到 `dev`，merge 后删除功能分支（本地 + 远端）。
- **发布版本**：决定发版时 `dev` PR 到 `master` 并 merge；`dev` 分支保留不动。随后 bump 版本（CHANGELOG + 三处版本号）、打 `vX.Y.Z` tag 推送触发 `release.yml`。
- **master** 仅包含已发布版本内容；长期存在的分支只有 `dev` 与 `master`。

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
#   check:frontend = check:i18n (locale key parity) + check:docs (markdown links) + check:gates (架构/样式门禁) + prettier --check + eslint --max-warnings 0 + tsc --noEmit + vite build
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
- [ ] 新后端→前端事件名：在 `src-tauri/src/core/events.rs` 与 `src/core/api/events.ts` 两端登记（kebab-case）。
- [ ] 新 Tauri 命令：前端经 `core/api/<域>.ts` 封装调用，组件不得裸 `invoke`。
- [ ] `npm run check:gates` 全过（裸中文 JSX / 颜色 / any 闸门 / 分层 / 文件规模 / 命令注册）。

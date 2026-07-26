# 系统架构

## 总览

AIO 是 Tauri 2 桌面应用：

```text
SolidJS UI
  ├── features/chat
  ├── features/settings
  ├── core/store + types + utils
  └── shared/components
          │ invoke / event
Rust commands
  ├── LLM 与 Agent
  ├── 配置、附件、项目和更新
  ├── Provider、Engine、MCP、Skill、Git 与 LSP
  └── SQLite / 安全存储 / 进程状态
```

前端负责交互、导航和响应式状态；Rust 负责持久化、网络请求、文件与进程访问，以及需要信任边界的工具执行。

## 前端

- `src/features/chat`：聊天页、项目侧栏、Agent 过程、消息分支、分享和问题面板。
- `src/features/settings`：Provider、MCP、Skill、用量、子智能体和应用设置。
- `src/core/store`：全局 SolidJS signals/store 与后端同步操作。
- `src/core/shortcuts.ts`：键盘快捷键和斜杠命令注册。
- `src/shared/components`：Markdown、图标、导航和通用交互组件。

路由以聊天页和嵌套设置页为主。设置页的 Provider 详情使用独立子路由。

## Rust 后端

`src-tauri/src/lib.rs` 初始化 SQLite、Tauri 插件和全局状态，并注册全部 invoke command。主要模块：

- `commands/llm/`：普通流式聊天、Agent 循环、子智能体、用量、摘要、标题与 `/btw`；
- `commands/provider_config.rs`：Provider v2 配置、连接测试和模型获取；
- `commands/engine.rs`：本地引擎安装、启动、停止与状态；
- `commands/mcp.rs`：MCP 生命周期、工具、Resources、Prompts 与审批；
- `commands/skill.rs`：全局/项目 Skill、市场和 NPX 导入；
- `commands/project.rs`：项目 CRUD 与 `.aio/` 初始化；
- `commands/lsp.rs`、`commands/git.rs`：项目语言服务和 Git 操作。

## 插件边界

- Provider：Google、Anthropic、Ollama 和 OpenAI-compatible。
- Local Engine：llama.cpp 与 vLLM；当前 UI 仅开放 llama.cpp。
- MCP Transport：stdio、HTTP 与 Streamable HTTP。
- LSP：管理不同语言服务器的启动、请求、诊断和关闭。

Manager 在启动时注册插件，command 根据标识符查找实现。新增实现应复用现有 trait，不新增平行调度层。

## 状态与持久化

长期数据：

- SQLite：聊天、附件关系、项目和用量；
- JSON：Provider、MCP、Skill、模型目录和应用设置；
- 系统凭据库：API Key 与 MCP 密钥；
- 项目 `.aio/`：项目级 MCP、Skill 和权限。
- 项目 `.aio/knowledge.json`：可选的跨会话项目知识。

运行时状态：

- LLM stream 与取消令牌；
- 当前本地引擎子进程；
- MCP 连接、调用任务与待审批请求；
- LSP client；
- 子智能体执行句柄。

窗口销毁时会取消流任务、终止本地引擎、清空 MCP 连接并中止在途工具调用。

## 普通聊天数据流

1. 前端将消息与附件转换为 Provider 消息。
2. Rust 根据 API URL 选择 Provider。
3. Provider 发起流式请求。
4. Rust 通过 Tauri event 推送文本、推理内容、用量和完成状态。
5. 前端更新话题，Rust 将消息与附件关系写入 SQLite。

## Agent 数据流

1. 项目和助手确定 Agent 模式、模型、Skill 与 MCP Server。
2. 系统提示词注入项目路径和可用工具。
3. 模型返回工具调用。
4. 权限模块返回允许、询问或拒绝。
5. 内置工具或 MCP Server 执行，并把结果回灌模型。
6. Agent 继续迭代，前端持续展示步骤、子智能体和用量。

Plan 模式只研究并输出计划；工作流模式会拆分并自动执行任务。子智能体使用受角色约束的工具集，不能再次委派。

### 权限与重试

权限模块先加载模式默认规则，再叠加 `.aio/permissions.json`。匹配会考虑工具名、Server、模式和路径；拒绝规则优先，之后按优先级选择。需要询问时，后端创建待审批请求并通过事件交给前端，收到允许或拒绝后恢复挂起调用。

文件写入类工具会在审批前计算 Diff。执行成功后，Git 工具再次捕获实际变更并写入消息的 `fileChanges`，用于展开 Diff 和按文件/批量恢复。

工具失败默认最多重试 2 次，间隔 500 毫秒。重试包装位于统一工具执行路径，避免各工具自行实现不同策略。

### 子智能体与结果保全

`delegate_task` 和 `delegate_tasks` 复用单任务执行函数。批量委托用 Tokio 并发执行 1–10 个任务，并由 Semaphore 将实际并发限制为默认 5；结果按子任务收集后一次回灌主 Agent。

Agent 中止或流式事件失败时，已经产生的步骤、工具输出和文件变更仍会合并进当前助手消息并持久化，避免只保留空白最终回复。

### 跨会话记忆

启用 `knowledgeEnabled` 后，系统提示词注入 `.aio/knowledge.json` 中的现有条目，并向 Agent 增加 `remember`、`recall` 工具。知识按项目隔离、按 key 更新，最多保留 50 条。

## MCP 数据流

1. 加载全局配置并叠加项目配置。
2. stdio/HTTP 插件建立连接并完成 initialize。
3. 获取 Tools、Resources 和 Prompts。
4. 仅把当前助手启用的工具暴露给模型。
5. 工具调用经过权限检查、审批和超时控制。
6. 结果作为 tool 消息进入后续模型请求。

## 更新数据流

1. Tauri updater 根据 `tauri.conf.json` 的 endpoint 请求 GitHub Release 中的 `latest.json`。
2. 后端把“最新、可更新、服务未就绪、网络或其他失败”转换为结构化结果。
3. 前端展示版本和 Release notes；用户确认后下载签名产物并显示进度。
4. 下载完成后由 updater 安装，应用通过重启命令重新启动。

发布公钥固化在 Tauri 配置中，签名私钥只存在于发布环境；本地普通构建不能生成可替代正式 Release 的更新产物。

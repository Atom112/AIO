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
- `commands/memory.rs`、`commands/embedding.rs`：项目记忆（RAG）事实 CRUD/检索与嵌入供给；
- `services/memory/`：每项目记忆库（facts + FTS5 + 向量打分）、混合检索与记忆工具；
- `plugins/embed/`：嵌入（向量化）插件（Ollama / OpenAI 兼容）。

## 插件边界

- Provider：Google、Anthropic、Ollama 和 OpenAI-compatible。
- Local Engine：llama.cpp、Ollama 与 vLLM（通过引擎扫描自动发现，参见 [providers-and-models.md → 引擎扫描](../usage/providers-and-models.md#引擎扫描与自动发现)）。
- MCP Transport：stdio、HTTP 与 Streamable HTTP。
- Embed：Ollama 与 OpenAI 兼容的文本嵌入（向量化），为项目记忆提供语义检索。
- LSP：管理不同语言服务器的启动、请求、诊断和关闭。

Manager 在启动时注册插件，command 根据标识符查找实现。新增实现应复用现有 trait，不新增平行调度层。

## 状态与持久化

长期数据：

- SQLite：聊天、附件关系、项目和用量；
- JSON：Provider、MCP、Skill、模型目录和应用设置；
- 系统凭据库：API Key 与 MCP 密钥；
- 项目 `.aio/`：项目级 MCP、Skill 和权限。
- 项目 `.aio/knowledge.json`：可选的跨会话项目知识；
- 项目 `.aio/memory/memory.sqlite`：项目级语义记忆（RAG）库，含事实、版本审计、FTS5 全文与向量索引。

运行时状态：

- LLM stream 与取消令牌；
- 当前本地引擎子进程；
- MCP 连接、调用任务与待审批请求；
- LSP client；
- 子智能体执行句柄。

窗口销毁时会取消流任务、终止本地引擎、清空 MCP 连接并中止在途工具调用。

### 近期性能变更

- **阻塞 I/O 迁移**：shell 命令执行（`shell_tools::execute_command`）、Git 操作（`git_tools::execute_git_tool`）、文件操作（`file_tools::execute_file_tool`）均在 tokio `spawn_blocking` 池中执行，不阻塞主运行时线程。
- **图片懒加载**：`ChatInterface` 使用 `ResizeObserver` 实现流式自动滚动，图片加载完成后进行滚动位置修正。
- **代码块页眉**：缩减页眉高度，增加粘性复制按钮以减少滚动距离。

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

启用项目记忆（`memoryEnabled`，可在项目设置中每项目覆盖）后，Agent 获得 `remember` / `recall` / `search_memory` / `update_memory` / `forget_memory` 五个工具，事实写入每项目 `.aio/memory/memory.sqlite`（语义向量 + FTS5 关键词混合检索），并在 Agent 运行时按 token 预算自动注入相关记忆到系统提示词稳定前缀（index=1）。嵌入由 `plugins/embed` 提供（Ollama 默认，或 OpenAI 兼容），未配置嵌入时自动降级为关键词检索。

P2 起实现自进化：Agent 轮次结束后后台异步提取事实（sleep-time compute，去抖 + 每项目开关），写入时对向量近邻高分事实由 LLM 仲裁（merge / update / supersede / keep_separate，失败 fail-safe），`fact_versions` 表记录完整版本审计链；每项目活跃事实超过上限时按 importance 乘访问衰减归档最低分非 pinned 事实。前端提供记忆面板（搜索 / 列表 / 编辑 / 钉住 / 归档 / 删除 / 版本历史 / 重建索引 / 清理超额 / 清空）。

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

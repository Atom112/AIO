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
- MCP Transport：stdio 与 HTTP。
- LSP：管理不同语言服务器的启动、请求、诊断和关闭。

Manager 在启动时注册插件，command 根据标识符查找实现。新增实现应复用现有 trait，不新增平行调度层。

## 状态与持久化

长期数据：

- SQLite：聊天、附件关系、项目和用量；
- JSON：Provider、MCP、Skill、模型目录和应用设置；
- 系统凭据库：API Key 与 MCP 密钥；
- 项目 `.aio/`：项目级 MCP、Skill 和权限。

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

## MCP 数据流

1. 加载全局配置并叠加项目配置。
2. stdio/HTTP 插件建立连接并完成 initialize。
3. 获取 Tools、Resources 和 Prompts。
4. 仅把当前助手启用的工具暴露给模型。
5. 工具调用经过权限检查、审批和超时控制。
6. 结果作为 tool 消息进入后续模型请求。

# 配置参考

## 存储位置

AIO 使用 Tauri 提供的应用数据目录和系统配置目录，应用标识为 `com.loch.aio`。具体根路径由操作系统和安装方式决定，不应在脚本中写死。

主要数据包括：

| 文件或目录 | 内容 |
| --- | --- |
| `chat_history.db` | 助手、话题、消息、附件关系、项目关联和用量日志 |
| `attachments/` | 按 SHA-256 去重后的附件 |
| `provider-configs.json` | Provider 配置，不含新写入的明文 API Key |
| `mcp-servers.json` | 全局 MCP Server |
| `skills.json` | 全局 Skill |
| `projects.json` | 项目路径与元数据 |
| `models-catalog.json` | 更新后的模型目录 |
| `mcp-registry-cache.json` | MCP Registry 缓存 |
| `skill-market-cache.json` | Skill 市场缓存 |
| `profile-model-overrides.json` | 子智能体模型覆盖 |
| `custom-subagent-profiles.json` | 自定义子智能体 |
| `config.json` | 应用通用配置 |

仓库仍保留兼容旧配置的 `activated_models.json` 和 `fetched_models.json` 读写路径。本地模型列表继续使用激活模型数据。

## 应用配置

`config.json` 当前持久化：

| 字段 | 行为 |
| --- | --- |
| `api_url` | 兼容旧版的默认 API URL |
| `default_model` | 兼容旧版的默认模型 |
| `local_model_path` | 兼容旧版的本地模型路径 |
| `knowledgeEnabled` | 是否启用项目跨会话记忆，默认 `false` |
| `autoStartEnabled` | 是否随系统启动，默认 `false` |

后端 `AppConfig` 还包含以下运行字段：

| 字段 | 当前有效值 |
| --- | --- |
| `auto_retry_enabled` | 默认 `true` |
| `auto_retry_count` | 默认 `2` |
| `auto_retry_delay_ms` | 默认 `500` 毫秒 |
| `maxConcurrentSubagents` | 未配置时使用并发上限 `5` |

这些重试和并发字段当前没有设置界面，也不会由 `config.json` 的磁盘结构持久化。不要通过手工添加 JSON 字段宣称已经修改运行值；对外开放前需要同时扩展加载、保存与设置界面。

## SQLite 数据

`chat_history.db` 使用 SQLite，并在启动时执行幂等迁移。主要表包括：

- `assistants`；
- `topics`；
- `messages`；
- `attachments` 与 `message_attachments`；
- `usage_log`。

数据库开启外键约束。删除助手或话题会级联清理消息；附件文件在失去所有消息关联后删除。Agent 步骤、工具调用、推理内容、Token 与文件变更会随消息保存，不使用单一 JSON 历史文件。

## 系统凭据库

Provider API Key 和 MCP 环境变量/请求头中的密钥优先写入系统凭据库。JSON 中只保存 `hasStoredKey`、`hasStoredSecret` 等状态及 `${KEYRING:account}` 占位符。

若平台凭据库不可用，安全存储模块会使用应用数据目录中的加密回退文件 `secure-store.json`。这仍是敏感文件：

- 不要提交、同步或粘贴到 Issue；
- 不要手工修改；
- 迁移设备时不能假设旧系统凭据可直接读取。

旧 Provider 配置可能仍带有兼容字段 `apiKey`，新代码不会继续把 Key 明文写盘。打开旧配置后应通过设置页重新保存，随后检查备份中是否仍含旧密钥。

## WebView localStorage

以下界面偏好保存在 WebView `localStorage`，不属于项目配置：

| 键或类别 | 内容 |
| --- | --- |
| `aio-shortcut-bindings` | 自定义快捷键 |
| `theme-color` | 主题色 |
| `chat-reasoning-level` | 推理强度 |
| `chat-web-search` | 联网搜索开关 |
| `aio-last-agent-project-id` | 最近使用的 Agent 项目 |
| 左右侧栏状态与宽度 | 聊天布局 |
| `aio_ignored_update_version` | 忽略的更新版本 |
| `aio-local-auto-start-confirmed` | 本地引擎启动确认 |
| `user-avatar-path` | 用户头像文件路径 |

清理 WebView 数据会恢复这些界面偏好，但不会删除 SQLite 聊天记录、Provider 配置或项目 `.aio/`。

## 项目配置

创建项目时会初始化：

```text
.aio/
├── mcp-servers.json
└── skills.json
```

按功能使用后还可能出现：

```text
.aio/
├── permissions.json
└── knowledge.json
```

- `permissions.json`：项目工具的允许、询问和拒绝规则；
- `knowledge.json`：跨会话知识，类别为 `decision`、`pattern`、`convention` 或 `note`，最多保留 50 条。

项目路径和元数据登记在应用的 `projects.json`，对应项目助手及其 `project_id` 关联保存在 SQLite。MCP 与权限格式示例见[MCP 与 Skill](../usage/mcp-and-skills.md)。

## 附件白名单

| 类型 | 扩展名 | 大小上限 |
| --- | --- | --- |
| 图片 | png、jpg、jpeg、webp | 10 MiB |
| 文档 | pdf、docx、pptx | 30 MiB |
| 文本 | txt、md、json、csv、log、xml、yaml、yml、ini、tsv | 5 MiB |

解析前会检查扩展名、大小和沙箱路径。图片转换为 base64 数据 URI 供模型请求使用；其他类型提取为文本。原附件按哈希复制到应用数据目录，数据库保存引用关系。

## 网络与命令安全

- Provider、MCP 和联网工具的 URL 会经过 HTTP URL 校验与 SSRF 防护。
- Provider 的 `proxyUrl` 只影响该 Provider，不会成为全局系统代理。
- stdio MCP 使用命令和参数数组启动，不执行拼接后的 Shell 字符串。
- Agent Shell 工具会分类检查删除、覆盖及其他危险命令。
- 文件工具将路径限制在项目根目录或应用允许的沙箱内。
- MCP 与内置工具根据 Agent 模式和项目权限规则执行允许、询问或拒绝。

更改配置前建议退出 AIO，并备份整个应用数据目录。不要在应用运行时直接编辑 SQLite，也不要把配置备份视为无敏感信息。

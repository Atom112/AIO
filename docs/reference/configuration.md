# 配置参考

## 存储位置

AIO 使用 Tauri 的应用数据目录以及系统配置目录。具体根路径由操作系统决定，应用标识为 `com.loch.aio`。

主要数据包括：

| 文件或目录 | 内容 |
| --- | --- |
| `chat_history.db` | 助手、话题、消息、附件关系、项目关联和用量日志 |
| `attachments/` | 按 SHA-256 去重后的附件 |
| `provider-configs.json` | Provider v2 配置，不含明文 API Key |
| `mcp-servers.json` | 全局 MCP Server |
| `skills.json` | 全局 Skill |
| `projects.json` | 项目路径与元数据 |
| `models-catalog.json` | 更新后的模型目录 |
| `mcp-registry-cache.json` | MCP Registry 缓存 |
| `skill-market-cache.json` | Skill 市场缓存 |
| `profile-model-overrides.json` | 子智能体模型覆盖 |
| `custom-subagent-profiles.json` | 自定义子智能体 |
| `config.json` | 应用通用配置 |

仓库仍保留兼容旧配置的 `activated_models.json` 和 `fetched_models.json` 读写路径。本地模型列表使用其中的激活模型数据。

## SQLite 数据

`chat_history.db` 使用 SQLite，并在启动时执行幂等迁移。主要表包括：

- `assistants`；
- `topics`；
- `messages`；
- `attachments` 与 `message_attachments`；
- `usage_log`。

数据库开启外键约束。删除助手或话题会级联清理消息；附件文件在失去所有消息关联后删除。

## 密钥

Provider API Key 和 MCP 环境密钥优先存入操作系统凭据库。若平台凭据库不可用，安全存储模块可使用应用数据目录中的加密回退文件 `secure-store.json`。

不要手工把密钥写入 Provider 或 MCP JSON。排查问题时也不要将完整配置、日志或截图中的密钥提交到 Issue。

## 项目配置

创建项目时会在项目根目录初始化：

```text
.aio/
├── mcp-servers.json
└── skills.json
```

首次保存自定义权限规则后会增加 `.aio/permissions.json`。项目路径和元数据登记在应用的 `projects.json`，对应项目助手及其 `project_id` 关联保存在 SQLite。

## 附件白名单

| 类型 | 扩展名 | 大小上限 |
| --- | --- | --- |
| 图片 | png、jpg、jpeg、webp | 10 MiB |
| 文档 | pdf、docx、pptx | 30 MiB |
| 文本 | txt、md、json、csv、log、xml、yaml、yml、ini、tsv | 5 MiB |

解析前会检查扩展名、大小和沙箱路径。图片保留为 base64 数据 URI；其他类型提取为文本。

## 网络与命令安全

- Provider、MCP 和联网工具的 URL 会经过 HTTP URL 校验和 SSRF 防护。
- stdio MCP 使用参数数组启动进程，不通过字符串拼接 Shell。
- Agent Shell 工具对删除、覆盖和其他危险命令分类检查。
- 文件工具将路径限制在项目根目录或应用允许的沙箱内。
- MCP 工具根据 Agent 模式和项目权限规则执行允许、询问或拒绝。

更改配置前建议退出 AIO，并备份整个应用数据目录。不要在应用运行时直接编辑 SQLite。

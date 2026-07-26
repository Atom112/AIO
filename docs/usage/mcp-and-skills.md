# MCP 与 Skill

## 两类扩展

- MCP Server 向模型提供工具、Resources 和 Prompts。
- Skill 向助手注入可复用的指令、说明和工作流程。

两者都支持全局配置和项目配置。项目配置位于绑定目录的 `.aio/` 中，切换项目时重新加载。

## MCP Server

AIO 当前注册两种传输：

| 传输 | 配置 |
| --- | --- |
| stdio | 命令、参数、工作目录和环境变量 |
| HTTP | URL 和请求头 |

设置页支持添加、删除、启停、测试连接和查看服务器状态。连接建立后可列出 Tools、Resources 和 Prompts；聊天只向模型提供当前助手或项目启用的工具。

新建项目时会自动写入 `__aio-filesystem__`，它通过当前 AIO 可执行文件的 `--fs-server` 模式提供受项目根目录约束的文件工具。

## MCP Catalog

MCP 页面可从官方 Registry 目录读取可安装服务器。安装流程会：

1. 检查所需运行时；
2. 选择 stdio 或 HTTP 交付方式；
3. 收集必填参数和密钥；
4. 写入对应范围的 `mcp-servers.json`；
5. 默认标记为自动启动。

目录结果会缓存在 `mcp-registry-cache.json`。安装第三方服务器前应检查发布者、命令和访问范围。

## 工具权限与审批

权限检查综合以下信息：

- Agent 模式；
- 工具名称与 MCP Server ID；
- 调用参数；
- 项目 `.aio/permissions.json` 中的自定义规则。

结果为 `allow`、`ask` 或 `deny`。需要确认的调用会生成审批请求，用户响应后才执行。自动模式仍受明确拒绝规则和危险命令保护，不能把“自动”理解为绕过全部安全限制。

## Resources 与 Prompts

已连接的 MCP Server 可以暴露：

- Resources：先列出 URI，再按需读取内容；
- Prompts：先列出模板，再传入参数获取消息。

是否可用取决于服务器返回的 capabilities。状态对象中的资源和 Prompt 数量用于界面展示，不支持时为零。

## Skill

“设置 → Skill 管理”提供三种来源：

- Skill 市场；
- 已下载的全局或项目 Skill；
- 从 NPX 包发现并导入的 Skill。

市场数据按分类、热门或趋势浏览，并使用 6 小时缓存。项目 Skill 保存在 `.aio/skills.json`，全局 Skill 保存在应用数据目录。启用 Skill 后，还需要在对应助手或项目设置中绑定，才会注入对话。

## 项目文件

```text
project/
└── .aio/
    ├── mcp-servers.json
    ├── permissions.json
    └── skills.json
```

`.aio/` 会影响 Agent 能调用的外部能力，适合纳入项目版本控制前先检查其中是否包含环境相关路径。密钥值应通过系统凭据库引用，不应提交明文。

故障处理见[MCP 与 Skill 排查](../troubleshooting.md#mcp-server-无法启动)。

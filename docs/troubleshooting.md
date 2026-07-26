# 故障排查

## Provider 无法连接

1. 在“设置 → 供应商设置”重新运行连接测试。
2. 检查 API URL 是否包含多余的 `/chat/completions`。
3. 确认 API Key 对目标模型有权限。
4. 确认服务的响应格式属于 Google、Anthropic、Ollama 或 OpenAI-compatible。
5. 自建服务若位于内网，检查 URL 是否被 SSRF 策略拒绝。

如果模型目录能显示但请求失败，目录元数据并不代表账号已开通该模型。

## 本地 llama.cpp 启动失败

- 确认选择的是可读取的 `.gguf` 文件。
- 首次启动需要联网下载引擎；检查防火墙和代理。
- 确认 `8080` 端口未被其他程序占用。
- 模型过大时降低模型规模或关闭其他占用内存/显存的程序。
- 移动模型文件后重新选择路径并添加模型。

当前 UI 不提供 vLLM。不要通过修改 `engine_type` 配置绕过界面。

## MCP Server 无法启动

1. 查看服务器状态和错误信息。
2. stdio Server：确认命令可在系统 PATH 中找到，参数和工作目录有效。
3. NPX Server：确认已安装 Node.js/npm，且网络可访问包源。
4. HTTP Server：确认 URL、请求头和服务 capabilities。
5. 从 Catalog 安装时，先运行运行时检查并补齐必填密钥。
6. 项目 Server：确认当前选择了正确项目。

服务器连接成功但看不到工具时，检查项目/助手是否绑定该 Server，以及 `enabledTools` 白名单。

## MCP 工具一直等待确认

- 普通 Agent 模式会对敏感操作请求确认。
- 检查是否存在未处理的审批弹窗。
- 检查 `.aio/permissions.json` 是否把工具配置为 `ask` 或 `deny`。
- 自动模式不会覆盖明确拒绝规则和危险命令保护。

## Skill 没有生效

1. 确认 Skill 已下载或导入。
2. 确认当前范围是全局还是项目。
3. 在助手或项目设置中启用该 Skill。
4. 切换项目后等待 Skill 列表重新加载。
5. NPX Skill 更新失败时检查包管理器和网络。

## 附件无法上传

检查扩展名和大小：

- 图片最大 10 MiB；
- PDF、DOCX、PPTX 最大 30 MiB；
- 文本文件最大 5 MiB。

PDF 或 Office 文件能上传但无有效文本时，文件可能是扫描件、加密文件或包含当前解析器不支持的结构。

## Agent 无法访问项目文件

- 确认创建项目时选择的是目录而不是文件。
- 确认项目路径仍然存在。
- 检查当前助手是否属于该项目。
- Plan 模式不会修改文件。
- 文件工具和内置文件系统 MCP 都受项目根目录限制。
- 查看问题面板是否提示 LSP 未安装；LSP 失败不影响普通文件读取。

## 用量统计为空

用量数据在每轮 LLM 调用完成后写入 SQLite。旧消息可能没有 Token 字段；被取消或未返回用量的 Provider 也可能记录为零。

## 源码依赖安装失败

如果 `npm ci` 报告 `@aio/models-data` 不存在，确认目录结构：

```text
parent/
├── AIO/
└── aio-models-data/
    └── dist/data/models.json
```

两个仓库必须处于同一父目录，且模型数据仓库包含构建后的 `dist/data/models.json`。

## Rust 或 Tauri 构建失败

- 使用 Rust stable 和 Node.js 20。
- 安装当前平台的 Tauri 2 prerequisites。
- Linux 安装 WebKitGTK、AppIndicator、Rsvg 和 patchelf。
- 先运行 `npm run build`，再在 `src-tauri` 运行 `cargo check`。
- 发布签名或 updater artifact 错误只影响正式发布构建，不影响普通开发运行。

仍无法解决时，在 [GitHub Issues](https://github.com/Atom112/AIO/issues) 提供系统版本、AIO 提交、复现步骤和脱敏后的错误日志。

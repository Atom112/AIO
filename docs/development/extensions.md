# 扩展开发

## 新增 OpenAI-compatible 供应商

通常不需要修改 Rust。模型服务若兼容 OpenAI `/models` 与 `/chat/completions`：

1. 在 `aio-models-data` 中添加 Provider 与模型元数据。
2. 更新 AIO 使用的相邻目录内容。
3. 必要时在 `src/core/utils/modelLogo.ts` 添加图标映射。
4. 用设置页的自定义 URL 和测试连接验证。

只有请求或流式响应格式不同于现有实现时，才新增专用 Provider 插件。专用插件必须在 `openai_compat` 之前注册。

## 新增 Provider 插件

1. 在 `src-tauri/src/plugins/provider/` 添加模块。
2. 实现现有 Provider trait，包括匹配条件、模型 URL、请求构建和流解析。
3. 在父模块声明新模块。
4. 在 `ProviderManager::new()` 注册，并保持 OpenAI-compatible 位于最后。
5. 添加针对请求映射和流事件的最小测试。

## 新增本地引擎

1. 在 `src-tauri/src/plugins/engine/` 添加实现。
2. 实现 `LocalEnginePlugin` 的标识、平台、扩展名、安装路径、启动和进度解析。
3. 在 `EngineManager::new()` 注册。
4. 如需自动安装，接入现有 `EngineInstaller`。
5. 在前端引擎选项中显式开放，并提供格式过滤和必要的安全提示。

后端注册不等于用户功能完成。vLLM 就是当前示例：插件存在，但设置页尚未开放。

## 新增 MCP 传输

1. 在 `src-tauri/src/plugins/mcp/` 添加模块。
2. 实现 `McpServerPlugin` 的 start、initialize、tools、resources、prompts、call 和 stop 行为。
3. 在 `McpServerManager::builtin()` 注册。
4. 扩展 `McpTransport` 数据类型及前端镜像类型。
5. 更新设置表单和 Catalog 交付映射。

使用现有 JSON-RPC connection，不要为新传输复制请求关联、能力协商和错误处理。

## 新增文件解析器

1. 在 `src-tauri/src/utils/file_parser.rs` 的白名单匹配中添加扩展名。
2. 在同一模块实现解析函数，并复用大小、扩展名和沙箱检查。
3. 更新 `ChatPage.tsx` 的前端选择白名单。
4. 为有效文件、超限文件和错误内容保留一个最小测试。
5. 更新[配置参考](../reference/configuration.md#附件白名单)。

不要仅修改文件选择器；Rust 白名单才是信任边界。

## 新增 Tauri command

1. 放入职责对应的 `commands` 模块，并保持 command 足够薄。
2. 添加描述参数和行为的 Rust doc comment。
3. 在父模块导出。
4. 在 `src-tauri/src/lib.rs` 的 `generate_handler!` 注册。
5. 前端使用类型化参数调用，避免新增 `any`。

## 验证

```bash
npm run build
cd src-tauri
cargo check
```

涉及引擎、MCP、文件或系统凭据库时，还需要使用 `npm run tauri dev` 做真实桌面流程验证。

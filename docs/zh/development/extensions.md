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

后端注册后，引擎通过 `EngineManager` 自动发现并加入扫描结果。vLLM 是当前示例：插件注册后即被引擎扫描管线覆盖，不需要单独的前端设置页。三种引擎（llama.cpp、vLLM、Ollama）现在共享统一的引擎卡 UI，状态指示器和启停操作统一在 Provider 详情页处理。

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

## 新增本地嵌入模型（Embedder）

1. 在 `src-tauri/src/commands/model_download.rs` 的 `MANIFEST` 追加 `kind: "embed"` 条目（`id` / `label` / `dims` / `pooling`（mean/cls）/ `files`，URL 指向公开可下载的 ONNX 导出仓库，如 Xenova / Qdrant / Alibaba-NLP），安装目录默认为 `<app_data_dir>/models/embed/<id>/`，可在设置页「Embedding 模型管理」自定义。
2. `src-tauri/src/plugins/embed/local_onnx.rs` 的 `load_model` 按 manifest 加载本地 ONNX（`UserDefinedEmbeddingModel` + 对应 `Pooling`），进程内按模型 id 惰性缓存；内置模型 `all-MiniLM-L6-v2` 走 `include_bytes!`（开箱即用，不可删除）。
3. 嵌入模型为本地模型（provider 恒为 `local`）；在线嵌入（OpenAI 兼容端点）自 v0.8 起不再支持，相关 `openai_compat` 实现与 `embedding_save_api_key` 命令已移除。
4. 全局启停注册表存于 `AppConfig.embeddingModels`（缺省视为启用），由 `embedding_model_set_enabled` 维护；`embedding_test` 仅做本地连通性验证。

## 验证

```bash
npm run build
cd src-tauri
cargo check
```

涉及引擎、MCP、文件或系统凭据库时，还需要使用 `npm run tauri dev` 做真实桌面流程验证。

最小测试应贴近新增行为：

- Provider：请求映射、URL 匹配和流事件解析；
- Engine：平台支持、扩展名、安装状态和进度解析；
- MCP Transport：initialize、请求关联、capabilities 与断开清理；
- 文件解析器：有效文件、超限、错误内容和沙箱外路径；
- 权限或工具：至少覆盖允许、拒绝和关键失败路径。

优先在现有模块测试中加入一个能复现行为的用例，不为单一实现新增平行测试框架。

## 文档同步

扩展进入 UI 后，同一改动应检查：

- `README.md` 是否需要更新功能边界；
- 用户指南是否需要增加配置步骤；
- `reference/configuration.md` 是否需要增加字段、文件或安全说明；
- `troubleshooting.md` 是否需要增加可操作的失败处理；
- `CHANGELOG.md` 是否需要记录发布变化。

只注册后端插件但没有前端入口时，应在开发文档标记为内部能力，不写成普通用户可完成的流程。

## RAG / 项目记忆改动检查清单（回归护栏）

改动涉及 `services/memory/`、`plugins/embed|cache|rerank/`、记忆工具或检索链路时，除 `npm run verify` 外还应满足：

- [ ] 检索链路改动必须附评估：仓库内 `tests/rag_eval/dataset.json`（40 事实 + 12 查询）上跑 `cargo test --lib eval_rag -- --ignored --nocapture`，在 PR 描述贴出「纯向量 / FTS / 混合」NDCG@10、MRR、P@5 对比表；允许混合不劣于单腿（护栏断言 < 0.15 NDCG 降幅）；
- [ ] 历史缺陷回归用例不得删除：判重阈值走余弦（非 RRF 分）、category 过滤生效、≤2 字中文查询可命中、superseded/archived 不进检索与注入；
- [ ] 新增配置字段必须同时写入 `AppConfig` 与 `AppConfigDisk`（config.rs 完整三处：save / load v2 / legacy 默认），并同步 `reference/configuration.md` 与双语 i18n key；
- [ ] 多维能力一律全局 + 项目级双轨（`resolve_project_*`）并带维度/可用性守卫，禁止静默降级；
- [ ] 新增 Tauri 命令在 `lib.rs` 注册；新增事件 kebab-case；异步后台任务（提取/整理/抽取）必须有预算封顶或去抖，失败静默且可观测。

### 稀疏检索（SPLADE）评估结论（P3-2）

当前稀疏腿 = FTS5（trigram + BM25 + 短查询 LIKE 回退），与稠密腿（vec0/暴力余弦）构成混合基线。**种子评测集上混合路 NDCG 不劣于单腿即视为达标，暂不引入 SPLADE/Qwen3-sparse**；引入时机：评测集扩展后其增益能被 NDCG 数据支撑，且具备本地 ONNX 部署路径（未证实收益前不得替换 FTS5 稀疏腿）。

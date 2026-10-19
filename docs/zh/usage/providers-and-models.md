# 供应商与模型

## Provider 类型

AIO 后端按顺序匹配专用 Provider，最后使用 OpenAI-compatible 实现兜底：

| 类型              | 适用场景                                            |
| ----------------- | --------------------------------------------------- |
| Google            | Google 原生生成接口及其流式响应格式                 |
| Anthropic         | Anthropic Messages 接口                             |
| Ollama            | 本机或局域网 Ollama 服务                            |
| OpenCode          | OpenCode Go（`opencode.ai`）托管推理                |
| OpenAI-compatible | OpenAI 及兼容 `/chat/completions`、`/models` 的服务 |

供应商列表和模型元数据来自相邻依赖 `@aio/models-data`。Provider 配置只保存开关、API URL、启用模型等运行信息；模型能力、上下文窗口和展示名称由目录提供。

### OpenCode Go

OpenCode Go 订阅（Go / Go Plus）的 API Key 在 AIO 中作为自定义 Provider 使用：API URL 填 OpenAI 兼容端点 `https://opencode.ai/inference/go/openai/v1`，模型经「从 API 拉取模型」获取。AIO 自动为每个话题的聊天请求附带 `x-opencode-session` 会话头（OpenCode Go 网关的必需字段，也用于其路由与提示词缓存优化）。注意其 Anthropic 协议端点（`/inference/go/anthropic/v1`，服务 Claude / MiniMax / Qwen 系模型）暂不在支持范围内。

## 配置流程

1. 在“设置 → 供应商设置”打开 Provider。
2. 确认 API URL。自定义服务应填写 API 根路径，不必重复附加 `/chat/completions`。
3. 如网络需要，为当前 Provider 单独填写 HTTP/HTTPS 代理 URL，例如 `http://127.0.0.1:7890`。
4. 输入 API Key，并用“测试连接”验证。
5. 从目录或服务返回结果中启用模型。

请求失败时先检查 URL、Key 和服务是否支持所选模型。AIO 会校验 HTTP URL，并阻止不符合当前安全策略的地址。

代理按 Provider 保存，不会修改系统代理，也不会影响其他 Provider、本地 llama.cpp、MCP 或自动更新。用户名、密码等敏感信息不应直接写入代理 URL。

## 模型目录与在线拉取

设置首页显示模型目录的 Provider、模型数量、目录版本和更新时间。手动刷新会下载新的 `models-catalog.json`，不会自动启用新增模型。

Provider 详情会合并两类模型：

- 目录模型：来自 `@aio/models-data`，包含展示名称、上下文窗口、发布时间和能力；
- 在线模型：通过 Provider 的模型接口拉取并保存到 `fetchedModels`。

自定义 Provider 没有目录元数据时，应先使用“从 API 拉取模型”；服务不提供兼容模型接口时，可以手工添加模型 ID。

模型行可能展示以下能力标签：

| 标签      | 含义                       |
| --------- | -------------------------- |
| Vision    | 支持图片输入               |
| Tools     | 支持工具调用               |
| Reasoning | 支持推理能力或原生推理内容 |
| Streaming | 支持流式响应               |
| JSON      | 支持 JSON mode             |

`preview`、`beta`、`experimental` 和 `alpha` 是上游模型状态，不代表 AIO 会额外放宽兼容性或安全检查。

## 本地 llama.cpp

当前设置界面提供 llama.cpp：

- 模型格式：GGUF；
- 默认服务地址：`http://127.0.0.1:8080/v1`；
- 引擎自备：AIO 检测本机 llama.cpp（未安装时设置页提供官方下载链接，不自动下载）；
- 进程管理：启动新模型前停止旧进程，应用销毁时清理子进程。

模型文件可以由用户自行选择（不会复制到应用数据目录，删除或移动原文件后需要重新选择路径），也可以直接从内置模型市场一键下载（见下节）。

## 本地模型市场

设置页的「本地模型市场」子页（llama.cpp / Ollama 引擎可选）内置 GGUF 模型搜索与下载，无需打开浏览器即可获取本地模型：

- **双源检索**：HuggingFace 与 ModelScope 的 GGUF 模型合并展示（自动模式），默认按下载量排行（跟随两站自身的默认排序）；搜索为主入口，关键词即时查询。列表随下载源选择切换（ModelScope / HuggingFace 只显示对应平台的模型），各源列表独立缓存 6 小时；点击刷新会同时刷新两个平台的列表与缓存。
- **参数量分段**：列表按参数量分档（≤1B / 1-3B / 3-7B / 7-14B / 14-32B / 32B+），方便按硬件条件筛选。
- **硬件推荐**：启动时探测 CPU / 内存 / GPU（NVIDIA 显存），按 Q4_K_M 量化（约 0.62 字节/参数）估算显存或内存预算，在列表顶部横幅给出推荐参数量，并将适合当前机器的模型标记置顶。
- **下载体验**：单文件与多分片 GGUF（`-00001-of-00002` 等）全量下载；暂停（保留断点）/ 续传（HTTP Range）/ 取消；下载前校验目标磁盘剩余空间；进度按文件与来源实时显示。
- **下载源**：可选「自动 / ModelScope / HuggingFace」；自动模式按 ModelScope → hf-mirror.com → huggingface.co 逐源回退（国内网络默认可用 ModelScope），也可使用供应商代理。受限仓库（如 Llama）需先在网页上同意协议，再在市场卡片中填入 HuggingFace Token（保存在系统密钥链，仅本机可读）。
- **自动接线**：下载完成后自动注册到所选引擎的本地模型（供应商页「本地模型文件」列表中可见，可一键启动）；llama.cpp 以首个分片作为入口文件，Ollama 启动时经 `ollama create` 导入。
- **目录与迁移**：下载目录默认 `<应用数据目录>/models/local`，可在市场卡片中更改；更改时已下载模型自动迁移（跨盘回退为复制），且供应商配置中的模型路径引用同步重写。
- **列表缓存**：市场查询结果在本地缓存 6 小时（stale-while-revalidate：先渲染缓存、后台静默刷新），重新打开设置页秒开。

## 引擎扫描与自动发现

应用启动时自动检测系统上已安装的本地推理引擎，并统一管理其状态。

| 引擎      | 标识符      | 状态管理                       | API URL                    |
| --------- | ----------- | ------------------------------ | -------------------------- |
| llama.cpp | `llama_cpp` | 安装检测、启动、停止、健康检查 | `http://localhost:8080/v1` |
| vLLM      | `vllm`      | 安装检测、启动、停止、健康检查 | `http://localhost:8000/v1` |
| Ollama    | `ollama`    | 安装检测、启动、停止、健康检查 | `http://localhost:11434`   |

### 扫描机制

`scan_installed_engines` 命令遍历已注册的引擎插件，返回每种的：

- `supported` — 当前平台是否支持；
- `installed` — 系统上是否安装了引擎二进制或运行环境；
- `version` — 检测到的版本号（如有）；
- `default_port` / `default_api_url` — 默认连接地址。

### 引擎状态

每种引擎有独立的状态管理，包括四种状态：

- **未安装**：系统未检测到引擎；
- **已安装**：引擎可用，等待用户启动；
- **运行中**：引擎进程正在运行且健康检查通过；
- **启动失败**：引擎无法正常启动（检查日志中的错误原因）。

### UI 位置

Provider 详情页显示引擎状态指示器。用户可以在 Provider 页面对安装的引擎执行启动/停止操作。llama.cpp、vLLM 和 Ollama 共享统一的设置体验：

- GGUF 文件选择器（llama.cpp / Ollama）；
- 模型卡片展示；
- 启停按钮与进程管理；
- 健康检查（llama.cpp/vLLM 使用 `/health`，Ollama 使用 `/api/version`）。

Ollama Provider 的 UI 已与 llama.cpp 统一，不再需要独立的 Provider 设置页面。三种引擎统一通过 Provider 详情页中的引擎卡管理。

## 模型选择与覆盖

- 聊天助手可以绑定首选模型；未绑定时使用当前全局选择。
- 项目可以绑定模型并为 Agent 提供工作目录上下文。
- 内置和自定义子智能体可配置独立的云模型覆盖。
- 子智能体未配置覆盖时继承主 Agent 模型，本地模型也通过继承使用。

Provider 配置文件及密钥存储位置见[配置参考](../reference/configuration.md)。

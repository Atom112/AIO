# AIO 全量代码审查报告（2026-10）

> 审查与修复同轮完成。审查方法：4 个独立专项并行深读（Rust 安全 / Rust 性能与架构 / 前端安全 / 前端性能与可维护性）+ 父代理对全部 HIGH 级发现逐条读码复核 + 修复后 `npm run verify` 全套回归。所有发现均以 `文件:行号` + 代码证据核实，无臆测项。
>
> 配套详细报告：前端性能/可维护性专项全文见 [frontend-perf-review-2026.md](./frontend-perf-review-2026.md)；其余专项结论已并入本文。

---

## 一、Dependabot 告警修复

GitHub Dependabot 面板 71 条告警均已 fixed/dismissed（无 open），但**本地 lockfile 实测仍有 13 条 RustSec 漏洞**（npm 侧 `npm audit` 0 条）。本轮处置：

| 依赖                   | 动作              | 说明                                                                                                                                                                                                                                                                                          |
| ---------------------- | ----------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `zip`                  | **4.6.1 → 9.0.5** | RUSTSEC-2026-0367/0368/0369（0369 为 CVSS 9.1 critical：`extract()` 经符号链接/Unix 权限逃逸）。AIO 自身只读 docx/pptx（deflate），故随升级收敛为最小特性集 `default-features=false, features=["deflate"]`，顺带移除 13 个不必要解压器依赖（依赖总数 741 → 728）                              |
| `tauri-plugin-updater` | 2.10.1 → 2.14.0   | API 兼容（AIO 仅用 check/download_and_install/request_restart 薄封装）；**其内部 zip 仍锁 `^4`**（上游 2.14.0 尚未升 9），已确认实际触发面为 minisign 签名校验**之后**的 tauri-bundler 官方制品解压，并在 `src-tauri/.cargo/audit.toml` 带条件忽略 + 复查条件注明（updater 升 zip>=9 后删除） |
| `h2`                   | 0.4.15 → 0.4.20   | RUSTSEC-2026-0258（无界空 DATA 帧 DoS，经 reqwest 传递引入）                                                                                                                                                                                                                                  |
| `quinn-proto`          | 0.11.16 → 0.11.19 | RUSTSEC-2026-0338/0339/0340/0341/0342（5 条：流重组内存耗尽绕过、零长 DATAGRAM 绕过接收上限、ACK panic 等）                                                                                                                                                                                   |
| `rustls`               | 0.23.45 → 0.23.46 | RUSTSEC-2026-0346/0347/0348/0349（Debug 泄露 TLS 密钥、ECH 拒绝后数据误发、Brotli 1GiB 预认证分配、降级）                                                                                                                                                                                     |
| `event-listener`       | 5.4.1 → 5.4.2     | unsound 告警（`!Send` tag 跨线程）                                                                                                                                                                                                                                                            |

**结果：`cargo audit` 0 漏洞**（10 条 unmaintained/unsound 警告均为深层传递依赖，见"遗留项"）。

## 二、安全审查（2 HIGH / 2 MEDIUM 已修，1 MEDIUM + 1 LOW 遗留）

### 已修复

**[SEC-1 · HIGH] web_fetch SSRF 主防线被 IPv4-mapped IPv6 整链绕过** — `utils/web_tools.rs` `is_public_addr` 的 V6 分支注释声称处理了映射形态，实际未检查 `::ffff:0:0/96`。攻击者发布一条 `::ffff:169.254.169.254` 的 AAAA 记录即可通过全部静态检查 + 解析后校验 + IP 固定，双栈套接字将连接路由到内嵌 IPv4。
**修复**：V6 分支前置 `to_ipv4()` 映射检查（覆盖 mapped + compatible 两形态），并补 6to4 `2002::/16` 与 NAT64 `64:ff9b::/96` 内嵌 v4 判定；新增 14 条回归断言。**注：测试当场抓出我初版 NAT64 内嵌位索引错误（96 位前缀 = segments 0..5，v4 在 6..7）——回归测试的必要性自证。**

**[SEC-2 · HIGH] `sandboxUnsandboxedFallback` 默认值违背安全契约** — 字段文档承诺 "Default **false** — the runtime never breaks the sandbox boundary unless explicitly opted in"，但 `AppConfig::default()` 为 `true`（全新安装生效）且磁盘结构的 serde 属性也是 `default_true`（审查与初版修复都只抓到前者）。该开关是 Windows 完全权限档下**唯一的脱沙箱升级通道**。
**修复**：四源对齐为 false（`models/config.rs` 默认、serde 属性改 `#[serde(default)]`、`config.rs` 磁盘默认、迁移路径），并加"默认一致性"回归测试——正是该测试暴露了第四处 `true`。

**[SEC-3 · MEDIUM] LLM 输出链接可整体导航应用窗口** — 聊天/工作台渲染的 `<a href>` 无点击拦截、主窗口无 `on_navigation`，无边框无地址栏的可信窗口可被替换为任意远程页面（应用内钓鱼）。
**修复**（双层）：`Markdown.tsx` 事件委托拦截 `<a>` → `openSafeUrl`（http/https 白名单 + 系统浏览器，所有渲染面共用该组件一处收敛）；`lib.rs` 窗口构建补 `on_navigation` 白名单（tauri://localhost、http(s)://tauri.localhost、dev 1420、asset 协议、about:）作纵深，拒绝时留 warn 日志。

**[SEC-4 · MEDIUM] DOMPurify 配置面过宽 + `aio-image://` 盲解码** — html profile 本身放行 `form/input/select/textarea/style/template`，配置又显式加 `target`，模型可在气泡内渲染仿冒交互 UI；`renderer.image` 未转义插值且前端对解码路径零校验。
**修复**：`FORBID_TAGS` 封表单类标签、`ADD_ATTR` 移除 `target`；`aio-image` 解码后强制"绝对路径 + attachments 段"双校验（后端 asset scope 之外的纵深）；`src/alt/title` 全部走 `escapeHtmlAttr`。

**[SEC-5 · MEDIUM] 持久化读-改-写竞态 + 原子写 tmp 冲突** — `mcp_trust.rs` 与 `plugins/mcp/mod.rs` 的 RMW 无互斥（后写覆盖先写丢数据）；`fs_util::atomic_write` 固定 tmp 名，两个并发写同一目标会争抢同一临时文件，A 的 rename 可能携带 B 的半写内容——**击穿"绝不半写"的核心承诺**。
**修复**：两处 RMW 套进程级互斥（照 `secure_store::FALLBACK_MUTEX` 范式）；tmp 名加 uuid 唯一后缀；新增并发写回归测试（双线程 × 20 次交替写 64KB，断言终态必为某一方完整内容且无 tmp 残留）。

**[SEC-6 · LOW] 三处加固** — `taskkill` 裸名 spawn 违反项目自身 SEC-06 约定（改走 `resolve_executable`）；`mcp_fs_server` 把含文件内容的全量参数打进 stderr（经 `mcp-server-stderr` 事件广播进 UI，改为只记工具名+path）且 `&line[..200]` 字节切片在中文 JSON 上必 panic（改字符边界截断）+ stdin 行长无上限（加 16MB 上限 + 截断行拒绝）；`secure_store::get()` 无锁并发首访可生成两把密钥致条目永久失解（fallback 分支补 `FALLBACK_MUTEX`）。

### 遗留（建议下轮处理）

- **[SEC-7 · MEDIUM]** `url_validation` 仅静态校验：MCP HTTP 传输（自带 `${KEYRING:}` 注入的真实 API key header）与供应商连接测试对"域名解析到内网/元数据 IP"零防护。建议把 web_fetch 的 `resolve_validated_addr` + pinned client 提取为公共 helper 复用。
- **[SEC-8 · LOW]** 原生 Windows 命令路径（非 AppContainer 档）`CreateProcess` 与 `AssignProcessToJobObject` 之间的进程树逃逸 TOCTOU 窗口（AC 路径已用挂起+先入 Job 消除）。建议复用 `win_appcontainer` 的挂起顺序。
- **[SEC-9 · INFO]** secure_store 降级密钥文件为静态混淆（文档已自认）；建议 Windows 侧改用 DPAPI 包裹密钥文件。

## 三、性能审查（3 HIGH / 主要 MEDIUM 已修，部分 MEDIUM 遗留）

### 已修复

| 级别       | 问题                                                                                                                                                                                                                                                     | 位置                                   | 修复                                                                                                                                                                |
| ---------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| HIGH       | `reqwest::Client` 每次调用重建，连接池完全失效；ollama 就绪轮询**每 500ms 新建一个 Client**                                                                                                                                                              | `plugins/llm.rs`、`engine/ollama.rs`   | 双 `LazyLock` 全局单例（流式/非流式各一）+ 轮询探测单例；Client 内部 Arc，clone 零拷贝                                                                              |
| HIGH       | `scan_installed_engines` 阻塞子进程探测链（`llama-server --version`、`sc query`、逐候选 `python -c "import vllm"`）直接跑在 async worker 上，启动可占用数秒至数十秒；vLLM 还把同一个加载 torch 的 import **跑两遍**（外加每候选一次 `python --version`） | `commands/engine.rs`、`engine/vllm.rs` | 整链 `spawn_blocking`（`EngineManager.all_plugins` 改返回 Arc 克隆以跨闭包）；探测合并为单次 `import vllm; print(__version__)` 兼取版本（每候选 3 次子进程 → 1 次） |
| MEDIUM     | 主聊天库 `chat_history.db` 未开 WAL——agent 回合里最高频写入点（每轮 usage INSERT + 多 MB 检查点快照）默认 journal 每次提交 2-3 次 fsync                                                                                                                  | `core/db.rs`                           | 与记忆库同款三 PRAGMA：`journal_mode=WAL` + `busy_timeout=5000` + `wal_autocheckpoint=100`                                                                          |
| MEDIUM     | 多 MB 检查点快照同步写在 async worker 上                                                                                                                                                                                                                 | `turn_runner.rs`                       | 序列化后 `spawn_blocking` + `await`（顺序语义不变：下一轮开始前本轮快照必落盘）                                                                                     |
| MEDIUM     | `post_chat_completion` 非 Anthropic 路径对整个消息数组无条件深拷贝（每轮一次，长会话数百 KB）                                                                                                                                                            | `plugins/llm.rs`                       | 借用原 body 直接序列化，仅 Anthropic 分支转换                                                                                                                       |
| MEDIUM     | ollama 启动超时不杀子进程（注册表仍记"运行中"），与 llama.cpp 行为不一致                                                                                                                                                                                 | `engine/ollama.rs`                     | 超时路径 kill+wait+撤销注册                                                                                                                                         |
| MEDIUM     | 5 个同步 Tauri 命令跑在主线程：`count_tokens_cmd`（输入框 live preview 高频触发，BPE 编码可达数十 ms）、两个 usage 聚合（GROUP BY + 全局锁）、`get_active_checkpoint`、`is_local_server_running`                                                         | `commands/llm/usage.rs` 等             | 全部改 `#[tauri::command(async)]`                                                                                                                                   |
| HIGH(前端) | **子代理流逐 token 写 Store 完全绕过 80ms 批处理管线**（后端粒度已核实为逐 SSE token；主流 llm-chunk 有批处理，子代理没有）——每 token 一次 `setDatas(produce)` + steps 数组复制，`maxConcurrentSubagents` 并发放大                                       | `chatEventListeners.ts`                | 键控（assistant::topic::subagent::kind）批处理管线，80ms 窗口聚合；round 变化先冲刷保序；done/error 同步冲刷避免终态晚到                                            |
| HIGH(前端) | 右栏拖拽每 mousemove 同步写 localStorage（60-240 次/秒）                                                                                                                                                                                                 | `ChatPage.tsx` + `usePanelResize.ts`   | 照左栏"预览信号 + 释放提交"模式重构；ChatPage 与 HarnessPage 两处消费点同步更新                                                                                     |
| LOW        | 截断提示把 UTF-8 字节数当字符数（中文虚高 3 倍，误导模型）                                                                                                                                                                                               | `utils/truncate.rs`                    | 改 `chars().count()`                                                                                                                                                |

### 遗留（按优先级）

1. **[H-3] `load_app_config` 工具热路径重复加载**（磁盘读+JSON 解析+keychain IPC，一次 execute_command 2-3 次、零缓存）。建议：turn_runner 已加载的 config 经参数下传，或加 mtime 失效的进程内缓存。改动面较大，适合独立 PR。
2. **[M-4] 代码索引 per-chunk autocommit**（~1 万 chunk ≈ 2 万次独立 fsync）：每文件包一个事务即可。
3. **[M-6] `stop_engine_internal` 同步 `child.wait()` 阻塞 worker**：改 tokio Child 或 spawn_blocking。
4. **[M-7] harness stop 固定 3s sleep**：改 50ms 轮询协作取消状态，3s 仅作兜底。
5. **[M-8] recall/search_memory stats 探针逐条写**（每次检索 5-7 个小事务）：合并单事务。
6. **[M-10] `try_read_package_meta` 超时泄漏 npm 子进程与读线程**：超时分支 kill。
7. **[M-11] harness 设置类 async 命令内联同步 fs I/O**：照 skill.rs 的 HOR-4 spawn_blocking 范式补齐。
8. **[前端 H-2] `saveSingleAssistantToBackend` 每个 tool result 全量克隆整个 Assistant + 全量 IPC**：需后端 append 粒度配合，属架构级改造。
9. **[前端 M-4/M-5/M-12]** TokenStatsBar 每工具事件 O(n) 重扫；harness 工具事件未批处理；尾窗分页 append 方向缺锚定（上翻时 round-start 追加顶替视口）。

## 四、可维护性 / 架构 / 复用性

### 修复

- **i18n 违规 ×2**：`chat.ts:552` 硬编码 `'默认话题'`（会随英文界面落库显示）改 `t('chat.defaultTopic')`；`ChatInterface` 命令注册的中文 label/description（经 /help 原样输出）改 P3 占位符模式（`COMMAND_TRANSLATIONS` 已有对应键）。
- **模块级无界缓存 ×2**：`AgentProcessBlock` 的 `animatedStepIds`（全会话只增不减）加上限清空策略；`perRoundQueue`（流式中途切页时条目+dequeueTimer+订阅闭包永久滞留）加 16 轮上限 + 逐出时清 timer（真实会话同时只有 1-2 个活跃轮，上限只会命中泄漏轮）。
- **吞错可见化 ×3**：崩溃恢复快照写失败、终态标记失败（失败时下次启动误判"可恢复崩溃残留"）、用量记录写失败、代码索引向量写失败（静默降级为纯关键词检索，违背项目自身"NULL vector must be VISIBLE"原则）——全部补 `tracing::warn!`。

### 核实为优秀的方面（无需改动）

1. **分层纪律零违规**（grep 全量验证）：services/utils/plugins 无一处反向依赖 commands；commands 层确实薄。
2. **203 个 `#[tauri::command]` 文档注释 0 缺失**（AGENTS.md 硬性要求，脚本核验）。
3. **StreamManager 换代协议**（结构化键、两阶段停止、身份匹配注销）设计精良且带 6 个并发语义集成测试；本轮 Rust 测试从 429 → 431 全绿。
4. **file_tools::safe_path** 全分支 `starts_with(base)` 终检 + symlink 解析、**MCP 信任门**指纹键控 + 伪造豁免防护、**exec_resolve** EXE-first 防影子替换、**AppContainer FFI** 结构布局逐字段核对无误——安全基本功扎实。
5. 请求路径无 unwrap/expect（全部在测试代码）；无锁跨 `.await`；全树 `dangerouslySetInnerHTML`/`eval`/`window.open` 零命中，innerHTML 仅 3 处且均复核。

### 建议下轮（不阻塞）

- **组件复用三件套**：`ChatComposer`（chat↔harness 输入区逐字节重复且 harness 缺 textarea 自增高）、`PanelResizeHandle`（4 处重复）、`AcrylicContextMenu`（3 处重复）；另 `CollapsibleTaskList`（HarnessPlanStrip ≡ TaskBoardPanel）、共享 `fmt()`（3 份拷贝）、`ApprovalCard` 外壳。
- **`any` 棘轮清偿**：存量 77 处，热点在 `apiMessages.ts`（核心消息构建 12 处）与 `setDatas` 过滤器 `(a: any)`。
- **rgba 棘轮**：存量 173，建议每轮 PR 顺手清 10-20 落 CSS 变量。
- **>500 行组件拆分**：LocalModelMarket 1147、ProviderDetailPage 1133、ChatInterface 1001 等 10 个；建议先把文件规模棘轮收到 1150 防继续膨胀。
- **分享预览 sanitize 单一化**：ShareModal 与主链路两套 DOMPurify 配置，建议抽共享 `sanitizeMarkdown(html)`。
- **Vite dev 加同款 CSP 头**（生产 CSP 已阻断远程图片 beacon，dev 无）。

## 五、验证结果（全绿）

| 检查                                                 | 结果                                                 |
| ---------------------------------------------------- | ---------------------------------------------------- |
| `check:i18n`                                         | 1200 keys 双语对齐 ✓                                 |
| `check:docs`                                         | 34 文件链接 ✓                                        |
| `check:gates`                                        | 全过（rgba=173≤173、any=77≤77 棘轮未反弹）✓          |
| prettier / eslint --max-warnings 0 / tsc --noEmit    | ✓                                                    |
| vitest                                               | 78/78 ✓                                              |
| vite build                                           | ✓                                                    |
| cargo fmt --check / clippy --all-targets -D warnings | ✓                                                    |
| cargo test                                           | **431 passed, 0 failed**（含本轮新增 2 个回归测试）✓ |
| cargo audit                                          | **0 漏洞**（10 条警告见遗留说明）✓                   |
| npm audit                                            | 0 漏洞 ✓                                             |

## 六、本轮变更清单

**依赖**：`Cargo.toml`（zip 9 + updater 2.14）、`Cargo.lock`、新增 `src-tauri/.cargo/audit.toml`。

**Rust（19 文件）**：`web_tools.rs`（SSRF）、`models/config.rs` + `core/config.rs`（沙箱默认值四源）、`fs_util.rs`（唯一 tmp + 并发测试）、`mcp_trust.rs` + `plugins/mcp/mod.rs`（RMW 互斥）、`plugins/mcp/stdio.rs`（taskkill SEC-06）、`mcp_fs_server.rs`（日志收敛+stdin 上限+截断 panic）、`secure_store.rs`（get 互斥）、`plugins/llm.rs`（Client 单例+去 clone）、`engine/{mod,ollama,vllm}.rs`（Arc 插件表+轮询单例+超时杀进程+探测去重）、`commands/engine.rs`（spawn_blocking）、`core/db.rs`（WAL）、`checkpoints.rs` + `usage.rs` + `code.rs`（吞错可见化+async 命令）、`turn_runner.rs`（检查点 offload）、`truncate.rs`（字符数）、`file_parser.rs`（zip 9 API）、`lib.rs`（on_navigation）。

**前端（7 文件）**：`Markdown.tsx`（链接拦截+FORBID_TAGS+aio-image 闸+转义）、`ChatPage.tsx` + `usePanelResize.ts` + `HarnessPage.tsx`（右栏拖拽防抖）、`chatEventListeners.ts`（子代理批处理）、`AgentProcessBlock.tsx`（缓存有界化）、`ChatInterface.tsx` + `store/chat.ts`（i18n）。

<div id="top"></div>

# Changelog

## [Unreleased]

### 🔒 全仓深度审查修复（CODE_REVIEW_REPORT.md：阶段 0/1/2，P0×2 + P1×31 全量 + 高价值 P2）

**阶段 0（P0 紧急止血）**

- **项目级 MCP 配置信任门（P0-1）**：仓库自带 `.aio/mcp-servers.json` 的服务器不再零确认自动 spawn（恶意仓库一个 JSON 文件即可任意执行命令的供应链攻击面）。打开/切换项目时弹确认窗完整展示 command/args/env/URL；信任标记保存在应用数据目录（仓库之外）并绑定配置指纹——仓库改动即自动失效需重新确认；后端 `start_mcp_server` 对未信任的项目级配置一律拒绝（纵深防御）；AIO 自身生成的 `__aio-filesystem__` 条目按「命令=当前进程 exe」逐字校验豁免；新增 `list_untrusted_project_mcp_servers` / `trust_project_mcp_server` 命令与 `ProjectMcpTrustModal` 确认界面
- **本地模型命令路径强校验（P0-2）**：新增 `core/path_safety::safe_join`，`local_model_delete` / `local_model_download` / `local_model_cancel_download` 对前端传入的 id / 文件名拒绝绝对路径（含 Windows 盘符/UNC）、`..` 回溯与保留字符——堵住「任意目录递归删除 / 任意路径写入」原语
- **MCP 密钥占位符 off-by-one（MCP-3）**：`${KEYRING:...}` 解析切片差一字节使 account 恒带前导冒号，stdio env 与 HTTP header 的所有托管密钥解析必失败；修复并支持同值多占位符循环解析

**阶段 1（安全组）**

- **会话授权放大收紧（SEC-02）**：execute_command 会话规则拒绝链式命令（批准 `cargo build` 不再放行 `cargo build && curl 外发`）；write_file 系规则收窄为精确单文件路径（原父目录 `/**` glob 一次批准即全项目写通行）
- **沙箱读边界（SEC-01）**：Linux bwrap 用 tmpfs 遮蔽 $HOME 并只读回绑工具链目录（`~/.ssh`/云凭据/`.config` 令牌对命令不可见）；macOS SBPL 同构 home 读拒绝 + 工具链例外；Auto/Workflow 模式下 `network=true` 的命令从免审批升级为需确认（一次提示注入不再能静默外发任意可读文件）；Windows AppContainer 原本已阻断
- **MCP 生命周期批（MCP-2/6/7/15 + INFRA-1）**：McpRequestManager 改 AbortHandle 真实驻留（停止/关窗真正中止在途调用，原 insert-后-立即-remove 使 abort 永远空转）；start 失败路径与窗口销毁显式杀子进程（stdout reader 持 Arc 使 kill_on_drop 永不触发）；重复 start 互斥；审批等待 Drop 守卫清理；LSP initialize 失败同根修复
- **IPC 信任边界统一（HOR-3/MEM-10/INFRA-5/MCP-13）**：新增后端原生对话框 `pick_save_path` / `pick_open_path`，`save_binary_file` / `copy_stored_image` / `memory_export/import` 只接受本会话经对话框确认的路径（导入限 .json + 16MB）；`call_mcp_tool` 删除可伪造的 agent_mode 参数（缺省映射 Off 恰好跳过全部权限检查——结构性 fail-open），`run_agent_turn` 模式改由 assistant 数据库记录服务端裁决，Off 模式 fail-closed
- **原子写全仓接入（HOR-1）+ 崩溃恢复（PROV-7/INFRA-4）**：`fs_util::atomic_write`（temp+fsync+rename）接入全部 18 处持久化写入点；provider-configs.json 解析失败改隔离 `.corrupt.bak`（原直接删除可丢全部供应商配置）；secure_store fallback 加互斥 + 原子写 + 损坏隔离（原静默当空库、下次保存抹掉全部密钥）；`.fallback-key` 原子 + 0600
- **Critical 旗标逃逸（SEC-07）**：PowerShell `-Command`（单横线）纳入载荷递归检查；新增 `Remove-Item -Recurse` 系统路径词法检测

**阶段 2（正确性/数据完整性组）**

- **主循环批（LOOP-1/2/3/4/6/9）**：StreamManager 条目带回合代次号、epilogue 身份匹配删除（换代竞态下旧任务超窗不再误删新任务→孤儿/双主循环）；checkpoint 移入循环体每轮保存（循环内崩溃可从最近完成轮恢复，原每回合仅一次且立即 terminal）；压缩归档失败/取消把 early 消息原样放回（残缺上下文不再继续下一轮）；context 预算下限 2k + reserve 按比例（小窗口本地模型不再预算 0 触发每轮压缩风暴）；主循环包 catch_unwind + 兜底 epilogue（panic 不再泄漏 StreamManager 条目、卡死 isThinking）；SSE 行缓冲 4MB 上限
- **SSE 批（PROV-1/2/4/6 + PROV-3）**：字节级行缓冲、只在完整行上解码（逐 chunk `from_utf8_lossy` 把被 TCP 分段切开的多字节 UTF-8 变乱码写入会话历史——中文流式主场景）；Anthropic/OpenAI SSE error 事件浮出（截断回复不再被当成功持久化）；Anthropic 事件处理补 suppress_events（子代理流不污染主聊天）；OpenAI 流式补 `include_usage`；google.rs 补 chat_completions_url 覆写——默认配置下 Gemini 聊天从必现 404 修复为 `/v1beta/openai/chat/completions`
- **引擎与下载批（ENG-02/03/04/06/07/13）**：HF Token 只发 huggingface.co 官方域（镜像不再收令牌）；健康检查改轮询（llama 30s / vLLM 180s、2xx 判定、轮询间检查进程退出——14B+ 模型加载 >7s 此前必被误杀）；下载完整性三道防线（终长核对 + 已存在文件大小/魔数复检 + 206 Content-Range/If-Range 续传防跨源拼接）；引擎 kill 后 wait（不再泄漏 Unix 僵尸）；无人读的 piped stdout 改 null（64KB 管道填满引擎静默挂死——Ollama 长会话必现）；Token 保存失败上抛
- **MCP HTTP 传输批（MCP-4/5/8）**：SSE 块消费连同分隔符一起 drain（先发心跳/注释块的服务器此前 100% 连接失败）；JSON-RPC error/id 双校验（200+error 不再变空结果假成功、通知块不再被误当响应）；重定向一律拒绝（自定义认证头不再被转发到重定向目标——https→http 降级即 key 外送）
- **记忆子系统批（MEM-01..05）**：update_memory keyless 分支改原地更新（不再每订正一次复制一条）；consolidate Supersede 方向反转修正（每日自动整理不再把更准确的信息标记过时）；prune 走 set_status（归档清 vec0/FTS，不再继续被召回）；代码重建先清 vec_code 幽灵向量；memory_clear 清向量/关系表/代码表，extract 判重遇幽灵视为无重复（不再静默丢新事实）
- **前端批（CHAT-1..6 + FE-01/02/03/05）**：导出 HTML 全文转义 + iframe sandbox（srcdoc 明文注入→应用内钓鱼面）；压缩持久化话题守卫（切话题后旧 count 裁新话题可清空整个话题历史并落盘）；switchToProject 补 setCurrentProjectId（冷启动不再在错误项目根操作文件）；Stop 按流归属键（切话题后不再打空、被迫切回才能停）；PDF 多页按源裁剪（原每页压入整幅图）；llmToolResult 未命中保持引用稳定（工具事件不再整列消息重挂载——解锁渲染层重构）；keyring 托管字段只读（编辑不再毁掉密钥引用）；快捷键徽章/命令面板改 data-* 属性选择器（class 串选择器抛错/恒 null）；new-assistant 默认键改 Ctrl+Alt+N（原与 new-project 冲突永不可触发）
- **杂项 P2**：LSP 事件族（3 个）+ models-catalog-updated 双端登记消灭裸串；MCP 工具无 description 不再被丢弃；MCP stderr 行长上限 2KB

修复全程逐条先在当前代码核实（7 板块并行核实子代理 + 父级复核 P0/P1 全部证据链）后才动手；每修一条补回归测试，新增 Rust 测试 33 个（220→253）、前端测试 6 个（18→24）。

### 🤖 Subagents（子智能体）

- **多智能体并行文件编辑**：业界调研（Claude Code / Amp / Vibe Kanban / Magentic-One / OpenHands）后落地两层防冲突机制——
  - **worktree 物理隔离**（`subagentIsolationMode=auto` 默认）：git 项目上被等待的写入类子任务自动获得独立 git worktree + 分支（项目同级 `<项目>.aio-wt/`、基于当前 HEAD、运行期锁定、创建时记录基线提交），完成后提交子分支并自动合并回当前分支（Auto 直并 / Normal 整批一次审批）；冲突安全中止并报告冲突文件清单，主检出有进行中的 merge/rebase 时拒绝合并（绝不破坏用户状态），子智能体自己 commit 的成果按基线识别不会丢失；非 git/网络路径自动回退共享模式；`delegate_task(s)` 新增 `isolation` 参数支持 per-task 覆盖
  - **共享模式文件锁基座**：`write_file` / `replace_in_file` / `delete_file` / `make_directory` 按文件互斥锁（120 秒超时报持有者，Windows 锁键大小写折叠）+ 原子写（tmp+rename，保留目标权限位）+ 跨写者 hashline 强制（文件被其他智能体修改过后 `replace_in_file` 必须带 `anchor_hash`、覆盖性 `write_file` 必须带新参数 `expect_hash`；`read_file` 返回完整文件哈希），丢失更新转为可恢复的重读错误；内置文件系统 MCP server 的工具变体从 Agent 工具集中过滤（变更类一律丢弃、读类去重），任何文件写入都无法绕过锁咽喉点；worktree 合并后按文件刷新 last-writer
- 设置新增「并行写入隔离」选项（自动 / 始终独立工作区 / 共享目录仅文件锁）

### 📋 任务清单与争议确认（主 Agent 交互工具）

- **任务清单**（`create_tasks` / `update_task` / `list_tasks`）：多步工作拆解为可跟踪任务，聊天区顶部实时任务面板（进度 x/y、待办/进行中/已完成、可折叠），每话题独立、内存态不落盘
- **`ask_user` 争议点提问**：需求歧义/多方案/风险取舍时 Agent 以提问气泡暂停等待用户作答——每问题 2-4 个选项（推荐项置首带「推荐」标记）、支持自定义回答输入、可整体跳过由 Agent 自行决定；一次最多 4 问，回答回灌后继续执行；新命令 `respond_agent_question`、新事件 `agent-tasks-changed` / `agent-question-requested`

## [v0.8.0]

<sup>Released on **2026-10-10**</sup>

### 🧠 Memory（项目级自进化 RAG 记忆系统）

- **P1 基础设施**：每项目独立 `memory.sqlite` + Embedder 插件（内置本地 ONNX / Ollama / OpenAI 兼容）+ `remember` / `recall` / `search_memory` 记忆工具 + 混合检索（FTS5 + 向量）自动注入上下文 + 设置 UI（应用设置嵌入分区 / 项目设置记忆区块）+ 中英双语文档
- **P2 自进化**：后台自动事实提取（Epilogue 挂载 + 去抖）、LLM 仲裁（merge / update / supersede + `fact_versions` 版本审计链）、容量治理（pinned 保护 / prune 裁剪）、记忆面板 MemoryPanel
- **P3/P4 向量与代码级 RAG**：sqlite-vec（vec0）SQLite 内向量检索、二阶段精排（本地重排 + 查询改写 + 多查询 RRF 融合）、代码库增量索引（`code_chunks`）与 `search_code` 工具、子智能体只读 `recall` / `search_code`、NDCG 离线评测
- **内置小体积本地嵌入模型**：开箱即用无需外部服务；支持按名称拉取模型、暂停 / 取消 / 断点续传、删除清理实例缓存、切换目录自动迁移（并修复下载失败问题）
- **重排模型多选下载器**（本地 ONNX），移除在线 rerank 端点支持
- **记忆导出 / 导入**：系统对话框选路径的文件模式，默认文件名带项目短哈希标识身份
- 全仓代码审查修复 M1–M6；每项目独立嵌入模型选择；移除旧跨会话记忆开关（RAG 方案取代）

### 🤖 Subagents（子智能体）

- **双通道完成通知**：后台 `delegate_task` 完成后推送系统通知并在下一轮注入摘要；新增 `get_subagent_result` 拉取工具（阻塞等待 + 超时兜底），支持「先并行派出、阶段二汇总」的编排模式
- **视觉子智能体（第 10 个内置角色）**：无工具、专注图像理解；非视觉主模型发送图片时自动路由到已激活的视觉模型生成结构化描述并入消息，失败回退原图发送
- **子智能体状态条上拉菜单 + 独立对话视图**：消息流完全复用主话题界面样式；流式生成期间自动跟底、手动滚动可靠解除跟随、话题切换后贴底展示

### ⚡️ Agent / LLM

- **提示词工程**：工具描述、跨工具协作约定、输出风格规则体系化
- **`/compact` 对齐主流智能体**：强制立即执行、准确结果反馈（完成 / 跳过 / 归档 / 失败）、失败归档回退
- **全面 i18n**：压缩摘要提示词与归档占位文案跟随语言；全部后端错误文案经 `ierr!` 宏本地化（zh-CN / en-US 切换即时生效）

### 🎨 UI/UX

- **话题树连接线**：圆角肘形 SVG 连接线、真实树形层级视觉；可折叠话题分支（悬停显示切换按钮 + 高度动画）；折叠状态跨重启持久化
- **侧栏激活态**：主题色内发光取代实心填充与边框高亮，按行高分级（大行双层高斯 / 小行单高斯衰减）
- **左侧栏**：拖拽展开 / 收起跟手 + 窗帘式软渐隐过渡；三栏改分界竖线；移除用户头像
- **聊天**：流式生成期间自动跟底、手动滚动解除自动跟随；回底按钮上移避让状态条；子智能体菜单触发器数量加量词
- **MCP 市场列表前端持久缓存**：秒开渲染（6h TTL + 后台静默刷新）
- **设置页去主题色化系列**：操作按钮统一次操作样式、选项栏 32px 等高、NumberInput 组件（细线 chevron 步进、适配暗色）、按钮宽度对齐
- **Skill 市场**：移除分类下拉；npx 扫描结果缓存为模块级信号（切换 / 重挂载不重扫）
- Usage 页移除缓存命中率卡片
- 修复：Dropdown 箭头从未显示、圆角透明窗口恢复（Linux 保持不透明保性能）、项目设置弹窗与记忆面板层叠冲突、供应商列表本地引擎重复条目

### 🛠 Engineering

- 全库中文注释翻译为英文（211 文件）

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.8.0]: https://github.com/Atom112/AIO/releases/tag/v0.8.0

## [v0.7.6]

<sup>Released on **2026-08-02**</sup>

### 🛡 Security（安全加固）

- **Agent 工具循环上限（AGT-01/02）**：单次 Agent 最大工具调用轮数可配置（默认 25，设置 → 应用 → 最大工具调用轮数），命中时优雅收尾而非硬中断；Workflow 模式强制续环改为最多 2 次提醒后退出，避免无界 API 成本
- **子智能体权限继承（AGT-03）**：子智能体工具执行继承父 Agent 模式，Normal/Plan 下写删/执行按父模式走审批，profile 工具白名单保留
- **子智能体并发上限统一（AGT-11）**：单发/后台 `delegate_task` 与批量 `delegate_tasks` 共享同一并发信号量（默认 5，可配置），后台子智能体跨轮不再无界累积
- **web_fetch SSRF 加固（AGT-06/07）**：逐跳重定向 SSRF 校验；生成图片仅允许栅格格式并拒绝 SVG
- **Linux/macOS 命令沙箱（AGT-05）**：bwrap（Linux）/ sandbox-exec（macOS）+ setpgid 进程组清理
- **capability 收窄（CONF-02）**：`fs:allow-write-file` 由 `$HOME/**` 收窄到 `$APPDATA/**`，导出/另存改走 Rust 命令
- **MCP 环境白名单化（CONF-04）**：stdio 子进程仅透传必要环境变量 + 用户显式配置
- **导出 HTML 注入修复（D5）**、外链 scheme 校验（D4）、glob `**` 权限匹配（AGT-08）

### ⚡️ Performance（性能）

- **WebKitGTK（Linux）性能降级**：`backdrop-filter` 关闭、消息动画改纯 opacity，Windows/macOS 视觉不变
- Markdown 渲染缓存、聊天历史 `messageEls` 剪枝、持久化单事务、`save_assistant` 走 `spawn_blocking`

### 🐛 Fixed（其它修复）

- 修复审查发现的格式漂移（prettier 四文件）

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.7.6]: https://github.com/Atom112/AIO/releases/tag/v0.7.6

## [v0.7.5]

<sup>Released on **2026-07-29**</sup>

### 📝 Documentation（文档）

- **中英文文档重构**：原有 13 份中文文档迁移至 `docs/zh/`，新增 14 份英文文档至 `docs/en/`，目录结构完全镜像
- **Agent 工具参考文档**（`docs/zh|en/usage/agent-tools.md`）：28 个内置工具的完整参考，跨 6 类别（文件/Git/LSP/Web/命令/编排/知识），每项含参数表、返回值、限制和安全特征
- **权限系统参考文档**（`docs/zh|en/reference/permissions.md`）：三层模型、评估顺序、5 种 Agent 模式内置默认规则、`.aio/permissions.json` 文件格式与字段参考
- **新增文档索引页**：`docs/README.md` 中英双语快速跳转
- **根 README 全英文化**：`README.md` 改为纯英文，原双语版保留为 `README.zh-CN.md`

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.7.5]: https://github.com/Atom112/AIO/releases/tag/v0.7.5

## [v0.7.4]

<sup>Released on **2026-07-29**</sup>

### ✨ Added（新增）

- **Catppuccin VSCode 语言图标**：代码块页眉语言类型前添加官方 Catppuccin vscode-icons SVG 图标（40+ 语言），CSS 变量定义 Mocha 调色板
- **引擎扫描与多引擎状态**：启动时自动扫描本地引擎（llama.cpp / Ollama / vLLM），Provider 详情页集成引擎状态与启停按钮
- **Ollama Provider 统一**：Ollama 设置页新增 GGUF 文件选择器、进程管理与模型选择卡片，与 llama.cpp 体验一致

### ⚡️ Changed（变更）

- **代码块页眉高度减半**：`py-2.5` → `h-6`（24px），上下圆角构成完美半圆
- **复制按钮重构**：从页眉移至代码区右上角悬浮，`position: sticky` 实现垂直滚动跟随（`overflow-clip` 容器），半透明玻璃效果，悬停代码块时渐显
- **本地引擎状态指示器简化**：去除绿色圆点，改用绿色文字，与文件系统指示器分隔

### 🐛 Fixed（修复）

- **Ollama 模型导入修复**：异步 `ollama create`、正确的 HTTP 状态码检查、健康检查改用 `/api/version`
- **Ollama/Local 引擎调用修复**：chat invoke 中防御性 apiKey 回退、模型选择时的端口与引擎类型路由
- **文件系统指示器**：修复启动时卡在黄色状态的问题
- **`LiveModel.owned_by` 后向兼容**：添加 `serde(default)` 避免反序列化旧配置崩溃
- **新启引擎自动停止其他引擎**：避免多引擎同时运行冲突

### 📝 Documentation（文档）

- **中英文文档重构**：原有 13 份中文文档迁移至 `docs/zh/`，新增 14 份英文文档至 `docs/en/`，目录结构完全镜像
- **Agent 工具参考文档**（`docs/zh|en/usage/agent-tools.md`）：28 个内置工具的完整参考，跨 6 类别（文件/Git/LSP/Web/命令/编排/知识），每项含参数表、返回值、限制和安全特征
- **权限系统参考文档**（`docs/zh|en/reference/permissions.md`）：三层模型、评估顺序、5 种 Agent 模式内置默认规则、`.aio/permissions.json` 文件格式与字段参考
- **新增文档索引页**：`docs/README.md` 中英双语快速跳转
- **根 README 全英文化**：`README.md` 改为纯英文，原双语版保留为 `README.zh-CN.md`

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.7.4]: https://github.com/Atom112/AIO/releases/tag/v0.7.4

## [v0.7.3]

<sup>Released on **2026-07-27**</sup>

### ✨ Added（新增）

- **暗色模式**：新增深色/浅色模式切换开关（设置 → 应用），状态持久化到 localStorage
  - 浅色模式保持现有亚克力紫蓝渐变风格
  - 暗色模式使用纯色深灰背景（`#0a0e1a`），降低文字与边框亮度以减少视觉疲劳
  - 所有内联样式边框迁移为 CSS 变量 `--border-dim`，暗色模式下统一 alpha=0.3 确保清晰可见
  - Tailwind `border-white/*` 类通过运行时样式注入 + 编译期 CSS 双重覆盖
  - 亚克力面板（`--acrylic-bg/border`）、表面色（`--surface-bg`）、文字色（`--text-base-rgb`）全面 CSS 变量化
- **主题色无关的 Agent 状态栏**：聊天输入框上方的 Agent 提示框背景改为表面色，不再跟随主题色变化

### ⚡️ Changed（变更）

- **全局色彩系统 CSS 变量化**：`rgba(124,154,191,*)` → `rgba(var(--primary-rgb),*)`（159 处）、`rgba(255,255,255,*)` → `rgba(var(--text-base-rgb),*)`（394 处）、`rgba(18,22,35,*)` → `rgba(var(--surface-bg),*)`（23 处），全面消除硬编码颜色值
- **NavBar 导航链接**：`text-white/50` → `text-theme-secondary`，hover 态 `text-white/85` → `text-theme-primary`

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.7.3]: https://github.com/Atom112/AIO/releases/tag/v0.7.3

## [v0.7.2]

<sup>Released on **2026-07-27**</sup>

### ✨ Added（新增）

- **前端全面 i18n 国际化**：22 个 TSX 组件中 ~170 处硬编码中英文字符串替换为 `t()` 调用，新增 205 个翻译键（526 → 731），en-US / zh-CN 完全同步
- **PDF 导出重构**：话题分享的 PDF 导出从浏览器打印对话框改为直接文件保存，使用 html-to-image 渲染 + jsPDF 合成，预览与输出像素级一致

### ⚡️ Changed（变更）

- **PDF 预览保留**：PDF 标签页保留 iframe 实时预览，渲染引擎与导出完全一致（同一 DOM 源）

### 🐛 Fixed（修复）

- **PDF 导出空白问题**：修复 jsPDF html2canvas DOM 克隆丢失样式导致视觉空白，改用 html-to-image 直接捕获已渲染 DOM

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.7.2]: https://github.com/Atom112/AIO/releases/tag/v0.7.2

---

## [v0.7.1]

<sup>Released on **2026-07-25**</sup>

### ✨ Added（新增）

- **`delegate_tasks` 批量子智能体工具**：单次调用并行创建多个子智能体（1-10 个），支持 shared context 注入公共背景信息，总耗时约等于最慢的子任务
- **子智能体并发上限控制**：新增 `maxConcurrentSubagents` 配置项（默认 5），通过 Tokio Semaphore 限制同时运行的子智能体数量
- **批量委托一次审批**：`delegate_tasks` 整个批次仅需一次权限确认（非 Auto 模式），审批对话框汇总展示所有子任务

### ⚡️ Changed（变更）

- **`execute_single_delegate` 重构**：提取 `delegate_task` 和 `delegate_tasks` 共用的子智能体执行逻辑，消除代码重复
- **系统提示词更新**：Agent 模式提示词中新增 `delegate_tasks` 工具说明和批量并行推荐

### 🐛 Fixed（修复）

- **安全漏洞修复**：升级 `seroval`（critical CVE: Promise 反序列化类型混淆）、`postcss`（high: 路径遍历）、`dompurify`（low: 沙箱绕过）
- **Rust 依赖安全升级**：修复 4 个 HIGH 级漏洞 — `lopdf` 栈溢出、`quick-xml` 双漏洞（O(n²) 属性检查 + 无界命名空间分配）、`quinn-proto` QUIC 流重组 OOM

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.7.1]: https://github.com/Atom112/AIO/releases/tag/v0.7.1

---

## [v0.7.0]

<sup>Released on **2026-07-20**</sup>

### ✨ Added（新增）

- **各配置档独立模型覆盖**：8 个内置子智能体配置档支持独立模型选择，新增自定义配置档 CRUD
- **左侧边栏薄条折叠**：折叠为 48px 图标条，悬停显示展开按钮，支持右键上下文菜单
- **设置页工具多选下拉**：子智能体允许/禁止工具改用 acrylic 多选下拉组件
- **LobeHub 风格流式动画**：消息流式输出时添加微光扫过叠加层 + 内容淡入
- **Agent 步骤卡片入场动画**：从左滑入的展开动画，运行中步骤叠加脉冲动画
- **Agent 步骤持久化**：应用重启后保留步骤历史记录
- **工具调用时渲染结果截断**：防止超长工具输出撑爆界面

### ⚡️ Changed（变更）

- **全面 UI 润色**：
  - 隐藏内置 `__aio-filesystem__` MCP 服务器列表
  - 所有 emoji 替换为 SVG Icon 组件
  - 消息入场动画改为 LobeHub 风格纯淡入（移除 slide-up + scale）
  - 简化 WelcomeScreen / ChatInterface 话题切换过渡逻辑
  - ProjectSidebar 圆角 `rounded-3xl`、名称截断、导航态处理
  - ReasoningButton 颜色编码等级指示器，更新下拉背景
  - 标签「编辑后重发」→「编辑」，新增项目指示点
  - TopicSidebar 移除冗余样式
  - Dropdown 组件文本居中，对称 14px 内边距

- **设置页统一与优化**：
  - 统一搜索栏与刷新按钮为 acrylic 风格（ProviderList / SkillList / McpServerList）
  - AppSettings 快捷键区域背景匹配颜色设置页，增加项间距 2→4px
  - UsageSettings 模糊背景改为纯色深色背景
  - ProviderList 模型供应商文字大小/颜色匹配本地推理引擎，增加分割线
  - ProviderDetailPage 移除删除供应商按钮，状态标签圆形边框
  - ModelRow 为 experimental/alpha/preview/beta 状态标签添加圆角
  - SkillList 用 acrylic Dropdown 替换 category select，添加刷新反馈
  - McpServerList 添加结果计数，移除浏览文本与手动添加按钮

### 🐛 Fixed（修复）

- 修复前端 `tool_call_id` / `tool_calls` 字段 snake_case 与 Rust 端 `#[serde(rename_all = camelCase)]` 不匹配导致 Agent 停止后请求 400 错误
- 修复组件重渲染导致步骤缓冲队列（pendingIds/dequeueTimer）重置，入场动画截断（提升为模块级队列，跨 remount 保持去抖定时器）
- 修复页面切换后 Agent 耗时读数错误增长（改用不可变步骤数据的 timestamp + duration 推导）
- 修复右键菜单动画未触发（index.css 显式添加 `@keyframes contextMenuIn/contextMenuOut`）
- 修复 Rust 端多处 UTF-8 截断 panic（LLM/MCP/HTTP/Git/Web 工具改为 `floor_char_boundary()`）
- 修复 Message struct 缺少 `#[serde(rename_all = camelCase)]`
- 修复 Markdown 分段从 `<For>` 改为 `<Index>` 避免 key 冲突
- 修复模态框视口居中问题
- 修复侧边栏折叠按钮层叠顺序
- 修复折叠状态下按钮悬停颜色误用主题色
- 修复 SkillList 缺失 Icon 导入导致运行时崩溃
- 修复 ProviderDetailPage Dropdown 标签未闭合
- 修复 AppSettings 快捷键项间距问题

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.7.0]: https://github.com/Atom112/AIO/releases/tag/v0.7.0

---

## [v0.6.2]

<sup>Released on **2026-07-12**</sup>

### ✨ Added（新增）

- **子智能体（Sub-agents）系统**：完整的委托、并行执行、实时反馈工作流系统
- **Token 用量持久化与统计面板**：
  - 后端：messages 表增加 `input_tokens` / `output_tokens` 列，新建 `usage_log` 表
  - 后端：LLM 流式调用后自动写入 usage_log（per-round 粒度）
  - 后端：新增 `get_usage_summary` / `get_usage_summary_by_model` 查询命令
  - 前端：设置页新增「使用量」子页，含统计卡片（总 Token / 请求 / 费用 / 活跃天数）
  - 前端：GitHub 风格 365 天热力图、按模型用量分布条形图
- **LSP 诊断集成**：编辑器内实时显示编译错误/类型错误/代码警告
- **Token 用量与上下文窗口统计**：实时监控当前对话 token 消耗
- **键盘快捷键与命令面板**：快速操作入口
- **斜杠命令（Slash Commands）**：输入 `/` 触发命令菜单
- **Web 搜索工具**：Agent 可直接搜索网页
- **Git 工具集成**：Agent 可直接运行 git 命令
- **文件编辑工具**：Agent 模式原生文件读写能力
- **MCP Resources / Prompts 支持**：扩展 MCP 协议能力
- **Agent 工作过程时间线**：ZCode 风格分步展示工作流程，每步显示类型图标、耗时与可折叠内容
- **修复 CSP 策略**：`img-src` 添加 `data:` 和 `blob:` 以支持模型/供应商 Logo 头像显示

### 🐛 Fixed（修复）

- 综合性安全加固（修复 GitHub Code Scanning 发现的 8 项问题）

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.6.2]: https://github.com/Atom112/AIO/releases/tag/v0.6.2

---

## [v0.6.1]

<sup>Released on **2026-07-09**</sup>

### ✨ Added（新增）

- **Agent `execute_command` 工具**：支持在系统中执行命令
- **重新生成应用图标集**：从根 logo.svg 统一生成

### ⚡️ Changed（变更）

- Skills npx 发现重写：以 `npx skills list --json` 为主方法，提升稳定性
- 缓存 npx 发现结果，避免标签页切换时重复扫描

### 🐛 Fixed（修复）

- 修复聊天自动滚动在用户向上滚动后未立即停止的问题
- 修复 Skills 作用域切换无法正确区分全局/项目技能
- 修复 Windows 上 npx 命令路径通过 PATHEXT 搜索解析
- 修复 Skills 保存/加载数据丢失问题（添加详细日志追踪）
- 修复设置页「应用信息」子页面无法滚动的问题

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.6.1]: https://github.com/Atom112/AIO/releases/tag/v0.6.1

---

## [v0.6.0]

<sup>Released on **2026-07-09**</sup>

### ⚡️ Changed（变更）

- **文件操作重构**：将 Agent 模式的文件系统工具从 MCP 子进程改为 in-process 原生调用，彻底解决子进程断连导致文件操作失败的问题。工具名和参数保持完全兼容，前端无需任何改动
- 优化 Agent 工作过程折叠：多轮工具调用结果合并到一条消息气泡中，减少界面混乱

### 🐛 Fixed（修复）

- 修复 Agent 模式下三种模式（Normal/Auto/Plan）的中断与错误处理
- 修复多个工具调用并发时结果批处理与递归 LLM 调用的协调问题
- 修复 `call_mcp_tool` 和 `list_mcp_tools` 使用合并配置以支持动态 MCP server
- 统一 AgentModeSelector 和 ProjectSelector 按钮样式
- 修复工具调用优化相关问题

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.6.0]: https://github.com/Atom112/AIO/releases/tag/v0.6.0

---

## [v0.3.1-Beta]

<sup>Released on **2026-02-11**</sup>

### 🐛 Fixed（修复）

- 修复了更换头像时缓存不会自动清除的问题

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.3.1-Beta]: https://github.com/Atom112/AIO/releases/tag/v0.3.1

---

## [v0.3.0-Beta]

<sup>Released on **2026-02-10**</sup>

### 🐛 Fixed（修复）

- 修复了首次打开应用时话题未选中的问题

### ✨ Added（新增）

- 初步实现了登录相关功能，但所有涉及后端与登录相关内容都处于试验状态

### ⚡️ Changed（变更）

- 优化了聊天上下文处理逻辑，重构了设置界面

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.3.0-Beta]: https://github.com/Atom112/AIO/releases/tag/v0.3.0

---

## [v0.2.13-Beta]

<sup>Released on **2026-02-01**</sup>

### ⚡️ Changed（变更）

- 优化消息输入框文件上传逻辑
- 增加单次对话上下文限制

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.2.13-Beta]: https://github.com/Atom112/AIO/releases/tag/v0.2.13

---

## [v0.2.12-Beta]

<sup>Released on **2026-01-31**</sup>

### ✨ Added（新增）

- 添加用户头像，AI供应商logo头像，支持用户更换头像

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.2.12-Beta]: https://github.com/Atom112/AIO/releases/tag/v0.2.12

---

## [v0.2.9-Beta]

<sup>Released on **2026-01-28**</sup>

### ✨ Added（新增）

- 新增`.deb`、`.rpm`和`.dmg`安装包

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.2.9-Beta]: https://github.com/Atom112/AIO/releases/tag/v0.2.9

---

## [v0.2.0-Beta]

<sup>Released on **2026-01-28**</sup>

### ✨ Added（新增）

- 集成`llama.cpp`支持运行`.gguf`本地模型

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.2.0-Beta]: https://github.com/Atom112/AIO/releases/tag/v0.2.0

---

## [v0.1.7-Beta]

<sup>Released on **2026-01-28**</sup>

### ✨ Added（新增）

- 添加了话题自动总结和话题重命名

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.1.7-Beta]: https://github.com/Atom112/AIO/releases/tag/v0.1.7

---

## [v0.1.6-Beta]

<sup>Released on **2026-01-27**</sup>

### ✨ Added（新增）

- 可用模型列表持久化存储

### 🐛 Fixed（修复）

- 修复了切换页面导致聊天框内文件卡片渲染失败的问题 **closes [#2](https://github.com/Atom112/AIO/issues/2) ([8e634d](https://github.com/Atom112/AIO/commit/8e634dabe85167d6d60448374c3fed45b0fa5950))**

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.1.6-Beta]: https://github.com/Atom112/AIO/releases/tag/v0.1.6

---

## [v0.1.5-Beta]

<sup>Released on **2026-01-27**</sup>

### ⚡️ Changed（变更）

- 优化了模型调用逻辑

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.1.5-Beta]: https://github.com/Atom112/AIO/releases/tag/v0.1.5

---

## [v0.1.3-Beta]

<sup>Released on **2026-01-26**</sup>

### ✨ Added（新增）

- 增加了文本文件上传功能
- 拖入文件上传时显示浮层提示

### 🐛 Fixed（修复）

- 修复了因事件监听器泄露导致的聊天文字重复问题 **closes [#1](https://github.com/Atom112/AIO/issues/1) ([ec957a](https://github.com/Atom112/AIO/commit/ec957a38f74b7fc0fa70f7e97e8fbbf193550ee6))**

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.1.3-Beta]: https://github.com/Atom112/AIO/releases/tag/v0.1.3

---

## [v0.1.0-Beta]

<sup>Released on **2026-01-24**</sup>

### ✨ Added（新增）

- AIO 初始版本发布。
- 集成了核心功能：综合API调用、模型选择与AI文本对话

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.1.0-Beta]: https://github.com/Atom112/AIO/releases/tag/v0.1.0

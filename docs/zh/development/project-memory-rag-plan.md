# 项目级自进化 RAG 记忆系统 — 实施计划

> 目标版本：AIO >= 0.8
> 范围：Agent 在项目内工作时持续生成、自主更新与该项目相关的事实，存入基于 SQLite 的向量检索存储，实现「项目级 RAG + 跨会话自进化记忆」。
> 状态：**规划中（未动代码）**。本文档为实施方案，供评审后按 P0-P4 分阶段落地。

---

## 0. 摘要

在现有「跨会话项目记忆」（.aio/knowledge.json + remember/recall 工具，子串匹配、上限 50 条）基础上，升级为**向量化、可自进化**的语义记忆系统：

1. **存储**：每个项目一个独立 SQLite 数据库 <项目根>/.aio/memory/memory.sqlite：事实表 + 版本历史表 + 向量索引 + FTS5 全文索引。
2. **向量**：目标采用**针对 SQLite 设计的向量插件 sqlite-vec（vec0 虚拟表）**；存储层做成 trait，若插件在三平台打包/签名上受阻，可无缝降级为「同表存 BLOB + Rust 余弦暴力扫描」（项目级事实规模下性能足够）。
3. **向量化（Embedding）**：新增 Embedder 插件抽象，首期实现 Ollama（默认，bge-m3 中文友好）与 OpenAI 兼容 /v1/embeddings（覆盖主流云端与本地网关）；不可用时自动降级为「关键词 + 时新度」检索并明确提示。
4. **自进化**：写入 = 显式工具（remember 升级）+ 后台自动事实提取（每轮 Agent 结束后异步、去抖、可开关）；更新 = 相似事实**合并 / 订正 / 淘汰**（LLM 仲裁 + 版本追踪 + 有效期），冲突时旧事实标记 superseded 而非删除；遗忘 = 访问加权 + 每项目容量上限 + 归档桶。
5. **检索**：混合检索（向量 kNN + FTS5 关键词 → RRF 融合 → 元数据过滤 + 时新度加成）→ **按 token 预算注入系统提示词**（自动注入 + 显式 search_memory / 升级版 recall），稳定前缀位置，兼容 prompt cache。

落地节奏：P0 验证性 Spike（sqlite-vec 三平台可用 + 嵌入延迟）→ P1 最小可用 RAG（存储 + 显式工具 + 自动注入）→ P2 自进化（自动提取 + 合并/冲突/遗忘 + 记忆面板）→ P3 向量插件正式接入与混合检索优化 → P4（可选项）离线嵌入 / 代码库索引 / 跨项目记忆。

---

## 1. 背景与目标

### 1.1 目标

- **项目级 RAG**：Agent 在某个项目（project_id）内工作时，能检索并利用「该项目此前所有会话沉淀下来的事实」，不再每次会话从头摸索。
- **自进化**：事实由 Agent 工作过程自然产生（对话、工具结果、文件/Git 变更），系统在后台自动提取、合并、订正、淘汰，无需人工维护。
- **本地优先**：全部数据落在用户磁盘（项目内），不强制上云；嵌入服务可配（本地引擎或用户自有 API）。
- **与现有架构一致**：复用 Tauri 命令 / 权限模型 / Skill / i18n / 文档同步等既有工程约定。

### 1.2 非目标（本期不做）

- 通用「多项目知识库」/ 全局记忆（P4 可评估）。
- 全仓库代码文件向量化索引（P4 单独立项，与 Git 变更/文件解析器联动）。
- 知识图谱 / 实体关系图（吸收 Zep/Graphiti 的「时间有效性」思想，但本阶段不做图查询）。
- 云端同步、团队共享记忆。

---

## 2. 现状分析（代码级）

### 2.1 Agent 运行模型（编排器模式）

src-tauri/src/commands/llm/mod.rs（共 4716 行）：

- run_agent_turn（L2836 起）是主循环。工具集按模式装配（L2969-3063）：
  - **Plan 模式**：think + project_map + web；
  - **Normal / Auto / Workflow**：delegate_task + delegate_tasks + create_workflow + think + web + project_map；
  - 知识工具（remember/recall）仅在 `is_agent_mode && knowledge_enabled && project_id.is_some()` 时追加（L3045）；
  - 另有 read_skill（L3049）、read_artifact（L3054）。
- **关键事实：主循环不直接持有文件/Git/Shell/LSP 工具**，实际文件操作由**子智能体**完成：build_subagent_tools（L1170）按 profile 过滤注入 file/shell/git/lsp/web 工具；**子智能体目前没有 remember/recall 工具**（L1173-1215 无知识工具）。
- 工具分发：execute_builtin_tool（约 L968 起）按 tool_name 分支，remember/recall → knowledge::execute_*（L1066-1069）；think / LSP / git / 文件工具各成分支。
- 上下文管理：模型感知 context_budget（L2905 附近，reserve 窗口的 15%）、compress_context 压缩早期消息（L4015 起，**稳定前缀系统块永不压缩**）、token 用量写 usage_log（L3265）、必达 Epilogue（L3932，emit terminal done）。
- 子智能体并发上限：max_concurrent_subagents（默认 5）信号量。

### 2.2 现有知识系统（=「核心记忆层」雏形）

src-tauri/src/utils/knowledge.rs（364 行）：

- 存储：`<项目根>/.aio/knowledge.json`，KnowledgeEntry { key, content, category(decision|pattern|convention|note), created_at, updated_at }。
- 写入：remember 工具，按 key 去重原地更新，**上限 50 条**（超出丢最旧）。
- 读取：recall 工具，key/content 大小写不敏感**子串匹配**；knowledge_to_prompt 把全部条目按 category 分组拼成 Markdown 注入系统提示词（run_agent_turn L3102-3120，插入位置 index=1）。
- 开关：全局 AppConfig.knowledge_enabled（默认 false，AppSettings「跨会话项目记忆」开关 L612-630；i18n key app.knowledge.title/description）。

**评估**：这是业界分层记忆中的「core memory」层，但缺三样东西：其一，语义检索（现在只能子串命中）；其二，规模与衰减（50 条上限、无权重/淘汰策略）；其三，自动沉淀（只靠模型主动调 remember，且 Normal 模式会触发审批，见 2.4）。

### 2.3 存储与基础设施

- 全局 SQLite：core/db.rs init_db 创建 app_data_dir/chat_history.db（rusqlite bundled，未启用 load_extension 特性），连接经 DbState（parking_lot Mutex）共享；已有 projects 表（id/name/path）。
- 项目：commands/project.rs，app_data_dir/projects.json 索引；每个项目初始化 `<项目根>/.aio/`（skills.json、mcp-servers.json（内置 **aio-filesystem** stdio MCP）、project.json 占位）；file_tools::resolve_project_root(app, project_id) 由 project_id 解析根路径（L973 等复用）。
- 依赖现状：Cargo.toml 无向量/ONNX/相似度 crate；已有 tiktoken-rs（utils/token_counter.rs）、regex、futures、dashmap、parking_lot 可复用。

### 2.4 权限模型

core/permission.rs（959 行）三层 allow/ask/deny：

- Normal：白名单读放行、写 Ask、**兜底 * → Ask（priority 0）**；Auto/Workflow：兜底 * → Allow（删除仍 Ask）；Plan：写/删 Deny。
- 因此现状：**Normal 模式下调用 remember 会触发审批弹窗**（走 execute_builtin_tool 权限检查，命中兜底 Ask）；Auto/Workflow 直接放行。
- 计划为记忆工具新增**显式默认规则**（读类 Allow；写类 Normal 下 Ask 或 Allow 由评审定，Auto/Workflow 默认 Allow），用户仍可在 .aio/permissions.json 覆盖。

### 2.5 前端现状

- 设置：features/settings/components/AppSettings.tsx（全局开关区）；features/chat/components/ProjectSettingsModal.tsx（项目级配置）。
- Agent 过程：core/store/store.ts 的 AgentStep { id, type: thinking|tool_call|content|subagent, ... }；AgentProcessBlock.tsx 渲染时间线；ToolCallBubble / ToolApprovalBubble 展示工具调用与审批。
- 事件：llm-round-start / llm-chunk / subagent-* / mcp-server-* 等，kebab-case 约定（AGENTS.md）。
- i18n：core/i18n，key 必须同时存在于 zh-CN 与 en-US 两个 locale（npm run check:i18n 强制）。

### 2.6 约束与工程规范（AGENTS.md / docs）

- 新 Rust 模块须在父 mod.rs 声明；新 Tauri 命令须在 commands/mod.rs 导出并在 lib.rs 的 generate_handler! 注册；命令有 doc comment、保持薄。
- 前端：Tailwind 工具类、禁 Emoji、禁硬编码 rgba、i18n 双语；事件 kebab-case。
- 验证：npm run verify（i18n 校验 + 文档链接 + prettier + eslint + tsc + vite build + cargo fmt/clippy/test）全绿。
- 扩展指南：docs/zh|en/development/extensions.md（新增 command / 插件 / 工具的最小测试与文档同步规范）。

---

## 3. 业界主流方案与启示

调研对象：Mem0、Letta（MemGPT）、Zep（Graphiti）、LangMem、Claude Code 自动记忆、SQLite 向量插件生态、本地嵌入模型。

### 3.1 记忆分层（Letta / MemGPT）

Letta 把 Agent 记忆分成三层：**core memory**（常驻上下文，结构化 blocks，可由 memory 工具编辑）、**archival memory**（语义向量库，按需召回）、**recall memory**（完整对话历史）。
参考：https://docs.letta.com/guides/agents/context-engineering

**启示**：AIO 已有「episodic」（chat_history.db 消息表）+「core」（knowledge.json）；缺的正是「semantic（archival）」这一层，本项目新增的事实库就是它。

### 3.2 写入路径与事实提取（Mem0）

Mem0 的 ADD 管线：原始消息 → LLM 提取事实（带归属、分类、时间戳）→ 与既有记忆做相似度比对 → 新增或更新；检索支持 similarity + 元数据过滤；操作集是 ADD / SEARCH / UPDATE。
参考：https://github.com/mem0ai/mem0/blob/main/skills/mem0/references/architecture.md

**启示**：事实必须**经 LLM 提取与粗筛**（不是什么都存）；写入先**向量查重**再落库。remember / recall 对应 ADD / SEARCH，新增的 update_memory 对应 UPDATE。

### 3.3 冲突与时间（Zep / Graphiti）

Zep 的 Graphiti 用**时序知识图谱**：事实作为边，带 valid_at / invalid_at 时间窗；新事实与旧事实冲突时**旧边失效而非删除**，支持按时间查询「当时的事实」。
参考：https://help.getzep.com/graphiti/getting-started/overview；论文解读：https://github.com/lhl/agentic-memory/blob/main/references/rasmussen-zep.md

**启示**：AIO 用轻量版：事实表带 valid_from / valid_until / status；冲突经 LLM 仲裁后将旧事实标记 superseded 并记录 fact_versions 审计链，可追溯「某时刻的项目状态」。

### 3.4 后台整合（sleep-time compute）

Letta 提出 **sleep-time compute**（arXiv:2504.13171）：Agent 交互结束后，后台异步对会话做压缩、抽取、记忆整理，代价与主交互解耦。
参考：https://arxiv.org/abs/2504.13171

**启示**：事实提取、合并、衰减全部放**异步后台任务**，绝不在主 Agent 循环内同步执行，避免拖慢交互与占用主连接。

### 3.5 自动记忆与记忆策略（Claude Code / LangMem）

Claude Code 的 memory 体系：CLAUDE.md 常驻（相当于 core）+ 允许模型**自动追加**项目记忆（Auto memory 实验能力）；LangMem 把「记忆管理」设计成带 ADD/SEARCH/UPDATE/DELETE 工具与策略层的 memory manager。
参考：https://code.claude.com/docs/zh-CN/memory；LangMem：https://langchain-ai.github.io/langmem/

**启示**：其一，记忆写入应**策略化**（重要性阈值、类别白名单、去重规则），避免垃圾进库；其二，沉淀后的知识对用户**可见、可删、可审计**；其三，自动记忆与显式工具并存，用户可关。

### 3.6 SQLite 向量插件选型

| 方案                     | 形态                                                                                                                          | 优点                                                                                            | 顾虑                                                                                                               |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| sqlite-vec（asg017）     | SQLite 扩展，vec0 虚拟表，纯 SQL kNN（MATCH ... ORDER BY distance），支持 cosine / dot / L2，官方 Rust crate 可静态链接或加载 | 专为 SQLite 设计（正合「针对 SQLite 的向量数据库插件」需求）；API 简单；元数据列可直接过滤；MIT | 0.1.x 预发布版 API 变动；需为 Win/macOS/Linux 打包原生库；macOS 加固运行时签名；维度在建表时固定，换模型需重建索引 |
| sqlite-vss / VectorLite  | 扩展 / Rust 原生                                                                                                              | VectorLite 内置 ONNX 本地嵌入（bge 小模型），离线可用                                           | 生态/维护一般；ONNX runtime 体积大（数十 MB）；模型文件需随包分发                                                  |
| 纯 Rust 暴力余弦（自研） | 事实表存 float32 BLOB，内存/游标打分                                                                                          | 零原生依赖、零打包风险；同一 schema 可平滑升级                                                  | 5 万条 x 1024 维全表扫描约数 ms 到数十 ms（可接受）；无 SQL 内 kNN                                                 |

参考：sqlite-vec：https://github.com/asg017/sqlite-vec；VectorLite：https://github.com/mmailhos/vectorlite

**决策**：**优先 sqlite-vec（vec0）为正式载体**（P0 先验证三平台编译/加载/签名）；存储层抽象为 MemoryStore trait，把「向量打分」做成可插拔实现（vec0 或 Rust 暴力），P1 可先以纯 Rust 实现跑通全链路，P3 切 vec0 并保留降级开关。

### 3.7 向量化（Embedding）选型

| 来源                                   | 方式                               | 场景                                                                          |
| -------------------------------------- | ---------------------------------- | ----------------------------------------------------------------------------- |
| Ollama（/api/embeddings）              | bge-m3（中文强）、nomic-embed-text | 本地优先默认项；复用 EngineManager 探测 base URL；AIO 用户群本地引擎占比高    |
| OpenAI 兼容 /v1/embeddings             | text-embedding-3-small 等          | 云端/网关（OpenAI、SiliconFlow、各家代理）；复用现有 apiUrl/apiKey 或独立配置 |
| llama.cpp 服务 /v1/embeddings          | 需 --embeddings 启动参数           | 已有 llama_cpp 引擎，作为第二本地选项（P3 评估）                              |
| 内置 ONNX（VectorLite / fastembed-rs） | bge-small 等                       | 完全离线、无外部服务（P4 评估，注意体积）                                     |

参考：Ollama 嵌入模型：https://ollama.com/search?c=embedding；nomic-embed-text-v2：https://ollama.com/library/nomic-embed-text-v2-moe

**决策**：本地与在线并存、默认本地 Ollama（bge-m3）；embedder 不可用时**不阻塞**：事实照常入库，检索降级为 FTS5 关键词 + 时新度，UI 明确提示「未配置嵌入，仅关键词检索」，并支持一键补建索引。嵌入模型选择交互**与 LLM 供应商选择一致**：provider 下拉（ollama / openai_compat）+ 模型下拉（Ollama 从 /api/tags 动态拉取、OpenAI 兼容从 /v1/models 过滤或以文本输入）+ 测试连接，用户可自由选择嵌入模型（详见 4.5）。

### 3.8 设计原则汇总（本方案采纳）

1. 记忆分层：core（knowledge.json）/ semantic（事实库，本期新增）/ episodic（chat_history.db，已有）三层各司其职。
2. 写入先提取、后查重、再落库；提取与整理走后台异步。
3. 更新不覆盖式抹除：版本链 + superseded 状态 + 有效期，可审计、可回滚。
4. 检索混合：向量 + 关键词 + 元数据 + 时新度；按 token 预算注入稳定前缀。
5. 本地优先、用户可控：每项目隔离、容量上限、可见可删、权限规则可覆盖。
6. 全链路可降级：无插件 → 暴力余弦；无 embedding → 关键词；召回失败 → 不注入（对对话质量 fail-open，对数据安全 fail-safe）。

---

## 4. 总体设计

### 4.1 架构总览

```text
前端（SolidJS）
  AppSettings（记忆与检索分区）  ProjectSettingsModal（每项目开关）  MemoryPanel（记忆面板）
        | invoke / event（memory-* kebab-case）
        v
commands/memory.rs（薄命令层）
  status | search | list | get | add | update | delete | reindex | clear | stats
        |
        v
services/memory/
  MemoryStoreManager（DashMap + LRU 缓存 + 后台提取调度）
    MemoryStore（每项目一个：facts/versions 表 + 向量索引 + FTS5 + 检索/注入/RRF/预算）
        | 嵌入向量
        v
plugins/embed/  Embedder trait
  ollama | openai_compat | (llama_cpp, P3) | (onnx, P4)

Agent 循环集成点（commands/llm/mod.rs）
  1) 工具装配：remember / recall / search_memory / update_memory / forget_memory
  2) 自动注入：每轮 user 消息 → 检索 top-k → 系统前缀块（index=1，稳定前缀）
  3) 自动提取：Epilogue 后 spawn 后台 extract_facts_from_transcript
```

### 4.2 记忆分层映射

| 层             | 载体                                             | 生命周期            | 本期动作                                                                 |
| -------------- | ------------------------------------------------ | ------------------- | ------------------------------------------------------------------------ |
| Core           | .aio/knowledge.json（KnowledgeEntry）            | 常驻系统提示词      | 保留；首次启用 RAG 时迁移为事实并标记来源；继续提供 remember/recall 兼容 |
| Semantic（新） | .aio/memory/memory.sqlite 的 facts 表 + 向量索引 | 按需召回 + 自动注入 | 本期主体                                                                 |
| Episodic       | chat_history.db 的 messages 表                   | 会话历史            | 不动；作为提取管线的输入源                                               |

### 4.3 存储设计

**位置**：`<项目根>/.aio/memory/memory.sqlite`（随项目走、随项目删、天然隔离、不与全局 chat_history.db 争用 Mutex）。

路径决策（用户确认）：选**项目路径**而非系统 app_data 路径。理由：其一，与既有 .aio 约定一致（knowledge.json、skills.json、mcp-servers.json 均在项目内），事实本质是项目资产，随项目移动/备份/删除；其二，天然隔离，不占系统目录，删除项目即连带清理；其三，系统路径方案在项目目录移动后会留下孤儿数据，还需按 project_id 反查。配套措施：文档建议把 `.aio/memory/` 加入项目的 .gitignore（项目无 .gitignore 时 AIO 首次创建记忆库可自动写入该条目）。

空间预估（默认上限 2000 条/项目）：事实文本约 0.3-0.6 MB；向量按 bge-m3 1024 维 float32 计约 8 MB（384 维模型仅约 3 MB）；vec0 与 FTS5 索引开销约 1-2 MB；合计**每项目约 10-15 MB**，万条级最坏约 60-80 MB；可选 int8 量化使向量体积再降约 4 倍。结论：RAG 存储占用很小，不是磁盘关注点，项目路径方案无空间障碍。

- 连接管理：新增 MemoryStoreManager（全局状态，lib.rs 注册），DashMap<project_id, Arc<MemoryStore>> + LRU（如 idle 30 分钟逐出）；每个 store 持有独立 parking_lot::Mutex<rusqlite::Connection>，WAL + busy_timeout(5000)；绝不占用 DbState 主连接。
- 开关优先级：app 级 memory_enabled 为总闸；每项目可覆盖（ProjectSettingsModal 开关）。
- 同步策略：显式工具路径即时落库；提取/合并/衰减走后台（见 4.6）。
- sqlite-vec 接线：rusqlite 需启用 load_extension 特性；P0 验证静态链接（sqlite_vec crate 的 sqlite3_vec_init + Connection::load_extension_auto）或按靶平台分发预编译插件（bundle.resources，注意 macOS 签名）；vec0 表：`CREATE VIRTUAL TABLE vec_facts USING vec0(embedding float[1024] distance_metric=cosine)`，维度建表时固定，换模型/维度需重建向量表（reindex 命令承载）。

### 4.4 向量与混合检索

1. query 文本 → Embedder 出向量（不可用则跳过向量分支）。
2. 三路召回：
   - 向量：vec0 kNN（k=30，cosine；降级态改为 Rust 余弦扫描同表 BLOB）；
   - 关键词：FTS5（content/category/source 字段，BM25 排序，k=30）；
   - 元数据：category / source_type / 文件路径过滤（可选参数）。
3. RRF 融合（reciprocal rank fusion，k 取 60 常规值）合并排名，附加**时新度加成**（updated_at 近 90 天内轻微加分，系数可配）。
4. token 预算选择：用现有 utils/token_counter.rs 估算候选串接 token 数，按 memory_injection_budget（默认 3000）依次收拢，超预算截断并标注「已截断 n 条」。
5. 输出格式：Markdown 列表，每条带序号、类别、置信度、更新时间与来源（文件/消息 id），便于模型判断可信度。

### 4.5 Embedder 插件抽象

新增 plugins/embed/mod.rs：

```rust
pub trait Embedder: Send + Sync {
    fn id(&self) -> &str;                 // ollama | openai_compat | ...
    fn model_key(&self) -> String;        // 如 bge-m3:latest
    fn dimensions(&self) -> usize;        // 实际维度（Ollama API 返回）
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String>;
    fn is_available(&self) -> bool;
}
```

- OllamaEmbedder：POST {base}/api/embeddings {model, input}；base 由 EngineManager 探测或用户手动填；bge-m3 默认，nomic-embed-text 备选。
- OpenAiCompatEmbedder：POST {apiUrl}/embeddings，Authorization Bearer {apiKey}；模型/维度可配（text-embedding-3-small 等）。
- 注册：EmbedderManager::new() 内登记，dispatch 按配置 provider 选择（与 ProviderManager / EngineManager 同一模式）。
- 在线 vs 本地：两种都支持，默认本地 Ollama（自动探测正在运行的 Ollama 并列出可用嵌入模型）。在线成本极低：2000 条事实全文重索引约 20 万 token（text-embedding-3-small 约 0.004 美元），日常增量可忽略；代价是事实文本按用户配置的 provider 出域（key 走 secure_store，UI 明示）。
- 本地模型体积与性能：embedding 属小模型（0.03B-0.6B 参数量），CPU 即可推理、无需 GPU。常用体积：all-MiniLM-L6-v2 约 46 MB（384 维）、nomic-embed-text 约 274 MB（768 维）、bge-m3 约 1.2 GB（1024 维，中文与多语言最佳，默认推荐）。首用需下载一次；Ollama 加载后常驻内存约 0.2-2 GB（用户已在跑本地 LLM 时边际成本很小）；单句编码延迟 10-100 ms，批量 30 条约数百 ms，满足每轮一次查询。P4 可加 ONNX 进程内嵌入（bge-small-zh 约 95 MB），断网可用、按需加载释放。
- 模型获取方式（用户提问确认）：**不打包进安装包，首次使用时按需下载**。默认本地路径走 Ollama：启动时探测 Ollama 可用性并查询模型是否已存在（ollama list），缺失时调用 POST /api/pull 拉取并流式展示进度（模型文件归 Ollama 管理，位于用户 Ollama 模型库，与聊天模型同源）；P4 的 ONNX 路径则下载到 app_data_dir/models/embeddings/（断点续传 + 进度事件 + 磁盘空间预检）。理由：bge-m3 约 1.2 GB，而记忆为默认关闭功能，打包会拖累所有用户的安装包体积与发布流水线；按需下载搭配多模型可选（46 MB 到 1.2 GB）更灵活；且与仓库既有先例一致（installer.rs 明示引擎二进制不打包、app_data_dir 承载运行时产物）。
- 首次启用引导：用户开启项目记忆 → 应用检测嵌入就绪状态（provider 已配且模型已下载 → 就绪；未下载 → 待下载；未配 → 未配置）→ 未就绪时引导进入 EmbeddingSetup 向导，同时允许「稍后配置」先以关键词模式使用（事实照常入库，就绪后一键补嵌）。
- 模型一致性约束：同一项目内所有向量必须由同一嵌入模型生成，因此嵌入 provider/model 为**全局配置**（切换需 reindex），与「每项目启用开关」解耦（开关在项目设置弹窗，嵌入模型在应用设置）。
- 配置：AppConfig 新增 memory_embedding 组（provider / model / api_url / api_key（存 secure_store）/ dimensions）；embedding_test 命令做连通性验证（对齐 test_provider_connection 交互）。

### 4.6 自进化管线（核心）

#### 写入：三个来源

1. **显式工具**：remember（升级）→ 写事实库：先按 key 精确查重，再按向量 top-3 近邻查重；若近邻相似度高于阈值（如 0.92），进入「合并判定」而非直接新增。knowledge_enabled 开启时保持对 knowledge.json 的向后兼容双写（迁移期结束后可下线）。
2. **自动提取**（P2）：run_agent_turn 的 Epilogue（L3932 前后）检测 memory_enabled + auto_extract 且项目激活，spawn 后台任务 extract_facts_from_transcript：
   - 输入：本轮 user 消息 + assistant 最终内容 + 关键工具结果（截断版），最近约 10k 字符，窗口可配；
   - 输出：LLM 提取 JSON [{content, category, importance 0-1, evidence}]，schema 约束；
   - 过滤：importance >= 阈值（默认 0.6）、单轮上限（默认 5 条）、类别白名单校验；
   - 去抖：同一项目两次提取间隔 >= 60s；同一时刻仅一个提取任务（per-project 异步锁）；
   - 失败处理：不阻塞主流程，静默降级（tracing error + 计数）。
3. **子智能体事实**：v1 子智能体不直接写记忆（避免并发写乱序）；其总结中的「关键发现」由主循环提取覆盖。v2 可选：给子智能体注入只读 recall。

#### 更新：LLM 仲裁 + 版本链

新事实与既有事实「同 key」或「向量近邻高分」时，调用**仲裁提示词**（复用当前激活模型，JSON 输出）：

```text
action: merge | update | keep_separate | supersede
target_id: 目标既有事实 id（merge/update/supersede 时）
merged_content: 合并后的内容（merge 时）
reason: 一句话理由
```

- merge：两条合并为新版本（fact_versions 追加，旧版本保留）；
- update：以新事实为准订正（版本递增，记录 why）；
- supersede：旧事实标 superseded + valid_until=now，新事实为当前态（冲突场景，如「项目已从 Cargo 迁移到 pnpm」）；
- keep_separate：两件事不同（阈值误报），不动。
- **fail-safe**：仲裁调用失败/超时 → 默认 keep_separate（宁可冗余，不误删误并）。仲裁记入 versions 审计。

#### 遗忘：容量与衰减

- 每项目活跃事实上限 memory_max_facts（默认 2000）；超出时按 score = importance 乘访问衰减因子排序，最低分先入 archived（软删，不硬删），用户面板可永久清除。
- pinned 事实（用户钉住）永不自动归档。
- 后台衰减任务（受 sleep-time 调度）：定期重算 score 并标记 stale；仅影响内存与查询排序，不动数据本体。

### 4.7 检索与注入

- 触发点 A（自动）：每轮循环收到新的 user 消息时（或工具结果明确变更项目状态后），重算「项目记忆块」，仅在内容变化时替换 messages_for_api 的 index=1 系统块（**稳定前缀保护 + prompt cache 友好**，compress_context 已保证稳定块不压缩）。
- 触发点 B（显式）：recall / search_memory 工具由模型按需调用，返回结构化 result（前端 AgentStep 复用现有 ToolCall 渲染）。
- 预算：默认注入 3000 tokens（可配 memory_injection_budget），上限受 context_budget 约束（取 min）；检索失败或为空 → 注入块省略并在该轮标注（不打断任务）。
- 顺序：注入块放在 locale 指令之后、其余系统块之前（沿用现有 knowledge 块位置 index=1），保证跨版本稳定。

### 4.8 工具面（模型视角）

| 工具          | 动作                                            | 权限默认（Normal）  | 说明                            |
| ------------- | ----------------------------------------------- | ------------------- | ------------------------------- |
| remember      | 新增/更新事实（key+content+category）           | Allow（新默认规则） | 升级自现有；语义查重 + 仲裁合并 |
| recall        | 语义/关键词混合检索 top-k                       | Allow               | 升级自现有；返回带分数与来源    |
| search_memory | 带过滤器检索（category/source/k/时间窗/最低分） | Allow               | 新增                            |
| update_memory | 按 id 订正事实                                  | Ask（可改 Allow）   | 新增；受仲裁保护                |
| forget_memory | 按 id 归档/清除事实                             | Ask                 | 新增；pinned 保护               |

子智能体：v1 不注入记忆工具（保持工具面收敛，事实由主循环提取覆盖）；v2 增加只读 recall。

### 4.9 权限集成

- 在 permission.rs 的 default_rules_for_mode 为五个记忆工具添加显式规则（Normal/Auto/Workflow/Plan 各一组）：读类（recall/search_memory）全部 Agent 模式 Allow（Plan 含）；写类（remember/update_memory/forget_memory）Normal 下 Ask、Auto/Workflow 下 Allow、Plan 下 Deny。
- 规则与系统工具共用 server_id **aio-filesystem** 兼容旧规则；用户可在 .aio/permissions.json 覆盖。
- execute_builtin_tool 分发分支新增 memory::execute_* 分支（在现有 remember/recall 分支位置扩展）。

### 4.10 配置与前端

**AppConfig 新增字段**（models.rs 定义，默认值语义与 knowledge_enabled 一致）：

- memory_enabled（总闸，默认 false）
- memory_auto_inject（默认 true，随 memory_enabled 生效）
- memory_auto_extract（默认 true，后台提取）
- embedding_provider（ollama | openai_compat，默认 ollama）
- embedding_model / embedding_api_url / embedding_dimensions（api key 存 secure_store）
- memory_max_facts（默认 2000）、memory_injection_budget_tokens（默认 3000）、memory_extract_debounce_secs（默认 60）

**前端**：

- AppSettings 新增「嵌入与记忆」分区（仅全局配置，不含每项目开关）：嵌入 provider/模型选择 + 测试连接（embedding_test 命令）、自动注入/自动提取的默认开关、容量与预算设置、统计（事实数/检索命中/最近提取时间）、重建索引按钮。
- ProjectSettingsModal：在「模型选择（绑定模型）」区块之后、MCP 服务器与 Skill 管理之前（ProjectSettingsModal.tsx 模型区块与 MCP 区块之间）新增「项目记忆」区块：启用开关（默认关）+ 嵌入可用性状态 + 打开记忆面板入口 + 重建索引快捷入口。
- 新增 MemoryPanel 组件：全文/语义搜索、事实列表（类别/置信度/更新时间/版本/来源）、编辑/钉住/归档/删除、版本历史、迁移 knowledge.json 入口。
- AgentProcessBlock：记忆相关 tool_call 复用现有时间线；自动注入时在消息区显示轻量提示「已注入项目记忆 n 条（约 x tokens）」。

---

## 5. 数据模型（DDL 草案）

memory.sqlite（每项目一份，rusqlite 执行，全部语句幂等）：

```sql
CREATE TABLE IF NOT EXISTS facts (
  id TEXT PRIMARY KEY,                  -- uuid v4
  key TEXT,                             -- 稳定去重键（来自 remember，可为 NULL）
  content TEXT NOT NULL,                -- 事实正文（自然语言）
  category TEXT NOT NULL DEFAULT note,  -- decision|pattern|convention|note|entity|architecture|task
  importance REAL NOT NULL DEFAULT 0.5, -- 提取置信度/重要性 0..1
  status TEXT NOT NULL DEFAULT active,  -- active|superseded|archived
  embedding BLOB,                       -- float32 LE，维度见 embedding_dim
  embedding_model TEXT,                 -- 生成该向量的模型，如 bge-m3
  embedding_dim INTEGER,                -- 实际维度
  confidence REAL,                      -- 提取置信度 0..1
  source_type TEXT,                     -- conversation|tool|file|git|user
  source_refs TEXT,                     -- JSON 数组 {type, id, file, commit, messageId}
  valid_from TEXT,                      -- ISO 时间，生效起始
  valid_until TEXT,                     -- ISO 时间，失效时间（superseded 时置 now）
  created_at TEXT NOT NULL,             -- ISO 时间
  updated_at TEXT NOT NULL,             -- ISO 时间
  access_count INTEGER NOT NULL DEFAULT 0,  -- 检索命中次数（衰减用）
  last_access_at TEXT,                  -- ISO 时间
  pinned INTEGER NOT NULL DEFAULT 0     -- 用户钉住，禁止自动归档
);
CREATE INDEX IF NOT EXISTS idx_facts_status ON facts(status);
CREATE INDEX IF NOT EXISTS idx_facts_category ON facts(category);

CREATE TABLE IF NOT EXISTS fact_versions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  fact_id TEXT NOT NULL REFERENCES facts(id) ON DELETE CASCADE,
  version INTEGER NOT NULL,
  reason TEXT NOT NULL,                 -- create|update|merge|supersede|restore
  content_before TEXT,
  content_after TEXT,
  related_fact_ids TEXT,                -- JSON 数组（merge 的来源事实）
  judge_model TEXT,                     -- 本次仲裁使用的模型
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS meta (
  k TEXT PRIMARY KEY,
  v TEXT NOT NULL
);

-- sqlite-vec（P3 接入；P1/P2 用 Rust 暴力余弦读取 facts.embedding BLOB）
CREATE VIRTUAL TABLE vec_facts USING vec0(
  embedding float[1024] distance_metric=cosine,
  fact_id text +,
  category text +,
  status text +
);

-- FTS5 关键词索引（内容 + 类别 + 来源，content 外挂 facts 表）
CREATE VIRTUAL TABLE fts_facts USING fts5(
  fact_id UNINDEXED, content, category, source_type,
  content=facts, content_rowid=rowid
);
```

说明：

- 换嵌入模型/维度：重建 vec_facts（reindex 命令）：清空向量表 → 全量重 embed → 重插；facts 文本与 FTS5 不受影响。
- 迁移：首次启用 RAG 时把 knowledge.json 条目转为 facts（source_type=user，category 映射，pinned=1 可选），并在 memory/meta 标记已迁移。
- 删除项目：删除项目目录 .aio 即连带清除；project.rs 的 delete_project 顺带清理 memory 连接缓存。

---

## 6. 模块划分与代码落点

### 6.1 Rust 后端（新增/修改）

| 文件                                                                                       | 动作                                   | 内容                                                                                                                                                                                                                   |
| ------------------------------------------------------------------------------------------ | -------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| src-tauri/src/services/memory/（mod.rs + store.rs + extract.rs + retrieval.rs + judge.rs） | 新增（按评审定放 services 或 core 下） | MemoryStore（连接/schema/CRUD/向量打分 trait impl）、事实提取调度、混合检索与注入格式化、仲裁调用                                                                                                                      |
| src-tauri/src/services/mod.rs                                                              | 新增                                   | 声明 services/memory                                                                                                                                                                                                   |
| src-tauri/src/plugins/embed/（mod.rs + ollama.rs + openai_compat.rs）                      | 新增                                   | Embedder trait + 两个实现 + EmbedderManager 注册                                                                                                                                                                       |
| src-tauri/src/commands/memory.rs                                                           | 新增                                   | 薄命令层（status/search/list/get/add/update/delete/reindex/clear/stats），doc comment 齐全                                                                                                                             |
| src-tauri/src/commands/embedding.rs                                                        | 新增                                   | 嵌入供给命令（test/status/list_ollama_models/pull/delete）：调 Ollama /api/pull 流式解析状态与进度、经事件推送、下载状态存于 EmbedderManager（DownloadState，单模型单任务 + 可取消）；Ollama 模型列表经 /api/tags 获取 |
| src-tauri/src/commands/mod.rs                                                              | 修改                                   | 导出 memory                                                                                                                                                                                                            |
| src-tauri/src/commands/llm/mod.rs                                                          | 修改                                   | 工具装配追加记忆工具（L3045 区域扩展）；execute_builtin_tool 分发新增 memory 分支；Epilogue 前 spawn 后台提取；知识块替换为记忆块（index=1 稳定位）                                                                    |
| src-tauri/src/core/models.rs                                                               | 修改                                   | AppConfig 新增 memory_* 字段与默认值                                                                                                                                                                                   |
| src-tauri/src/core/state.rs + lib.rs                                                       | 修改                                   | MemoryStoreManager、EmbedderManager 注册进 Tauri manage 与清理逻辑                                                                                                                                                     |
| src-tauri/src/core/permission.rs                                                           | 修改                                   | 五个记忆工具的默认规则（各模式）                                                                                                                                                                                       |
| src-tauri/src/utils/knowledge.rs                                                           | 修改                                   | 兼容层：RAG 启用时写入同时落 facts；提供迁移辅助                                                                                                                                                                       |
| src-tauri/Cargo.toml                                                                       | 修改                                   | 视 P0 结论增加 sqlite-vec 及（可选）相关 crate；rusqlite 增 load_extension 特性                                                                                                                                        |
| src-tauri/tauri.conf.json                                                                  | 修改                                   | 若采用插件分发：bundle.resources 按靶平台加 sqlite-vec 原生扩展文件                                                                                                                                                    |

### 6.2 前端（新增/修改）

| 文件                                                  | 动作 | 内容                                                                                                                                                                                                                         |
| ----------------------------------------------------- | ---- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| src/core/types/memory.ts                              | 新增 | MemoryFact / MemorySearchResult / MemoryStatus / EmbeddingConfig 类型（后端镜像）                                                                                                                                            |
| src/core/store/store.ts                               | 修改 | memoryStatus 信号 + 相关 actions（invoke 封装）                                                                                                                                                                              |
| src/features/settings/components/AppSettings.tsx      | 修改 | 「嵌入与记忆」分区（provider 选择、模型列表/手动输入、下载管理、测试连接、统计、重建索引；不含每项目开关）                                                                                                                   |
| src/features/settings/components/EmbeddingSetup.tsx   | 新增 | 嵌入供给向导：provider 选择，Ollama 已装模型列表（/api/tags）+ 推荐模型下载入口（bge-m3/nomic/MiniLM，显示体积），流式进度条与取消/重试，删除模型释放空间，连接测试；被 AppSettings 内嵌，并从 ProjectSettingsModal 引导跳转 |
| src/features/chat/components/ProjectSettingsModal.tsx | 修改 | 模型区块与 MCP 区块之间插入「项目记忆」区块（开关默认关 + 嵌入状态 + 记忆面板入口）                                                                                                                                          |
| src/features/chat/components/MemoryPanel.tsx          | 新增 | 记忆面板（搜索/列表/编辑/删除/版本/迁移）                                                                                                                                                                                    |
| src/features/chat/components/AgentProcessBlock.tsx    | 修改 | 记忆自动注入提示 chip；memory 类 tool_step 时间线样式                                                                                                                                                                        |
| src/core/i18n/locales/zh-CN.json + en-US.json         | 修改 | 全部新增文案 key（check:i18n 强制成对）                                                                                                                                                                                      |

### 6.3 命令与事件清单（新增）

命令（lib.rs 注册）：记忆类：memory_get_status / memory_search / memory_list / memory_get / memory_add / memory_update / memory_delete / memory_reindex / memory_clear / memory_stats；嵌入供给类（commands/embedding.rs）：embedding_test / embedding_status / embedding_list_ollama_models / embedding_pull_ollama_model / embedding_delete_ollama_model。

事件（kebab-case）：memory-status（状态变更）、memory-updated（事实写入/合并/淘汰）、memory-extract-progress（后台提取进度，低频）、embedding-download-progress（嵌入模型下载进度 {provider, model, status, completed, total, error}）、reindex-progress（重建索引进度）。

---

## 7. 分阶段实施计划

总体原则：每阶段产出可独立验证的增量，P1 即可让用户用上「存得住、搜得着」的项目 RAG，P2 才引入自动进化的复杂度。

### P0 验证性 Spike（约 3-5 个工作日）

目标：消除三项最大技术风险，输出决策基线。

1. **sqlite-vec 可落地性**：在 Windows / macOS / Linux 三平台各编译一次：
   - 静态链接方案：sqlite-vec crate（loadable 特性）+ rusqlite bundled + load_extension 特性 + Connection::load_extension_auto；
   - 动态加载方案：预编译 dylib/so/dll 放 bundle.resources，运行时 load_extension(path)；
   - 记录：编译耗时、体积、macOS 签名/加固运行时的处理方式（签名白名单或静态链接规避）。
2. **嵌入延迟与维度**：Ollama 实测 bge-m3 与 nomic-embed-text 的：首调延迟（含模型加载）、批量 30 条吞吐、向量维度；OpenAI 兼容 /v1/embeddings 同测。决定默认模型与 batch 策略。
3. **暴力余弦性能基线**：构造 1 万 / 5 万条 1024 维事实集，测 Rust 全表扫描打分耗时，验证降级路径可用。
4. **Rust 暴力余弦与 vec0 的返回一致性**：同一数据集上比对 top-k 一致性（容差内即可，决定 P3 是否需双实现维护）。

**产出**：P0 报告（四份实测数据 + 选型结论 + Cargo 依赖锁定），评审是否按 P1 开工。

### P1 最小可用 RAG（约 2-3 周）

目标：项目事实能存、能搜、能注入，全链路可测、可回退。

工作项：

1. services/memory 骨架：MemoryStore（schema 幂等初始化、CRUD、knowledge.json 迁移）、MemoryStoreManager（LRU + 每项目连接）；
2. 向量打分 trait：P1 先实现 Rust 暴力余弦（BLOB 读取 + 归一化点积），预留 Vec0Score 的空实现；
3. plugins/embed：Embedder trait + ollama + openai_compat 实现 + EmbedderManager；embedding 服务可用性探测；
4. 工具升级：remember（写入+查重+知识块双写兼容）、recall（混合检索）、新增 search_memory；permission.rs 默认规则；execute_builtin_tool 分发；
5. 自动注入：每轮 user 消息触发检索，token 预算裁剪，替换 index=1 系统块（内容 hash 去重，避免无谓替换）；
6. 配置与命令：AppConfig memory_* 字段；memory.rs 命令（status/search/list/get/add/update/delete/reindex/clear/stats）；embedding.rs 嵌入供给命令（test/status/list_ollama_models/pull/delete，Ollama /api/pull 流式进度 → embedding-download-progress 事件 + 可取消）；lib.rs 注册；
7. 前端：ProjectSettingsModal 新增「项目记忆」区块（模型选择下方、MCP/Skill 上方，含嵌入就绪状态与引导提示）；AppSettings「嵌入与记忆」分区：provider 下拉（ollama / openai_compat）+ 模型下拉（Ollama /api/tags 动态拉取或手动输入）+ EmbeddingSetup 下载向导（推荐模型入口与体积、流式进度条、取消/失败重试、删除释放空间、embedding_test 连接测试）+ i18n 双语 key；
8. 单元/集成测试（见第 8 节）与 npm run verify 全绿。

**验收**：

- 新建项目 → 开启记忆 → 两轮不同会话内：会话 A 让 agent 记住架构决策，会话 B 提问可检索到并注入系统提示词；
- embedding 断开时：检索自动降级关键词并提示；恢复后 reindex 补嵌；
- 下载交互验收：首次启用引导可达；下载中进度/取消/失败重试/空间不足提示符合预期，下载完成即自动可用；下载中嵌入不可用但关键词降级照常；删除模型后可重新下载；
- remember/recall 与旧行为兼容（knowledge.json 仍可读）；permissions.json 可覆盖默认规则；
- verify 全绿；三平台冒烟（Win 必须，mac/Linux 视 CI）。

### P2 自进化（约 3-4 周）

目标：事实自动产生、自动演化，人工维护趋近于零。

工作项：

1. 后台提取管线：extract_facts_from_transcript（输入窗口、schema 约束 JSON 输出、importance 过滤、单轮上限、去抖、per-project 锁、失败静默降级），Epilogue 挂载；
2. 仲裁与版本链：judge 调用（merge/update/keep_separate/supersede + fail-safe 默认 keep_separate）、fact_versions 落审计、superseded 状态与 valid_until；
3. 遗忘与容量：score 衰减计算、archived 软删桶、pinned 保护、memory_max_facts 容量治理；
4. 记忆面板：MemoryPanel（搜索/列表/编辑/钉住/归档/删除/版本历史/迁移入口），ProjectSettingsModal 入口；
5. 事件：memory-status / memory-updated / memory-extract-progress；AgentProcessBlock 注入提示 chip；
6. 质量探针：检索命中率（用户追问时是否命中原事实）、事实去重率、仲裁动作分布——导出 stats 命令与面板展示。

**验收**：

- 连续 10 轮不同类型任务（改代码/查文档/修 bug）后，事实库自动沉淀 >= 20 条且无重复主题井喷；
- 同一事实两次写入：第二次触发 update/merge，版本链 >= 2，旧版本可查；
- 冲突场景（如依赖迁移）：旧事实 superseded、新事实 active，检索返回新事实；
- 关掉自动提取后系统行为与 P1 等价（开关隔离）；
- 记忆面板可完成全部人工治理操作且权限规则生效。

### P3 向量插件正式接入与混合检索优化（约 2-3 周）

目标：把 P1 的暴力余弦替换为 sqlite-vec（vec0），检索质量与性能双提升。

工作项：

1. vec0 表接入：建表/写入/批量 upsert、Cargo 依赖与资源打包（按 P0 结论）、降级开关（config 或编译特性）；
2. FTS5 混合：fts_facts 建表与同步、RRF 融合、时新度加成参数化；
3. 检索质量评测：构造查询集（30-50 条带标注 query），对比三种模式（纯向量 / FTS / 混合）的 NDCG@10，定默认参数；
4. reindex 完善：换模型/维度时的全量重建 + 进度事件；
5. （评估）llama.cpp /v1/embeddings 作为第二本地嵌入；memory export/import（JSON 打包随项目迁移）。

**验收**：三平台 release 构建通过；混合检索质量不低于纯向量；重建索引中断可续跑；切换嵌入模型后数据正确迁移。

### P4（可选项，视用户反馈）

- 离线嵌入：VectorLite 或 fastembed-rs（ONNX），完全断网可用；
- 代码库级索引：结合 file_parser + git 变更，将关键文件摘要/chunk 入向量库（与事实库并行表），实现「代码级 RAG」；
  NaN

---

## 8. 测试与质量

### 8.1 Rust 单元测试（随模块落地）

- schema：初始化幂等、迁移（knowledge.json 转 facts）、外键级联；
- 写入：key 去重、向量近邻阈值触发的合并判定、pinned 保护、容量裁剪；
- 仲裁：注入假 judge（trait 注入 mock）覆盖 merge / update / keep_separate / supersede / 失败默认 keep_separate；
- 检索：固定向量集上的 top-k 正确性（与手算 cosine 比对）、RRF 排序稳定、token 预算裁剪边界；
- 权限：五个记忆工具在 Normal/Auto/Workflow/Plan 下的 allow/ask/deny（对齐 permission.rs 既有测试模式）；
- 降级：embedder 不可用路径、vec0 不可用路径的纯关键词检索。

### 8.2 集成测试

- 脚本化 LLM：替换真实 HTTP 为本地 mock 服务（复用现有 streaming 测试思路），跑完整 run_agent_turn，断言：remember 调用落库、自动提取触发且去抖、注入块出现在 messages_for_api 的 index=1、memory-updated 事件发出；
- 并发：同一项目两个 topic 同时运行 Agent，确保 per-project 锁串行化提取、无连接争用；
- 迁移：旧 knowledge.json 数据升级后 recall 仍可命中。

### 8.3 前端与质量门禁

- check:i18n 双语 key 成对；check:docs 链接有效；prettier / eslint / tsc / vite build 全绿；
- 手工验收流：设置页测试连接、面板编辑/删除/钉住、权限覆盖、降级提示；
- 性能探针：注入耗时（每轮 50ms 内为佳，其中检索 10ms 内）、提取任务后台不阻塞交互、内存占用（LRU 连接上限）。

---

## 9. 风险与缓解

| 风险                              | 影响                              | 缓解                                                                                             |
| --------------------------------- | --------------------------------- | ------------------------------------------------------------------------------------------------ |
| sqlite-vec 三平台打包/签名困难    | P3 延期或功能不可达               | P0 先验证；存储 trait 隔离，暴力余弦兜底；必要时以 Rust 暴力余弦为正式实现、vec0 仅加速          |
| Embedding 服务不可用（离线/未配） | 检索退化为关键词                  | 事实照常入库；UI 明确状态提示；一键 reindex 补嵌；P4 引入 ONNX 离线嵌入                          |
| 自动提取产生垃圾事实              | 记忆库污染、误导 Agent            | importance 阈值 + 类别白名单 + LLM 粗筛 + 用户面板治理 + 容量上限；提取结果可见可删              |
| 注入 token 开销                   | 上下文成本上升、prompt cache 失效 | 预算硬上限（3000） + 内容 hash 变更才替换 + 稳定前缀位置；自动注入可开关                         |
| 并发写/提取竞态                   | 事实丢失或重复                    | per-project 异步锁 + 串行提取 + upsert 幂等 + 版本链防丢                                         |
| 仲裁 LLM 调用成本                 | 每周期隐性费用                    | 仅在近邻高分或同 key 时仲裁 + 去抖 + 后台执行 + 失败 fail-safe                                   |
| 旧行为破坏（knowledge 兼容）      | 用户升级不适                      | 双写兼容期 + 迁移标记 + 可关闭；recall 返回结构向后兼容字段                                      |
| 数据隐私                          | 项目事实外泄                      | 全部本地存储；嵌入走本地 Ollama 或用户自配 API（key 入 secure_store）；无遥测                    |
| 本地嵌入模型体积/RAM              | 首拉几千 KB 到 1 GB+，常驻内存    | 默认 bge-m3 可换 nomic/MiniLM；Ollama 空闲自动卸载；P4 引入 ONNX 按需加载；UI 展示下载与内存状态 |
| 项目目录被误提交 git              | 记忆库进入版本库                  | 文档建议 .gitignore 忽略 .aio/memory/；项目无 .gitignore 时 AIO 建库时自动追加                   |

---

## 10. 文档与 i18n 同步

- docs/zh|en/usage/：新增「项目记忆与检索」使用说明（开关、配置、面板操作、常见问题）；
- docs/zh|en/reference/configuration.md：补充 memory_* 配置项说明；
- docs/zh|en/development/architecture.md：新增 services/memory 与 plugins/embed 模块说明；
- docs/zh|en/development/extensions.md：新增「新增 Embedder / 接入新向量后端」指南；
- CHANGELOG：按版本记录；i18n 两个 locale 同步新增文案。

---

## 11. 实施验收清单（总）

- [ ] P0 报告评审通过（sqlite-vec 可落地性结论 + 嵌入实测数据）；
- [ ] 事实库每项目隔离存储，schema 幂等，迁移无损；
- [ ] 写入三通道（显式工具 / 自动提取 / 迁移）与更新四动作（新增/更新/合并/淘汰）均有测试覆盖；
- [ ] 混合检索 + 预算注入在真实模型下命中率达标（P3 评测定义基线）；
- [ ] 权限、i18n、文档、verify 四项门禁全绿；
- [ ] 三平台冒烟通过，无窗口期明示降级路径。

---

## 12. 参考资料

- Letta（MemGPT）上下文工程与记忆块：https://docs.letta.com/guides/agents/context-engineering
- Letta sleep-time compute 论文：https://arxiv.org/abs/2504.13171
- Mem0 架构（ADD/SEARCH/UPDATE + 混合记忆）：https://github.com/mem0ai/mem0/blob/main/skills/mem0/references/architecture.md
- Zep / Graphiti 时序知识图谱：https://help.getzep.com/graphiti/getting-started/overview；解读：https://github.com/lhl/agentic-memory/blob/main/references/rasmussen-zep.md
- LangMem（记忆管理 agent 与策略）：https://langchain-ai.github.io/langmem/
- Claude Code Memory（CLAUDE.md 与自动记忆）：https://code.claude.com/docs/zh-CN/memory
- sqlite-vec（针对 SQLite 的向量插件，vec0）：https://github.com/asg017/sqlite-vec
- VectorLite（Rust 原生 + ONNX 本地嵌入）：https://github.com/mmailhos/vectorlite
- Ollama 嵌入模型目录：https://ollama.com/search?c=embedding；nomic-embed-text-v2：https://ollama.com/library/nomic-embed-text-v2-moe

---

## 附：已确认决策（评审结论）

1. 总开关默认**关闭**；开关放「更多 → 项目设置」弹窗，位于模型选择（绑定模型）区块下方、MCP 与 Skill 管理上方，即 ProjectSettingsModal.tsx 模型区块之后新增「项目记忆」区块。
2. 记忆写工具（remember / update_memory / forget_memory）默认 Allow（Normal / Auto / Workflow；Plan 模式 Deny），用户可经 .aio/permissions.json 覆盖。
3. 新建 src-tauri/src/services/ 目录承载 services/memory（含 mod.rs 声明）。
4. 记忆注入预算暂定 3000 tokens，P1 实测后校准。
5. 本地嵌入模型**不打包进安装包**：启用项目记忆并选择本地嵌入时才按需下载（Ollama pull 或 P4 的 ONNX 缓存到 app data），UI 展示模型体积与下载进度、可选轻量模型；是否内置极轻量兜底模型（如 ~46 MB 的 MiniLM）留待 P4 评审。

（完）

# AIO SolidJS 前端性能 + 可维护性 + 组件复用专项审查报告

审查范围:`src/`（features/chat、features/harness、core/store、core/api、features/settings 抽查、shared 抽查），关键结论均以 `文件:行号` + 代码摘录给出，全部经过逐行核对；后端 `src-tauri` 仅在需要验证事件粒度时交叉引用。审查基准：AGENTS.md（Tailwind-only、i18n t() 双语对齐、禁 emoji、shared 组件归 `src/shared/`、store 域拆分、组件禁裸 invoke）。

---

## 总评

代码库整体工程素养高：18+7+4 个全局流监听器统一挂在根 Layout（生命周期即应用生命周期，不泄漏）；chat 主流做了 80ms 批处理 + `produce` 原地改写以避免行重挂载；消息列表与 AgentProcessBlock 均有尾窗分页（IntersectionObserver + scroll anchoring，清理正确）；`core/api/*` 类型化完整、零裸 invoke、零 ts-ignore；`check-gates` 棘轮机制在管住颜色/any/文件规模。

主要问题集中在三类：**①子代理流与 harness 部分事件绕过了批处理管线**（唯一真正的高频性能缺陷）；**②`saveSingleAssistantToBackend` 全量克隆 + 每个 tool result 全量 IPC**（大会话下的放大器）；**③harness 对 chat 的"复制粘贴式复用"**（输入区/侧栏/菜单/任务条共 6 组逐字节重复）。另有若干 i18n 漏网、模块级无界 Set、右栏拖拽逐像素写 localStorage 等中等问题。

---

## CRITICAL

未发现会直接导致功能损坏或必然崩溃的问题。以下 HIGH-1 是最接近 Critical 的高频性能缺陷。

## HIGH

### H-1 子代理流（subagent-chunk/subagent-reasoning）逐 token 写 Store，完全未批处理

- 位置:`src/features/chat/hooks/chatEventListeners.ts:802-807`

```ts
listen<SubagentDeltaPayload>(EVENTS.subagentChunk, (e) =>
  handleSubagentDelta('content')(e.payload),
),
listen<SubagentDeltaPayload>(EVENTS.subagentReasoning, (e) =>
  handleSubagentDelta('thinking')(e.payload),
),
```

- 每个 delta 走 `handleSubagentDelta`(235-267) → `mutateSubagentSteps`(203-230)，即**每个 token 一次 `setDatas(...produce(...))` + `steps.slice()` 整数组复制**：

```ts
steps[subIdx] = { ...subStep, subagentSteps: mutate(subStep.subagentSteps || []) };  // :224-227
const next = steps.slice(); next[i] = { ...next[i], summary: ... };                  // :250-251
```

- 后端粒度已核实为逐 SSE token（`src-tauri/src/commands/llm/streaming.rs:302-316`，`delta.content` 每次几个字符即 emit `SUBAGENT_CHUNK`）。主流 `llm-chunk` 有 80ms `pushToBatch`（chatEventListeners.ts:68-194），子代理流没有。快供应商 50-200 token/s，`maxConcurrentSubagents` 并发子代理时是 3× 该频率的 Store 写 + O(steps) 数组拷贝，直接打满主线程，大 steps 场景（`subagentSteps` 可上千）还会 GC 抖动。
- **修复**：把 `pushToBatch`/`flushStreamBatch` 泛化成 `(key) -> batch` 的通用机制，子代理 delta 按 `(parentAssistantId, parentTopicId, subagentId, round, kind)` 键控入批，80ms flush 时一次性 `mutateSubagentSteps`；或在 `handleSubagentDelta` 内对 running step 做文本追加而非 `slice()` 重建。

### H-2 `saveSingleAssistantToBackend` 每个 tool result 全量 `JSON.parse(JSON.stringify())` 克隆整个 Assistant

- 位置:`src/core/store/chat.ts:319-329`

```ts
export const saveSingleAssistantToBackend = async (id: string) => {
  const asst = datas.assistants.find((a) => a.id === id);
  if (!asst) return;
  try {
    await saveAssistantApi(JSON.parse(JSON.stringify(asst)));   // 全量深克隆
```

- 调用频率（均已核实）：`llmToolResult` **每个工具结果一次**（chatEventListeners.ts:650）、每 round-start（:313）、done（:471）、压缩（:685）。一个 Agent 轮次几十个 tool result 时，每次都把**该 Assistant 全部 topics × 全部 messages（含 toolCalls.fullContent / agentSteps.fullResult 大对象）**克隆两遍再经 IPC 序列化。AgentProcessBlock 注释自证单轮 agentSteps 可达 5MB 级，大会话下这是最大的重复开销源。
- **修复**：对齐 harness 的粒度模型（`harness_append_message` 单消息追加）——后端增加 `append_message` 级别的增量保存，前端 tool-result 只传 `(topicId, message)`；或至少把 tool-result 路径的保存做 1-2s 防抖合并。

### H-3 右栏拖拽期间每次 mousemove 同步写 localStorage

- 位置链：`ChatPage.tsx:689-691`

```ts
createEffect(() => {
  savePanelWidth('right', rightPanelWidth());
});
```

配合 `usePanelResize.ts:178-182`（`startResizeRight` → `attachResizeDrag`，每个 mousemove 调 `setRightPanelWidth`）→ **每像素一次 `localStorage.setItem`**（同步 I/O，60-240 次/秒）。左栏没有此问题（拖拽期间只更新 `leftDragWidth`，释放时才 `setLeftPanelWidth`，usePanelResize.ts:164）。`HarnessPage.tsx:107-110` 同样把 `savePanelWidth` 直接塞进 setter，问题相同。

- **修复**：右栏照抄左栏的"预览信号 + 释放提交"模式；或 `savePanelWidth` 在 effect 内加 200ms 防抖。

---

## MEDIUM

### M-1 i18n 违规：store 层硬编码中文 `'默认话题'`（落库数据，切英文也显示中文）

- `src/core/store/chat.ts:552`（`ensureProjectAssistant`）：

```ts
topics: [{ id: Date.now().toString(), name: '默认话题', history: [], summary: '' }],
```

- 该话题名持久化并显示于 TopicSidebar，绕过了 `t('chat.defaultTopic')`（ChatPage.tsx:126 是正确用法）。修复：与 store 解耦——由调用方传入 `t('chat.defaultTopic')`，或在 store 导出 `initI18nReady` 后惰性取词。

### M-2 i18n 违规：ChatInterface 内硬编码中文命令 label/description，经 `/help` 原样输出

- `ChatInterface.tsx:216-228 / 230-255`：

```ts
const cmdFocusInput = registerCommand({
  id: 'focus-input',
  label: '聚焦输入框',
  description: '将光标聚焦到消息输入框',
```

- CommandPalette 有 `COMMAND_TRANSLATIONS` 兜底（core/shortcuts.ts:151-157），但 `chatCommands.ts:349` 的 `/help` 直接拼 `c.label`（未走 `getCommandDisplayLabel`），中文原样进入消息内容。修复：注册处用 `t()`（shortcuts.ts:517-518 已有正确范例），同时 `/help` 改用 `getCommandDisplayLabel`。

### M-3 模块级无界缓存泄漏（AgentProcessBlock）

- `AgentProcessBlock.tsx:72` `const animatedStepIds = new Set<string>();` —— **全应用会话期只增不减**，每个渲染过的 step id（含 `crypto.randomUUID()`）永久保留；对照 ChatInterface.tsx:66-68 的 `animatedMessageIds` 有 topic 切换清理（:278-288），此处缺失。
- `AgentProcessBlock.tsx:87` `const perRoundQueue = new Map<string, RoundRenderQueue>();` —— 仅在"inactive 转换"路径清理（:658 `perRoundQueue.delete(roundKey)`）；**流式进行中切换话题/页面时该 round 的 queue 连同 `dequeueTimer` 永久滞留**（注释明确说明 remount 时 onCleanup 不能清它，但没有任何兜底全局清理）。
- **修复**：把 `animatedStepIds` 的清理挂到 ChatInterface 的 topic 切换 effect（与 `animatedMessageIds` 同点）；`perRoundQueue` 增加按 `startTime` 的过期清扫（如 > 1h 回收）。

### M-4 TokenStatsBar 的 stats memo 全量扫描 history，且被每个 tool 事件触发

- `TokenStatsBar.tsx:69-76`：

```ts
for (const msg of history) {
  if (msg.role === 'assistant') {
    input += msg.inputTokens || 0;
    ...
    tools += msg.toolCalls?.length || 0;
```

- memo 跟踪了每条消息的 `toolCalls?.length`，`llm-tool-call` 监听器每次 `msg.toolCalls = newToolCalls`（chatEventListeners.ts:565）都会使 memo 失效 → **每个工具事件 O(history) 重扫 + 价格重算**。1000+ 消息 × 每秒多次 tool 事件时可感知。
- **修复**：在 store 侧维护累计信号（tool 结果落账时增量更新），或 memo 里只扫描 `contextTrimCount` 之后 + 缓存上次结果做增量差分。

### M-5 harness 事件部分未批处理 + `pushContentStep` 逐帧追加 step 无聚合

- `harnessEventListeners.ts`：text/thinking delta 有 80ms 批（:147-157），但 `tool_call_start/update/end`（:252-301）、`subagent_delta`（:305-308）、`notice`（:315-317）每事件一次 `upsertDraft` → **每次整 `harnessDrafts` Record 重建 + `agentSteps` 数组复制**（:59-71, 84-96）。
- `pushContentStep`（:119-130）对每个 `subagent_delta` 帧**新增一个 step** 而非聚合到 running step（对照 chat 的 `appendTimelineStep`，chatEventListeners.ts:82-107）。后端 Claude 适配器按 assistant 帧（非 token）发 `SubagentDelta`（claude.rs:759-807），所以不是 token 级失控，但一个长 subagent 数十帧 = 数十个 step 对象 + 每帧全数组重建。
- **修复**：`upsertDraft` 内按 id 聚合 running content step（复用 chat 语义）；tool_call_update 高频流（Bash 逐行输出）可复用同一 80ms 批队列。

### M-6 `any` 使用集中在消息构建与 setDatas 过滤器（全 src 共 75 处）

热点（已核实）：

- `features/chat/utils/apiMessages.ts:43-130` —— 核心消息构建 util 12 处 any：`buildApiMessages(m: any): any[]`、`ensureToolMessagesComplete(msgs: any[])`。
- `MessageBubble.tsx:57` `msg: any;`（整个消息行组件的 props 无类型）+ :26,220,494,509,568,583,636 过滤器 any。
- `setDatas` 过滤器惯用 `(a: any) => a.id === ...` 约 20 处（ChatPage.tsx:334-338、chatCommands.ts:186-357、chat.ts:364-373、engine.ts:141 等）。
- 其余属后端 JSON 边界的合理豁免（core/types/mcp.ts、stream.ts:22 自带预算注释）。
- **修复**：在 `core/store/types.ts` 定义 `setDatas` 谓词类型（Solid 的函数路径签名支持 `(a: Assistant) => boolean`，无需 any）；`apiMessages.ts` 直接用已有的 `Message`/`ToolCallDisplay` 接口。

### M-7 chat ↔ harness 输入区/状态栏复制粘贴（逐字节相同，且复制不全）

已核实的四组重复（class 字符串完全一致）：

1. 输入容器：`HarnessChatInterface.tsx:264` ≡ `ChatInterface.tsx:809`（`rounded-xl box-border flex flex-col gap-[10px] mt-[3px] p-[10px] transition-all duration-200` + `background: rgba(0,0,0,0.25)`）。
2. textarea：`HarnessChatInterface.tsx:268` ≡ `ChatInterface.tsx:814`（整串 class 逐字相同）。
3. 发送/停止复合按钮：`HarnessChatInterface.tsx:290-303` ≡ `ChatInterface.tsx:920-933`。
4. 状态栏 frame：`HarnessChatInterface.tsx:169` ≡ `ChatInterface.tsx:604`（+ 相同 surface-bg/border 样式）。

- 且 harness **缺了** ChatInterface.tsx:187-200 的 textarea 自动增高 effect——复制导致的功能缺口。
- **修复**：提取 `shared/components/ChatComposer.tsx`（输入容器 + textarea + 自动增高 + SendStopButton + SlashCommandMenu 插槽），两页各自只传配置。

### M-8 侧栏基础设施四重复制：ResizeHandle ×4、Acrylic 右键菜单 ×3

- 拖拽胶囊柄（`w-[10px] h-12 rounded-[20px] backdrop-blur-md ...`）4 份：`ProjectSidebar.tsx:504`、`TopicSidebar.tsx:251`、`HarnessTopicSidebar.tsx:103`、`HarnessToolSidebar.tsx:480`。
- 亚克力右键菜单（`contextMenuIn/Out` 动画 + `--acrylic-bg`）3 份：`ProjectSidebar.tsx:442`、`TopicSidebar.tsx:541`、`HarnessTopicSidebar.tsx:219`；配套 `closeMenu/menuAnimatingOut/menuDivShown` 状态机也各有拷贝（ProjectSidebar.tsx:106、HarnessTopicSidebar.tsx:44）。
- **修复**：`shared/components/PanelResizeHandle.tsx` + `shared/components/AcrylicContextMenu.tsx`（后者封装 openMenu/closeMenu/Portal/动画状态机，调用方只给菜单项渲染函数）。

### M-9 硬编码 rgba 主题色（rgba 棘轮的存量债，建议一次清理）

rgba literal 棘轮（check-gates.mjs:88-112）允许存量，但下列语义色均有现成 CSS 变量可替换，属高性价比清偿：

- `MessageBubble.tsx:612-615` 删除按钮 `rgba(248,113,113,0.1/0.35/0.75)`（`--color-danger` 已存在）；`:749-751` `rgba(80, 220, 100, 0.7/0.08/0.15)`（success 语义）。
- `AgentProcessBlock.tsx:52-56, 105-107` STEP_CONFIG/状态色 `rgba(240,160,64,*)`、`rgba(224,85,85,*)`、`rgba(76,175,144,*)`、`rgba(156,163,175,*)`、`rgba(140,120,220,*)` —— 建议在 `index.css` 定义 `--step-tool-running / --step-error / --step-ok / --step-subagent` 等语义 token。
- `CommandPalette.tsx:55-58` CATEGORY_COLORS 三色；`core/store/chat.ts:435-443` 运行时注入的 `rgba(240,240,240,0.3)` 暗色边框修正。

### M-10 HarnessPlanStrip 与 TaskBoardPanel 结构性重复

- `HarnessPlanStrip.tsx:28-104` ≡ `TaskBoardPanel.tsx:32-141`：同折叠机制（grid-rows 0fr/1fr）、同摘要行（连 i18n key 都共用 `agent.tasks.statusLine`，HarnessPlanStrip.tsx:48）、同图标/圆点语义。仅行渲染器与数据源不同。
- **修复**：`shared/components/CollapsibleTaskList.tsx`（props: entries/summaryLine/rowRenderer）。

### M-11 >500 行组件清单与拆分建议（features/.tsx 棘轮 2060，AGENTS 目标 <900，当前棘轮已显著松动未收紧）

| 文件                                     | 行数 | 拆分建议                                                            |
| ---------------------------------------- | ---- | ------------------------------------------------------------------- |
| settings/components/LocalModelMarket.tsx | 1147 | 搜索栏/结果卡/下载任务行/硬件档案 4 块独立组件                      |
| settings/ProviderDetailPage.tsx          | 1133 | 模型分组列表、连接测试块、5s 存活探针各自成组件                     |
| chat/components/ChatInterface.tsx        | 1001 | 消息窗口（+分页哨兵）、AgentStatusBar、ChatComposer 三块下沉 shared |
| chat/hooks/chatEventListeners.ts         | 991  | 按域拆文件（llm-_/subagent-_/approval-*），注册函数聚合             |
| settings/components/AppSettings.tsx      | 948  | 各设置分区（语言/主题/记忆/快捷键）独立组件                         |
| chat/components/MemoryPanel.tsx          | 884  | 列表区+详情区拆分                                                   |
| settings/components/SkillList.tsx        | 831  | 市场浏览/本地列表两个子组件                                         |
| chat/ChatPage.tsx                        | 797  | 尚可，但 handleSendMessage 90 行可再下沉 utils                      |
| chat/components/MessageBubble.tsx        | 783  | 动作按钮区(386-658)、文件变更区(663-792) 提取                       |
| chat/components/AgentProcessBlock.tsx    | 751  | StepCard 已内聚，可拆出 render-queue 机制为 hooks                   |

（core/.ts 上限 900 未超；`core/shortcuts.ts` 725、`core/store/chat.ts` 517 合规。）

### M-12 消息尾窗分页在"追加"方向缺锚定（windowing 正确性边界）

- `ChatInterface.tsx:325-327`、`HarnessMessageList.tsx:65`：`windowStart = max(0, len - visibleCount)`。流式期间新消息 append 使 `len` 增长 → **窗口整体前移**：用户上翻阅读旧消息时（followPaused），新一轮 round-start 追加占位消息会让窗口顶部旧消息卸载、底部挂载新消息，视口上方内容高度突变（loadEarlier 的 prepend 方向有 rAF 锚定 :344-347，append 方向没有）。
- **修复**：append 时若 `autoScroll === false` 则同步 `visibleCount += 增量` 保持 windowStart 不变，或对 append 方向做同款 scroll anchoring。

---

## LOW

- **L-1 布局持久化键跨页共享**：`layout.ts:10-17` 的 `chat-left-panel-width / chat-right-panel-width` 被 HarnessPage.tsx:75-78 直接复用 → 拖 harness 侧栏会改 chat 侧栏宽度（键名带 chat 前缀但实际是全局宽度）。若有意共享，应改名为中性键；若无意，是隐式耦合。
- **L-2 ui-prefs 读取无守卫 + 枚举未校验**：`ui-prefs.ts:6-11,17-18,26-27` 模块顶层裸 `localStorage.getItem`（对照 layout.ts:20-34 / harness.ts:78-86 明确做了 try/catch——禁用 storage 环境下该模块加载即抛）；`localStorage.getItem(REASONING_KEY) as ReasoningLevel` 无白名单校验（对照同文件 :54-62 `loadDownloadSourcePref` 的正确做法），脏值会原样传给后端。
- **L-3 engine 健康轮询无并发护栏**：`engine.ts:101-124` `setInterval(async () => ...)` 500ms 轮询，双模型连点会并发两个 poll 同写 `localEngineConnected`；async 回调可重叠（probe >500ms 时）。建议进入时置 busy 标志/清旧 interval。
- **L-4 死代码**（全树无引用，已 grep 验证）：`core/api/chat.ts:175-176 stopLlmStream`；`core/store/chat.ts:77 pendingToolCall/setPendingToolCall`（注释还停留在"listened to by ChatPage"的旧架构）；`AgentProcessBlock.tsx:533` 空 `onCleanup(() => {});`。
- **L-5 全局监听注册无防重入**：`Layout.tsx:87/92/97` 丢弃三个 register 函数返回的 unlisten promises（生命周期设计上可接受，注释自洽），但三个函数都没有 `chat.ts:84-87 mcpListenersInstalled` 那样的守卫——dev HMR 会重复注册 → 消息翻倍。低危（仅开发态），建议补 flag。
- **L-6 HarnessPage 连接防抖计时器无卸载清理**：`HarnessPage.tsx:206-233` `connectTimer` 只在下次 effect 清，无 `onCleanup`——切页后 300ms 内仍会触发一次 `harnessConnectTopic`（幂等，故仅卫生问题）。
- **L-7 符号字符当图标**：`↗/↘` 出现在 TokenStatsBar.tsx:180,188、MessageBubble.tsx:366,374、UsageSummaryCards.tsx:94-95、ModelBreakdown.tsx:128-129；`◔` 在 HarnessChatInterface.tsx:253。AGENTS.md 图标规范要求走 `<Icon/>`（因不"独占元素内容"而绕过了门禁）。建议 Icon.tsx PATHS 增加 arrow-up-right/arrow-down-right/context。
- **L-8 数字格式化三份拷贝**：`fmt()` 在 TokenStatsBar.tsx:20-24、UsageSummaryCards.tsx:17、ModelBreakdown.tsx:12 各一份（core/utils/format.ts 已有 formatSize；i18n 已有 formatNumber）→ 统一进 `core/utils/format.ts`。
- **L-9 MessageBubble 动作按钮 5 连重复**：:389-426/:429-450/:460-533/:534-607/:611-657 同款"展开标签按钮"结构（class 串逐字重复 4 次，且 copy/branch 用 Icon、resend/edit/delete 用内联 SVG path 绕开 Icon 注册表）；resend 与 edit 的删除重建逻辑 :466-514 vs :540-588 约 95% 相同 → 提取 `ActionButton` + `deleteForResendReply` helper。
- **L-10 大会话下每事件 O(n) 扫描（可接受但需压测关注）**：`pushToBatch` 每块 find assistant+topic（chatEventListeners.ts:161-162）、`flushStreamBatch` findIndex（:116-118）、`llmToolResult` 逆序扫 history 至命中（:601-644）、`SubagentDone/Error` 全 history 扫（:924-984，有意为之）、ChatInterface `messageEls` 剪枝 effect（:169-179，append 时 O(n)）。assistants/topics 数量小，风险主要在 history 千级场景。
- **L-11 diagnostics 全量重算**：`diagnostics.ts:145-158` 每次 LSP publish 全文件×全诊断重算计数（Agent 批量写文件时突发多次）；可改增量或防抖。

## INFO

- chatTurnEffects.ts:388 `/\[(?:历史归档|Archive) #id\]/` 正则耦合双语持久化格式——现设计已兼容两语种并幂等合并，属有意的持久化格式，非缺陷；注意未来再加语言需同步正则。
- engine.ts:62 `confirm(...)` 与 :94 `window.confirm(...)` 混用；错误路径大量 `alert(err)`/`alert(t('error.save'))` 混用（ChatPage.tsx:411,520 等）——建议统一走 shared Toast。
- 中文 console 日志（chatTurnEffects.ts:337-339、ChatPage.tsx:716-720、chat.ts:107）——门禁只查 JSX，日志是灰区，建议统一英文。
- ChatInterface.tsx:591-606 在 JSX 里用 IIFE 计算 `mode`——建议提为 `createMemo`（当前依赖被 memo 捕获可以工作，但脆弱且难读）。
- HarnessTopicSidebar 等的 `hover` 用 JS 写 style（:133-135）而 chat 侧同功能已改 classList 方案（ChatInterface.tsx:652-658 注释明说不做 JS 样式写入）——复制时未同步该修复。
- 设置页整体质量好：ProviderDetailPage 的 5s 探针有 `onCleanup` 清理（:464-475）、FE-4 diff-check 防抖持久化；重复模式（debounced persist + form signal 组）可抽 `useDebouncedPersist`，收益有限。
- 无原生 `<select>`/`<input type=checkbox>`、无 ts-ignore、无 emoji 字符（✓/✗ 均在字符串上下文，合规）、`core/api/*` 全部薄封装且文档齐全——分层门禁实效良好。

---

## 复用性评估（专项）

**已正确复用（无需动）**：HarnessMessageList 复用 chat 叶子组件（Markdown/ThinkBlock/AgentProcessBlock，HarnessMessageList.tsx:21-23）；harness 复用 chat 的 `usePanelResize`（跨 feature import，见下）；全局规范件（Dropdown/Switch/Checkbox/buttonStyles/DiffView/Icon）两侧一致；`core/api/harness.ts` 是 API 层范本。

**应统一（按收益排序）**：

1. **ChatComposer**（M-7）：ChatInterface:808-936 ≡ HarnessChatInterface:263-306。chat 侧顺带补齐 harness 缺失的 textarea 自动增高。落点 `src/shared/components/`。
2. **PanelResizeHandle + AcrylicContextMenu**（M-8）：4×/3× 拷贝是侧栏改动时的最大漂移源（已有实证：hover 方案漂移，见 INFO）。
3. **CollapsibleTaskList**（M-10）：TaskBoardPanel/HarnessPlanStrip 合并。
4. **ApprovalCard 外壳**：ToolApprovalBubble.tsx:108 ≡ HarnessApprovalBubble.tsx:76 容器样式逐字同（`rounded-[10px] px-3.5 py-3 ... shadow-[0_4px_20px_rgba(0,0,0,0.3)]`），内部选项区保持各自主渲染。
5. **fmt 数字格式化**（L-8）→ `core/utils/format.ts`。
6. **ActionButton**（L-9）→ `src/shared/components/`，并把 resend/edit 的重复删除逻辑收敛。
7. **跨 feature import**：`HarnessPage.tsx:19` `import { usePanelResize } from '../chat/hooks/usePanelResize'`——按 AGENTS.md 分层应下沉 `src/shared/hooks/`（同时解决 M-12 的布局键语义问题）。

---

## 优先行动清单

1. 子代理 delta 批处理（H-1）——半天级改动，流式子代理场景立竿见影。
2. 工具结果增量持久化（H-2）——需后端配合 `append_message` 粒度。
3. 右栏拖拽持久化防抖（H-3）——10 行改动。
4. `'默认话题'` + ChatInterface 命令 label i18n 修复（M-1/M-2）——`check:i18n` 外的漏网点，顺手修。
5. `animatedStepIds`/`perRoundQueue` 泄漏清理（M-3）。
6. ChatComposer / PanelResizeHandle / AcrylicContextMenu 三件套提取（M-7/M-8）。
7. apiMessages.ts 与 setDatas 过滤器去 any（M-6）。
8. 尾窗 append 锚定（M-12）与 rgba 存量清偿（M-9）。

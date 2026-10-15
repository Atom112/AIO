<div id="top"></div>

# Changelog

## [Unreleased]

### 🪜 第十二轮：模式阶梯改名 + 完全权限内置化 + 工作流功能移除（PR #131）

- **Agent 模式阶梯改名（wire 值稳定，assistants DB 无迁移）**：`off`=「只读」、`normal`=「工作区内修改」、`auto`=「完全权限」、`plan`=「计划」不变——
  - **只读（off + 项目）成为真实智能体档位**：工具集为 think + project_map + web + 并行安全内建集（文件/git/LSP 读取按 `is_parallel_safe` 过滤），权限 Off 臂改为 10 条 allow（priority 90）+ `builtin-readonly-deny-other` Deny `*`（priority 10）——读取放行、其余一律拒绝；无项目时保持纯对话（`isChatMode` 改为项目感知，slash 命令与检查点续跑的 skills 门控同步修复）；`sendMessagePrep` 让 off+项目也注入模式系统提示（只读契约提示：可读不可改、需求改动时建议切换档位）
  - **工作区内修改（normal）**：`write_file`/`replace_in_file`/`mkdir` 直接允许（沙箱仍限制在项目根内）；`delete_file`/`execute_command`/`git_add`/`git_commit`/其他 MCP 仍逐次确认
  - **完全权限（auto）自主权内置化**：不再依赖独立开关——沙箱不可用 fail-closed 仅作用于 Auto、网络命令闸（SEC-01）在 Auto 一律放行（进入模式时风险确认文案吸收原「完全放手」的外泄权衡）、沙箱拒绝自动扩权仅按 Auto 判定；`fullAutoMode` 配置字段端到端删除（Rust 结构体/磁盘归一化/patch 命令/TS 类型/API/store/设置页开关/模式胶囊脉冲红点）
  - 模式选择器重排：只读（眼睛）/工作区内修改（扳手）/完全权限（闪电）/计划（文档），风险确认仅在进入完全权限时弹出（会话内一次）
- **工作流功能端到端移除**：`create_workflow` 工具与分发臂、`execute_workflow` 引擎、`ToolCallStats` 失败闸、`WORKFLOW_*` 事件（双端注册表同步删）、`WorkflowVisualization` 组件与 4 个监听器、`workflowState` store（任务面板状态独立为 `core/store/task-board.ts`——create_tasks 任务清单功能保留）、`workflowStepMode` 配置、相关 i18n keys 与 docs 章节；`delegate_ctx` 不再携带 stats
- **权限引擎语义精化（Off/Plan 档位的排序正确性）**：用户/会话 Deny 仍全局优先；内置模式兜底改为按优先级参与——Off 的 allow 阶梯（90）高于其 deny 兜底（10）、Plan 的硬边界 Deny 提到 100（高于会话 always-allow 50 与自定义默认 10，会话授权不再能打开 Plan 的命令执行），并修复 `test_plan_mode_execute_command_denied` 缺失的会话互斥锁；SEC-02 回归测试改用 `delete_file` 探针（Normal 的 write 已 Allow）
- **设置页两处 UI 收敛**：① 语言选择器改为复用共享 `Dropdown` 新增的 `ghost` 变体（透明底 + border-dim + 小箭头，与原手写样式一致），主代理工具集下拉同款——三种下拉一种组件；② 供应商列表自定义供应商区块移到目录之前（不再滚一大圈）
- **旋钮跨 0 方向感知**：最大工具调用轮数 / 最大并发子智能体在 0 吸附时按调节方向越过——从 -1 往上调到 1、从 1 往下调（或直接输入 0）到 -1
- 文档同步（chat-and-agent 模式表、permissions、agent-tools、architecture、app-settings 快捷键循环描述）；`cargo +1.97.1 test` 357 通过、clippy/fmt 绿；`npm run check:frontend` 全绿（i18n 1016 keys、rgba 棘轮 171）
- **模式菜单去描述小字 + 自定义供应商分区重组（PR #133，用户反馈二连）**：① 模式选择菜单每行只留图标 + 名称（介绍性小字移除，说明保留为收起态胶囊的悬停 tooltip；菜单 260→190px、行高收紧单行）；② 供应商列表分区重组——已启用自定义供应商独立小节（标题「已启用（N）」）置于目录之前，未启用自定义供应商单独成组（新标题「自定义（未启用）」）插在「本地推理引擎」与目录「未启用」之间，两份重复的行 JSX 收敛为 `CustomProviderRow` 组件（props 回调，AGENTS.md 禁重复）
- **分组接缝修复（PR #135）**：已启用自定义区块与下方无标题的目录已启用列表之间此前 0 间距粘连（其他分组间都有 mt-5 标题间距，唯独这个接缝没有任何 margin）——已启用自定义行容器补 `mb-5`，分组节奏与其他分区一致；「未启用」divider 条件同步放宽（只要有未启用目录行即显示，上方恒有本地引擎/未启用自定义区块），消除同类粘连
- **推理强度真参数化 + GPT 式滑杆（思考强度开关从提示词升级为 API 参数）**：此前的推理档位只注入系统提示词（对推理模型的原生思考预算无实际控制力）——现档位作为真实请求参数下发：
  - **支持检测**：模型目录 `capabilities.reasoning` 标记为权威来源，目录缺失的自定义模型退回保守 id 启发式（o1/o3/o4/gpt-5/gemini-2.5/grok 等）；不支持的模型维持原提示词注入路径
  - **参数映射**：OpenAI 兼容端点（含 Gemini OpenAI 兼容层、OpenRouter）发送 `reasoning_effort: low|medium|high`；Anthropic 端点由 `to_anthropic_body` 换算为 `thinking` 思考预算（低 2048 / 中 8192 / 高 16384 tokens，`max_tokens` 不足时自动抬升，thinking 激活时丢弃 temperature 满足 API 约束）；参数仅在主对话生效（子智能体不继承）
  - **提示词去重**：原生参数生效的回合不再注入推理提示词——参数即权威，对不外露思维链的模型（o 系 / gpt-5）要求可见思考标签只会污染可见答案
  - **UI 改版**：ReasoningButton 弹层改为 GPT 式滑杆——水平 4 档（关闭/低/中/高）、拖拽/点击吸附最近档位、方向键步进（role=slider）、选中档位高亮；触发按钮高亮色统一为主色（移除 3 处硬编码 rgba，棘轮 169→166）；i18n 收敛 5 个不再使用的描述 keys（1017→1012）
  - 测试：cargo +3（`reasoning_effort_lands_in_body` / `reasoning_effort_maps_to_thinking_budget` 含预算抬升与未知值不注入分支 / FE nudge 抑制断言）；文档 chat-and-agent zh/en 同步
- **推理滑杆视觉升级（每档一色，PR #138）**：低=薄荷绿 / 中=琥珀 / 高=紫罗兰（关闭保持中性灰），档位色经 `@theme` CSS 变量下发（`--color-reasoning-low/medium/high`）——轨道填充变为档位色 `color-mix` 渐变（透明头→全色尾）并叠加前景色星点闪烁层（`reasoning-twinkle` 动画）；档位刻度点亮至当前档位；拇指获得档位色光晕、拖拽时放大；选中档位标签与触发按钮读数同色（rgba 棘轮 166→167，新色全部走变量、hex 门禁 color=0 保持）
- **滑杆与任务清单过渡动画（PR #139）**：① 推理滑杆档位切换加入过渡——拇指 `left/transform/box-shadow`、轨道填充 `width`、刻度点颜色均 200ms ease-out，点击/拖拽/方向键换档时滑动而非瞬跳；② 状态条任务清单展开收起改为 `grid-template-rows: 0fr↔1fr` 纯 CSS 高度动画（无需测量内容高度，行保持挂载）+ 内容淡入淡出；③ 展开收起箭头由完整箭头（arrow-left 旋转）改为实心三角 SVG（Icon 库新增 `triangle-down`，路径级 fill 覆盖 stroke 风格）
- **浅色模式弹层亚克力统一（PR #140）**：此前浅色模式下各弹层/弹窗与主面板质感割裂——主面板是 0.72 半透明亚克力（彩色辉光透底），而菜单/弹层是 0.88~0.96 近不透明深色卡片、弹窗是 `--panel-bg` 纯实色灰块或 `--color-dark-700` 主题无关深蓝块。现引入统一弹层 token：`--popup-bg`（菜单类，浅色 0.78 / 深色 0.92）+ `--popup-blur`（blur 24px saturate 150%）+ `--dialog-bg`（模态类，浅色 0.88 / 深色 0.95），15 处组件面统一换用——Dropdown/SlashCommandMenu/CommandPalette/Toast/ReasoningButton/AgentModeSelector（菜单+风险弹窗）/ModelSelector/SubagentMenu/ChatInterface（分支菜单+toast）/ProjectCreateModal/ProjectSettingsModal/MemoryPanel/ProjectMcpTrustModal/ShareModal；`--panel-bg` 死 token 删除；顺带修复 Dropdown 选项 `.selected` 类无样式定义的历史遗留（选中项补主色高亮 + hover 反馈）；SlashCommandMenu 与 ShareModal 两处硬编码颜色消除（rgba 棘轮 167→165）；新增 `scripts/gen-theme-preview.mjs` 主题预览生成器（真实编译 CSS 渲染关键弹层，浅色/深色并排，配合 headless Edge 截图做视觉回归）
- **弹层质感调优 + 弹窗复用主背景（PR #141）**：① 弹层透明度再降（`--popup-bg` 浅色 0.78→0.68、深色 0.92→0.88），模糊加倍（`--popup-blur` blur 24px saturate 150% → blur 40px saturate 160%），亚克力通透感更强；② 浅色模式弹窗背景从实色块改为**复用主页面亚克力彩色背景**——`--dialog-bg` 变为「表面色 0.62 渐变罩层 + `var(--app-bg)` 彩色辉光」复合背景，配合各弹窗 `background-attachment: fixed`（6 处：项目创建/项目设置/记忆面板/MCP 信任/分享导出/完全权限风险确认），弹窗显示的是它恰好悬浮其上的主背景切片，与主页面视觉无缝衔接；深色模式 `--dialog-bg` 同步降到 0.92 保持克制；③ 修复 ProjectCreateModal 用 `bg-[var(--dialog-bg)]` 类的隐患——复合背景是多层背景列表，`background-color` 不接受会整条失效，已改内联 style
- **弹层通透度再调 + 无模糊兜底（PR #142）**：`--popup-bg` 浅色 0.68→0.48、深色 0.88→0.82，弹层更接近"纯玻璃"质感；浅色弹窗罩层 tint 0.62→0.5，彩色辉光透入更明显；新增 `@supports not (backdrop-filter: blur(1px))` 优雅降级块——不支持 backdrop-filter 的极老 WebKitGTK 自动回退到 0.95/0.97 近实色弹层（纯透明+无模糊的弹层会不可读），不会发生灾难性降级
- **设置页下拉菜单统一 + 弹层模糊修复 + 主代理工具集设置移除（PR #143）**：① AppSettings 的手写「并行写入隔离」下拉（自定义 trigger + 0.92 近实色 inline 面板 + `&#9662;` 文本箭头）与语言选择器样式不一致——改为复用共享 `Dropdown`（ghost 变体），SubagentModelSettings 的模型选择器箭头同步换成 `triangle-down` 图标；② 弹层模糊"不生效"的根因是 `--popup-bg` 0.48 太透+面板与底层文字无明度差——`--popup-bg` 回升至浅色 0.62 / 深色 0.82，且 Dropdown 菜单从 inline style 收敛为 `.dropdown-menu` CSS 类（样式集中管理）；③ 主代理工具集设置项移除——`mainAgentToolAccess` 配置端到端删除（AppConfig 结构体/磁盘归一化/patch 命令/TS 类型/API/agent-prompts/sendMessagePrep/ChatPage/i18n），主代理恒为全工具集（含 execute_command，简单任务直接完成不再委托往返），混合/纯编排策略下线
- **并行写入隔离设置项移除（PR #144）**：`subagentIsolationMode` 配置端到端删除（AppConfig/PatchAppConfig/TS 类型/API/AppSettings UI/i18n），隔离策略固定为自动——git 项目上被等待的写入类子任务自动获得独立 git worktree（完成后合并回当前分支），非 git/网络路径回退共享模式（文件锁保护）；`delegate_task(s)` 的 per-task `isolation` 参数保留（模型的单任务逃生门）；工具描述文案同步去掉"按应用配置覆盖"的说法
- **弹窗背景一致性修复 + 透底亚克力化（PR #145）**：项目记忆面板与项目设置面板背景色不一致（记忆面板明显更暗）——根因是 `background-attachment: fixed` 在祖先有 backdrop-filter/transform 时静默失效，导致 `--dialog-bg` 自绘辉光按弹窗自身尺寸定位：内容少的矮弹窗（设置）被 500px 半径辉光覆盖显得亮、内容多的高弹窗（记忆）下半截落在辉光半径外只剩深色罩层。现改为真正的透底亚克力：`--dialog-bg` 从复合背景（自绘辉光）变为纯半透明 tint（浅色 0.55 / 深色 0.85），彩色来自 backdrop-filter 直接模糊主页面真实背景——任意高度的弹窗视觉完全一致；6 处弹窗的外层遮罩从 `bg-black/60` 降为 `bg-black/25`（去掉遮罩自身 backdrop-blur，避免双重模糊和 backdrop root 干扰）
- **用量页热力图点击滚动条闪现修复（PR #146）**：无用量日（无明细列表）→ 有用量日（列表挂载）切换时，界面右侧滚动条闪现一次——根因是明细区块挂载时播放 `animate-row-in` 入场动画，`translateY(6px)` 在动画期间把父滚动容器的可滚动溢出区域撑大 6px（CSS 变换计入可滚动溢出），动画结束归位后滚动条消失；有用量→有用量只更新行内容不重挂载所以无此现象。经 CDP 驱动 headless Edge 逐帧采样复现页实证（`translateY` 版稳定复现 6px 溢出帧、opacity-only 版零溢出帧），修复为明细区块换用纯透明度入场（新增 `fadeOnly` keyframes + `.animate-fade-only` 工具类）；新增 `scripts/gen-heatmap-repro.mjs` + `scripts/cdp-repro-run.mjs` 复现工具链
- **子智能体「不限制」并发档位 panic 修复（PR #147）**：最大并发子智能体设为 -1（不限制，完全自主 opt-in）时每个 agent 回合启动即 panic——`Semaphore::new(usize::MAX)` 超出 tokio 信号量上限 `MAX_PERMITS = usize::MAX >> 3`，tokio-rt-worker 线程 panic、主循环兜底 epilogue 接住后回合直接夭折（报错 `a semaphore may not have more than MAX_PERMITS permits`）。新增 `resolve_subagent_concurrency` 把不限制档位映射为 `Semaphore::MAX_PERMITS`（信号量真实合法上限，acquire 语义不变），0→1、None→5、正数透传保持原语义；+2 单测（unlimited→MAX_PERMITS、常规档位透传）
- **MCP stdio 子进程树泄漏修复 + autoStart 单飞去重（PR #148）**：宿主侧核实沙箱探测报告时做进程树取证，发现同一 Svelte MCP server（npx 型 stdio）被拉起 **3 份并存**（3 个 npx 链全部在应用启动后 4 秒内生成；fs server 仅 1 份）。两个叠加根因：① `initMcpServers` 有 5 个触发点（应用启动/项目选择/ChatPage effect/模式选择/topic 操作），项目打开瞬间并发触发——各调用在首个 start 完成前快照 statusMap 看不到 connecting 态，对同一 server 重复 start（后端 start 全程持互斥锁、注册前先杀旧连接的重启语义，最终状态表只剩一份，但前两份泄漏）；② `stdio.rs` 的 `close()` 只 `child.kill()` **直接子进程**——Windows 上 exec_resolve 对 `npx`/`.cmd` shim 经 `cmd /c` 包装 spawn（SEC-06 PATHEXT 解析），杀掉 cmd.exe 后 npx → node 孙进程全部成为孤儿；fs server 的直接子进程就是真身、无包装层，所以从不泄漏。修复：新增 `kill_child_tree`——Windows 用 `taskkill /T /F`（CREATE_NO_WINDOW）自上而下整树终止、kill/wait 兜底回收；FE 新增 `autoStartsInFlight` 单飞集合跨并发 `initMcpServers` 调用去重（显式重启不受影响）。现场取证与清理：taskkill /T 终止两条孤儿链、活链保留，随后同 id 重启不再泄漏
- **完全权限模式无人值守自愈（沙箱拒绝自动补救三连，PR #149）**：round16 探测暴露完全权限（Auto）下沙箱拒绝仍需人工看守的三个缺口，逐一修复（`execute_builtin_tool` 内，主代理与子代理共用）：① **ask-once 节流误伤 Auto**——30 分钟防弹窗窗口同时节流了 Auto 的自动扩权，同一操作（如所有 git 命令共享 "git" token）首次自动补救后 30 分钟内后续命令全部裸吃拒绝；现 Auto 不弹窗也不受窗口节流，命中签名即补救（签名判定收敛为 `sandbox_denial_signature` 辅助函数，触发与兜底共用）。② **授权未落地自动降级**——会话授权的 ACL 写入可能被 EDR 拦截（round 10/11 实录：D:\Git\cmd 的 SetNamedSecurityInfoW 被 error 5 拒绝），授权后重试仍在沙箱内撞同一堵墙；现 Auto 授权后仍命中拒绝签名时自动跳出沙箱兜底重试一次，不再盲试。③ **成功授权永久记住**——会话授权随进程退出失效，无人值守跨会话每次重撞；现 Auto 授权落地且命令成功时自动把目录写入 `sandboxWindowsToolchainDirs`（等价弹窗「永久记住」，Auto 进入时的风险确认已覆盖该授权面），后续会话免重新授权。已知边界：python `subprocess` 内部的 spawn 失败（命令本身 exit 0 时）对工具边界不可见，无法触发补救——属沙箱边界固有盲区。文档 permissions.md / chat-and-agent.md zh+en 同步
- **借鉴 Codex：放手与沙箱双管齐下（`.git` 历史保护 + 兜底开关，PR #150）**：对照 Codex CLI 的「静态边界保安全、圈内自洽保放手」哲学补两块拼图：① **沙箱内 `.git` 递归只读（`sandboxProtectGit`，默认开）**——防提示注入经 shell 篡改 Git 历史：Windows 实现踩过两个深坑后才落地（DENY ACE 在 AppContainer **两段式 package 检查中不被求值**，DENY 落盘后沙箱写照样成功；REVOKE 又只对 explicit ACE 生效、对继承态 ACE 无效）——最终机制为三步：PROTECTED 写回冻结继承固化 ACE → 重读 → SetEntriesInAclW([REVOKE package, GRANT package R+X]) + PROTECTED 写回，package 段从此找不到写授权、读经 RX 放行、宿主用户 ACE 原样保留（AppContainer e2e 实测：沙箱内写 `.git` 被拒、`type .git\HEAD` 可用、项目根写入不受影响）；Linux 为 `--ro-bind-try .git` 盖在 RW 根绑定之上、macOS 为 SBPL 拒写规则追加在 allow 之后（后匹配者胜），两平台纯函数构建器 +2 单测；shell 级 `git add/commit` 在沙箱内被拒，Git 写操作走内置 `git_*` 工具（宿主进程）；worktree 隔离工作区豁免（其 `.git` 是指回主仓库的指针文件）；② **「跳出沙箱兜底」开关化（`sandboxUnsandboxedFallback`，默认开）**——保守用户可关闭 #149 的运行时边界突破，关闭后被拒命令保持拒绝、模型自行绕路，对齐 Codex「静态配置定边界、运行时永不破例」。配置均文件级（与 sandboxRequired 同层，无 UI）；文档 permissions.md / chat-and-agent.md zh+en 同步；cargo 365 通过（+1 AC e2e +2 构建器单测）/ clippy / fmt 干净
- **安全审查修复：Auto 沙箱边界收紧（升级面收窄 + 可见性 + 负例测试，PR #151）**：外部审查报告指出 #149/#150 组合后「完全放手」达成但「沙箱安全」在 Auto 下不成立（自愈链可把任意沙箱拒绝升级为无沙箱执行、签名文本可伪造、跳出沙箱不可见且静默持久化扩权），逐项核实属实并修复：
  - **升级面收窄（核心）**：Auto 的无沙箱重试仅剩一类——not-found（解释器/工具链不可见）+ 会话授权未落地（EDR 拦 ACL 写）+ `sandboxUnsandboxedFallback` 显式开启；**access-denied 类（写目标被拒、`.git` 保护、项目外路径）永不升级**——拒绝文本来自模型可影响的命令输出（`echo Access is denied & exit /b 1` 即可命中），不能作为突破边界的决策点。决策拆为纯函数 `auto_remediation` / `escalate_after_failed_grant`
  - **默认值翻转**：`sandboxUnsandboxedFallback` 默认 true → **false**（安全默认；需要 npx 类「目录不可见」自动脱离沙箱的场景显式开启）——默认路径下运行时绝不突破沙箱边界
  - **可见性**：脱离沙箱的重试在**工具结果文本中显式标注**（「本次命令已脱离 AppContainer 沙箱执行」+ network 声明在该路径不生效的提示），模型与用户都可见；非 Auto 弹窗路径的用户选择同样标注；每回合预算上限 8 次（回合开始重置，超限保持原错误）
  - **fail-closed 补洞**：`sandbox_available()` 在 Windows 恒为 true（Job Object 无外部依赖），`sandboxWindows="off"` 时 Auto + 零确认 + 零沙箱静默通过、`sandboxRequired` 形同虚设——fail-closed 守卫现在一并校验 `sandboxWindows == "appcontainer"`
  - **`.git` 拒绝引导**：git 打印 "Permission denied" 不匹配扩权签名、模型拿到无解释错误——现在命令含 `.git` 且命中拒绝签名时在结果中附引导（改用内置 git_* 工具 / 关闭 sandboxProtectGit）
  - **文案对齐**：`agent.mode.autoDescription` 与完全权限风险弹窗补上「未受信任 MCP server 仍逐次确认」门槛与收窄后的沙箱表述（zh/en）；agent-prompts Auto 段同步；文档删除与运行时行为不符的表述、改写为真实边界
  - **e2e 扩充**：`git log` 只读可用性（复刻生产转发授权路径）；`ren .git .git-old` 对象级绕过实证（审查 2.5 猜想——挪走可成功，但挪走后的旧历史仍只读，保护承诺 = 历史不可改写而非目录不可挪动，与 Codex ro-bind 同级，文档如实记录）
  - **已知未修（审查 P1/P2 遗留）**：嵌套仓库/子模块 `.git` 不在保护范围；`auto_persist` 保留（成功授权的目录写入 toolchain dirs，结果文本已标注）；MCP 未信任强制 Ask 属设计保留（补披露未改行为）；审查环境中的 AC e2e 失败为嵌套沙箱干扰——宿主环境 366 通过（+1 git-log/改名 e2e）/ clippy / fmt / i18n / check:frontend 全绿
- **Auto 模式 MCP 工具确认框修复（项目信任与 spawn 门同源，PR #152）**：round17 实测（MCP 工具已可见可用、Skill 可读、异步唤醒通过）暴露「完全权限模式下 MCP 工具调用仍弹确认框要手点」——根因是 Auto 工具门只看每-server 的 `trusted` 布尔（MCP 设置页开关），而**项目级 server 的信任标记不在配置 JSON 里**（在项目信任存储 `mcp-trust.json`，与 spawn 门同源，项目打开时经指纹确认对话框写入）→ 项目 server 的 `trusted` 恒为 false → 每次工具调用强制 Ask（60 秒超时还会中断无人值守回合）。修复：Auto 工具门的信任判定并入 `project_server_spawn_allowed`（spawn 门同一函数）——项目级 server 经项目打开信任确认后即免逐次确认，全局 server 仍以设置页开关为准（未受信任的双保险语义保留：过不了 spawn 门的 server 连工具都注入不进来）。Ask 提示文案同步区分两条信任路径；文档 chat-and-agent zh/en 更新
- **模式阶梯新增「本机全权」档（danger = 无沙箱显式 opt-in，PR #153）**：回应「为何 DSH/Codex 能完全不受限而 AIO 不能」——缺的只是一档显式 opt-in。新增 `AgentMode::Danger`（wire `danger`，按助手持久化，无需迁移）：与完全权限相同的全自主许可矩阵，且**命令完全脱离 OS 沙箱**以完整用户权限原生执行——无路径/网络限制、无 `.git` 保护，全局沙箱配置在该档不生效（执行时按模式决定，不改全局配置；`sandboxWindows=off` + `sandboxRequired=false` 的手工配置路径被此档取代）。进入时弹出**独立红色风险警告**（会话内一次，与 Auto 的黄色警告区分）：明确告知命令将对整台机器生效、仅建议一次性测试机/容器/VM 使用。语义对齐：`AgentMode::is_full_autonomy()`（Auto|Danger）贯通许可矩阵（permission.rs 15 处 builtin 规则 modes）、网络确认门跳过、子代理批量审批跳过、未受信任 MCP 门跳过；fail-closed 守卫仍只约束 Auto（danger 的「沙箱不可用」即其语义）。FE：模式菜单新增红色档位（alert-triangle 图标）、独立警告文案（zh/en 5 个新 key）、模式循环快捷键纳入、状态条胶囊标签。定位：Codex `--dangerously-bypass-approvals-and-sandbox` / DSH danger-full-access 的对等物——边界由环境本身承担的一次性场景才使用。文档 chat-and-agent zh/en（模式表 + 新章节）
- **cmd /S /C 引号语义修复（本机全权档实测暴露，PR #156）**：round19 实测（切到本机全权档后命令在真实主机原生执行）暴露两个引号缺陷——裸命令被套上双重引号报「不是内部或外部命令」、参数内双引号被剥导致 git commit -m "msg" 拆散、python -c "..." 报 unterminated string literal。根因与修复分两层：① **native 路径**（danger 档 / sandboxWindows=off）此前用 arg() 把整条命令按 MSVCRT 规则包成单参数（含空格加引号、内部引号转义）→ cmd 收到双重引号包裹的文本；改为 **cmd /S /C "<命令>" 规范形态**（raw_arg + 包裹引号）——/S 下 cmd 只剥最外层包裹引号，内部引号（转发解释器的带引号路径 + 参数内引号）原样保留，零引号/单引号/多引号命令均正确；② **AC 沙箱路径同步升级** /C → /S /C "<命令>"——不带 /S 包裹时多引号命令同样触发 cmd「剥首尾引号」规则（实测复现 harness 报告的 unterminated string literal）。+2 Windows e2e（native 引号保真、native 转发解释器），368 通过 / clippy / fmt 干净。read_skill 报「MCP server 未连接」为 harness 侧配置（skills 为空时内置 read_skill 不注入，同名 MCP 工具路由至其 server；该 server 未连接），非 AIO 回归
- **Windows 默认 shell 切换 PowerShell（shellWindows 配置，PR #157）**：对齐 DSH/Codex 类 harness 的现代 shell 形态——`execute_command` 在 Windows 上默认走 **PowerShell**（pwsh 7 经 PATH 解析，缺失回退 `powershell.exe` 5.1；`shellWindows: "cmd"` 可切回），新增 `WindowsShell` 选择 + 配置字段（models/core 往返）。三个关键技术点：① **`-EncodedCommand`（UTF-16LE base64）**传命令——native argv 与 AC 原始命令行两条路径都免疫引号转义问题；② **退出码传播**——脚本尾部 `exit $LASTEXITCODE`（纯 cmdlet 时回退 `$?` 判定），否则 pwsh -Command 恒退出 0，错误检测/熔断/扩权签名全部失效；③ **转发解释器的 `& ` 调用前缀**——pwsh 不执行裸引号字符串。AC 路径的 Store 别名（WindowsApps reparse point）在 AppContainer 内不可执行 → 自动回退 System32 powershell.exe 5.1（ALL APPLICATION PACKAGES ACE 恒可用）；错误签名扩充 pwsh 形态（`is not recognized`/`无法将`/`Access to the path is denied`）；危险命令词法已覆盖 pwsh 形态（Remove-Item 等，既有）。+3 Windows e2e（native pwsh 执行 + 退出码传播、AC 内 pwsh），370 通过 / clippy / fmt 干净
- **后台子代理完成不停转圈修复 + 10 角色专属图标/配色/动效（PR #160）**：① **状态条转圈不停**——`collectSubagents` 扫全部助手消息的 agentSteps，但 `subagent-done`/`subagent-error` 处理器只更新 `history` **最后一条**消息；后台（wait=false）子代理在父回合推进后才完成时，其 step 不在最后一条消息 → 状态永不翻转 → 状态条菜单与行内 spinner 永转。修复为按 `subagent_id` 全历史搜索定位（done/error 两处），后台完成的子代理即刻标记完成/失败。② **10 个子代理角色视觉重构**——Icon 库新增 5 个角色图标（`review-search` 审查 / `bug` 诊断 / `pen-line` 写作 / `flask-check` 测试 / `compass` 架构），`PROFILE_STYLES` 从 4 个角色扩到全部 10 个（探索/实现/审查/诊断/写作/测试/需求/架构/看图/全能力），配色走新增的 `--role-<id>-rgb` CSS 变量（rgba 棘轮 0 新增，TSX 内字面量归零）；工作中状态新增**环绕动效**（`subagentOrbit` 圆点环绕角色徽章 + `subagentPulse` 微脉冲），应用于状态条子代理菜单行内徽章。深浅色主题截图验证（`scripts/gen-role-icons-check.mjs`）；check:frontend 全绿（i18n 1007 keys、rgba 棘轮 168≤173）
- **Skill 两条链路修复：npx 导入分源读取 + 项目设置面板陈旧 store（PR #158）**：round20 反馈的两个问题。① **skills CLI 来源的 npx 导入必失败**——discovery 会把 skills CLI（`npx skills add` 生态）安装的 skill 列出来（source_type=skills-cli/skill-dir，name 不是 npm 包），但 import 一律执行 `npx <name>` → npm 报 could not determine executable to run；修复：import 分源——发现结果携带的 sourcePath 是磁盘安装目录时直接读取 SKILL.md/skill.md/README.md（元数据 version/description 同步以目录为准），仅真正的 npm 包（bin 打印 Markdown）才走 npx 执行，且该失败形态的报错附指引；SkillConfig 新增 `sourceDiskPath` 字段，refresh 同样分源（磁盘来源重读目录、npm 来源重跑 npx）。② **项目设置面板的 skill 列表陈旧**——商店下载/npx 导入发生在设置页（独立 store 写入方），面板打开时依赖各处「都记得 setSkills」的隐式约定，漏一处就显示未配置（用户实测：重进商店再回来才恢复）；修复：ProjectSettingsModal 打开时主动 `initSkills(项目)` 重读（与 memory status 同一模式）
- **审批「始终允许」持久化 + 弹窗超时统一 600s（PR #161）**：回应「完全权限下未预配置的工具，同意后应自动加入配置、以后不再触发」。此前两个缺口：审批气泡的「始终允许」只注册进程级会话规则（重启即失效），且未信任 MCP 门与网络命令闸根本不看会话/自定义规则（点了也白点，每次调用照弹）。三连修：① **持久化**——项目内调用的「始终允许」把派生规则原子写入 `.aio/permissions.json`（去重；无项目上下文退化会话级；写失败回退会话级并告警），派生口径与会话规则一致（命令→首 token 前缀、文件工具→同一文件精确路径、MCP→同 server 同工具）；`PermissionRule` 新增 `auto` 派生标记——持久化的命令前缀规则与会话规则同等受链式操作符守卫（SEC-02），手写规则语义不变；`ApprovalContext` 携带 project_path，5 处 request_tool_approval 调用点贯通。② **门闸尊重用户规则**——未信任 MCP 门（Auto/Danger）与网络命令闸（Normal）新增 `user_allow_rule_matches` 检查：覆盖该调用的用户 Allow 规则（持久化或会话级）视为已配置，直接放行。③ **超时对齐主流 harness**——工具审批弹窗与沙箱扩权对话框等待 60s→600s（问答弹窗本就 600s），超时仍按「未批准/忽略」处理、气泡自动消失。FE：审批事件载荷新增 `project_bound`，「始终允许」按钮据此在「始终允许」（持久化，悬停提示写入路径）与「本次会话始终允许」间切换；i18n +2 keys（1009）。文档 permissions（新「始终允许」章节 + 字段表补 commandPrefix/auto + 实时生效说明）/agent-tools/chat-and-agent/architecture zh+en 同步；cargo 374 通过（+4：持久化往返去重、auto 链式守卫、手写规则豁免、user_allow_rule_matches）/ clippy / fmt / check:frontend 全绿

### 🎨 界面视觉修复五连（本轮用户反馈）

- **流式闪烁根修（消息身份稳定化）**：`flushStreamBatch`（80ms 批处理）此前每次 flush 用 3 次 `setDatas` 路径更新——Store 的路径 setter 会**重建路径上每个对象**（消息引用每 80ms 变一次），`<For>` 按引用比较 → 流式中的消息行每个 flush 重挂载一次 → 气泡入场动画（`animate-stream-fade-in`、ThinkBlock 展开动画）逐 token 重放（用户看到的"闪烁卡顿"）。现流式路径全部消息更新改为 `produce()` 原地变异（solid-js/store）：flushStreamBatch 合并为单次 produce（content+reasoning+agentSteps）、子代理步骤/回合开始/工具调用/工具结果/交接/子代理起止共 8 组监听器同步转换——消息对象引用稳定，细粒度代理仍精确通知；`MessageBubble.fileChanges` 改 `createMemo` 跟踪 `msg.toolCalls`（原地变异下引用比较失效的配套修复）
- **自动滚动修复（主视图 + 思维链内部）**：① 话题切换回底 snap 的 rAF 渐进校正循环，此前在 content-visibility 占位行逐帧撑高时提前退出 → 用户看到"闪到消息记录最前端"；现退出条件收紧为「触底且 scrollTop 连续 2 帧稳定」，守卫上限 40→60；② ThinkBlock 内部此前完全不跟随流式增长——新增 body 滚动跟随（流式中贴底、滚轮上翻即暂停、回到底部自动恢复）；AgentProcessBlock StepCard 思考体同款处理
- **子智能体视图逐 token 重挂载修复**：`SubagentConversationView` 回合列表 `<For>` → `<Index>`（turns memo 每 delta 重建 turn 对象，`<For>` 引用比较导致子代理视图 ThinkBlock 同样逐 token 重放入场动画）；Index 按位置持行，DOM 跨重建存活
- **任务清单并入状态条（DSH 风格）**：TaskBoardPanel 从消息记录上方的独立灰主题面板（`bg-gray-800` 硬编码）改为状态条内嵌分区——展开时状态条向上生长显示任务行（CSS 变量配色、in_progress 主色脉冲点、completed 划线），点击摘要行可收起；收起态状态条仅多一行「任务清单：N项 已完成M项 待处理K项」（i18n 双语）
- **授权/提问弹窗并入状态条**：ToolApprovalBubble 与 ask_user 问答不再以卡片插入消息流（打断阅读位置），改在状态条内展开（max-h 45vh 可滚动，与状态行以分隔线相隔）
- **输入框高度响应式修复**：textarea 高度此前只在 onInput 手动设置、只在 Enter 键分支重置——按钮发送（或任何清空输入的路径）后高度不回落，需再按一次回车才恢复。现高度由 `createEffect` 从 `props.inputMessage` 信号驱动：任何路径（输入、发送清空、/命令填充）都即时重算（空→40px，长文→min(scrollHeight, 200)）
- **ask_user 提问改为逐题引导 + 问题卡占满状态条**：一次请求携带多个问题时按步展示——作答当前题后点「下一题」才显示下一题（可「上一题」回改，头部显示进度「第 N / M 题」，单次提交协议不变）；状态条内同时只显示第一个待答请求，应答后自动出下一个；问题卡去掉 420px 宽度上限、占满状态条宽度
- **上拉菜单越界修复**：任务清单并入状态条时引入的容器 `overflow-hidden` 曾把子智能体上拉菜单与 git 分支下拉裁剪在状态条内——移除裁剪（摘要行自带顶部圆角补偿），两菜单恢复完整越界展开
- **上下文窗口 token 口径修复（对齐主流 harness）**：① 短对话显示 500k+ 的根因不是思维链——思维链从不回传 API（`append_assistant_message` 无 reasoning 字段），显示值本就是 API 报告的末次 prompt 真实大小；真正的根因是 `context_tokens` 不落库，重启/重载后回退到 `inputTokens`（agent 回合 = 各轮 input 之和，成本口径）→ 数值虚高数倍。现 `messages.context_tokens` 列（迁移 v16）贯通 append/persist/branch/load 全路径，回退逻辑移除（向前扫描最近已知值，legacy 数据显示 0=未知）；② 发送下一条消息后归零的根因是新的空助手消息成了"最后一条"——现保持显示最近一次测量值，且 agent 循环每轮完成后发无内容用量 ping（`produce` 原地应用，无行重挂载），上下文条按 API 调用逐轮增长（Claude Code / Codex 行为）
- **缓存命中率分母修复**：后端 PROV-15 已将所有供应商归一化为「input_tokens = prompt 总量、cached_tokens 为其子集」，但前端命中率分母仍用 (input + cached) 双重计入缓存部分——真实 80% 命中显示为 ~44%（即用户看到的"一直四十几"）。分母改回纯 input 后显示真实命中率
- **手动 /compact 对齐主流 harness**：调研 DSH（`dsh-compaction-basic`：手动 compactNow 保留区=0）、pi/oh-my-pi（compaction.md：切点回退 + 保留区 20k）、ZCode（`core/src/compact/manual.ts`：强制立即压缩 + provider_usage 优先的 token 口径）与 Claude Code（全量摘要）后对齐——① 手动 `/compact` 缺省压缩全部历史，不再出现"上下文很小，无需压缩"（唯一跳过是空对话）；`/compact <n>` 保留 n token 尾部（至少 4 条）；自动压缩仍保持 20k 尾部；② 保留区 token 估算改为 CJK 感知（汉字 ≈1 token，其余 ≈4 字符/token）——原 chars/4 对中文低估约 4 倍，是"明明很大却提示很小"的元凶；③ 跳过反馈按原因区分（空对话/压缩中切话题/数据保护中止），替换笼统文案
- **压缩体验三连修**：① 压缩过程视觉反馈——消息记录内嵌"正在压缩上下文"胶囊 + SVG 圆弧动效（参照 DSH 的记录内压缩指示），手动 /compact 与回合结束自动压缩均生效；② 压缩不再删除消息记录——引入 `topics.context_trim_count`（迁移 v17）作为上下文边界：LLM 请求只发 摘要 + `history.slice(边界)`（主请求 / btw 侧问 / 运行中压缩事件全路径），完整对话记录始终可见（对齐 DSH 表面遮蔽 / pi 原始条目保留）；③ 消息记录分段渲染——切话题先只挂载最近 40 条，顶部哨兵 IntersectionObserver 滚到顶自动增量加载更早分页（滚动锚定 + 预标动画跳过，绝对索引保持 resend/typing 逻辑不变），修复长对话切换卡顿
- **压缩摘要升级 + 读回通道（融入 pi 设计）**：① 结构化压缩指令（共享模块 `summarize_prompt.rs`，驱动手动/自动与前端 `summarize_history` 两个摘要器）：pi 的章节结构（目标/约束与偏好/进展/关键决策/下一步/关键上下文 + read-files/modified-files 清单）+ DSH 的逐字规则（路径/命令/错误串/标识符/数值/函数签名不可转述），旧检查点迭代合并而非照抄；max_tokens 700/1024 → 2000；② pi 式摘要输入：消息序列化为纯文本回合（`[User]` / `[Assistant]` / `[Assistant tool calls]` / `[Tool result]`），工具结果 2000 字符预算，超长跨度保留首条用户消息 + 最近尾部（30 万字符上限）——原 JSON 全量形式可能超出摘要器自身窗口；③ 成功路径也归档原始跨度：摘要写完后 best-effort 归档被压缩消息，占位符 `[历史归档 #id]` 附加进摘要（旧归档 id 迭代携带），Agent 模式下模型可用 `read_artifact` 读回被压缩细节——恢复通道超过主流基线；摘要注入标签同步说明该机制
- **归档保真度对齐管线（读回通道无损化）**：`archive_early_messages` 原按每条 2000 字符截断写归档——一条打满 20KB 上限的工具结果在恢复通道里被静默丢掉 90%。单条预算提升为 `TOOL_RESULT_MAX_BYTES`（管线自身对落库工具结果的截断上限，20000 字节），归档成为压缩跨度的无损副本且体积有界（典型每次压缩 ~0.5–1.5MB，硬上限 ~4MB）；读回端 30k 头尾截断不变
- **两处 UX 修复**：① 流式气泡在切页面返回后消失——`ChatPage.onMount` 每次挂载都从 SQLite 重载全部助手数据并整体覆盖 store，而流式中的助手占位消息要到回合结束才落库，切走再切回就被抹掉；改为仅冷启动加载（`isFirstAppLaunch`），18 个流监听器本就是全局注册（CHAT-12），离开页面期间 store 持续更新，冷启动后 store 即实时真相，首次加载失败仍会在下次挂载重试；② 子智能体模型配置下拉把所有供应商的云模型混成一列——按来源供应商（`owned_by` = 供应商配置 displayName）分组渲染，分组头复用输入框 ModelSelector 的分段样式，行内渲染不变
- **完全放手模式（风险自担的自主权交还用户）**：按 DSH/Codex 的维度拆分（"少问"与"能碰什么"分离）落实——① `AppConfig.fullAutoMode` 开关（默认关，设置页开启需确认风险，文案明示外泄权衡）：Auto/Workflow 模式下**两类确认一并免弹**——沙箱拒绝的命令自动按推荐方案执行（可派生目录 → 会话授权后重试；否则跳出沙箱重试一次），声明网络访问的命令不再逐条确认（SEC-01 闸门的显式豁免，注入外泄风险由用户自担）；项目 Deny 规则仍然生效；② 最大工具调用轮数支持 **-1 = 不限**（0 为禁用值：UI 输入 0 自动吸附为 -1，后端钳制到最小 1；配置链路整体改为 i32，共享 `effective_max_rounds` 同时作用于主循环与子智能体派生），上下文预算/压缩/停止按钮仍是后盾；③ 常驻状态标识——Auto/Workflow + 完全放手激活时模式胶囊显示脉冲红点 + 提示，完全放手状态永不可见即最危险；④ 最大并发子智能体数同样 **-1 = 不限**（后端信号量映射 `usize::MAX`，同样禁 0），与轮数语义对齐；⑤ **-1 为硬下限**——三层强制：`NumberInput` 钳制（输入 -2/-5 落到 -1，并修复输入 "-" 中间态被 fallback 重置、负数无法键入的问题）、`patch_app_config` 持久化前钳制（< -1 → -1，0 → 1）、磁盘加载归一化（手改 config.json 的越界值同样收回），配置文件不可能存在低于 -1 的值
- exe 已重建（dist 重嵌）；`npm run check:frontend` 全绿（rgba 棘轮持平 173）

### ✅ 第十一轮复测：三项目标全部确认（全绿轮）

- **第十一轮结论**：① 主代理直执通过（full 工具集 + 提示词策略修复生效，时间线无子代理卡片）；② 子智能体实时流式通过（四项操作员观察全中：卡片进度行/正文流式/工具行实时/思考块）；③ **P1-6 判决性确认**——授权后重试返回 git 自身的 `拒绝访问。`（EDR 层）而非「AppContainer 沙箱启动失败」，随后 `ver` 正常，沙箱未被砖；④ 记忆语义排序正常。终端日志拍下完整管线时序：触发 `16:21:49.163` → 授权重试 `16:22:00.429`（**11 秒完成点击→路由→重试**——round-10「解耦」误判的最终铁证）
- **授权失败重试去重（本轮日志驱动的小修）**：`grant_dir_access` 只缓存成功——失败目录此前**每个 spawn 重试一次失败的 ACL 写 + 一条 WARN**（实测 1 分钟 3 条）；现在失败也按 (suffix, dir) 进程级缓存：首次尝试记录+告警一次，此后整进程跳过（新 app 会话重新尝试一次）。污染配置（如「永久记住」一个 EDR 看守目录）不再刷日志、不再付逐命令的失败 ACL 调用
- **宿主侧配置清理**：第十一轮测试 4 的「永久记住」把 `D:\Git\cmd` 写入了 `sandboxWindowsToolchainDirs`（该目录 ACL 写恒被 EDR 拒，条目只会产生日志噪音）——已移除；行为验证不受影响（P1-6 修复使污染条目无害，会话级与永久级表现一致）
- 夹具仓库（`dd48c1b`）：报告第 13 节 + 三项确认记录

### 📡 第十轮后需求：子智能体 DSH 级实时流式 + 主代理工具集设置

- **子智能体实时流式（对齐 DSH 观感）**：此前子代理的正文/思维链被 `suppress_events` 整体吞掉，独立会话视图只显示回合末 300 字符摘要、思维链完全不渲染、巨型工具参数生成期间（~48KB 单次写入 ≈ 数分钟）前端零反馈。现：① 新事件 `subagent-chunk`/`subagent-reasoning`（`SubagentStreamCtx` 注入 `stream_one_round`，delta 转发到子代理专属通道）；② 独立会话视图按回合分组渲染——思维链用 ThinkBlock（流式自动展开、完成后折叠）、正文实时 Markdown、工具行；③ **巨型工具参数进度**：每 2KB 发一次 `streaming` 步骤（「write_file: 12KB」），主时间线紧凑卡片同步显示最新子步骤——「卡思考 5 分钟」不再可能；④ 旧数据（无 round 字段）回退旧分组逻辑，历史会话渲染不变
- **主代理工具集设置（`mainAgentToolAccess` 暴露到设置 UI）**：设置 → 通用 新增三选项下拉（混合=默认：读直执/写委托；全部=含 execute_command，简单任务主代理直接完成、无需委托往返；纯编排=严格委托）。`patch_app_config` 支持该字段（含取值校验）；后端 `full` 模式原有能力不变，此前为 config-only 无 UI 入口
- 事件名双端登记（events.rs + events.ts kebab 守门）；前端全部门禁绿（rgba 棘轮持平 173——复用共享 Dropdown 组件而非复制手写下拉）

### 🎉 第十轮：扩权弹窗管线验证成功 + 三项取证修订 + P1-6 级联修复

- **扩权弹窗管线全程验证通过（九轮悬案闭环）**：触发（终端 `[sandbox-expand]` 日志）→ 弹窗渲染（操作员确认）→ 应答路由 → 决策处理（python=操作员秒点跳过/忽略→按设计返回原错误；git=会话授权→重试），各环节均工作。DB 时序证据（python 委托总耗时 17.4s）排除 60s 超时路径，前端无自动应答逻辑——现场「弹窗与调用解耦」的判断系操作员快速点击被误读
- **P1-6（真 bug，已修）：会话/工具链目录授权失败砖掉整个沙箱**：扩权「授权（本次会话）」后重试，该目录的 ACL 写被 EDR 拒（error 5）→ extra R+X 目录走 fail-closed → 整个 spawn 失败，**同会话后续所有 shell 命令被砖**。修复：extra 目录（配置工具链 + 会话授权）改 best-effort（失败记日志、跳过、沙箱继续）；fail-closed 仅保留给沙箱必需项（profile/项目根/AC 临时目录）——缺失附加授权 = 访问更少（fail-safe），不是沙箱失效；docs zh/en 同步
- **自愈判定反转（取证修订）**：记忆库实况 = 配置已重置 `all-MiniLM-L6-v2/384`、**全部 12 条活跃事实带真实向量**（含第 8 轮旧事）——第六轮引入的死配置自愈完全生效。现场「相关度仍 0.02」是 RRF 融合分的固有量级（排名分 1/(60+rank)），与语义降级无关；判定标准缺陷在轮次提示词（我的锅），非产品
- **测试 3 收口**：`max_lines=-1` 单独 → 32KB 字节上限独立生效（截断标记）；`max_bytes=-1` 同时传 → 全文——+1 决定性单测（44KB 文件，现场因文件太小未验证）
- **P3-7（UX backlog）**：巨型工具调用参数生成期间（48KB 单次 write）前端约 5 分钟无反馈——测试设计应避免巨型单次写入（分片追加）；工具参数流式进度指示记为 backlog
- 夹具仓库（`1799afd`）：报告第 12 节（三重判定修订 + P1-6/P3-7 入清单）

### 🛡️ 第九轮 P0-3：write_file 漏传 content 静默清空文件（数据丢失级）

- **P0-3（第九轮核心发现）**：`write_file(path=...)` **漏传 `content`** 时经 `unwrap_or("")` 落入空串 → 已存在文件被**静默覆盖为 0 字节**（实测 README.md 被清空，靠子智能体原文恢复）。三层修复：① `content` 缺失或非字符串 → 硬错误（不再默认空串）；② **显式空串覆盖非空文件要求 `expect_hash`**（「证明你读过你要摧毁的内容」；新建空文件保持放行——第八轮合法场景不受影响）；③ 工具 spec 同步（content 必填说明 + 空覆盖语义）。+2 回归测试覆盖五象限（缺失/空串非空/空串+hash/新文件空/非字符串类型）
- **入参校验缺口（第九轮边界表）**：`read_file` 的 `max_lines`/`max_bytes` 低于 -1 此前被静默当作「不限」——现硬错误；spec 说明澄清 **-1 只解除行数上限、不解除字节上限**（需完整大文件须同时传 `max_bytes=-1`，对齐第九轮「名义不限行数仍被 32KB 截断」的观察）
- **扩权弹窗之谜（第九轮报告「弹窗未触发」）**：本地探针实测触发条件全过（转发✓/拒绝签名匹配✓/操作键✓——注意 AC 子进程 cmd 输出是**英文** "Access is denied." 而非中文，两种均匹配）；round 9 全程 154 秒容不下任何 60s 等待 → 弹窗确实未触发，但触发进程已消失无法追溯二进制版本。已给触发路径加**判决性日志**（触发/超时/签名不匹配三线 tracing），下次运行必留铁证；另澄清：**文件工具的「路径越界」是工具层硬边界（设计如此），扩权弹窗只管 shell 命令的 access-denied**——第九轮用文件越界去测弹窗属于测错了入口
- 夹具仓库（`69c2c3f`）：报告第 11 节 + P0-3 入清单；README.md 行尾 CRLF 经 `git restore` 恢复（LF 化导致的 65 行伪 diff 清除）

### 🧠 第八轮发现闭环：remember 覆盖语义可见化 + 子代理工具口径

- **remember 同 key 覆盖的结果区分（第八轮核心发现）**：取证修正——「旧版本静默丢失」在存储层**不成立**（`upsert_fact` 同 key 原地更新、id 不变，旧内容经版本审计写入 `fact_versions`，`get_versions`/记忆面板可查）；真正的缺陷是**工具层不可见**：新建与覆盖返回完全同文的「已记住」，模型无从得知自己覆盖了既有条目。现：结果按四种情形区分（新建 / 仲裁更新 / 仲裁合并 / 同 key 覆盖），全部带 `id`（衔接 `update_memory`/`forget_memory` 的 id 入参）；`key` 参数 spec 写明覆盖语义与「需保留版本演进请勿复用 key」；+1 存储层回归测试钉死契约（同 key 二写 id 不变 + `fact_versions` 存档 v1 + `find_active_id_by_key` 定位）
- **子智能体工具集口径文档化（第八轮发现 2）**：`agent-tools.md` zh/en 补「主体口径」注——工具总表/权限矩阵描述**主代理**；子智能体按 `SubagentProfile` 白名单装配（explorer/architect 显式拒绝 `git_*`，9 只读工具；coder/general 持有 Git 读取），未注入工具调用得到 fail-closed 报错；`remember` 文档行的 `key` 参数同步覆盖语义
- **第八轮其余条目定性**：`web_fetch` 仅 HTTPS（spec 已有明文，无需改）；`delete_file` 不能删目录（设计如此，目录删除走 shell）；文件系统边界（中文/空格/空文件/深层自动建目录/多点）全通过；夹具仓库报告补 P2-4（shell-git 与 delete_file 能力不对齐——预期分层，宿主侧代提交兜底）/ P2-5（子代理无 Git 工具——设计预期）两条目 + 10.2 勘误，宿主侧清理 `deep/` 残留并落地第七/八轮未提交项（`a89159d`）

### 🔎 第六轮复测两项产品修复（两项修复确认生效之外）

- **路径越界错误可区分化（报告 9.1 caveat）**：`safe_path` 的 `must_exist` 探测在「绝对路径越界 + 目标不存在」时把 canonicalize 的裸 io 错误（`os error 3`）直接透传——与「项目内文件确实不存在」同构，无法区分网关拦截与真实缺失（实测 `read_file D:\Windows\win.ini` 与 `read_file 不存在的库内文件` 报同款错）。现：越界+探测失败返回专属「路径越界」文案；探测成功的既有路径（canonical 终检）与「库内缺失 → 路径不可访问」语义均不变。实现：`lexically_inside` 纯函数（`..` 词法展开、Windows 扩展前缀剥离、大小写不敏感组件比较——项目内绝对路径大小写变体不误判）；+4 单测（越界+缺失→越界文案、库内缺失保留不可访问、存在越界目标仍被 canonical 终检拒绝、项目内绝对路径放行含大小写变体）
- **嵌入死配置自愈（关闭跨 5 轮未执行的环境侧待办）**：第五轮查明项目嵌入配置为旧 ollama 模型名（`bge-m3:latest`，本地注册表永不解析）→ 全库向量 NULL、检索降级关键词——修复此前依赖用户在设置页手动换模型，实测 6 轮无人执行。现：`maybe_self_heal_embedding_cfg`——配置不可解析 **且库内无任何向量** 时自动回退本地默认模型（all-MiniLM-L6-v2）并持久化（模型/维度 meta 同步 384）；**有向量的库绝不自动重置**（用户决策域）、禁用态不自愈。下次 remember 即写入真实向量、recall 恢复语义区分度；+2 单测（死配置重置+持久化往返、有向量/禁用跳过）
- 夹具仓库同步（`7de02c1`）：报告第 2 节 P0-1 更正标注（delete_file 可用、60s 证据注明为第一轮旧数据）、第 5 节清单按轮次实测状态勾选、第 6 节 shell 命令行记录「9009→拒绝访问」演化（转发生效的铁证）；宿主侧落地第六轮未提交变更（报告第 9 节/README 清理/a_alpha.txt 删除——沙箱内 git 提交被 EDR 阻断）

### 🔬 第五轮复测两项闭环：workflow 失败门控 + 记忆降级可见性

- **workflow 失败门控（新增 P1-4，报告缺陷确认）**：此前步骤失败（子智能体错误，或其全部工具调用失败——如第五轮 explorer 的 read_file 路径不可访问）不中止链条，错误仅以文本喂给下一步，总结仍报「执行完成（N/N）」。现：① 步骤失败默认**中止后续步骤**（标记「已跳过」，前端工作流面板新增灰色 skipped 状态 + 双语文案）；② 步骤级 `continue_on_error: true` 逃生舱（尽力而为步骤）；③ 总结诚实计数（「成功 M/N，失败 K，跳过 S」），任一步骤失败则结果 `is_error=true`；④ 取消立即传播（不再被记为失败步骤）。实现：`ToolCallStats` 原子计数器穿透 DelegateContext → execute_single_delegate → execute_subagent（+2 单测：全失败签名、并发原子性）
- **记忆嵌入降级可见性（隐藏 P0，第五轮「相关度恒 0.02」取证钉死）**：实测库内 10 条事实 embedding 全部 NULL——项目级嵌入配置为旧 ollama 模型名（`bge-m3:latest`），本地注册表永不解析 → `resolve()` 每次必败 → remember 静默 fail-open 存 NULL，recall 降级为纯 FTS 关键词匹配（相关度 = RRF 排名分 ≈ 0.02，无语义区分度），模型与用户全程无感知。现：remember/update 结果附「[!] 未生成语义向量（原因）」警告、recall/search_memory 标注「当前为关键词降级检索」并点名原因（`embedding_unavailable_reason` 复用 resolve 错误文本）
- **记忆库 WAL 卫生（第五轮观察的定性修正 + 修复）**：1.4MB WAL vs 4KB 主库实为**未达 SQLite 默认 autocheckpoint 阈值（1000 页 ≈ 4MB）**而非读连接阻塞（-shm 常驻系连接 LRU 设计）；写放大来自版本审计/FTS/consolidate 后台重写。现：`wal_autocheckpoint=100`（约 400KB 边界）+ remember/update/forget 写会话后显式 `wal_checkpoint(TRUNCATE)`（失败仅记日志，绝不因并发读者失败工具调用）
- **环境侧提示（非代码问题）**：本机嵌入配置需在设置 → Embedding 模型中切换为本地可用项（旧 ollama 配置在 v0.8 后永不解析）；切换走设置页流程（其会清理旧向量），修复后新记忆即带语义向量

### 🐛 第四轮复测捕获：replace_in_file 中文 old_string 必 panic（UTF-8 边界）

- **工具稳定性 bug（报告 7.5「副作用」记录，DB 取证钉死根因）**：writer 子智能体用精确字符串替换对中文文档做追加时返回「文件工具执行线程 panic」（文件哈希未变、内容无损）。根因：`file_tools.rs` 匹配计数循环按**字节 +1** 推进游标——`old_string` 首字符为多字节 UTF-8 字符（任何中文）且文件中至少匹配 1 次时，下一轮 `content[pos..]` 切片落在字符中间 → panic。**触发条件宽松到致命：中文项目里精确替换几乎不可用**（writer 被迫改用行范围模式恰好因为目标行以 ASCII `|` 开头而侥幸逃过）
- **修复**：游标改为跳过**整个匹配**（`pos = abs_pos + old_s.len()`）——匹配末尾必在字符边界上；顺带修正重叠计数语义（`"aa"` 在 `"aaaa"` 旧代码误计 3 处，现为标准的非重叠 2 处，与 `str::matches` 一致）
- +3 回归单测：中文单匹配替换（修复前必 panic）、中文追加替换（writer 精确场景复现）、非重叠计数语义

### 🔤 Harness 第三轮（HARNESS_FIX_REPORT.md）五项闭环 + 一个隐藏 P0

- **控制台输出 GBK 解码（隐藏 P0，报告只当「正常现象」一笔带过）**：cmd.exe 在中文 Windows 按 GBK 输出本地化消息，此前被按 UTF-8 有损解码成乱码——`拒绝访问。` 变成 mojibake，导致**签名检测在中文系统上全部失效**（扩权弹窗从未触发过、环境类失败判别失明）。现 stdout/stderr 按「UTF-8 严格优先 → 系统 ANSI 代码页（MultiByteToWideChar CP_ACP）回退」解码（无新依赖）；真实 cmd E2E 验证中文往返（+1）；GBK 字节夹具单测（+1）
- **解释器转发 v2**（报告 P0-2 根因修正）：DB 取证发现 tester 全程用 `cmd /c python --version` 形态——上一版只识别首 token 裸名，cmd 包裹全部漏转发。现解包 `cmd`/`cmd.exe` + flags 后识别操作 token（`cmd /c python -V`、`CMD.EXE /C /F npm install`、`python.exe` 后缀形态全覆盖）；转发列表加入 `git`/`cargo`（报告观察到 shell 无 git；两者常装于 Program Files，默认 AAP ACE 使转发后的绝对路径在 AC 内真实可执行）；+3 单测组（cmd 包裹改写/形态识别/操作级去重键）
- **扩权触发扩展**：转发目标解释器在沙箱内「不是内部或外部命令 / 9009」也触发扩权弹窗（与拒绝访问同等待遇）；去重键升级为**操作级**（`cmd /c X` 与裸 `X` 同键，无关 cmd 调用不再互撞）；docs zh/en 同步
- **熔断器 v2**（报告 P1-1 三条可选建议全采纳）：① 阈值可配（`toolBreakerThreshold`，0=默认 6，下钳 2）② 第 3 次连续环境类失败起在结果文本追加剩余预算预警 ③ **环境类/业务类失败区分**——仅「命令不存在/权限被拒/缺依赖」签名计数；`pytest` 测试未通过这类业务失败**不再计入**（模型修真实 bug 的重试是合法行为，由轮数上限兜底）；+3 单测
- **shell/内置工具边界文档化**（报告 P1-3）：permissions.md zh/en 新增「shell 运行在沙箱 token 下，部分 OS 级操作（如 `cmd dir` 枚举）可能被拒；内置文件工具经宿主进程执行不受影响——属预期分层」
- **报告勘误（chat DB 取证）**：P0-1 的「`delete_file` 60s 审批超时」为过时信息（round 3 全程无人调用过该工具；Auto/Workflow 免确认已在运行 build 生效）；P0-1 的 `del` 文件删除被拒是本机 EDR 层（DACL 之外的路径过滤，产品代码不可修）——扩权弹窗「跳出沙箱执行」即为其产品级出口

### 🔓 沙箱扩权请求（Gemini 式 UX 闭环，AppContainer 路线收尾）

- **拒绝即问，一键补救**：命令在 Windows AppContainer 沙箱内被拒（WinError 5 签名：`Access is denied` / `拒绝访问`，覆盖 cmd/PowerShell/Python）时，弹出选择对话框——「授权解释器目录并重试（本次会话）」/「授权并永久记住（写入 `sandboxWindowsToolchainDirs`）」/「跳出沙箱执行（仅本次）」/「忽略」，支持输入任意目录路径授权。把「模型盲试烧轮数」（第一轮 tester 25 轮）和「用户全程不知情」收敛为「用户点一下」；EDR/机器级过滤（DACL 之外、代码修不了的层）从此有产品级出口
- **实现零新前端组件、零新事件**：复用 ask_user 问答管线（前端 AgentQuestionBubble 原样渲染；监听器仅按 request_id 路由）——`tasks.rs` 提取 `ask_user_questions` 底层通道（超时参数化：ask_user 600s / 扩权 60s），决策映射/去重窗口/目录合并均为纯函数（+6 单测：选项映射、跳过降级、自定义路径校验、无提示降级、30 分钟去重窗口、合并去重保序）
- **会话级 R+X 授权表**（`win_appcontainer.rs`）：扩权批准的目录进程内存生效、退出即失效，与配置级 `sandboxWindowsToolchainDirs` 案例不敏感合并去重后进入同一 fail-closed R+X 列表；「永久记住」选项走 best-effort 写入配置（失败仅降级为会话级并记日志）；+1 单测（增/去重/空忽略）
- **无人值守安全降级**：同一首命令 token 30 分钟内最多弹一次（+可回拨测试）；60 秒无响应按「忽略」返回原始错误；stop 令牌穿透（cancelled 正常传播）——自动运行场景完全退化为既有行为（熔断器兜底），不会卡住回合
- docs zh/en `permissions.md` 沙箱节同步

### 🔁 Harness 复测（第二轮）四项闭环

- **`delete_file` 免确认（Auto/Workflow）**（复测建议 1）：Auto/Workflow 模式下 `delete_file` 改为 Allow——与已 Allow 的 `execute_command` 对齐（`del`/`rm` 本就是命令，且删除被沙箱限定在项目内）；Normal 模式保持 Ask（与写/提交/建目录的逐次确认一致），Normal 下的静默白名单走项目级 `.aio/permissions.json`（自定义高优先级 Allow 规则，机制已有）。修复无人值守审批超时（复测子智能体调 `delete_file` 60s 无人批准）；docs zh/en 同步
- **脚本解释器转发**（复测建议 2）：`python`/`python3`/`py`/`pip`/`node`/`npm`/`npx` 作为命令首 token 且为裸名时，自动改写为宿主 PATH 解析的绝对路径（SEC-06：仅 PATH 解析、不查项目目录，防栽种）——沙箱 shell 内这些名字原本一律 9009（`where` 也找不到）；Windows AppContainer 同时对解释器安装目录做**尽力 R+X 授权**（新 `best_effort_rx_dirs` 参数，失败记日志继续：系统位置本就可读，不应因授权失败杀掉整条命令），解决解释器目录无 ALL APPLICATION PACKAGES ACE 不可见的问题（实测 `D:\Anaconda3` 授权后 AC 内可 `type` 读取）。注意：本机存在 DACL 之外的路径级过滤层（读放行、执行/列举仍拒），e2e 测试因此标 `#[ignore]`，干净环境手动复跑；docs zh/en 同步
- **工具失败熔断器**（复测建议 3）：同一工具在主循环或子代理循环内**连续失败 6 次**（环境阻断特征：缺解释器/权限拒绝/资源不可用；成功即重置计数）→ 本回合内停用该工具，后续调用短路返回带原因的提示（引导改道或如实上报），不再执行——从执行器层面兜住「tester 烧满 25 轮重试」（复测确认提示词预算仅在任务描述写死时有效，机制层未拦）。主循环与子代理循环各自持独立熔断器；+4 单测
- **`cmd dir` 拒绝 vs 工具正常**（复测观察 4）：确认为合理权限分层，不改（沙箱文件工具走进程内实现，不经 AC token）

### 🔒 Windows 沙箱删除授权修复（harness 会话报告全部条目闭环）

- **AppContainer 授权掩码补 DELETE**（报告第 9 条根因）：项目根与 AC 临时目录的授权掩码 `READ|WRITE|EXECUTE` 不含标准权限 `DELETE`（0x10000）与 `FILE_DELETE_CHILD`——沙箱内一切删除/重命名被「拒绝访问」（实测：LLM 会话 cmd del/rmdir、PowerShell Remove-Item、Python os.remove 全灭，CPython 的 pyc 原子替换也失败，留下 `*.pyc.<随机>` 孤儿残骸；任何做原子重命名的构建工具都会踩中）。现掩码补齐 `DELETE | FILE_DELETE_CHILD`；Linux bwrap（`--bind`）与 macOS（`allow file-write*`）语义本就含删除，三平台对齐
- **顺手钉出两个 SEC-14 潜伏 FFI bug**（此前的 ACE 扫描从未真正工作过，全靠 `SetEntriesInAclW` 的 trustee 合并语义兜底才没出事）：① `ACL_SIZE_INFORMATION` 结构体字段序错误——`AceCount` 在 winnt.h 中是首字段，错序后读到的是 `AclBytesFree`(=0)，扫描永远看到 0 条 ACE；② `ACCESS_ALLOWED_ACE_TYPE` 常量错写成 1（那是 DENY）——allow-ACE 全被类型过滤器跳过。修复后：授权去重、掩码升级探测、SEC-14 项目删除回收（移除 ACE）首次真正可用
- **掩码感知的授权升级**：授权缓存键含掩码；DACL 扫描改按「缺失位」判断（`dacl_sid_missing_permissions`），旧版本窄掩码授权在下次命令时自动追加累积 ACE（Windows 对同 SID 的多条 allow-ACE 按位 OR）——存量项目目录无需手动迁移；+1 回归测试（真实 FFI：旧掩码授权→断言缺失位→升级→断言无缺失）
- **AC 内 `del` 端到端测试**：标记 `#[ignore]`——本机环境对 AC 进程的 DeleteFile 有 DACL 之外的额外拒绝层（完整性标签/EDR/嵌套沙箱），正确授权下仍无法通过；干净环境可手动 `cargo test appcontainer_delete -- --ignored` 复跑

### 🔧 其余三项修复

- **web_fetch max_bytes 错误文案**（报告问题 3）：「响应体过大」听起来像页面问题，实为调用方上限——改为「响应体超出 max_bytes 读取上限（{len}B > {limit}B）…可调大 max_bytes 重试」（报告 LLM 当时就误判了一次）
- **子智能体环境指引**（报告问题 1/2）：系统提示补两行——环境受阻（缺解释器/权限拒绝）时记录失败并继续、不反复重试（对应 tester 烧满 25 轮）；并行 worktree 同伴从同一 HEAD 分叉互相看不见（对应 writer/coder 并行时 README 误判 math_utils 未创建）；delegate_task 的 isolation 参数说明同步补充
- **`.aio/` 自动进 .gitignore**（报告遗留项）：git 项目添加时自动维护 `.gitignore`（无则创建、缺则追加、幂等、非 git 仓库不动），消除 `git status` 里永久的 `? .aio/` 噪音；+4 单测（创建/追加/幂等/非仓库）

### 🔧 依赖对齐：tauri 双端版本错配修复

- **Rust crate 升级对齐 npm 包**：tauri-build 的版本守门检查（NPM 包与 Rust crate 必须同 major/minor）在真实重建时拦截了 dev——`tauri` crate 停在 2.11.5 而 `@tauri-apps/api` 已随宽松范围 `^2.11.1` 漂到 2.12.3；`tauri-plugin-opener` crate 停在 2.5.4 而 npm 侧是 2.7.1（错配由早前 `npm audit fix` 类更新潜伏引入，因增量构建缓存一直未触发该检查）。现 crate 升至 **tauri 2.12.3**（=npm api 2.12.3）、**opener 2.7.1**（=npm 2.7.1）
- **npm 范围收紧防再漂移**：三个 `@tauri-apps/*` 运行时包从 `^`（允许 minor 漂移）收紧为 `~`（锁死 minor）——api `~2.12.3`、plugin-dialog `~2.7.1`、plugin-opener `~2.7.1`，杜绝 audit-fix 类操作再次单侧漂移触发此错配
- **验证**：cargo 全量 319 passed / clippy 0（tauri 2.12 无弃用 API）；`tauri dev` 真实重跑 build.rs 版本检查通过、app 正常启动

### 🔧 LSP 查询竞态修复（文档同步 + 诊断等待）

- **查询前同步文档到语言服务器**（harness 端到端测试发现）：AIO 此前从不发送 `textDocument/didOpen`，而语言服务器对「未分析过的文件」的符号/诊断查询返回空——懒启动后的首轮查询必然踩中（实测：pyright 拉起成功但 lsp_symbols 返回「文件中无符号」、read_lints 漏掉已埋好的类型错误；预热后同一引擎 CLI 立刻报出该错误，证明是竞态而非漏检）。现在 `lsp_symbols/definition/references/hover` 查询前会把文件当前磁盘内容 didOpen（首次）/didChange（后续，版本号递增）进服务器，服务器先分析再应答；read_lints 对显式指定的文件同样同步
- **read_lints 空诊断有界重试**：诊断经 publishDiagnostics 异步到达，查询赶在服务器（重）分析完成前会误报「无问题」；现为空且有活跃服务器时最多重试 3×700ms
- **LspClient 与 Tauri 运行时解耦**：事件（status/stderr/diagnostics）经注入的 `LspEventSink` 闭包上报（生产包一层 `app_event_sink`），集成测试传 no-op——插件层不再依赖 AppHandle 类型，也为测试铺路
- **+1 集成测试**：node 假 LSP 服务器（stdio JSON-RPC，完整握手）验证「未同步→空符号 / didOpen→符号可见 / didChange→仍可用」三段式回归；node 缺席时静默跳过

### 🔧 Agent 工具链修复（web_search / LSP）

- **web_search 换真实搜索引擎链**：原实现打的是 DuckDuckGo **Instant Answer API**（`api.duckduckgo.com`）——那是主题摘要查询接口而非搜索后端，绝大多数查询（尤其中文词）返回空载荷，导致 web_search 几乎每次都报「未找到相关结果」（工具可用性检查中暴露：连续 3 次不同查询均空）。现改为四级回退链：**Bing 结果页（主源，普通 HTTP 客户端即可用、国内免代理可达）→ DDG html 端点 → DDG lite 端点 → IA API 兜底**；搜索请求伪装成桌面浏览器形状（UA + Accept/Accept-Language，DDG html/lite 对非浏览器指纹会弹 JS 挑战页）；解析层支持三套标记（Bing `b_algo` 块 + DDG `result__a`/`result-link` 两种布局），统一解出 `uddg` 重定向与 Bing `/ck/a` base64url 重定向的真实 URL、过滤广告位（y.js/aclick/b_ad）、解码命名+数字 HTML 实体；响应 256KB 上限流式读取（SEC-08 同款）；+4 解析单测 + 真实端到端冒烟验证（「python 官方文档」返回带标题/URL/摘要的真结果）
- **LSP 工具按需启动语言服务器**：`lsp_definition/references/hover/symbols` 此前在服务器未运行时报「请先使用 auto_detect_ls 工具启动语言服务器」——但那是前端 Tauri 命令，Agent 工具集中不存在，整条链路死胡同（检查中确认 python 链不可用）。现改为按需懒启动：首次调用对应语言文件的 LSP 工具时自动 spawn 并完成握手（per-(项目,语言) 互斥锁防并发双启、启动成功上报 `lsp-server-status` 事件与前端自动启动路径对齐）；启动失败（典型：服务器未安装）报错附各服务器安装指引（pyright/rust-analyzer/tsserver/gopls）。后台记忆代码索引路径保持被动不拉起服务器；read_lints 空态提示同步修正为可行动指引
- **docs**：agent-tools（zh/en）LSP 段落更新为按需启动语义、web_search 描述更新为 Bing 优先

### 🎨 设置页 UI 微调

- **并行写入隔离下拉样式统一**：原生 `<select>`（黑底内联样式、与卡片风格脱节）替换为与上方语言选择一致的自定义下拉（同按钮/面板/选中项样式、选中项 check 图标、点击外部关闭、ARIA listbox 语义）
- **「自动」选项文案精简**：`自动（git 项目用独立工作区）` → `自动`（en: `Auto (worktree on git projects)` → `Auto`），说明语义仍保留在该行描述文案中

### 🔒 审查复核收尾 · 批次 2-6（供应商安全 / MCP 传输 / 引擎 / 数据面 / 同步命令异步化）

**批次 2 — 供应商安全与 SEC-06 残角**

- **PROV-5 host 精确匹配**：provider 分发谓词从全串 `contains` 改为解析后 host 后缀匹配（新增 `url_host_matches`）——`api.anthropic.com.evil.io` 等仿冒域名不再命中原生插件（历史行为：x-api-key 照发给任意主机）；anthropic/google/ollama 三插件谓词全部收紧（ollama 的 11434/ollama 标记同步限定在解析后的 host/port 上）；`is_anthropic_url` 保留（stream_options 体形状判定专用）但同获精确匹配；+2 回归测试（lookalike 全落 openai_compat 兜底、真子域名仍匹配）
- **SEC-06 残角清零**：vllm 的 `python3`/`python` 裸名候选改经 exec_resolve PATH-only 解析（cwd 植入 python.exe 劫持面消除）；ollama 两处「解析失败回退裸名 `ollama`」改为显式报错；2 个既有 vllm 测试断言随新语义更新（候选为 PATH 解析出的绝对路径）
- **PROV-11 同型收口**：image_gen 传输错误经 `redact_urls_in` 脱敏后再进聊天消息（reqwest Display 含完整 URL 查询串）；btw 畸形 SSE 行补 tracing::warn（与主循环对齐，不再静默丢弃）
- **SEC-18 补告警**：assistant 列表加载路径的 Off 回退（此前的静默分支）补 tracing::warn，与 load_assistant_agent_mode 主路径一致

**批次 3 — MCP 传输三残角**

- **MCP-5 buffer 上限**：HTTP SSE 缓冲 4MB 硬顶——不发块分隔符的流不再无界增长至 60s 超时
- **PROV-1 同型（MCP SSE）**：逐 chunk `from_utf8_lossy` 改字节级缓冲 `push_sse_chunk`（完整 UTF-8 序列才解码、非法序列硬错误、跨 chunk 多字节字符零 U+FFFD）——MCP 工具结果/通知携带 CJK 载荷不再被 TCP 分段切坏；+2 测试（穷举字节边界两段切分、非法序列报错）
- **MCP-9 批量节流**：stdio stderr 事件按 300ms 窗口合并（32 行/16KB 提前冲刷），服务器刷屏不再形成逐行事件风暴（前端监听本就只做 DEV 打印，合并行零信息损失）
- **MCP-19 stdio 字符串 id**：`JsonRpcResponse.id` 反序列化接受数字字符串（"id":"7"）——真实服务器以字符串回显 id 时不再静默丢弃挂到超时；非数字字符串仍无法关联（保持丢弃）；+1 stdio 关联回归测试

**批次 4 — 引擎与下载**

- **ENG-03 端口预检**：start_local_server 在停完自家引擎后、spawn 前对 127.0.0.1:port 做连接探测——端口被无关进程占用时报明确错误，不再伪装成健康检查失败（或更糟：健康探测打到外来监听者误报"引擎就绪"）
- **ENG-05 嵌入侧并发守卫**：embedding_model_download 同 id 并发下载拒绝（此前无条件 insert 覆盖旧控制项——第二个任务可取消/暂停第一个的状态，首个退出守卫又能误删继任者条目）
- **ENG-18 市场列举 revision 回退**：HF `tree/main`→`tree/master`、ModelScope `Revision=master`→`Revision=main` 主备候选逐一回退（与下载器同策略）——master-only 仓库此前 404 无法列举文件

**批次 5 — 数据面四残角**

- **PROV-12 catalog 坏缓存回退**：AppData 目录缓存加形状校验（可解析 + providers/models 数组齐全才可用），损坏缓存跳过并回退 bundled 目录——可读但损坏的缓存不再每次启动永久遮蔽内置目录
- **MEM-02 对称性测试补齐**：Supersede 执行计划抽为纯函数 `plan_from_judge`（ConsolidatePlan 枚举：KeepSeparate/Merge/Update/SupersedeLoser），无 LLM 即可测方向契约；+2 测试（Supersede 让位旧事实 anchor 与 tools/extract 契约同向；Update/Merge/KeepSeparate 映射面——含模型合并稿优先、空回退拼接）
- **MEM-7 并发 miss 双开连接修复**：MemoryStoreManager.open 改 `DashMap::entry` 原子路径——并发未命中不再各自开一条 SQLite 连接（败者连接被静默弃置）；+1 同实例复用测试
- **LOOP-10 锁外附件读取**：message_for_api 拆两阶段（`message_metadata_for_api` 锁内仅取附件元数据行 → 释放锁 → `finish_message_for_api` 锁外读盘+base64+组装）——run_agent_turn 与 call_llm_stream 两入口的全局 DB 锁不再横跨整个历史的图片读盘（每回合全历史附件重读期间其他 DB 使用者全部等待）

**批次 6 — HOR-4 同步重 I/O 命令异步化（批次二，12 命令）**

- **skill 族**：list_skills / save_skill / delete_skill（全局+项目级 skills.json 读改写）→ async + spawn_blocking
- **memory 导入导出**：memory_export_file / memory_import_file（全量事实导出 / 16MB 导入）→ async + spawn_blocking
- **provider_config 族**：load_provider_configs（每 provider 一次 keyring 往返+遗留明文迁移写回）/ save_provider_configs（每非空 key 一次 keyring 写+原子写）→ async + spawn_blocking（抽出 `load_provider_configs_internal` / `save_provider_configs_internal` pub(crate) 体，供本就跑 blocking 池的本地模型目录迁移复用——其调用点同步改走 internal，消除后续 future-未-await 隐患）
- **config 族**：save_app_config / load_app_config（keyring+fs）/ save_activated_models / load_activated_models（每模型 keyring 往返）/ set_auto_start（注册表/launch-agent 写）/ is_auto_start_enabled → async + spawn_blocking（set_auto_start 长体抽 `_inner`）
- 两处下载迁移调用点改直接调 core 层 `save_app_config`（原引用已变 async 的命令层包装）
- 暂缓：scan_installed_engines 已是 `#[tauri::command(async)]`（不占主线程），其盘探测量为几次 exists() 检查；迁入 blocking 池需 EngineManager 插件 Box→Arc 化重构，收益不抵改动

验证：cargo fmt/clippy -D warnings 0 / **cargo 312 passed**（+7 新测试）/ eslint 0 警告 / gates rgba=173≤173 any=77≤77

### 🔒 审查复核收尾 · 批次 1（前端 FE 项 + 门禁棘轮收紧）

- **FE-06 模块级 t() 冻结消除**：DiffView 的 ACTION_LABELS 与 SubagentModelSettings 的 BUILTIN_PROFILES 原为模块加载期求值对象——切语言后"新增/修改/删除"标签与 10 个内置子代理档案名**永久停留旧语言**。改为渲染期惰性求值（`actionLabel(action)` 函数 / `builtinProfiles()` 工厂，消费方 createMemo 因 t() 在其内被追踪而随 locale 重算）
- **FE-07 AppSettings 逐键击全量样板消除**：新增后端命令 `patch_app_config`（load→merge→原子 save 单次后端完成，跑 blocking 池）——设置页三个字段（最大工具轮数/并发子代理上限/并行写入隔离）原为每键击 load→改→save **两次 IPC** 且有读改写竞态，现改单次 patch IPC + 每字段 400ms 防抖合批（select 即时）+ 卸载清理挂起定时器
- **FE-10 收尾**：AppSettings 自启动开关（最后一个手写 peer-checked 复制件）收敛到 shared `Switch`（variant="glow"）
- **门禁棘轮随实际存量收紧**：RGBA_BASELINE 179 → 173（存量实为 173）；any 注释链补 77 终值
- 复核说明：本批出自对 CODE_REVIEW_REPORT.md 181 条的全量复核（13 路分板块核查代理 + 抽验），本批为复核发现的漏项/残角收尾第一批

### 🧱 阶段4 第2项（<900 目标全线达成）：ChatInterface 893 行（三组件抽出）

- **ChatInterface <900 达标（1118 → 893，-20%），本批三组件 verbatim 抽出**：
  - **`components/SubagentMenu.tsx`（~175 行）**：子代理上拉菜单——触发按钮（计数 + 运行中 spinner）+ 下拉列表（状态图标/profile 样式/名称任务/耗时）；**自带开关状态与 outside-click 关闭器**（原 closeOverlays 的菜单分支随迁，ChatInterface 只留分支下拉的关闭职责）
  - **`components/PendingFileChips.tsx`（~85 行）**：待发附件预览条（图片缩略/文件图标 + 名称 + 移除按钮；重复 pending 守卫下才丢弃后端附件）
  - **`components/DiagnosticsToggle.tsx`（~68 行）**：LSP 问题面板切换按钮（诊断信号全为模块级 store——零组件依赖）
- **阶段4 第2项 <900 目标完成**：ChatPage **845**、ChatInterface **893**（阶段起点 1746/2058，-52%/-57%）；报告点名的拆分路径（useChatAutoScroll/MessageBubble/useChatSend/useAttachments/usePanelResize/useBtwQuestion/apiMessages/failedTurnCleanup/skillMessages/sendMessagePrep/slashHandlers/useMessageSelection/useCheckpointResume/topicActions/SubagentMenu/PendingFileChips/DiagnosticsToggle）**全部落地**

### 🧱 阶段4 第2项（<900 目标达成）：ChatPage 845 行（选择模式/Checkpoint/Topic 生命周期三模块抽出）

- **ChatPage 达标 <900（1746 → 845，-52%）**，本批三模块抽出（1026→845）：
  - **`hooks/useMessageSelection.ts`（106 行）**：选择模式家族——4 个信号（选择中/已选集合/分享弹窗/分享源话题）+ shareTopic 解析 + 5 个 handler（进入/切换/全选/取消/确认）随钩子走；ShareModal 的三行 onClose 收敛为 `closeShareModal`
  - **`hooks/useCheckpointResume.ts`（65 行）**：崩溃遗留回合的"继续未完成"入口——activeCheckpoint 信号 + 话题切换竞态守卫效果 + handleResumeTurn（后端恢复时代换快照）
  - **`utils/topicActions.ts`（124 行）**：五个生命周期动作（停止生成[CHAT-4 流归属键]/切聊天/切项目[CHAT-3 五步同步]/加话题/从消息分支）——依赖全为模块级 store 单例，纯函数零组件依赖；`DEFAULT_ASST_ID` + `createTopic` 工厂随迁为导出（onMount 的默认助手引导继续引用）
- **any 棘轮 78 → 77**：branchFromMessage 的 `(a: any)` 注解经 setDatas 泛型推断消除
- **过程修正**：createTopic 首版与原实现不符（臆造 id/name 生成方式）——经 git 原文核对后按 verbatim 修正（`Date.now().toString()` + `t('chat.untitledTopic')` 本地化默认名）

### 🧱 阶段4 第2项（<900 目标切片）：slash 处理器抽出（slashHandlers）

- **slash 双 handler 抽出（ChatPage 1202→1026）**：prompt 型（/review、/explain 等——resolvedBody 发往 LLM）与 action 型（/compact、/clear、/search、/settings、/help——命令气泡 + handler 执行 + 逐命令反馈）verbatim 抽至 `utils/slashHandlers.ts`（195 行）；响应式依赖几乎全为模块级 store 单例直接导入，组件级依赖仅 3 项（currentAssistant/activeTopic/startAgentTurn）经 `SlashHandlerDeps` 传递
- **prompt 型复用 buildOutgoingMessages**：slash handler 内联的系统栈与主发送路径逐字段相同——直接复用上批抽出的 sendMessagePrep，顺带消灭其残留的 `any[]` messagesForAI 与 `agentMode as any`
- **any 棘轮 90 → 78（净降 12）**：两 handler 的 setDatas `(a: any)`/`(t: any)`/`(h: any[])` 注解经泛型推断全部消除
- **+6 反馈矩阵测试**：/clear、/help、/settings 静默返回（仅命令气泡）；/compact 自定义反馈优先 + 回退文案；/search 反映开关态；未知命令通用反馈

### 🧱 阶段4 第2项（<900 目标切片）：出站消息组装抽出（sendMessagePrep）

- **视觉预处理 + 消息组装抽出（ChatPage 1267→1202）**：handleSendMessage 中段（自动视觉分析——非视觉模型先由视觉 agent 生成结构化图像描述替换原始 base64 附件；系统栈组装——助手提示词 + 输出风格 + 技能 + agent 提示词 + 推理级别 + 联网 + 话题摘要 → 历史 → 新用户消息 + 工具配对安全网）verbatim 抽至 `utils/sendMessagePrep.ts`（174 行，两个入口 `resolveOutgoingContent` / `buildOutgoingMessages` 全参数显式化便于测试）+ **6 个组装顺序回归测试**（无条件系统栈顺序 / 条件段（推理/联网/摘要/agent 提示词）的含入与排除 / 持久化用户消息形状）
- **any 棘轮 93 → 90（净降 3）**：`messagesForAI` 从 `any[]` 类型化为 `Message[]`；`(m: any)` flatMap 注解与 `agentMode as any` 双双消除（`Parameters<typeof buildAgentSystemPrompt>[0]` 索引类型）；getReasoningPrompt 随迁为导出

### 🧱 阶段4 第1项（子目标5/5 切片4）：压缩守卫收敛（CompressCtx）

- **共享压缩上下文（agent_loop 切片 4）**：两循环 4 处同构的压缩调用模式（`std::mem::take` → `compress_context` 12 参调用 → 回填；主循环预算门 + 溢出门、子代理预算门 + 溢出门——后者还包着一层私有薄包装 `compress_sub_context`）收敛为 `round_artifacts.rs` 的 `CompressCtx`（借用式上下文：window/client/api_url/api_key/model/ids/token/keep_recent_tokens/tag/locale + `compress()` 单方法）。**溢出恢复门**（重试预算 + 错误分类 `is_context_overflow_error` + 最小长度 >4 三条件谓词）收敛为 `try_overflow_recovery` / `overflow_recoverable` 纯谓词（+1 测试覆盖三条件的全部反例）；子代理薄包装删除；主循环预算门的 8 变量 clone 块随之消失
- **控制流层判定（收尾立场，写入模块文档）**：工具执行三阶段调度（collect futures → FuturesUnordered → 串行回填）两循环**结构同构但分支内容是域语义**——主循环的 delegation/ask_user/任务板/file-diff 捕获 vs 子代理的 profile 门控/worktree/心跳事件；强行统一需要数十个钩子的策略对象，抽象成本高于重复成本。产物构造层（切片1-3）+ 压缩守卫（切片4）已承载全部**真同构机器**（累计 18 处内联消除、9 测试）

### 🧱 阶段4 第1项（子目标5/5 切片3）：工具结果展示三元组收敛

- **工具结果归一化共享（agent_loop 切片 3）**：`ToolResult → (前端文本, 结构化 JSON 值, is_error)` 的三元组转换在主循环内联重复 **3 处**（串行 MCP 分支 / 串行内建分支 / 并行 Phase-2 各 ~18 行）+ 子代理私有同构 `tool_result_text`（8 行文本提取）。全部收敛到 `round_artifacts.rs` 的 `tool_result_display` / `tool_result_text` 单一实现（含 serde 序列化形状回归测试）；两循环改走共享实现——共享构造面累计 **14 处内联消除，8 个回归测试**

### 🧱 阶段4 第2项（收官）：MessageBubble 拆分（ChatInterface 2057→1118，-46%）

- **消息气泡抽出（阶段4 第2项最后切片，报告点名的 MessageBubble 路径）**：ChatInterface 的 732 行 per-message JSX（头像/气泡/附件/agent 过程块/生成图像/token 尾注/操作按钮（复制/分支/重发/编辑/删除/批量撤销）/文件变更 diff 面板）verbatim 抽至 `components/MessageBubble.tsx`（795 行，<900 达标）。约 20 个原组件作用域依赖全部经 `MessageBubbleProps`（24 字段惰性访问器接口）注入——Solid 响应式语义零变化（JSX 内读取本就惰性）；工具名聚合/行号统计（aggregateFileChanges/aggregateDiffStats）与 `as any` 冗余转型（5 处——msg 本就是 any）随迁清理；ChatInterface **2057 → 1118 行**（-46%，本阶段起点 2058）
- **any 棘轮 97 → 93（净降 4）**：5 处 `as any` 转型（msg 已是 any，转型纯噪音）删除；新组件按门禁纪律全程类型化

### 🧱 阶段4 第1项（子目标5/5 首切片）：轮次产物构造收敛（round_artifacts）

- **两循环共享轮次产物（公共 agent_loop 地基）**：主循环（turn_runner.rs）与子代理循环（subagents.rs）各自手搓同一套轮次产物构造——assistant 消息追加（role/content/tool_calls）两处逐字段相同；role:tool 消息 4 字段构造在主循环内联重复 **3 次**、子代理 1 次变体；`[Error] {e}` 文本映射两循环各 2-3 处。全部收敛到新模块 `commands/llm/round_artifacts.rs`（assistant 追加 / tool_calls JSON / tool 消息推送 / 错误文本四个共享函数）+ 5 个形状回归测试；两循环 9 处内联构造改走共享实现（null_content 参数保留主循环的空串→JSON null 约定）——后续完整 agent_loop 引擎抽取的地基
- **截断续写守卫共享（次切片）**：两循环的 `finish_reason="length"` 截断续写块（同一条续写指令 + 有界重试 + continue）逐字段相同、仅重试上限不同（主循环 MAX_LENGTH_RETRIES / 子代理 1）——收敛为 `try_length_continuation`（含续写预算与条件判定），两处接入；+2 测试（预算耗尽不续写 / 非 length 或有工具调用不续写）
- **截断续写守卫共享（次切片）**：两循环的 `finish_reason="length"` 截断续写块（同一条续写指令 + 有界重试 + continue）逐字段相同、仅重试上限不同（主循环 MAX_LENGTH_RETRIES / 子代理 1）——收敛为 `try_length_continuation`（含续写预算与条件判定），两处接入；+2 测试（预算耗尽不续写 / 非 length 或有工具调用不续写）

### 🧱 阶段4 第2项（拆分切片6）：主发送路径接入 useChatSend + CHAT-10 清理抽出（+6 单测）

- **主发送路径迁移（阶段4 第2项 useChatSend 路径收尾）**：handleSendMessage 的 agent 分支（模型覆盖解析 + 14 字段载荷）迁至 `startAgentTurn` 原语；CHAT-10 占位气泡清理经 `onInvokeError` 回调注入（失败语义零变化：alert + thinking/typing 复位 + 清理，由原语统一执行）；chat 模式的 `callLlmStream`/`generateImage` 分支保留原 catch（同一清理函数复用）
- **CHAT-10 清理抽出 + 6 单测**：空占位符丢弃 / 部分流保留（running 步骤收口 complete + 时长补记 / 调用中工具步收口 error / 孤儿 calling 工具补中断标记 / 非助手末条与空历史恒等返回）——`utils/failedTurnCleanup.ts` 纯历史数组变换，此前内联于 catch 块零测试
- **any 棘轮 100 → 97（净降 3）**：清理模块与测试全程类型化（`ToolCallDisplay`/`AgentStep` 索引类型），迁移走出的内联 any 大于新增

### 🧱 阶段4 第2项（拆分切片5）：useChatSend 共享发送原语（slash-prompt/resume 接入）

- **run_agent_turn 载荷组装收敛（阶段4 第2项 useChatSend 路径）**：三条发送路径各重复一份 14 字段载荷组装（凭证+ID+模式+项目+联网+locale+上下文窗口+技能）+ thinking 生命周期（前置置位 / 失败 alert+复位）。新建 `hooks/useChatSend.ts` 的 `startAgentTurn` 原语：载荷组装与生命周期单点化，调用方只传各自差异项（messages/agentMode/skills + 可选 overrides/profiles/resumeTurnId/onInvokeError 清理回调）；**两个干净调用点先接入**（斜杠 prompt 发送、检查点续跑——后者失败路径顺带补上 typingIndex 复位）；主发送路径（含占位气泡清理的差异化 catch）留待下一批迁移；接口全程无 any（类型取自 `RunAgentTurnArgs`），any 棘轮保持 100

### 🧱 阶段4 第2项（拆分切片4）：附件暂存 → useAttachments 钩子

- **附件逻辑钩子化（阶段4 第2项 useAttachments 路径）**：`handleFileUpload`（扩展名白名单预校验——59 个扩展两表、附件 API 存储调用、去重暂存、图片 `convertFileSrc` 预览 URL）从 ChatPage 抽至 `hooks/useAttachments.ts`——**纯 verbatim 搬迁**（初版曾多造一个与 ChatInterface 上传命令重复的选对话框包装，已收敛删除：对话框调用留在既有调用点，钩子只管暂存）；ChatPage **1421 → 1327 行**（本拆分程序累计 1746 → 1327）

### 🧹 P3 魔法串：内建工具名常量表（core/tool_names）

- **工具名单一来源（P3 魔法串）**：`"read_file"`/`"write_file"`/`"execute_command"`/`"delegate_task"` 等 23 个内建工具名此前以字符串字面量散布 **14 个生产模块**（subagent 配置表、permission 内建规则、mod/turn_runner 分发与比较、file_locks/diff 写入分类、mcp 审批分类…共 **366 处**）——改名任一工具要全仓追字符串。新建 `core/tool_names.rs` 常量表（22 个 `pub const`，逐个带文档），全量替换为 `tool_names::X` 引用；**作用域精确**：MCP 服务器自定义工具名（运行时来源）不收编；glob 模式（`"delete_*"`/`"*_file"`）是模式字面量非工具名，保持原样；注释行不动（文档中的工具名引用仍是散文）

### 🧱 阶段4 第2项（拆分切片3）：/btw 侧问浮层 → useBtwQuestion 钩子

- **BTW 钩子化（阶段4 第2项拆分程序）**：`/btw` 侧问的浮层信号、流式应答（`btw-chunk` 事件逐 chunk 更新 overlay、unlisten 先注册防首 chunk 丢失、done/error/异常三路收口）、开问（生成 overlay + 启动应答）与关闭四个操作从 ChatPage 抽至 `hooks/useBtwQuestion.ts`（模型/助手/话题三个响应式访问器经参数注入）；ChatPage **1496 → 1422 行**（本拆分程序累计 1746 → 1422）

### 🧱 阶段4 第2项（拆分切片续）：API 载荷消息构建 → utils/apiMessages（+11 单测）

- **消息构建三件套抽出（阶段4 第2项 "useChatSend/useAttachments 路径"）**：`truncateToolResult`（20k 头尾截断）/ `buildApiMessages`（历史消息 → API 载荷展开；孤儿 toolCall 原位合成中断错误结果）/ `ensureToolMessagesComplete`（tool_call 配对守卫——后端 `verify_tool_messages` 仅告警，此为修复侧）三个纯函数从 ChatPage 抽至 `utils/apiMessages.ts`，**并补 11 个单测**（此前零测试的配对/展开/截断逻辑，含连续回合 pending 冲刷与末端冲刷两个此前完全无覆盖的分支）；ChatPage **1615 → 1496 行**

### 🧱 阶段4 第2项（拆分切片）：ChatPage 面板拖拽 → usePanelResize 钩子

- **usePanelResize 钩子（阶段4 第2项 "ChatPage 按已有拆分路径执行 <900 目标" 的首切片）**：左面板"幕帘"拖拽（双向 1:1 光标追踪 + 幕帘揭示宽度 + 释放 easeOutCubic 吸附 + rAF 动画取消）与右面板百分比拖拽的全部状态与逻辑（4 信号 + 3 常量 + 挂载监听 + 3 函数 ≈ 130 行）从 ChatPage 抽至 `hooks/usePanelResize.ts`——纯布局逻辑自成一体，依赖（持久化宽度/折叠态）经参数注入；ChatPage **1746 → 1616 行**（<900 目标继续按 useChatSend/useAttachments 路径推进）

### 🧱 阶段4 第1项（子目标4/5）：主循环闭包 → AgentTurnRunner

- **主循环闭包结构化（阶段4 第1项子目标）**：`run_agent_turn_inner` 内的 ~1230 行 `async move` 闭包此前隐式捕获 **24 个自由变量**（22 行 let-clone 捕获列表：窗口/应用句柄、取消令牌、流管理器、身份键、代次号、模型连接参数、预算……）——捕获集不可见、无结构、每加一个字段悄悄变长。新建 `commands/llm/turn_runner.rs`：`AgentTurnRunner` 结构体显式化全部 24 个捕获（字段即契约，带文档）；`run()` 顶部按原局部名解构重命名（`app: app_c`、`token: token_inner`、`turn_gen_id: turn_id_c`……），**1230 行循环体原样迁入零改动**；mod.rs 构造一次 runner 取代捕获列表。mod.rs **2285 → 1055 行**（较阶段起点 2755 已 -70%）；panic 兜底 epilogue（LOOP-6）与 StreamManager 注册侧所需的 `_g` 克隆保留在 mod.rs 并改从原参数取源；后续子目标（subagents 与主循环抽公共 agent_loop）以此为收敛面
- **顺手清理**：mod.rs 仅为循环体服务的 9 项子模块导入与 6 项工具导入随迁删除

### 🧹 P3 性能杂项（记忆检索）：暴力回退评分流式化

- **vector_search 回退路径流式评分（P3 性能杂项，索引 MEM-17/18 无详情、按代码热点核实）**：vec0 不可用时的暴力余弦回退此前先**整料化**全部解码向量（`Vec<(String, Vec<f32>)>`：n 次 id 克隆 + n 个堆向量），再交给 `brute_force_topk` 评分时**再克隆一次**全部 id、排序后只留 k 条——两轮全量分配/克隆仅为了取 top-k。改为行循环内直接解码评分（每行解码作用域化、id 只触一次），行错误保持 `collect_rows` 同款跳过+告警语义；`brute_force_topk`/`bytes_to_f32` 的失用导入随删（topk 本体保留供代码检索共用）

### 🧹 P3 前端打磨（续）：console 中文硬编码清理（66 处）

- **console 消息语言统一（P3）**：console.log/warn/error 的 66 处中文硬编码消息全部翻译为英文——console 是开发者日志非 UI 文案（i18n 不适用），项目既有约定即英文（`[shortcuts]`/`[models]`/`[openSafeUrl]` 等标签本为英文，混杂中文造成日志风格漂移）；逐条人工映射翻译（保留 `[tag]` 前缀、错误对象参数与模板变量），组件/核心/共享层覆盖 25 个文件；扫描器复核归零

### 🧱 阶段4 第1项（子目标3/5）：delegate 参数打包工厂（DelegateContext）

- **继承执行上下文打包（阶段4 第1项子目标）**：`handle_delegate_task` / `handle_delegate_tasks` 各持 17 个独立参数，其中 15 个是主循环逐项克隆再按位传的"继承执行上下文"——主循环三处调用点（后台 fire-and-forget 委派 / 单任务并行委派 / 批量并行委派）各自维护一份 17 行克隆瀑布 + 参数瀑布，每加一个上下文字段要同步改 2 处函数签名 + 3 处调用点 + 1 处 workflow 转发。新建 `commands/llm/delegate_ctx.rs` 的 `DelegateContext`（owned、可 Clone/move 进 spawn）：两函数签名收敛为 `(ctx, arguments)`，函数体顶部一次性本地绑定（其余零改动）；三处调用点以单次 ctx 构造替代克隆瀑布；workflow 步骤转发同切——**新增上下文字段只改一个结构体定义**

### 🧱 阶段4 第1项（子目标2/5）：stream_one_round → streaming.rs

- **SSE 流轮次搬家（阶段4 第1项子目标）**：`stream_one_round`（请求构造 → POST → SSE 累积 → 事件下发，含 Anthropic 适配、工具调用累积、token usage 提取、图像生成收集）连同其累积产物 `RoundResult` / `ToolCallAccum` 从 mod.rs 整体搬到新文件 `commands/llm/streaming.rs`（442 行）——mod.rs 只留主循环编排，**2755 → 2325 行**（file-size 门禁 2770 内余量扩大）；字段 `pub(crate)` 化（主循环/subagents 直接读取）；subagents 复用入口随之切到 `super::streaming::stream_one_round`
- **顺手清理**：`ToolCallPayload` 的 mod.rs re-export（声称"保持路径稳定"但全仓无消费者）删除；搬家后 mod.rs 失用的 BTreeMap 导入删除

### 🧱 阶段4 第1项（子目标1/5）：消 14 模块 glob re-export（commands/llm）

- **消除“同一符号两条路径”（阶段4 第1项子目标）**：`commands/llm/mod.rs` 的 14 个 `pub(crate) use <module>::*;` glob re-export 全部移除——它们使每个子模块符号同时存在于 `llm::X` 与 `llm::<module>::X` 两条路径，且 8 个子模块又以 `use crate::commands::llm::*;` **glob 引回父模块**（连带吸入 mod.rs 的全部工具 use 头与 trait 导入），构成父⇄子循环吸入结构——正是同构漂移与遮蔽的温床
- **lib.rs**：9 个经 glob 注册的命令改全路径（`llm::titles::summarize_history`、`llm::usage::count_tokens_cmd` 等，与 checkpoints/tasks 既有风格一致）；对应 9 个命令函数 `pub(crate)` → `pub`
- **子模块显式 use 化**：8 个子模块的父 glob 替换为逐符号显式 use（真实来源模块路径）；mod.rs 自身使用的子模块符号改为头部显式 use 块；btw.rs 的 `super::streaming_http_client` 等改 `super::client::`；`is_git_repo` 升 `pub(crate)`（subagents 复用）；顺手清理各文件被 glob 掩蔽的 unused 导入

### 🧪 阶段5 第4批：并发 run_agent_turn 换代测试（阶段5第3项全部完成）

- **换代协议收敛 + 并发换代集成测试（报告阶段5第3项）**：流任务换代协议（注册 / replace_generation 取旧代→cancel→限时等待收尾 / remove_own 身份注销 / stop 取消）此前内联在 call_llm_stream 与 run_agent_turn 两处调用点（各一份 remove→cancel→wait + insert + remove_if，含 LOOP-6 panic 兜底 epilogue 的第三份 remove_if）——收进 `StreamManager` 成为单一来源（`state.rs`），六处调用点随迁。三个真实 spawn 任务 + CancellationToken 驱动的并发测试：**A** 协作旧代在等待窗口内完成收尾与自注销，新代注册后 stop 可找到并取消；**B** 旧代越窗滞留（卡在长命令）后补跑收尾——身份注销不腐蚀新代条目、stop 仍能找到新代（LOOP-1 核心回归：历史按键注销会把新代删成孤儿）；**C** 身份注销矩阵：持有者注销命中、他人 turn_id 注销零效果
- **tokio test-util dev-dependency**：`start_paused` 暂停时钟使睡眠时序自动推进——换代窗口时序测试确定且瞬时（3 测试共 0.02s）；仅测试构建合入特性，发布构建运行时不变

### 🧪 阶段5 第3批：下载中途 kill 恢复集成测试（共享下载引擎）

- **kill 恢复五场景集成测试（报告阶段5第3项）**：用 `http::Response` 合成 206/200 响应（`reqwest::Response: From<http::Response>`，`http` 声明为 dev-dependency——已在依赖树内零新增编译）直接驱动共享下载引擎，无需真实 HTTP 服务器。**A** 进程 kill 后重试：`.part` 检查点 + ETag sidecar 落盘 → 206 续传 → 追加 → 终长核对 → 原子 rename 出完整文件（kill 的字节一分不丢）；**B** 服务器中途断流（`wrap_stream` 未知长度 + Content-Length 虚高头，忠实模拟"头已声明、body 未送完"）：报"下载不完整"且检查点保留前缀+已收字节，绝不 rename 半截文件；**C** 取消：`.part` 与 `.etag` 一并清理；**D** 暂停：检查点保留全部已收字节供下次续传；**E** 全新下载（服务器忽略 Range 回 200）：truncate 分支，陈旧检查点被整体覆盖不与响应拼接
- **引擎 emit 参数化**：`stream_to_checkpoint` 的 `&tauri::Window` 改为 `impl Fn(&str, Value)` emit 闭包——无 Tauri app 也可测试引擎（测试驱动正是此次集成测试的前提）；命令层两处调用点以闭包包装 window

### 🧱 阶段4 第3项主体：MCP HTTP 请求统一 JSON-RPC 出口（与 stdio 同语义）

- **HTTP `rpc_call`/`rpc_notify` 统一出口（阶段4 第3项，根除 MCP-4/5 类分叉）**：HTTP 传输此前 7 个方法（initialize / list_tools / call_tool / list_resources / read_resource / list_prompts / get_prompt）各持一份手搓 `json!` 请求体 + 各自的 envelope 拆包——正是 MCP-4/5/19 类两传输语义分叉的温床。收敛为与 stdio `McpConnection::request` 同语义的单一出口：state 查找、id 生成、**共享 `JsonRpcRequest`/`JsonRpcNotification` 编解码**（两传输线格式单一来源）、超时包装、result 剥壳（缺 result → Null 同 stdio）全部单点实现
- **错误语义对齐**：`rpc_error` 补 -32601 → `MethodNotFound` 映射（stdio 已有；http 此前一律 `Remote`——两传输间的活语义分叉），其余错误码保持 `Remote`；畸形 error 对象降级默认值（-32603/unknown error）
- **回归测试**：-32601 → MethodNotFound / 非 -32601 → Remote / 畸形 error 默认值三态断言

### 🧱 阶段4 第5项：ProviderPlugin 聊天路径统一出口 + 双注册表消除

- **聊天路径统一出口（阶段4 第5项）**：`post_chat_completion` 此前在 Anthropic 分支硬编码 `x-api-key`/`anthropic-version` 头、else 分支硬编码 Bearer，并用 `is_anthropic_url` 做第二套 host 匹配（与插件注册表重复）。现在整个请求形状全部走插件系统：`chat_completions_url` 构端点（原有）、新增 trait 方法 **`apply_chat_auth`** 挂认证（默认 Bearer——OpenAI 兼容约定，空 key 不发头；Anthropic 覆写为其原生双头；Ollama 覆写为无认证）、Anthropic 判定改为 `plugin.identifier()` 注册表分发。Google 特殊性显式化：models 走原生端点（x-goog-api-key）而 chat 走 OpenAI 兼容端点（Bearer 文档形式）——apply_auth/apply_chat_auth 两方法分离使两个 API 面的差异成为类型系统内的一等公民，Google chat 行为零变化
- **boxed_for_url 双注册表消除（阶段4 第5项）**：Manager 改持 `Vec<Arc<dyn ProviderPlugin>>`，`for_url` 返回 **Arc 克隆**（插件自然超越管理器存活）——旧 `boxed_for_url` 按 identifier 重建 Box 的第二注册表整体删除（加新插件曾要同时登记两处，且对"已注册但未重建映射"的插件会静默误分发到兜底）；`provider_config.rs` 两处调用点随迁
- **回归测试**：新注册插件经 `for_url` 即刻生效且 Arc 超越管理器存活——旧双注册表对同一 URL 会错误重建出 openai_compat，该缺陷随测试固化为历史注记

### 🧱 阶段4 第6项（续）：core/utils/cache.ts SWR 缓存工厂

- **共享 localStorage JSON 缓存工厂（阶段4 第6项收尾）**：新建 `core/utils/cache.ts` 的 `createJsonCache`——读（惰性 getItem + JSON.parse + 形状校验降级 null + **解析备忘**：每次加载只解析一次，write 后刷新）、写（stringify + setItem，配额/禁用错误吞掉——缓存尽力而为不阻塞功能）、`isFresh`（三市场共用的 6h TTL 判定）。三个市场页各持的手写 readCache/writeCache 副本全部收敛：SkillList（多桶 + 桶时间戳 + 模式演进回填）、McpServerList（单对象）、LocalModelMarket（FE-17 手搓解析备忘被工厂取代）；5 个单测覆盖读写回路/损坏降级/备忘语义/配额吞错/TTL 边界（Node 环境注入内存 localStorage stub）

### 🧱 阶段4 第6项：设置层样板收敛（shared Switch glow 变体 + shared Toast）

- **共享发光开关（阶段4 第6项）**：设置页的内联发光开关（h-[22px] w-[40px] 半透明轨道 + 开启态主色辉光 + 4px 白色旋钮）此前是 6 份手工内联副本（ProviderList ×3、ProviderDetailPage、EmbeddingModelSettings、ModelRow）——为共享 Switch 组件加 `variant="glow"` 与 `stopPropagation` 属性后全部收敛；ProviderDetailPage 原"开关嵌在 label 按钮里"（button 嵌 button 的无效 HTML）重构为并排结构；rgba 字面量棘轮顺带 179→173
- **共享 Toast（阶段4 第6项）**：新建 `shared/components/Toast.tsx`——`useToast()` 钩子（信号 + 自动消失定时器）与 `ToastBubble` 气泡组件，替换 ProviderDetailPage / ProviderList 两页各自持的 toast 信号 + 19 处 `setToast({...})` + `setTimeout(setToast(null))` 内联对 + 两块逐字符相同的 JSX 气泡；顺带修两类定时器缺陷：旧 3s 定时器不再截断新 toast（替换时先清定时器）、卸载后不再触发悬空定时器（onCleanup 清理）

### 🧱 阶段4 第4项：下载器同构收敛（services::download 单一实现）

- **共享下载引擎（阶段4 第4项）**：`commands/model_download.rs`（嵌入/重排模型）与 `commands/local_model_download.rs`（本地 GGUF 市场下载）各持一份同构机器——per-id 控制表（pause/cancel 旗标）、控制注销守卫、流复制尾段（append/truncate → 分块循环 → 节流进度 → 暂停/取消轮询 → flush → ENG-04 终长核对 → 原子 rename），两副本已发生过一次漂移（ENG-5 身份匹配注销曾只存在于 GGUF 一侧）。全部收敛到新建 `services/download.rs` 单一实现：两下载器只保留域特有逻辑（多源回退 / If-Range+ETag / ENG-11 空间预检 / GGUF 魔数校验）；嵌入侧的注销守卫顺带升级为 ENG-5 身份匹配版（原版无条件移除）
- **回归测试迁移**：ENG-5 身份匹配注销测试上移共享模块（覆盖更全：注册→继任覆盖→前任退出不误删→继任可置位→继任退出正常清理→signal 空表返回 false）

### 🧪 阶段5 第2批：SSE/恶意 server 集成测试 + BTW 乱码修复

- **SSE 跨 chunk UTF-8 切割流测试（报告阶段5第3项）**：新增 `commands/llm/sse_lines.rs`——把主循环内联的字节级行缓冲抽为 `SseLineBuffer` 纯模块；穷举回归测试把含中文与 4 字节 emoji 的多行流按**每个字节边界**两段切分（外加逐字节喂入），重组行必须与整段喂入一致——任何边界产生 U+FFFD 即失败（PROV-1 的穷举防线）；另补 LOOP-9 超限与 CRLF/trim 行为测试
- **BTW 侧问乱码修复（PROV-1 漏网实例）**：`btw.rs` 此前仍逐 chunk `from_utf8_lossy`——中文 BTW 回答被 TCP 切开即乱码且无行长上限；改用共享 `SseLineBuffer`（同修双实现漂移）
- **恶意/边缘 MCP server 台本测试（报告阶段5第3项）**：http 传输的块消费循环抽为 `drain_sse_blocks` 纯函数，新增三个台本测试按真实消费模式（逐 chunk 喂入、块跨分段边界）演练：心跳先行→通知块→他人数字/字符串 id→我方响应命中且尾部不完整块留缓冲不误消费；我方 id 的 error 块穿透心跳浮出为 Remote 错误；流提前结束返回 None

### 🧱 阶段4/5 推进：记忆统计批量化 + MCP initialize 解析统一（P3 性能杂项 MEM-14 + 阶段4 同构收敛）

- **记忆统计批量化（MEM-14）**：`stats()` 的 5 条独立 COUNT 合并为单条条件聚合查询；8 个滚动计数器读取合并到单次锁持有（新增 `rolling_get_with` 连接内读取助手）；`bump_access` 的逐 id UPDATE 循环改为分块 `IN (...)` 批量更新（每次检索命中列表从 N 条语句降为 N/100 条）
- **MCP initialize 解析统一（阶段4 第3项子目标）**：stdio/http 两份同构的 serverInfo/capabilities 解析并入 `convert::parse_server_info`（HTTP 侧先剥 JSON-RPC envelope 的 result 层再走共享实现）——两传输从此不会漂移；补缺字段降级行为回归测试

### 🧱 阶段5：门禁与测试补强（审查修复计划第 5 阶段，部分完成）

- **事件登记全量守门**：新增 `events.test.ts` 双端一致性测试——解析 `events.rs` 的全部 `pub const` 事件与 `events.ts` 的 EVENTS 映射做双向比对（后端有前端无=死常量、前端有后端无=死监听），外加模块形状校验（重复值/幽灵导出）；任何一端漏登记立即红
- **颜色门禁扩展 rgba/rgb（2b）**：数字字面 `rgba(...)`/`rgb(...)` 与 #hex 同罪，但存量 179 处以棘轮收编（只降不升）；`rgba(var(--...))` 变量形式不计
- **英文文案启发式门禁（1b）**：裸英文 JSX 文本节点（`>English prose<`，≥2 词）判违——首个命中 `Official MCP Registry` 已改走 `t()`（新键 `mcp.detail.officialRegistry`）；模式刻意收窄，import/注释/tailwind class 不误报
- **分层门禁扩至 utils（FE-25）**：`core/utils/` 禁止裸 invoke——Tauri 封装属 `core/api/`；`utils/memory.ts`（40 个 memory/embedding/rerank 命令封装）整体迁至 `core/api/memory.ts`，`utils/models.ts` 的目录 invoke 下沉到新建 `core/api/catalog.ts`（utils 保留缓存+纯查询逻辑，类型 re-export 保持外部导入路径不变）
- **any 棘轮收编 163 → 100（FE-21 部分）**：新增 `core/api/streamPayloads.ts`（11 个流事件载荷类型，镜像 `chat.rs` 的 serde 结构），chatEventListeners 的 11 处 `listen<any>` 全部类型化；Solid store 路径谓词去掉 `any` 注解让路径推断接管；补 `ToolCallDisplay.content` 字段（运行时一直写入而类型缺失）

### 🔒 全仓深度审查修复 · 阶段 3（剩余 P2 全量：SEC/LOOP/MCP/MEM/PROV/ENG/INFRA/CHAT/FE/HOR）

**批次 1 — 安全读写与进程边界（SEC-04/06/08/09 + INFRA-3）**

- **文件读取上限（SEC-04）**：file_tools 全部读取路径加上限——`read_file` 32MB 硬顶 + 流式 SHA-256（多 GB 文件不再整读入内存再截断）；`write_file` expect_hash 校验与 `replace_in_file` 改流式哈希 + 尺寸门；`search_content` 每文件 1MB 扫描上限；diff 预览源读取 2MB 上限
- **可执行文件 PATH-only 解析（SEC-06）**：新增 `utils/exec_resolve`（绝不查当前目录；Windows PATHEXT 顺序归一 EXE 优先），替换全部裸名 `Command::new` spawn——git 工具/命令执行 shell/注册表 reg/资源管理器 explorer/skill npm/npx/MCP catalog 运行时检查/MCP stdio/LSP 客户端/引擎探测（where/which/sc/nvidia-smi/python）与沙箱包装器（bwrap/sandbox-exec）；三处重复的 PATHEXT 解析实现合并为单一实现（Windows CreateProcess 对裸名先查应用目录与当前目录——Agent 命令的 cwd 是模型可写的项目根，植入 `git.exe` 即劫持）
- **web_fetch 流式接收（SEC-08）**：响应体改 Content-Length 预检 + chunk 累积超限即中止，不再先整缓冲再限长
- **命令触碰标记（SEC-09）**：`execute_command` 成功后对项目根做有界 mtime 扫描（跳过 .git/node_modules 等，10 万条目/5000 标记上限），窗口内修改的文件记为 "command" 写入者——后续任何 Agent 写入被 hashline 闸门强制先重读（格式化/构建/代码生成命令静默失效模型陈旧上下文的丢失更新面）
- **zip 炸弹上限（INFRA-3）**：docx/pptx 条目解压 64MB/总量 256MB 上限，谎报小尺寸实际解压超限即中止

**批次 2 — 循环取消与注入隔离（LOOP-5/11/12）**

- **三处等待全部取消响应（LOOP-5）**：Phase-2 并行工具等待加 token select（停止后不再被慢速子任务钉住）；delegate_task 单发与批量并发的信号量排队等待均加 token select（排队中立即释放）
- **工具输出隔离标记（LOOP-11）**：工具结果统一以 `<tool_result>...</tool_result>` 哨兵包裹（单一咽喉点 truncate_tool_result），载荷内字面哨兵以零宽空格中和防伪造闭合标签逃逸；主/子 Agent 系统提示词同步声明不可信数据规则
- **create_workflow 步骤上限（LOOP-12）**：20 步硬顶（此前无上限，N 步=N 个完整子 Agent 回合，架空主循环 50 轮预算）

**批次 3 — MCP（MCP-12/14）**

- **连接池指纹键（MCP-12）**：池键改 `id::指纹前缀`（生效配置 transport 摘要）——项目级 overlay 给同一 id 不同 transport 时各项目各自连接互不串用；start 只重启同指纹变体，stop/remove 清全部变体；状态/工具列举按指纹感知查找
- **catalog 注入面（MCP-14）**：注册表数据进入 npx/uvx argv 前全量校验——包标识/版本（拒绝 `1.0 --flag` 冒充）、runtime 参数（只许长旗标与裸值）；env 名称 POSIX 文法 + 加载器/包管理器配置拒绝清单（NODE_OPTIONS/npm_config__/PIP__/UV_*）；header 名 token 文法；明文 http remote 仅限环回；安装路径对 argv 与 env/header 二次防御

**批次 4 — 记忆子系统（MEM-06/07/08/09/11）**

- **embed_one 单一实现（MEM-06）**：四处复制粘贴收敛为 `services::memory::embed_one`（统一维度漂移守卫——库内向量维度与当前模型不一致跳过向量化，杜绝写入永远查不到的死向量）；模型切换探测改用无守卫 `embed_raw`
- **连接缓存真实 LRU（MEM-7）**：超上限必淘汰最久未触条目（原仅 30 分钟 TTL 过期淘汰，全员"热"时无上界增长）
- **代码索引四项（MEM-08）**：目录扫描/元数据/读取全部 spawn_blocking；读取项目根 .gitignore（名称级/目录前缀/整路径三类模式）过滤；file_type 不追符号链接（杜绝符号链接环死循环与别名重复索引）；索引后清理磁盘已删文件的残留 chunks/FTS/状态行
- **整理互斥原子化（MEM-09）**：单语句 INSERT…ON CONFLICT…WHERE 认领（消除读后写窗口内两次整理并发双花预算）
- **检索锁与 N+1（MEM-11）**：同步 SQLite 检索移入 spawn_blocking；详情装配改批量 IN 查询（每 100 候选一条，消除逐 id 往返）

**批次 5 — Provider（PROV-8/9/10/11）**

- **嵌入模型缓存失败策略（PROV-8）**：实例缓存只存成功结果——失败（未下载/损坏）不再冻结到重启，下载后下次调用即可用
- **ONNX 加载下沉（PROV-9）**：embed/rerank 数百 MB 模型读盘+图构建移入 spawn_blocking，不再占用 async worker
- **重排全量打分（PROV-10）**：top_k 由硬编码 16 改整个候选集（上限 50）——原第 17+ 名保留 RRF 原始分（~0.01）与交叉编码器分（~0.0-1.0）混排，量纲不一致
- **URL 脱敏（PROV-11）**：错误消息与日志中的 URL 一律去查询串（Gemini `?key=...` 不再进入 toast/日志）；`classify_reqwest_error` 双脱敏；`post_chat_completion` 保留重试判定所需错误前缀

**批次 6 — 引擎与下载（ENG-05/08/10/11）**

- **下载控制表并发（ENG-05）**：同 id 并发下载拒绝；退出守卫按 cancel Arc 指针身份匹配只清理自己的控制项
- **引擎启动互斥（ENG-08）**：`start_local_server` 全程持锁——"停旧再启"序列原子化，两个并发启动不再同时存活
- **迁移悬空引用（ENG-10）**：目录迁移只重写实际迁移成功的 provider 路径（原无条件重写制造悬空引用）
- **空间与并发（ENG-11）**：全局并发下载上限（2 槽位，排队响应取消）；每文件 Content-Length 已知时权威空间检查（元数据 size=0 此前完全绕过预检）

**批次 7 — 前端流（CHAT-7/9/10/11/12）**

- **流状态全局化（CHAT-12）**：isThinking/typingIndex/pendingApprovals/pendingQuestions 迁至 `core/store/stream.ts`；18 个流事件监听器改由根 Layout 一次性注册存活全程（仅 3 个拖放监听器页面级）——中途切 /settings 不再销毁监听器导致流内容丢失、done 收尾（落盘/重命名/压缩）永不执行
- **流归属键批处理（CHAT-7）**：按 assistant::topic 分槽，目标消息 flush 时按 ID 重解析（快照下标在压缩裁剪后错位）
- **Reasoning 合批（CHAT-9）**：与 llmChunk 共用 flush 窗口，一次 setDatas 同时落 content/reasoning/agentSteps（原每 chunk 两次 setDatas + 全量步骤重建）
- **孤儿占位清理（CHAT-10）**：invoke 失败删除全空占位气泡、部分输出关闭运行步骤并标记中断（原留永久"思考中"空气泡）
- **重发停止等待（CHAT-11）**：150ms 魔法等待改轮询 isThinking（5s 上限），慢速收尾不再与新回合流交错

**批次 8 — 设置页（FE-04/06/08/09/12）**

- **持久化三层修复（FE-04）**：写操作串行化（排队任务执行时重读配置，读改写不再竞态丢更新）；文本字段 400ms 防抖合批；Ollama 5 秒轮询先比对内容无变化不写盘；离页冲刷未落盘内容
- **i18n/文案（FE-06/08/09）**：命令面板补 new-project 翻译条目（切换语言后该命令冻结在启动 locale）；Markdown 复制按钮 aria-label 走 t()；停止引擎失败裸错误改 i18n 包装
- **明文密钥不落盘（FE-12）**：profile 模型覆盖保存时 apiKey 前端传空、后端落盘前清空（运行时密钥每轮从供应商配置新鲜解析）

**批次 9 — 插件面与同步命令（HOR-2/4）**

- **能力收窄（HOR-2）**：移除 fs/shell 两个 Tauri 插件（前端零使用，全栈后端→注册即攻击面）；修复 opener 从未注册的真实 bug（safeUrl 调用此前运行时必失败）；asset 协议作用域从通配 `**` 收窄为 `$APPDATA/attachments/**`（与 tauri.conf.json 一致）
- **同步命令异步化（HOR-4/ENG-09）**：git 全部 5 命令（spawn git 子进程）、硬件档案探测、模型删除/列表/目录迁移、嵌入模型删除/迁移/取消清理——全部改 async + spawn_blocking（Tauri 同步命令跑主线程，此前大仓库分支查询/多 GB 目录删除直接卡 UI）

**批次 10 — 阶段 3 收尾（SEC-03R/05 + LOOP-7/8 + MCP-10）**

- **web_fetch DNS 解析校验 + IP 固定（SEC-03R）**：每跳先解析并校验全部地址为公网单播（私网/保留/metadata 段全拒），再经 ClientBuilder::resolve 固定到已验证 IP——封掉"域名指向内网"绕过与 check-then-connect 重绑定窗口
- **写入前目标复检（SEC-05）**：四个变更类文件工具写/删/建前复检规范化路径与首验一致（junction 中途替换改变规范化路径即拒绝），TOCTOU 窗口从整个工具体收窄到微秒级
- **检查点锁外序列化（LOOP-7）**：主循环先序列化再取 SQLite 锁（多 MB 上下文序列化不再阻塞全部数据库使用者）
- **精确 token 计数下沉（LOOP-8）**：近预算 tiktoken 全上下文编码（主/子两处）移入 spawn_blocking
- **stdio 崩溃自愈（MCP-10）**：stdout EOF/连续读错误按连接实例身份从池中精确移除（同 id 其他项目活变体保留）并广播断开——原状态表永久残留"已连接"、后续调用全部挂到超时

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

# Harness 工作台（CLI 工具兼容层）

Harness 工作台让 AIO 作为各类 CLI 编码智能体（Claude Code、Codex、OpenCode、Oh My Pi、Gemini CLI、Qwen Code、goose、DSH 等）的图形化兼容层：在 AIO 内驱动这些 CLI 工具干活，并把它们的消息记录、思维链、工具调用与文件改动以 AIO 原生界面呈现。

## 边界：AIO 在这里做什么

- **AIO 负责**：启动与监管 CLI 工具进程、把其事件流映射为原生 UI、转发用户输入与审批决定、在本机保存显示用的会话记录。
- **AIO 不负责**：该场景下 AIO 自身的工具集、沙箱、记忆、技能与子智能体完全不参与。工具执行与权限控制完全由所选 CLI 工具自身承担（例如其自带的沙箱模式、权限询问或 `--yolo` 类配置）。
- AIO 不注入任何系统提示词，也不改写 CLI 工具的行为；对话内容原样往返。

## 页面结构

页面与对话页同构，导航栏「终端」图标进入：

- **左侧两级栏**：一级为检测到的 CLI 工具（各带品牌 logo 与状态点：绿=已就绪、黄=可自动安装、灰=未安装；悬停工具行可展开/收起其工作区列表、添加工作区，自定义工具可删除）；二级为该工具下注册的工作区目录——每个工具的工作区列表可像对话页子话题一样独立收起/展开，折叠状态在重启后保留，添加工作区时自动展开。
- **中间消息区**：上方状态栏显示工具 / 工作区、运行状态（连接中 / 运行中 / 空闲 / 出错）与 Token 读数；消息气泡复用对话页组件（Markdown、思考块、Agent 过程块、Diff 视图）。
- **右侧话题栏**：当前（工具 × 工作区）下的话题列表，支持新建、重命名、删除。

话题标题默认「新话题」，首次发送消息后自动截取消息开头命名；也可手动重命名。

## 支持的 CLI 工具与安装

除 DSH 外均需要本机安装 Node.js 20+（`npx` 可用）。标注「自动安装」的工具在 AIO 内首次使用时经 `npx -y` 自动拉取适配器包，首次可能需要数十秒下载。

| 工具        | 接入方式                                                      | 安装命令（终端执行）                                                                                                                                                                                                                             |
| ----------- | ------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Claude Code | 原生驱动优先；未探测到原生二进制时回退官方 ACP 适配器（npm）  | 推荐：`irm https://claude.ai/install.ps1 \| iex`（官方安装器，自动更新）；或 `npm i -g @anthropic-ai/claude-code`（**同样走原生驱动**——AIO 会定位包内的真实二进制）。两种方式都需自行完成 `claude` 登录；登录状态（`~/.claude`）在两种安装间共享 |
| Codex       | 原生驱动优先（`codex` CLI）；未装时回退官方 ACP 适配器（npm） | `npm i -g @openai/codex`（或 winget `OpenAI.Codex`），并在终端完成 `codex` 登录；或 `npm i -g @agentclientprotocol/codex-acp`（并完成 `codex` 登录）                                                                                             |
| OpenCode    | 原生 ACP（`opencode acp`）                                    | `npm i -g opencode-ai` 或 `curl -fsSL https://opencode.ai/install \| bash`                                                                                                                                                                       |
| Oh My Pi    | 原生 ACP（`omp acp`）                                         | `irm https://omp.sh/install.ps1 \| iex`                                                                                                                                                                                                          |
| Gemini CLI  | 原生 ACP（`gemini --acp`）                                    | `npm i -g @google/gemini-cli`                                                                                                                                                                                                                    |
| Qwen Code   | 原生 ACP（`qwen --acp`）                                      | `npm i -g @qwen-code/qwen-code`                                                                                                                                                                                                                  |
| goose       | 原生 ACP（`goose acp`）                                       | 参见 goose 官方安装文档                                                                                                                                                                                                                          |
| DSH         | 原生 ACP（`dsh --profile acp`）                               | `npm i -g @deepseek-ai/dsh`                                                                                                                                                                                                                      |

暂不支持 crush（无 ACP/机器可读输出）与 aider（纯文本输出、无会话恢复，上游已停滞）。

### 自定义 ACP 工具

任何「说 ACP 协议」的 CLI 都能注册为 harness 工具：左侧栏底部「添加自定义 ACP 工具」填写显示名称、命令与参数（每行一个，如 `my-agent` + `acp`）即可。自定义工具与内置工具完全同权（工作区/话题/审批/恢复），可随时删除。命令由你自己编写，与 MCP stdio 服务器同等的信任边界。

## 使用流程

1. 左侧选择一个状态为「已就绪」（或「可自动安装」）的工具。
2. 点击「添加工作区」，从文件夹选择器或 AIO 项目列表注册一个目录。
3. 选中话题即自动建立与 CLI 工具的连接（进程启动 + 协议握手，无需先发送消息），输入框左下角随即出现该工具的配置控件；输入消息发送。CLI 工具的工作进展（文本、思维链、工具调用、文件 Diff）实时渲染在消息区；工具推送执行计划时，状态栏下方会出现可折叠的任务清单；离开 Harness 页后，运行中的工具会出现在对话页状态栏的后台任务菜单里，随时可回看。
4. 工具执行完毕后，本轮回复自动保存在 AIO 本地；重新打开话题即可回看完整记录。

### 权限审批

所选 CLI 工具请求执行敏感操作时，消息区底部会弹出审批气泡：工具名、参数预览、涉及文件的 Diff 预览，以及工具提供的选项（通常为「允许一次 / 总是允许 / 拒绝一次 / 总是拒绝」）。审批没有超时——工具会一直等待你的决定；切换页面不会打断等待。

Claude Code（原生接入）的审批选项为「允许一次 / 拒绝」（其协议不提供 always 档）；此外状态栏提供权限模式下拉：默认（逐次确认）/ 自动接受编辑 / 计划模式 / 不再询问 / 完全放手。模式在会话运行时即时切换，同时作为下一次新会话的默认模式。

Codex（原生接入）没有审批交互——无头 exec 模式固定不询问，沙箱与审批策略由 `~/.codex/config.toml` 决定（如 `approval_policy` / `sandbox_mode`）。该页面不会为 Codex 弹出审批气泡。

### 模型与思考强度

输入框左下角提供所选工具的配置控件（按工具实际暴露的能力自动出现，未提供的不渲染；上次使用过的工具会立即显示其最近上报的选项，原生接入的 Claude Code / Codex 首次使用也会直接显示预设；ACP 工具在本机从未跑过时，需先发出一条消息建立会话后才会出现其上报的选项）：

- **ACP 系工具**（OpenCode / Oh My Pi / Gemini CLI / Qwen Code / goose / DSH / 自定义 ACP 工具，以及回退 ACP 模式的 Claude Code / Codex）：下拉列表来自工具自身在会话建立时上报的配置项，选择后即时下发给工具——工具协议规定变更自下一回合生效。实测 Oh My Pi 上报三项：**模式**（默认 / 计划——只读规划）、**模型**（数百项供应商目录，下拉中以「供应商 · 模型名」标注来源）、**思考强度**（off / auto / high）。
- **Claude Code（原生）**：模型下拉可运行中热切换（协议 `set_model` 控制帧）；思考强度为启动参数（`--effort`），修改后会自动重启会话进程并在下次发送时恢复上下文。
- **Codex（原生）**：模型与思考强度按回合生效（每次发送都是新的 `codex exec` 进程，参数 `-m` / `-c model_reasoning_effort=`）；模型列表为常用预设。

除协议内选项外，个别工具还有 CLI flag 形态的开关——AIO 将其映射为「spawn 期选项」下拉（修改后自动重启会话生效）：

- **Oh My Pi：审批模式**——逐次确认（默认）/ 受限修改（写操作需确认）/ 完全权限（全部自动批准），对应 omp 的 `--approval-mode`（always-ask | write | yolo）。ACP 通道本身不暴露此面，AIO 在会话启动时注入对应 flag。
- **Gemini CLI / Qwen Code：审批模式**——默认（逐次确认）/ 自动编辑 / 完全权限 / 计划模式，对应 `--approval-mode`（default | auto_edit | yolo | plan）。
- **goose：权限模式**——完全自主（默认）/ 手动批准 / 智能批准 / 仅聊天，对应 `GOOSE_MODE` 环境变量（auto | approve | smart_approve | chat）。
- **Codex：沙箱**——工具默认 / 只读 / 工作区可写 / 完全访问，对应 `codex exec -s`（read-only | workspace-write | danger-full-access），每回合生效。

偏好（原生适配器的模型/思考强度、spawn 期选项）按工具持久化保存在本地，重启 AIO 后保持。

### 工具设置（认证与 API Key）

悬停左侧工具行的齿轮图标打开该工具的设置弹窗：

- **认证状态**：AIO 探测该工具自身的配置目录（如 `~/.claude/.credentials.json`、`~/.codex/auth.json`），显示是否已完成官方登录/配置。
- **API Key**（Claude Code / Codex / Gemini CLI / Qwen Code）：可填对应供应商的 API Key——保存在系统钥匙串（不写入工具自己的文件），仅在启动该工具的会话进程时注入为对应环境变量（`ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `GEMINI_API_KEY` / `DASHSCOPE_API_KEY`），对新会话生效。
- **官方文档**：一键打开该工具的认证/配置指南。

Oh My Pi / OpenCode / goose / DSH 使用各自的登录或配置体系（无 API Key 注入项），弹窗提供状态检测与文档入口。

### 工作区指令（CLAUDE.md / AGENTS.md 等）

每个 CLI 工具会读取工作区根目录下的约定指令文件。悬停工作区行的编辑图标即可在 AIO 内直接查看/编辑该文件（不存在时保存即创建）：

| 工具                                           | 指令文件      |
| ---------------------------------------------- | ------------- |
| Claude Code                                    | `CLAUDE.md`   |
| Codex / OpenCode / Oh My Pi / DSH / 自定义工具 | `AGENTS.md`   |
| Gemini CLI                                     | `GEMINI.md`   |
| Qwen Code                                      | `QWEN.md`     |
| goose                                          | `.goosehints` |

### 停止与恢复

- 运行中可点状态栏或输入区的「停止」：先走协议级取消，超时后强制结束进程树。
- 每个 CLI 工具维护自己的会话；AIO 记录其会话 ID。停止后再次发送会自动恢复原会话（工具支持恢复时）；应用重启后同样如此。个别工具不支持恢复时，重启后首轮会开新会话，AIO 内的历史记录仍然可见。
- 斜杠命令按所选工具自身的命令体系原样透传。个别工具的交互式命令在无头模式下可能无响应，此时用「停止」结束即可。

## 工具设置页（设置 → CLI 工具）

设置页新增「CLI 工具」分区（列表 + 每工具二级页）。工具配置以**工具自己的配置文件为唯一事实源**——AIO 直接读写 CLI 读取的同一份文件（`~/.claude/settings.json`、`~/.codex/config.toml`、`~/.config/opencode/opencode.json(c)`、`~/.omp/agent/config.yml`、`~/.gemini/settings.json`、`~/.qwen/settings.json`、goose 的 `config.yaml`/`permission.yaml`、DSH 的 `settings.yaml`），在 GUI 改的即 CLI 用的。

每个工具页包含：

- **概览**：安装/认证状态、配置文件探测（路径/是否存在/备份标记）、DSH 的 profile 层叠只读展示；
- **常用配置**（Claude Code / Gemini CLI / Qwen Code / OpenCode）：高频配置键的结构化表单——默认模型、默认权限模式（claude）、审批模式（gemini/qwen）、始终思考、会话清理天数、权限规则（allow/ask/deny 芯片编辑）、环境变量键值对等。每次修改直接写入工具自己的配置文件（保留其它键；清空/移除即恢复工具默认——与手工编辑同一语义）。OpenCode 使用 JSONC 配置文件时结构化编辑自动禁用（避免并行 .json 覆盖），用下方原始编辑器；
- **凭据**：认证探测 + API Key（OS 钥匙串，spawn 时注入）+ 官方文档；
- **高级配置**：每个配置文件的带校验原始编辑器——保存前按格式重解析校验（JSON/JSONC/TOML/YAML，解析失败拒绝落盘）、**首次写入前自动备份**（`<file>.aio-bak`，可一键还原）、外部修改的陈旧检测（编辑期间文件被外部改动则拒绝覆盖）。

信任边界：只写用户级文件（不碰企业/managed 层与任何凭据文件）；密钥永不写入配置文件。

## 数据与隐私

- 显示记录保存在 AIO 本地数据库（`harness_*` 表），仅用于回看与话题管理，不会发送到任何远端。
- CLI 工具自身的会话文件、认证凭据与配置位于各自默认目录（如 `~/.claude`、`~/.codex`、`~/.omp`），由各工具自行管理；AIO 不读取、不存储这些凭据。
- 子进程环境变量按白名单透传（PATH、HOME/USERPROFILE、代理等），AIO 保存的密钥不会注入。

## 排障

- **提示未安装 Node.js**：安装 Node 20+ 后重启 AIO 再试。
- **首次启动很慢**：`npx` 正在下载适配器包（Claude Code / Codex），属预期；后续启动走缓存。
- **连接中卡住**：检查对应 CLI 在终端是否能正常启动与登录（`claude` / `codex login` 等）；也可查看 AIO 日志中的 `[harness:...]` 输出。
- **Windows 提示找不到命令**：确保安装后将命令目录加入 PATH，并重启 AIO（GUI 进程启动时只继承精简环境）。

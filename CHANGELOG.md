<div id="top"></div>

# Changelog

本项目的所有显著更新将记录在此文件中。

## [v0.7.1]
<sup>Released on **2026-07-25**</sup>

### ✨ Added（新增）

- **`delegate_tasks` 批量子智能体工具**：单次调用并行创建多个子智能体（1-10 个），支持 shared context 注入公共背景信息，总耗时约等于最慢的子任务
- **子智能体并发上限控制**：新增 `maxConcurrentSubagents` 配置项（默认 5），通过 Tokio Semaphore 限制同时运行的子智能体数量
- **批量委托一次审批**：`delegate_tasks` 整个批次仅需一次权限确认（非 Auto 模式），审批对话框汇总展示所有子任务

### ⚡️ Changed（变更）

- **`execute_single_delegate` 重构**：提取 `delegate_task` 和 `delegate_tasks` 共用的子智能体执行逻辑，消除代码重复
- **系统提示词更新**：Agent 模式提示词中新增 `delegate_tasks` 工具说明和批量并行推荐

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

[v0.2.13-Beta]:https://github.com/Atom112/AIO/releases/tag/v0.2.13

---

## [v0.2.12-Beta]
<sup>Released on **2026-01-31**</sup>

### ✨ Added（新增）
- 添加用户头像，AI供应商logo头像，支持用户更换头像

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.2.12-Beta]:https://github.com/Atom112/AIO/releases/tag/v0.2.12

---

## [v0.2.9-Beta]
<sup>Released on **2026-01-28**</sup>

### ✨ Added（新增）
- 新增`.deb`、`.rpm`和`.dmg`安装包

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.2.9-Beta]:https://github.com/Atom112/AIO/releases/tag/v0.2.9

---

## [v0.2.0-Beta]
<sup>Released on **2026-01-28**</sup>

### ✨ Added（新增）
- 集成`llama.cpp`支持运行`.gguf`本地模型

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.2.0-Beta]:https://github.com/Atom112/AIO/releases/tag/v0.2.0

---

## [v0.1.7-Beta]
<sup>Released on **2026-01-28**</sup>

### ✨ Added（新增）
- 添加了话题自动总结和话题重命名

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.1.7-Beta]:https://github.com/Atom112/AIO/releases/tag/v0.1.7

---

## [v0.1.6-Beta]
<sup>Released on **2026-01-27**</sup>

### ✨ Added（新增）
- 可用模型列表持久化存储

### 🐛 Fixed（修复）
- 修复了切换页面导致聊天框内文件卡片渲染失败的问题 **closes [#2](https://github.com/Atom112/AIO/issues/2) ([8e634d](https://github.com/Atom112/AIO/commit/8e634dabe85167d6d60448374c3fed45b0fa5950))**

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.1.6-Beta]:https://github.com/Atom112/AIO/releases/tag/v0.1.6

---

## [v0.1.5-Beta]
<sup>Released on **2026-01-27**</sup>

### ⚡️ Changed（变更）
- 优化了模型调用逻辑

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.1.5-Beta]:https://github.com/Atom112/AIO/releases/tag/v0.1.5

---

## [v0.1.3-Beta]
<sup>Released on **2026-01-26**</sup>

### ✨ Added（新增）
- 增加了文本文件上传功能
- 拖入文件上传时显示浮层提示
  
### 🐛 Fixed（修复）
- 修复了因事件监听器泄露导致的聊天文字重复问题 **closes [#1](https://github.com/Atom112/AIO/issues/1) ([ec957a](https://github.com/Atom112/AIO/commit/ec957a38f74b7fc0fa70f7e97e8fbbf193550ee6))**

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.1.3-Beta]:https://github.com/Atom112/AIO/releases/tag/v0.1.3

---

## [v0.1.0-Beta]
<sup>Released on **2026-01-24**</sup>

### ✨ Added（新增）
- AIO 初始版本发布。
- 集成了核心功能：综合API调用、模型选择与AI文本对话

<p align="right"><a href="#top"><img src="/public/icons/top.svg"></img></a></p>

[v0.1.0-Beta]:https://github.com/Atom112/AIO/releases/tag/v0.1.0
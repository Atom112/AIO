<div align="center">
  <img src="./logo.svg" width="120" height="120" alt="AIO Logo">
  <h1>AIO (All-In-One AI)</h1>
  <p>
    <strong>一个轻量、跨平台，兼顾日常对话与代码 Agent 的 AI 桌面工作区</strong><br>
    <em>A lightweight, cross-platform AI workspace for chat and coding agents.</em>
  </p>

  <p>
    <img src="https://img.shields.io/badge/Tauri-2.11-blue?logo=tauri" alt="Tauri 2.11">
    <img src="https://img.shields.io/badge/Rust-stable-orange?logo=rust" alt="Rust stable">
    <img src="https://img.shields.io/badge/SolidJS-1.9-446b9e?logo=solid" alt="SolidJS 1.9">
    <img src="https://img.shields.io/badge/Platform-Windows%20|%20macOS%20|%20Linux-brightgreen" alt="Windows, macOS and Linux">
  </p>
</div>

---

![AIO 聊天工作区](docs/assets/screenshots/chat-overview.webp)

---

## 🌟 核心特性 | Features

### 1. 💬 多模型对话 | Multi-Provider Chat

- **多厂商接入**：支持 Google、Anthropic、Ollama 与 OpenAI-compatible API。
- **灵活模型管理**：按 Provider 启用模型，为不同助手和子智能体选择专属模型。
- **完整对话体验**：流式输出、推理内容、上下文压缩、自动标题与消息分支一应俱全。

### 2. 🧠 本地模型推理 | Local LLM

- **llama.cpp 驱动**：直接选择并运行本地 GGUF 模型。
- **首次自动安装**：缺少运行引擎时，AIO 会在启动模型时自动下载。
- **统一体验**：本地模型与远程模型共用助手、话题和模型选择器。

> [!NOTE]
> vLLM 插件已在 Rust 后端注册，但当前设置界面只开放 llama.cpp。普通用户请使用 GGUF 模型。

### 3. 🛠️ 项目 Agent | Coding Agent

- **五种工作模式**：对话、普通、自动、Plan 与工作流，按任务风险自由切换。
- **项目级工具**：文件、Shell、Git、LSP、联网搜索、MCP 和 Skill 都能围绕当前目录协作。
- **子智能体协作**：内置探索、实现、架构、诊断、审查、文档、测试等专业角色。
- **安全审批**：危险命令、文件修改和 MCP 工具调用遵循项目权限规则。

### 4. 📚 文件与扩展 | Files & Extensions

- **常用文件解析**：支持图片、PDF、DOCX、PPTX 和多种文本格式。
- **MCP 生态**：支持 stdio、HTTP、Streamable HTTP、Catalog、Tools、Resources 与 Prompts。
- **Skill 管理**：可从市场下载，也可导入 NPX Skill，并按全局或项目范围启用。

### 5. ✨ 贴心体验 | Productivity

- **快捷操作**：命令面板、可定制快捷键和 `/review`、`/fix`、`/btw` 等斜杠命令。
- **会话分享**：导出截图、Markdown、JSON 或 PDF。
- **用量统计**：查看 Token 汇总、模型分布和使用热力图。
- **自动更新**：通过 GitHub Releases 获取跨平台更新。

---

## ⚙️ 技术栈 | Tech Stack

| 层级 | 技术 |
| --- | --- |
| Frontend | SolidJS 1.9、TypeScript、Tailwind CSS、Vite |
| Desktop | Tauri 2.11 |
| Backend | Rust、Tokio、Reqwest |
| Storage | SQLite、JSON 配置、系统凭据库 |
| Local Engine | llama.cpp；vLLM 后端插件 |

---

## 🚀 快速开始 | Getting Started

### 📦 下载发行版 | Download

前往 [GitHub Releases](https://github.com/Atom112/AIO/releases)，下载适合当前系统的安装包。发布流程覆盖 Windows、Ubuntu 22.04，以及 Intel / Apple Silicon macOS。

安装后打开“设置 → 供应商设置”：

1. 配置远程 Provider，或在“本地推理引擎”中选择 GGUF 模型；
2. 启用模型并返回聊天页；
3. 选择模型，开始第一段对话。

想让 AI 参与代码工作？创建项目、绑定本地目录，再选择普通、自动、Plan 或工作流模式即可。

### 🏗️ 从源码运行 | Build from Source

需要 Node.js 20、Rust stable、当前平台的 [Tauri 2 系统依赖](https://v2.tauri.app/start/prerequisites/)，以及与本仓库同级的 `aio-models-data`：

```text
parent/
├── AIO/
└── aio-models-data/
```

```bash
git clone https://github.com/Atom112/aio-models-data.git
git clone https://github.com/Atom112/AIO.git
cd AIO
npm ci
npm run tauri dev
```

构建前检查：

```bash
npm run build
cd src-tauri
cargo check
```

> [!TIP]
> Linux 依赖、发布构建和模型目录说明都整理在[开发环境文档](docs/development/getting-started.md)中。

---

## 📖 文档导航 | Documentation

| 想了解什么 | 从这里开始 |
| --- | --- |
| 安装、首次配置与第一次对话 | [快速开始](docs/usage/getting-started.md) |
| Provider 与本地模型 | [供应商与模型](docs/usage/providers-and-models.md) |
| 助手、项目、Agent 与子智能体 | [聊天与 Agent](docs/usage/chat-and-agent.md) |
| MCP、Skill 与工具权限 | [MCP 与 Skill](docs/usage/mcp-and-skills.md) |
| 主题、更新与快捷键 | [应用设置与快捷键](docs/usage/app-settings-and-shortcuts.md) |
| 架构与扩展开发 | [开发指南](docs/README.md#开发指南) |
| 连接、引擎或构建问题 | [故障排查](docs/troubleshooting.md) |

完整目录见 [AIO 文档中心](docs/README.md)，版本变化见 [CHANGELOG.md](CHANGELOG.md)。

---

## 🤝 贡献与反馈 | Contributing

发现 Bug 或有新点子？欢迎前往 [GitHub Issues](https://github.com/Atom112/AIO/issues)：

- 提交 Bug 时请附上系统版本、AIO 提交和脱敏后的错误日志；
- 功能建议尽量描述真实使用场景；
- Pull Request 提交前请运行 `npm run build` 与 `cargo check`。

---

<div align="center">
  <p>如果 AIO 对你有帮助，欢迎点亮一个 ⭐！</p>
  <p><em>If AIO helps you, a star would mean a lot.</em></p>
  <p>
    <a href="./LICENSE">Apache-2.0 License</a>
    ·
    <a href="https://github.com/Atom112">Atom112</a>
  </p>
</div>

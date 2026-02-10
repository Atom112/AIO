
<div align="center">
  <img src="./public/icons/logo.png" width="120" height="120" alt="AIO Logo">
  <h1>AIO (All-In-One AI)</h1>
  <p>
    <strong>一个极致轻量、跨平台、支持本地 GGUF 与多厂商 API 的全能 AI 助手</strong><br>
    <em>A powerful, lightweight, cross-platform AI workspace supporting local GGUFs and multi-provider APIs.</em>
  </p>

  <p>
    <img src="https://img.shields.io/badge/Tauri-2.0-blue?logo=tauri" alt="Tauri">
    <img src="https://img.shields.io/badge/Rust-1.75+-orange?logo=rust" alt="Rust">
    <img src="https://img.shields.io/badge/SolidJS-1.8-76b5c5?logo=solid" alt="SolidJS">
    <img src="https://img.shields.io/badge/llama.cpp-CUDA--Supported-green?logo=nvidia" alt="llama.cpp">
    <img src="https://img.shields.io/badge/Platform-Windows%20|%20macOS%20|%20Linux-brightgreen" alt="Platform">
  </p>
</div>

---

## 🌟 核心特性 | Features

### 1. 📂 本地模型推理 | Local LLM Power
- **llama.cpp 内置支持**: 通过 `llama-server` 直接运行 `.gguf` 模型。
- **GPU 加速**: 完美适配 CUDA 12.4+，支持千万级参数模型在本地流畅运行。
- **免配置启动**: 自动关联本地路径，一键开启本地 AI 引擎。

### 2. 🌐 多厂商 API 集成 | Multi-Provider Integration
- **兼容 OpenAI 格式**: 支持所有遵循 OpenAI 接口规范的供应商（如 DeepSeek, Claude, Gemini, GPT-4）。
- **模型激活管理**: 灵活切换不同厂牌模型，支持 API 状态持久化记录。

### 3. 📄 深度文件解析 | Document Intelligence
- **全格式支持**: 直接拖拽 PDF、Docx、PPTX 或代码文件。
- **上下文自动提取**: 自动将长文本转化为 AI 记忆，提升对话精准度。

### 4. 🎨 现代交互设计 | Modern & Fluid UI
- **SolidJS 驱动**: 极致响应速度，丝滑的流式输出（Streaming）体验。
- **话题管理系统**: 针对不同任务创建“助手”与“话题”，支持自动对话摘要。

---

## 🛠️ 技术栈 | Tech Stack

- **Frontend**: [SolidJS](https://www.solidjs.com/) + CSS 霓虹主题 (Cyberpunk Aesthetic)
- **Backend**: [Rust](https://www.rust-lang.org/) + [Tauri 2.x](https://v2.tauri.app/)
- **Core Engine**: [llama.cpp](https://github.com/ggerganov/llama.cpp) (CUDA Enabled)
- **Persistence**: Json-based configuration & DashMap for state management.

---

## 🚀 快速开始 | Getting Started

### 📦 下载并安装发行版 | Download and Install Release
对于普通用户，我们推荐直接下载预编译发行包进行安装。

1.  **访问发布页面**: 前往 [GitHub Releases 页面](https://github.com/Atom112/AIO/releases) 。
2.  **下载对应版本**: 根据你的操作系统，下载相应的安装程序（例如 `.msi` for Windows, `.dmg` for macOS, `.deb` / `.rpm` for Linux）。
3.  **运行安装程序**: 双击下载的安装包，按照提示完成安装即可。
    *   **温馨提示**: 发行版已自动集成 `llama.cpp` 运行时，你无需手动配置。

### 🏗️ 从源码构建 | Build from Source
开发者或希望进行定制的用户可按照以下步骤从源码构建：

#### 环境准备 | Prerequisites
- [Rust](https://www.rust-lang.org/tools/install) (1.75+)
- [Node.js](https://nodejs.org/) (Bun 或 PNPM 推荐)
- **CUDA Toolkit** (如需显卡加速，请确保已正确安装)

#### 安装步骤 | Installation

1. **克隆仓库**:
   ```bash
   git clone https://github.com/Atom112/AIO.git
   cd AIO
   ```

2. **配置本地模型引擎 (llama.cpp) (仅源码构建需要)**:
   - 下载对应平台的 `llama-server` 二进制文件 (例如 `llama-server-win-x64.zip` 中的 `llama-server.exe`)。
   - 将主程序 `llama-server.exe` 及所有依赖的 `.dll` 文件（如 `cudart64_12.dll`, `cublas64_12.dll` 等）**完整地**放入 `src-tauri/resources/llama-backend/` 目录下。

3. **安装依赖并运行**:
   ```bash
   npm install
   npm run tauri dev
   ```

---

## ⚙️ 配置说明 | Configuration

### 运行本地 GGUF / Running Local GGUF
1. 进入 `设置 (Settings)` 页面。
2. 在“本地 LLM”区域点击“选择文件”，选中你的 `.gguf` 模型。
3. 点击“启动本地引擎”。系统将自动在 `http://127.0.0.1:8080/v1` 开启服务。
4. 模型将自动同步至“已激活模型列表”。

### 关于后端支持

本项目的后端支持目前处于试验阶段，若想体验，访问仓库：

[AIO-backend](https://github.com/Atom112/AIO-backend)

---

## 🤝 贡献与反馈 | Contribution

欢迎提交 Issue 或 Pull Request！

- **提交 BUG**: 请描述你的操作系统版本及模型规模。
- **功能建议**: 欢迎在 Discussion 区讨论更多文件解析格式支持。

---

## 📄 开源协议 | License

[Apache-2.0 License](LICENSE) - © 2024 Atom112

---

<div align="center">
  <p>如果这个项目对你有帮助，请给一个 ⭐️ 以示鼓励！</p>
  <p><em>Give a ⭐️ if this project helped you!</em></p>
</div>

Copyright © 2025 [Loch](https://github.com/Atom112).

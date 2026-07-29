# 快速开始

## 安装

从 [GitHub Releases](https://github.com/Atom112/AIO/releases) 下载对应平台的安装包。发布流程构建以下平台：

- Windows；
- Ubuntu 22.04；
- Intel macOS；
- Apple Silicon macOS。

应用支持自动检查和安装更新。若系统阻止启动，请先确认安装包来自项目 Releases 页面。

## 配置远程模型

![Provider 设置](../../assets/screenshots/provider-settings.webp)

1. 打开“设置 → 供应商设置”。
2. 选择目录中的供应商，或添加自定义 OpenAI-compatible 供应商。
3. 填写 API URL 与 API Key。
4. 测试连接，选择要启用的模型并保存。
5. 返回聊天页，从模型选择器中选择已启用模型。

API Key 会优先写入系统凭据库，不会与普通 Provider 配置一起明文保存。各 Provider 的差异见[供应商与模型](providers-and-models.md)。

## 配置本地 GGUF

1. 打开“设置 → 供应商设置 → 本地推理引擎”。
2. 选择一个 `.gguf` 文件。
3. 点击“添加到模型列表”。
4. 点击“启动本地 llama.cpp 引擎”。
5. 首次启动时等待应用下载并安装匹配平台的引擎。
6. 返回聊天页并选择本地模型。

本地服务默认使用 `127.0.0.1:8080`。同一时间只维护一个本地引擎进程，退出应用时会清理该进程。

## 完成第一次对话

1. 在聊天页选择模型。
2. 使用默认聊天助手，或创建自己的助手并设置系统提示词。
3. 创建话题并发送消息。
4. 需要文档上下文时上传受支持的图片、PDF、Office 或文本文件。

普通聊天不会执行项目工具。要让模型读取或修改代码，请创建项目并选择合适的 Agent 模式，详见[聊天与 Agent](chat-and-agent.md)。

## 下一步

- 了解 Provider 和本地引擎：[供应商与模型](providers-and-models.md)
- 为项目安装工具和提示能力：[MCP 与 Skill](mcp-and-skills.md)
- 自定义更新、主题和快捷键：[应用设置与快捷键](app-settings-and-shortcuts.md)
- 遇到连接或启动错误：[故障排查](../troubleshooting.md)

# Security Policy | 安全策略

**English** | **简体中文**

## English

### Supported versions

| Version                   | Supported                        |
| ------------------------- | -------------------------------- |
| latest stable release     | ✅                               |
| latest prerelease (alpha) | ✅                               |
| older releases            | ❌ — upgrade first, then re-test |

### Reporting a vulnerability

**Do not open a public issue for security problems.** Instead:

1. Use GitHub's private reporting: **Security → Report a vulnerability** on this repository (preferred), or
2. Contact the maintainer privately (QQ group **1130943633**, message @Atom112 / 群主).

Please include: affected version (environment snapshot from Settings → App → Feedback & Support), reproduction steps or PoC, and impact assessment. We aim to acknowledge within 72 hours and will keep you posted on fix progress. Coordinated disclosure: please hold public disclosure until the fix ships.

### Scope

The application binary, the update pipeline, the bundled MCP filesystem server, the agent sandbox/permission model, and anything in this repository. Out of scope: vulnerabilities in third-party CLI tools you connect via the Harness workbench (report those upstream), and issues requiring already-physical access to an unlocked machine.

## 简体中文

### 支持的版本

| 版本                  | 支持情况           |
| --------------------- | ------------------ |
| 最新稳定版            | ✅                 |
| 最新预发布版（alpha） | ✅                 |
| 更早版本              | ❌——请先升级再复测 |

### 报告漏洞

**安全问题不要开公开 issue。**请通过：

1. 本仓库的 **Security → Report a vulnerability**（GitHub 私密报告，首选），或
2. 私下联系维护者（QQ 群 **1130943633** 内 @Atom112 / 群主）。

请附：受影响版本（应用内「设置 → 应用 → 反馈与支持」的环境快照）、复现步骤或 PoC、影响评估。目标 72 小时内确认收到，修复进展会同步给你。请配合在修复发布前不公开披露。

### 范围

应用本体、更新管线、内置 MCP 文件系统服务、Agent 沙箱与权限模型及本仓库内的一切。不在范围内：经 Harness 工作台接入的第三方 CLI 工具自身的漏洞（请报给上游）、需要已物理接触解锁机器的问题。

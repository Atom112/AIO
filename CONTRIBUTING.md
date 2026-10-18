# 贡献指南 | Contributing Guide

**简体中文** | [English](CONTRIBUTING.en-US.md)

欢迎参与 AIO 开发！本文是**流程规范**（issue、分支、提交、测试、PR）；**编码规范**（代码风格、注释、接口、门禁）以 [AGENTS.md](AGENTS.md) 为唯一事实源——人类贡献者与 AI 代理都适用同一套编码标准。

- 交流渠道：[GitHub Discussions](https://github.com/Atom112/AIO/discussions)（问答与功能想法）、QQ 群 **1130943633**（中文社区）
- 安全漏洞报告：请勿开公开 issue，见 [SECURITY.md](SECURITY.md)

---

## 1. Issue 规范

提 issue 前先搜索现有 [Issues](https://github.com/Atom112/AIO/issues) 与 [Discussions](https://github.com/Atom112/AIO/discussions)，避免重复。

| 类型           | 入口                                                                                    | 要求                                                                                                                                                                 |
| -------------- | --------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Bug            | [Bug 报告表单](https://github.com/Atom112/AIO/issues/new?template=bug_report.yml)       | 附环境快照（应用内「设置 → 应用 → 反馈与支持 → 复制环境信息」）与最小复现步骤                                                                                        |
| 功能建议       | 先发 [Discussions「功能想法」](https://github.com/Atom112/AIO/discussions)              | 收集到足够 👍 后由维护者开 tracking issue；直接开 issue 请用[功能建议表单](https://github.com/Atom112/AIO/issues/new?template=feature_request.yml)，先说问题再说方案 |
| Alpha 体验反馈 | [Alpha 反馈表单](https://github.com/Atom112/AIO/issues/new?template=alpha_feedback.yml) | 一句话总评 + 满意度 + 阻碍/惊喜                                                                                                                                      |

标注约定（由维护者维护）：`bug` / `enhancement` / `needs-triage` / `area:*`（问题域）。一个 issue 只报告一个问题。

## 2. 分支模型

长期分支只有两条：**`dev`（集成分支）** 与 **`master`（仅含已发布内容）**。所有改动经 PR 进入 `dev`。

- **功能分支**：从最新 `dev` 切出，命名 `dev-<类型>-<主题>`，如 `dev-feat-memory-panel`、`dev-fix-settings-jank`。不建议 `dev/<name>`——Git 不允许在 `dev` 分支存在时创建 `dev/` 命名空间。
- **单一职责**：一个分支只做一件事（一个特性 / 一个修复），保持可独立评审、可干净回退。
- **保持新鲜**：开发期间定期把 `dev` 合入功能分支（merge 即可，无需 rebase 已推送的分支）。
- **命名含类型前缀**（`feat` / `fix` / `docs` / `chore` / `refactor`），与提交信息类型呼应。
- **外部贡献者**：fork 本仓库，在自己的 fork 上开分支，PR 回 `Atom112/AIO` 的 `dev`。
- **发布流程**（维护者执行）：`dev` → `master` PR 合并 → bump 三处版本号 + CHANGELOG → 打 `vX.Y.Z` tag 推送触发 [release.yml](.github/workflows/release.yml)。预发布 tag（如 `v1.0.0-alpha`，含连字符）自动标记为 GitHub prerelease，`releases/latest` 恒指稳定版。

## 3. 提交信息规范

采用 [Conventional Commits](https://www.conventionalcommits.org/zh-hans/v1.0.0/)——`type(scope): subject`，type 必须来自下表（与 [cliff.toml](cliff.toml) 的解析分组一一对应）：

| type          | 用途                        |
| ------------- | --------------------------- |
| `feat`        | 新功能                      |
| `fix`         | 缺陷修复                    |
| `docs`        | 文档                        |
| `perf`        | 性能优化                    |
| `refactor`    | 重构（无行为变化）          |
| `style`       | 格式 / 样式调整（不改逻辑） |
| `test`        | 测试                        |
| `chore(deps)` | 依赖升级                    |
| `chore`       | 构建 / 工具 / 杂务          |
| `revert`      | 回滚                        |

- **subject**：中文，一句话讲清「做了什么」；结尾不加句号。
- **正文**：解释**动机与方案**（为什么这么改），不是 diff 内容复读。
- **关联 issue**：正文中 `Closes #123`；有破坏性变更加 `BREAKING CHANGE:` 段说明迁移方式。
- **诚实归属**：AI 代理完成的改动照常遵守全部规范，不伪装人工成果；提交不因「是代理写的」而降低标准。

## 4. 本地测试与验证

环境搭建见[开发环境文档](docs/zh/development/getting-started.md)。提交前的验证基线（CI 会跑同一套）：

```bash
npm run verify
```

它包含两部分（可分别运行）：

- `npm run check:frontend`：i18n 双语键位校验 → 文档链接校验 → 架构/样式门禁（check:gates）→ prettier → eslint → tsc → vitest → vite build
- `npm run check:rust`：cargo fmt → clippy（-D warnings）→ cargo test

日常节奏建议：改动的每个提交至少跑**相关测试**（`npx vitest run <pattern>` / `cargo test <name>`）；PR 创建前跑完整 `npm run verify`。

**门禁是硬约束**：`check:gates` 覆盖裸中文 JSX、硬编码颜色、`any` 棘轮、组件分层、文件规模、子进程隐藏控制台等，违规即失败。不允许为让提交通过而放宽门禁阈值——门禁本身的调整必须单独 PR 并说明理由。禁止 `--no-verify`。

**测试书写**：前端测试与源码同目录（`*.test.ts`）；Rust 测试优先内联 `#[cfg(test)]` 模块，跨模块/重量级集成测试放 `src-tauri/tests/`。修 bug 的 PR 应附回归测试（先红后绿）。

## 5. Pull Request 流程

1. 从最新 `dev` 切分支，开发并自验（§4）。
2. 推送后向 `dev` 发 PR，按[模板](.github/PULL_REQUEST_TEMPLATE.md)填写：改动点、动机、验证证据（命令 + 结果）、UI 变化说明。
3. CI（[ci.yml](.github/workflows/ci.yml)）必须全绿；红了先修再请求评审。
4. 经 [CODEOWNERS](.github/CODEOWNERS) 指定的维护者评审通过后合并；合并后删除功能分支（本地 + 远端）。
5. 合并使用普通 merge（保留分支拓扑）；合并后不得 force-push `dev` / `master`。

PR 尽量小而聚焦：评审负荷控制在一次可读完的范围；大特性拆成多个可独立合并的 PR 串联推进（参考 CHANGELOG 中的「PR A/B/C/D/E」分期先例）。

## 6. AI 代理协作开发

AIO 的开发大量使用 AI 编码代理，也欢迎贡献者这样做。代理行为规范以 [AGENTS.md](AGENTS.md) 的「Agent 协作守则」一节为准，核心红线：

- **验证诚实**：声明「已验证」必须附实际执行的命令与结果；未运行的测试不得声称通过。
- **不越门禁**：不改门禁阈值、不 `--no-verify`、不为通过检查而特化代码。
- **先读后说**：对不确定的行为先读源码再断言，不臆测 API。
- **最小改动**：不顺手重构无关代码；相邻问题记 issue 而不是顺手改。
- **PR 必须**：代理产物与人写代码走完全相同的 PR 流程；代理不直接推送 `dev` / `master`。

## 7. 文档与截图

- **双语镜像**：`docs/zh/` 与 `docs/en/` 逐文件对应；任何一侧的实质变更必须在同一 PR 内同步另一侧。链接必须通过 `npm run check:docs`。
- **文档随代码**：行为变更 PR 附文档更新；未开放的插件/能力在文档中明确标注（见 [docs/zh/README.md](docs/zh/README.md) 顶部的约定）。
- **截图纪律**：截图不得包含个人数据、真实会话内容或本机路径；使用空白数据档案或虚构演示内容；统一 1600x1000 webp，存放 `docs/assets/screenshots/`。
- **一次性产物不入库**：中间报告、审查记录、阶段性计划不进 `docs/`——写在 PR 描述或 issue 里，事毕随 issue 归档。`docs/` 只保留随版本演进的用户与开发者文档。
- **CHANGELOG**：开发期间变更累积在 `## [Unreleased]` 段；发版条目由维护者按发布风格整理。

## 8. 安全与隐私红线

- 不提交 API Key、令牌、个人路径、真实用户数据；凭据一律走系统凭据库（代码内已有 `secure_store`）。
- 不在 issue / 截图 / 诊断包中暴露密钥或会话内容（应用内诊断包已自动脱敏）。
- 发现安全漏洞：按 [SECURITY.md](SECURITY.md) 私密报告。

---

感谢你的贡献！AIO 是个人维护的开源项目，评审与合并可能需要几天，请耐心等待。

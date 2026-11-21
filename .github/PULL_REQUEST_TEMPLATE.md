<!--
PR template. Read CONTRIBUTING.md before submitting; fill in every section and delete sections that do not apply.
中文：提交前请阅读 CONTRIBUTING.md（中文镜像 CONTRIBUTING.zh-CN.md）；按要求填写各节，不适用的小节可删除。
-->

## What & Why | 改动说明

<!-- What changed and why (motivation); reference the issue when applicable: Closes #123
     做了什么、为什么这么做（动机），必要时关联 issue：Closes # -->

## Verification | 验证

<!-- Claims of "verified" must include commands and results; state anything not run.
     声明「已验证」必须附命令与结果；未运行的部分如实说明。 -->

- [ ] `npm run verify` is green (or state the scope actually run: relevant tests / check:frontend / check:rust) / 全绿（或注明实际运行的范围）
- [ ] Bug fixes include a regression test (red first, then green); new features have corresponding tests / 修 bug 附回归测试（先红后绿）；新功能有对应测试

## UI notes | UI 变化（delete if not applicable / 无则删除本节）

<!-- Attach screenshots for frontend changes; no personal data, real conversations, or machine paths.
     前端改动附截图；截图不得包含个人数据、真实会话内容或本机路径。 -->

- [ ] New UI strings have bilingual i18n keys (zh-CN and en-US placeholders match) / 新 UI 文案已加 i18n 双语键（zh-CN 与 en-US 占位符一致）
- [ ] Component and style gates pass (`npm run check:gates`) / 遵循组件与样式门禁

## Docs | 文档（delete if not applicable / 无则删除本节）

- [ ] Behavior changes update both `docs/zh/` and `docs/en/` (or state there is no doc impact) / 行为变更同步了 `docs/zh/` 与 `docs/en/` 两侧文档（或注明无文档影响）

## Checklist | 检查单

- [ ] Branch cut from the latest `dev`; PR targets `dev` / 分支从最新 `dev` 切出，PR 目标为 `dev`
- [ ] The branch contains a single topic / 分支只含单一主题的改动
- [ ] Commit messages follow Conventional Commits with an English subject (type table in CONTRIBUTING §3); an optional Chinese translation follows the English body / 提交信息符合 Conventional Commits，subject 为英文；中文翻译可选并放在英文正文之后
- [ ] No gate thresholds were loosened; `--no-verify` was not used / 未放宽任何门禁阈值，未使用 `--no-verify`
- [ ] AI-agent changes were self-verified to the same standard / AI 代理参与的改动已按同等标准自验

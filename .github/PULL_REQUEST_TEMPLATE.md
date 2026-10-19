<!--
PR 模板：请在提交前阅读 CONTRIBUTING.md。填写各节；不适用的小节可删。
English summary at the bottom.
-->

## 改动说明 | What & Why

<!-- 做了什么、为什么这么做（动机），必要时关联 issue：Closes #-->

## 验证 | Verification

<!-- 声明「已验证」必须附命令与结果；未运行的部分如实说明 -->

- [ ] `npm run verify` 全绿（或注明实际运行的范围：相关测试 / check:frontend / check:rust）
- [ ] 修 bug 的改动附了回归测试（先红后绿）；新功能有对应测试

## UI 变化 | UI notes（无则删除本节）

<!-- 前端改动附截图；截图不得包含个人数据 / 真实会话内容 / 本机路径 -->

- [ ] 新 UI 文案已加 i18n 双语键（zh-CN 与 en-US 占位符一致）
- [ ] 遵循组件与样式门禁（`npm run check:gates`）

## 文档 | Docs（无则删除本节）

- [ ] 行为变更同步了 `docs/zh/` 与 `docs/en/` 两侧文档（或注明无文档影响）

## 检查单 | Checklist

- [ ] 分支从最新 `dev` 切出，PR 目标为 `dev`
- [ ] 分支只含单一主题的改动
- [ ] 提交信息符合 Conventional Commits（type 见 CONTRIBUTING §3）
- [ ] 未放宽任何门禁阈值，未使用 `--no-verify`
- [ ] AI 代理参与的改动已按同等标准自验

---

**English summary:** describe what changed and why, list verification evidence (commands + results), attach screenshots for UI changes (no personal data), and confirm docs are synced on both language sides. Conventional Commits required; single-topic branch; no gate loosening.

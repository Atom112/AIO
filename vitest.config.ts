import { defineConfig } from 'vitest/config';

// 单元测试仅覆盖纯逻辑模块（core/utils、core/api 的纯辅助、常量表），
// 不渲染组件 → Node 环境即可（无需 jsdom；含 tauri 依赖的模块在测试内 mock）。
export default defineConfig({
  test: {
    environment: 'node',
    include: ['src/**/*.{test,spec}.{ts,tsx}'],
  },
});

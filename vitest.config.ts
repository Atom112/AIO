import { defineConfig } from 'vitest/config';

// Unit tests only cover pure-logic modules (core/utils, pure helpers and constant
// tables in core/api) with no component rendering, so a Node environment suffices
// (no jsdom needed; modules with tauri dependencies are mocked inside the tests).
export default defineConfig({
  test: {
    environment: 'node',
    include: ['src/**/*.{test,spec}.{ts,tsx}'],
  },
});

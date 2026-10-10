import { describe, expect, it } from 'vitest';
import { EVENTS } from './events';

// Frontend event names must follow the same convention as the kebab-case gate test in backend core/events.rs
describe('EVENTS', () => {
  it('事件名全部为 kebab-case', () => {
    for (const [key, value] of Object.entries(EVENTS)) {
      expect(value, key).toMatch(/^[a-z][a-z0-9]*(-[a-z0-9]+)*$/);
    }
  });
  it('包含主循环事件与 MCP/工作流事件', () => {
    expect(EVENTS.llmChunk).toBe('llm-chunk');
    expect(EVENTS.subagentStart).toBe('subagent-start');
    expect(EVENTS.mcpServerStatus).toBe('mcp-server-status');
    expect(EVENTS.workflowComplete).toBe('workflow-complete');
    expect(EVENTS.btwChunk).toBe('btw-chunk');
  });
});

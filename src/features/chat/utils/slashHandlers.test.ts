/**
 * Tests for the slash action handler (阶段4 第2项拆分切片：slashHandlers).
 *
 * handleSlashActionMessage's feedback matrix: which commands append a feedback
 * bubble, which return silently, and the /compact custom-feedback precedence.
 */
import { describe, expect, it, vi, beforeEach } from 'vitest';

vi.mock('../../../core/i18n', () => ({
  t: (key: string, params?: Record<string, string>) =>
    params ? `${key}:${JSON.stringify(params)}` : key,
}));
vi.mock('../../../core/store/store', () => ({
  currentAssistantId: () => 'a1',
  currentTopicId: () => 't1',
  webSearchEnabled: (val: boolean) => () => val,
  // capture the history appends
  setDatas: (...args: unknown[]) => {
    const last = args[args.length - 1] as (h: unknown[]) => unknown[];
    calls.push(last);
  },
  reasoningLevel: () => 'off',
  resolveAssistantSkills: () => [],
  selectedModel: () => ({ model_id: 'm', api_url: 'u' }),
  currentProject: () => null,
  currentProjectId: () => null,
}));
vi.mock('../../../core/api/chat', () => ({ appendMessage: async () => {} }));
// The prompt-path assembly chain loads the global store; not needed for the action handler
vi.mock('./skillMessages', () => ({ buildSkillMessages: () => [] }));
vi.mock('./apiMessages', () => ({
  buildApiMessages: () => [],
  ensureToolMessagesComplete: (m: unknown[]) => m,
}));
vi.mock('../../../core/agent-prompts', () => ({ buildAgentSystemPrompt: () => null }));

import { handleSlashActionMessage } from './slashHandlers';

// setDatas calls: (historyUpdater) — run the updater to see the appended bubble
const calls: Array<(h: unknown[]) => unknown[]> = [];
const lastAppend = () => {
  const updater = calls[calls.length - 1];
  return updater([] as unknown[]) as Array<{ role: string; content: string }>;
};

interface CmdFixture {
  id: string;
  label: string;
  handler: (args?: string) => Promise<string | void>;
}

function resolved(cmd: Partial<CmdFixture> & { id: string; label: string }) {
  return {
    command: {
      description: '',
      category: 'chat' as const,
      defaultKeys: '',
      handler: async () => {},
      ...cmd,
    },
    args: '' as string,
    resolvedBody: undefined,
  };
}

beforeEach(() => {
  calls.length = 0;
});

describe('handleSlashActionMessage feedback matrix', () => {
  it('/clear runs the handler and appends only the command bubble (no feedback)', async () => {
    let ran = false;
    await handleSlashActionMessage(
      resolved({
        id: 'slash-clear',
        label: '/clear',
        handler: async () => {
          ran = true;
        },
      }),
    );
    expect(ran).toBe(true);
    // one setDatas call (the command bubble), zero feedback
    expect(calls).toHaveLength(1);
    const bubbles = lastAppend();
    expect(bubbles).toHaveLength(1);
    expect(bubbles[0].role).toBe('user');
    expect(bubbles[0].content).toBe('/clear');
  });

  it('/help and /settings also return without feedback', async () => {
    await handleSlashActionMessage(
      resolved({ id: 'slash-help', label: '/help', handler: async () => {} }),
    );
    expect(calls).toHaveLength(1);
    await handleSlashActionMessage(
      resolved({ id: 'slash-settings', label: '/settings', handler: async () => {} }),
    );
    expect(calls).toHaveLength(2); // one command bubble each, no feedback appends
  });

  it('/compact uses the handler string as feedback when provided', async () => {
    await handleSlashActionMessage(
      resolved({
        id: 'slash-compact',
        label: '/compact',
        handler: async () => 'compressed 42 messages',
      }),
    );
    const bubbles = lastAppend();
    expect(bubbles).toHaveLength(1);
    expect(bubbles[0].role).toBe('assistant');
    expect(bubbles[0].content).toBe('compressed 42 messages');
  });

  it('/compact falls back to the skipped message when the handler returns void', async () => {
    await handleSlashActionMessage(
      resolved({ id: 'slash-compact', label: '/compact', handler: async () => {} }),
    );
    expect(lastAppend()[0].content).toBe('✓ /compact — slash.feedback.compactSkipped');
  });

  it('/search feedback reflects the web-search toggle', async () => {
    await handleSlashActionMessage(
      resolved({ id: 'slash-search', label: '/search', handler: async () => {} }),
    );
    // the mocked webSearchEnabled(true) returns true
    expect(lastAppend()[0].content).toBe('✓ slash.feedback.searchOn');
  });

  it('unknown action commands get the generic executed feedback', async () => {
    await handleSlashActionMessage(
      resolved({ id: 'slash-mysql', label: '/mycmd', handler: async () => {} }),
    );
    const fb = lastAppend()[0].content;
    expect(fb).toContain('✓');
    expect(fb).toContain('slash.feedback.executed');
  });
});

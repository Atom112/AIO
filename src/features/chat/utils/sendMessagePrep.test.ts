/**
 * Tests for the outgoing-message preparation (阶段4 第2项拆分切片：sendMessagePrep).
 *
 * buildOutgoingMessages assembles the full API turn: the system stack order,
 * conditional segments (agent prompt / reasoning / web / summary), the history
 * flattening, and the persisted user-message shape.
 */
import { describe, expect, it, vi } from 'vitest';

vi.mock('../../../core/i18n', () => ({
  t: (key: string, params?: Record<string, string>) =>
    params ? `${key}:${JSON.stringify(params)}` : key,
  locale: () => 'zh-CN',
}));
// Skills depend on the global store; isolated here (assembly order only)
vi.mock('./skillMessages', () => ({
  buildSkillMessages: () => [{ role: 'system', content: 'SKILL' }],
}));
// vision helpers read the global store at module load; not exercised by the assembly test
vi.mock('../vision', () => ({
  modelSupportsVision: () => false,
  resolveVisionModel: () => null,
}));
// The agent prompt reads the preset registry; pin it to observe ordering only
vi.mock('../../../core/agent-prompts', () => ({
  buildAgentSystemPrompt: () => 'AGENT_PROMPT',
}));

import { buildOutgoingMessages, type BuildOutgoingMessagesArgs } from './sendMessagePrep';
import type { Assistant, Topic } from '../../../core/store/store';

function baseArgs(overrides: Partial<BuildOutgoingMessagesArgs> = {}): BuildOutgoingMessagesArgs {
  const asst = {
    id: 'a1',
    name: 'A',
    prompt: 'YOU ARE A',
    agentMode: 'off',
    topics: [],
  } as unknown as Assistant;
  const topic = {
    id: 't1',
    name: 'T',
    history: [{ id: 'h1', role: 'user', content: 'earlier' }],
    summary: undefined,
  } as unknown as Topic;
  return {
    currentAsst: asst,
    currentTopic: topic,
    outgoingText: 'hello',
    outgoingFiles: [
      { type: 'text', name: 'f.txt', mimeType: 'text/plain', size: 3, storagePath: '/store/f.txt' },
    ],
    userInput: 'hello',
    reasoningLevel: 'off',
    webSearchEnabled: false,
    isChatMode: true,
    projectInfo: null,
    ...overrides,
  };
}

describe('buildOutgoingMessages', () => {
  it('orders the unconditional system stack: prompt → style → skills → history → user', () => {
    const { messagesForAI } = buildOutgoingMessages(baseArgs());
    const roles = messagesForAI.map((m) => m.role);
    expect(roles).toEqual(['system', 'system', 'system', 'user', 'user']);
    expect(messagesForAI[0].content).toBe('YOU ARE A');
    expect(messagesForAI[1].content).toBe('chat.prompt.style');
    expect(messagesForAI[2].content).toBe('SKILL');
    // history flattened then the new user message last
    expect(messagesForAI[3].content).toBe('earlier');
    expect(messagesForAI[4].content).toBe('hello');
  });

  it('appends the reasoning prompt only for a known reasoning level', () => {
    const { messagesForAI } = buildOutgoingMessages(baseArgs({ reasoningLevel: 'high' }));
    const reasoning = messagesForAI.filter((m) => m.content === 'agent.prompt.reasoningHigh');
    expect(reasoning).toHaveLength(1);
    // order: after skills, before history
    expect(messagesForAI[3].content).toBe('agent.prompt.reasoningHigh');
  });

  it('suppresses the reasoning prompt nudge when the native effort parameter applies', () => {
    const { messagesForAI } = buildOutgoingMessages(
      baseArgs({ reasoningLevel: 'high', reasoningParamApplies: true }),
    );
    expect(
      messagesForAI.some((m) => typeof m.content === 'string' && m.content.includes('reasoning')),
    ).toBe(false);
  });

  it('includes the web-search prompt only when enabled', () => {
    const { messagesForAI } = buildOutgoingMessages(baseArgs({ webSearchEnabled: true }));
    expect(messagesForAI.some((m) => m.content === 'agent.prompt.web')).toBe(true);
  });

  it('includes the topic summary only when present', () => {
    const topic = {
      id: 't1',
      name: 'T',
      history: [],
      summary: 'PAST WORK',
    } as unknown as Topic;
    const { messagesForAI } = buildOutgoingMessages(baseArgs({ currentTopic: topic }));
    const summary = messagesForAI.find(
      (m) => typeof m.content === 'string' && m.content.includes('PAST WORK'),
    );
    expect(summary).toBeTruthy();
    expect((summary!.content as string).startsWith('agent.prompt.summary:')).toBe(true);
  });

  it('injects the agent system prompt only in agent mode with a project', () => {
    const withAgent = buildOutgoingMessages(
      baseArgs({
        currentAsst: { agentMode: 'auto' } as unknown as Assistant,
        projectInfo: { path: '/p', name: 'P' },
        isChatMode: false,
      }),
    );
    expect(withAgent.messagesForAI.some((m) => m.content === 'AGENT_PROMPT')).toBe(true);

    // no project -> no agent prompt even in agent mode
    const noProject = buildOutgoingMessages(
      baseArgs({
        currentAsst: { agentMode: 'auto' } as unknown as Assistant,
        projectInfo: null,
        isChatMode: false,
      }),
    );
    expect(noProject.messagesForAI.some((m) => m.content === 'AGENT_PROMPT')).toBe(false);
  });

  it('builds the persisted user message with display files and text', () => {
    const { newUserMsg } = buildOutgoingMessages(baseArgs());
    expect(newUserMsg.role).toBe('user');
    expect(newUserMsg.content).toBe('hello');
    expect(newUserMsg.displayText).toBe('hello');
    const df = newUserMsg.displayFiles![0];
    expect(df.name).toBe('f.txt');
    expect(df.storagePath).toBe('/store/f.txt');
  });
});

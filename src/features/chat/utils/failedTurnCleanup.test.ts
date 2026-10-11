import { describe, expect, it } from 'vitest';
import type { AgentStep, Message, ToolCallDisplay } from '../../../core/store/types';
import { cleanupFailedPlaceholder } from './failedTurnCleanup';

const userMsg = (id: string): Message => ({ id, role: 'user', content: 'hi' });
const asstMsg = (id: string, extra: Partial<Message>): Message =>
  ({ id, role: 'assistant', content: '', ...extra }) as Message;

const step = (id: string, status: AgentStep['status']): AgentStep =>
  ({ id, type: 'thinking', timestamp: 1_000, status }) as AgentStep;
const toolStep = (id: string, state: ToolCallDisplay['state']): AgentStep =>
  ({
    id,
    type: 'tool_call',
    timestamp: 1_000,
    status: 'running',
    toolCall: { id, type: 'function', function: { name: 'x', arguments: '{}' }, state },
  }) as AgentStep;
const toolCall = (id: string, state: NonNullable<ToolCallDisplay['state']>): ToolCallDisplay => ({
  id,
  type: 'function',
  function: { name: 'x', arguments: '{}' },
  state,
});

describe('cleanupFailedPlaceholder (CHAT-10)', () => {
  it('drops an untouched empty placeholder entirely', () => {
    const hist = [userMsg('u1'), asstMsg('a1', {})];
    const out = cleanupFailedPlaceholder(hist);
    expect(out).toHaveLength(1);
    expect(out[0].id).toBe('u1');
  });

  it('keeps a placeholder with content, closing running steps as complete', () => {
    const steps = [step('s1', 'running'), step('s2', 'complete')];
    const hist = [userMsg('u1'), asstMsg('a1', { content: 'partial', agentSteps: steps })];
    const out = cleanupFailedPlaceholder(hist, 3_000);
    expect(out).toHaveLength(2);
    const kept = (out[1] as Message).agentSteps!;
    expect(kept[0].status).toBe('complete');
    expect(kept[0].duration).toBe(2_000);
    expect(kept[1].status).toBe('complete');
  });

  it('marks a running tool_call step (calling state) as error, not complete', () => {
    const hist = [userMsg('u1'), asstMsg('a1', { agentSteps: [toolStep('s1', 'calling')] })];
    const out = cleanupFailedPlaceholder(hist, 2_000);
    const kept = (out[1] as Message).agentSteps!;
    expect(kept[0].status).toBe('error');
  });

  it('closes orphaned calling toolCalls with the interrupted marker', () => {
    const hist = [
      userMsg('u1'),
      asstMsg('a1', {
        content: 'partial',
        toolCalls: [toolCall('tc1', 'calling'), toolCall('tc2', 'success')],
      }),
    ];
    const out = cleanupFailedPlaceholder(hist);
    const tcs = (out[1] as Message).toolCalls!;
    expect(tcs[0].state).toBe('error');
    expect(tcs[0].error).toBe('[Interrupted by user]');
    expect(tcs[1].state).toBe('success');
  });

  it('identity when the last message is not an assistant placeholder', () => {
    const hist = [asstMsg('a1', { content: 'x' }), userMsg('u1')];
    expect(cleanupFailedPlaceholder(hist)).toBe(hist);
  });

  it('identity for an empty history', () => {
    expect(cleanupFailedPlaceholder([])).toEqual([]);
  });
});

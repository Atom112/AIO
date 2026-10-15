import { describe, expect, it } from 'vitest';
import type {
  AgentStep,
  Assistant,
  Message,
  Topic,
  ToolCallDisplay,
} from '../../../core/store/types';
import { sanitizeLoadedAssistants } from './interruptedCleanup';
import { TOOL_INTERRUPTED } from '../constants';

const userMsg = (id: string): Message => ({ id, role: 'user', content: 'hi' });
const asstMsg = (id: string, extra: Partial<Message>): Message =>
  ({ id, role: 'assistant', content: '', ...extra }) as Message;
const toolCall = (id: string, state: ToolCallDisplay['state']): ToolCallDisplay => ({
  id,
  type: 'function',
  function: { name: 'x', arguments: '{}' },
  state,
});
const toolStep = (id: string, state: ToolCallDisplay['state']): AgentStep =>
  ({
    id,
    type: 'tool_call',
    timestamp: 1_000,
    status: 'running',
    toolCall: toolCall(id, state),
  }) as AgentStep;
const thinkStep = (id: string): AgentStep =>
  ({ id, type: 'thinking', timestamp: 1_000, status: 'running' }) as AgentStep;

const topicOf = (id: string, history: Message[]): Topic =>
  ({ id, name: id, history, summary: '' }) as Topic;
const asstOf = (id: string, topics: Topic[]): Assistant =>
  ({ id, name: id, topics }) as unknown as Assistant;

describe('sanitizeLoadedAssistants（冷启动崩溃遗留清理）', () => {
  it('calling 工具调用标记为中断错误，正在运行的步骤收尾', () => {
    const msg = asstMsg('a1', {
      content: 'partial',
      toolCalls: [toolCall('t1', 'calling'), toolCall('t2', 'success')],
      agentSteps: [toolStep('s1', 'calling'), thinkStep('s2')],
    });
    const { assistants, changedAssistantIds } = sanitizeLoadedAssistants([
      asstOf('asst1', [topicOf('tp1', [userMsg('u1'), msg])]),
    ]);
    expect(changedAssistantIds).toEqual(['asst1']);
    const out = assistants[0].topics[0].history[1];
    expect(out.toolCalls![0]).toMatchObject({ state: 'error', error: TOOL_INTERRUPTED });
    expect(out.toolCalls![1].state).toBe('success');
    const steps = out.agentSteps!;
    expect(steps[0].status).toBe('error');
    expect(steps[0].toolCall).toMatchObject({ state: 'error', error: TOOL_INTERRUPTED });
    // 非工具类 running 步骤收为 complete，与 done 处理器语义一致
    expect(steps[1].status).toBe('complete');
    // 冷启动路径不设 duration（崩溃回合的真实耗时不可知）
    expect(steps[0].duration).toBeUndefined();
  });

  it('完全空白的末尾占位消息被丢弃', () => {
    const { assistants, changedAssistantIds } = sanitizeLoadedAssistants([
      asstOf('asst1', [topicOf('tp1', [userMsg('u1'), asstMsg('a1', {})])]),
    ]);
    expect(changedAssistantIds).toEqual(['asst1']);
    expect(assistants[0].topics[0].history).toHaveLength(1);
  });

  it('占位消息不在末尾时不丢弃（历史完整性优先）', () => {
    const hist = [
      userMsg('u1'),
      asstMsg('a1', {}),
      userMsg('u2'),
      asstMsg('a2', { content: 'ok' }),
    ];
    const { assistants, changedAssistantIds } = sanitizeLoadedAssistants([
      asstOf('asst1', [topicOf('tp1', hist)]),
    ]);
    expect(changedAssistantIds).toEqual([]);
    expect(assistants[0].topics[0].history).toHaveLength(4);
  });

  it('干净数据原样透传（引用不变，无待持久化项）', () => {
    const clean = asstOf('asst1', [
      topicOf('tp1', [userMsg('u1'), asstMsg('a1', { content: 'done' })]),
    ]);
    const { assistants, changedAssistantIds } = sanitizeLoadedAssistants([clean]);
    expect(changedAssistantIds).toEqual([]);
    expect(assistants[0]).toBe(clean);
  });

  it('跨话题/跨助手各自独立清理并报告变更集合', () => {
    const dirty1 = asstOf('asst1', [topicOf('tp1', [userMsg('u1'), asstMsg('a1', {})])]);
    const clean2 = asstOf('asst2', [
      topicOf('tp2', [userMsg('u2'), asstMsg('a2', { content: 'x' })]),
    ]);
    const dirty3 = asstOf('asst3', [
      topicOf('tp3', [userMsg('u3'), asstMsg('a3', { toolCalls: [toolCall('t9', 'calling')] })]),
    ]);
    const { changedAssistantIds } = sanitizeLoadedAssistants([dirty1, clean2, dirty3]);
    expect(changedAssistantIds).toEqual(['asst1', 'asst3']);
  });
});

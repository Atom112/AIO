/**
 * Tests for the per-assistant chat history search (searchChatHistory).
 *
 * Coverage: scope isolation (assistant boundary = project boundary), matching
 * semantics (case-insensitive / CJK substring), skipped rows (tool results,
 * id-less transients, multimodal array content), the displayText override,
 * snippet composition with ellipses, and the result cap.
 */
import { describe, expect, it } from 'vitest';
import { searchChatHistory } from './searchHistory';
import type { Assistant, Message, Topic } from '../../../core/store/types';

const msg = (partial: Partial<Message> & { role: Message['role']; id?: string }): Message =>
  ({ content: '', ...partial }) as Message;

const topic = (id: string, name: string, history: Message[]): Topic =>
  ({ id, name, history, summary: '' }) as Topic;

const assistant = (id: string, topics: Topic[]): Assistant =>
  ({ id, name: id, prompt: '', topics }) as Assistant;

describe('searchChatHistory', () => {
  it('returns empty for blank queries', () => {
    const a = assistant('a1', [
      topic('t1', 'T1', [msg({ id: 'm1', role: 'user', content: 'hello' })]),
    ]);
    expect(searchChatHistory(a, '')).toEqual([]);
    expect(searchChatHistory(a, '   ')).toEqual([]);
    expect(searchChatHistory(a, 'hello')).toHaveLength(1);
  });

  it('scopes strictly to the given assistant (project isolation)', () => {
    const other = assistant('a-other', [
      topic('t-other', 'Other project topic', [
        msg({ id: 'mo', role: 'user', content: 'secret term' }),
      ]),
    ]);
    const mine = assistant('a-mine', [
      topic('t-mine', 'My topic', [msg({ id: 'mm', role: 'user', content: 'nothing relevant' })]),
    ]);
    // Searching MY assistant never surfaces the other assistant's records…
    expect(searchChatHistory(mine, 'secret')).toEqual([]);
    // …and searching the OTHER assistant never leaks mine.
    expect(searchChatHistory(other, 'nothing')).toEqual([]);
    expect(searchChatHistory(other, 'secret')).toHaveLength(1);
  });

  it('matches case-insensitively and across CJK text', () => {
    const a = assistant('a1', [
      topic('t1', 'T1', [
        msg({ id: 'm1', role: 'user', content: 'Deploy the Retriever service' }),
        msg({ id: 'm2', role: 'assistant', content: '已部署检索服务，正在验证。' }),
      ]),
    ]);
    expect(searchChatHistory(a, 'retriever')[0].messageId).toBe('m1');
    expect(searchChatHistory(a, '检索')[0].messageId).toBe('m2');
  });

  it('skips hidden tool rows and id-less transient messages', () => {
    const a = assistant('a1', [
      topic('t1', 'T1', [
        msg({ id: 'mt', role: 'tool', content: 'needle here' }),
        msg({ role: 'user', content: 'needle there' }), // no id → not jumpable, skipped
        msg({ id: 'mu', role: 'user', content: 'a needle row' }),
      ]),
    ]);
    const hits = searchChatHistory(a, 'needle');
    expect(hits).toHaveLength(1);
    expect(hits[0].messageId).toBe('mu');
  });

  it('prefers the displayText override (user-message display text)', () => {
    const a = assistant('a1', [
      topic('t1', 'T1', [
        msg({ id: 'm1', role: 'user', content: 'raw-payload', displayText: 'visible words' }),
      ]),
    ]);
    expect(searchChatHistory(a, 'visible')).toHaveLength(1);
    expect(searchChatHistory(a, 'raw-payload')).toEqual([]);
  });

  it('skips multimodal array content without a displayText override', () => {
    const a = assistant('a1', [
      topic('t1', 'T1', [
        msg({ id: 'm1', role: 'user', content: [{ type: 'text', text: 'needle' } as never] }),
      ]),
    ]);
    expect(searchChatHistory(a, 'needle')).toEqual([]);
  });

  it('composes a first-hit snippet with ellipses on both sides', () => {
    const text = `${'x'.repeat(30)}needle${'y'.repeat(60)}`;
    const a = assistant('a1', [
      topic('t1', 'My Topic', [msg({ id: 'm1', role: 'user', content: text })]),
    ]);
    const [hit] = searchChatHistory(a, 'needle');
    expect(hit.topicName).toBe('My Topic');
    expect(hit.snippet.hit).toBe('needle');
    // The hit sits 30 chars in (> SNIPPET_BEFORE) and 60 chars from the end (> SNIPPET_AFTER)
    expect(hit.snippet.before).toBe(`…${'x'.repeat(18)}`);
    expect(hit.snippet.after).toBe(`${'y'.repeat(42)}…`);
  });

  it('caps results at 50 matches', () => {
    const history: Message[] = Array.from({ length: 80 }, (_, i) =>
      msg({ id: `m${i}`, role: 'user', content: `hit number ${i}` }),
    );
    const a = assistant('a1', [topic('t1', 'T1', history)]);
    expect(searchChatHistory(a, 'hit')).toHaveLength(50);
  });

  it('keeps emoji surrogate pairs intact at snippet boundaries', () => {
    // needle starts at index 21, so the 18-char lookback window starts at index 3 —
    // exactly the LOW surrogate of 👍 (indices 2-3). safeSlice must back off one
    // char so the pair survives the cut instead of being split in half.
    const text = `xx👍${'z'.repeat(16)} needle ${'y'.repeat(60)}`;
    const a = assistant('a1', [
      topic('t1', 'T1', [msg({ id: 'm1', role: 'user', content: text })]),
    ]);
    const [hit] = searchChatHistory(a, 'needle');
    expect(hit.snippet.before.startsWith('…')).toBe(true);
    // The full emoji sequence is present in the snippet (a split pair would
    // leave only the high surrogate behind)
    expect(hit.snippet.before).toContain('👍');
    expect(hit.snippet.after.endsWith('…')).toBe(true);
  });
});

import { describe, expect, it, vi } from 'vitest';

// t() reads locale JSON at module load; mock to keep the pure logic under test
vi.mock('../../../core/i18n', () => ({ t: (key: string) => key }));

import { TOOL_INTERRUPTED } from '../constants';
import { buildApiMessages, ensureToolMessagesComplete, truncateToolResult } from './apiMessages';

describe('truncateToolResult', () => {
  it('passes short content through unchanged', () => {
    expect(truncateToolResult('hello')).toBe('hello');
  });

  it('keeps 60% head + 40% tail past the 20k cap with a marker in between', () => {
    const content = 'a'.repeat(20001);
    const out = truncateToolResult(content);
    expect(out).toContain('chat.truncatedHeadTail');
    // head 12000 + tail 8000 + the marker text itself: sliced payload dominates the output
    expect(out.length).toBeGreaterThanOrEqual(20000);
    expect(out.endsWith('a')).toBe(true);
  });
});

describe('buildApiMessages', () => {
  it('passes through a plain message (role + content only)', () => {
    const out = buildApiMessages({ role: 'user', content: 'hi' });
    expect(out).toEqual([{ role: 'user', content: 'hi' }]);
  });

  it('carries images/displayFiles/toolCallId/name onto the payload', () => {
    const out = buildApiMessages({
      role: 'tool',
      content: 'ok',
      toolCallId: 't1',
      name: 'read_file',
      images: ['img1'],
      displayFiles: [{ path: 'a.txt' }],
    })[0];
    expect(out.toolCallId).toBe('t1');
    expect(out.name).toBe('read_file');
    expect(out.images).toEqual(['img1']);
    expect(out.displayFiles).toEqual([{ path: 'a.txt' }]);
  });

  it('expands assistant toolCalls into paired role:tool messages', () => {
    const out = buildApiMessages({
      role: 'assistant',
      content: 'working',
      toolCalls: [
        {
          id: 'a',
          type: 'function',
          function: { name: 'read_file', arguments: '{}' },
          state: 'success',
          content: 'file body',
        },
      ],
    });
    expect(out).toHaveLength(2);
    expect(out[0].toolCalls[0]).toEqual({
      id: 'a',
      type: 'function',
      function: { name: 'read_file', arguments: '{}' },
    });
    expect(out[1]).toMatchObject({
      role: 'tool',
      toolCallId: 'a',
      name: 'read_file',
      content: 'file body',
    });
  });

  it('synthesizes an interrupted error result for orphan calling toolCalls', () => {
    const out = buildApiMessages({
      role: 'assistant',
      content: '',
      toolCalls: [{ id: 'b', function: { name: 'run' }, state: 'calling' }],
    });
    expect(out[1].content).toBe(TOOL_INTERRUPTED);
  });

  it('drops id-less toolCall entries (unpairable, JSON.stringify-unsafe)', () => {
    const out = buildApiMessages({
      role: 'assistant',
      content: '',
      toolCalls: [
        { function: { name: 'x' }, state: 'success', content: 'y' }, // no id
        { id: 'c', function: { name: 'z' }, state: 'error', error: 'boom' },
      ],
    });
    expect(out[0].toolCalls).toHaveLength(1);
    expect(out[1].content).toBe('boom');
  });
});

describe('ensureToolMessagesComplete', () => {
  it('fills a placeholder for a toolCall whose role:tool response is missing', () => {
    const out = ensureToolMessagesComplete([
      { role: 'user', content: 'go' },
      { role: 'assistant', content: '', toolCalls: [{ id: 't9' }] },
      { role: 'user', content: 'next' },
    ]);
    const patched = out.filter((m) => m.role === 'tool');
    expect(patched).toHaveLength(1);
    expect(patched[0].toolCallId).toBe('t9');
  });

  it('leaves complete pairs untouched', () => {
    const msgs = [
      { role: 'assistant', content: '', toolCalls: [{ id: 't1' }] },
      { role: 'tool', content: 'ok', toolCallId: 't1' },
    ];
    expect(ensureToolMessagesComplete(msgs)).toEqual(msgs);
  });

  it('flushes pending ids when a new assistant(toolCalls) starts (consecutive turns)', () => {
    const out = ensureToolMessagesComplete([
      { role: 'assistant', content: '', toolCalls: [{ id: 'p1' }] },
      { role: 'assistant', content: '', toolCalls: [{ id: 'p2' }] },
      { role: 'tool', content: 'ok', toolCallId: 'p2' },
    ]);
    // p1 was still pending when the second assistant started -> placeholder inserted
    const patched = out.filter((m) => m.role === 'tool' && m.content.includes('pending'));
    expect(patched.map((m) => m.toolCallId)).toEqual(['p1']);
  });

  it('final-flushes unmatched ids at the end of the array', () => {
    const out = ensureToolMessagesComplete([
      { role: 'assistant', content: '', toolCalls: [{ id: 'z' }] },
    ]);
    const patched = out.filter((m) => m.role === 'tool');
    expect(patched).toHaveLength(1);
    expect(patched[0].toolCallId).toBe('z');
  });
});

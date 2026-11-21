import { describe, expect, it } from 'vitest';
import {
  ARCHIVE_MESSAGE_CONTENT_CAP,
  buildArchiveMessages,
  canApplyCompaction,
  type SummarizableMessage,
} from './compaction';

describe('buildArchiveMessages', () => {
  it('drops agent steps / tool calls / reasoning from the IPC payload', () => {
    const history = [
      {
        role: 'assistant',
        content: 'done',
        name: 'lookup',
        agentSteps: [{ type: 'tool_call', payload: 'x'.repeat(5_000_000) }],
        toolCalls: [{ function: { name: 'lookup', arguments: 'y'.repeat(2_000_000) } }],
        reasoning: 'z'.repeat(1_000_000),
      },
    ] as unknown as SummarizableMessage[];

    const payload = buildArchiveMessages(history);

    expect(payload).toEqual([{ role: 'assistant', name: 'lookup', content: 'done' }]);
    expect(JSON.stringify(payload).length).toBeLessThan(1_000);
  });

  it('caps each message content and keeps the truncation marker', () => {
    const [msg] = buildArchiveMessages([
      { role: 'tool', name: 'read_file', content: 'a'.repeat(ARCHIVE_MESSAGE_CONTENT_CAP + 50) },
    ]);
    expect(msg.content.startsWith('a'.repeat(ARCHIVE_MESSAGE_CONTENT_CAP))).toBe(true);
    expect(msg.content).toContain('chars truncated');
  });

  it('falls back to displayText when content is absent', () => {
    expect(buildArchiveMessages([{ role: 'user', displayText: 'hello' }])).toEqual([
      { role: 'user', content: 'hello' },
    ]);
  });

  it('never splits a surrogate pair at the cap', () => {
    const content = `${'a'.repeat(ARCHIVE_MESSAGE_CONTENT_CAP - 1)}😀tail`;
    const [msg] = buildArchiveMessages([{ role: 'user', content }]);
    expect(msg.content.startsWith('a'.repeat(ARCHIVE_MESSAGE_CONTENT_CAP - 1))).toBe(true);
    expect(msg.content).not.toContain('\uD83D');
    expect(msg.content).toContain('chars truncated');
  });
});

describe('canApplyCompaction', () => {
  const history = [{ id: 'a' }, { id: 'b' }, { id: 'c' }];

  it('allows the captured boundary when only messages after it were appended', () => {
    expect(
      canApplyCompaction({ contextTrimCount: 0, history: [...history, { id: 'd' }] }, 0, 2, 'b'),
    ).toBe(true);
  });

  it('rejects a stale trim boundary (another compaction won the race)', () => {
    expect(canApplyCompaction({ contextTrimCount: 2, history }, 0, 2, 'b')).toBe(false);
  });

  it('rejects a rewritten history at the boundary', () => {
    expect(
      canApplyCompaction({ contextTrimCount: 0, history: [{ id: 'a' }, { id: 'x' }] }, 0, 2, 'b'),
    ).toBe(false);
  });

  it('rejects a deleted/shortened history', () => {
    expect(canApplyCompaction({ contextTrimCount: 0, history: [{ id: 'a' }] }, 0, 2, 'b')).toBe(
      false,
    );
    expect(canApplyCompaction(null, 0, 2, 'b')).toBe(false);
  });
});

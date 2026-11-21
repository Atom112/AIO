/**
 * Harness event reducer tests: history replacement, command-menu updates and
 * structured subagent cards. The reducer is what turns the ACP-normalized
 * events into the same store shapes the chat components render.
 */
import { beforeEach, describe, expect, it } from 'vitest';
import { handleHarnessEvent } from './harnessEventListeners';
import {
  currentHarnessCommands,
  currentHarnessDraft,
  harnessMessages,
  selectHarnessTopic,
  setHarnessCommands,
  setHarnessDrafts,
  setHarnessMessages,
} from '../../../core/store/harness';
import type { Message } from '../../../core/store/types';

const event = (
  kind: string,
  fields: Record<string, unknown> = {},
  sid = 's1',
): Parameters<typeof handleHarnessEvent>[0] => ({
  tool_id: 'omp',
  session_row_id: sid,
  event: { kind, ...fields } as Parameters<typeof handleHarnessEvent>[0]['event'],
});

describe('harness event reducer', () => {
  beforeEach(() => {
    setHarnessMessages([]);
    setHarnessDrafts({});
    setHarnessCommands({});
    selectHarnessTopic('s1');
  });

  it('replaces the visible history from a session/load snapshot', () => {
    const replayed: Message[] = [
      { id: 'u1', role: 'user', content: 'hi' },
      { id: 'a1', role: 'assistant', content: 'hello' },
    ];
    handleHarnessEvent(event('history', { messages: replayed }));
    expect(harnessMessages()).toEqual(replayed);
  });

  it('replaces the slash-command menu for the topic', () => {
    handleHarnessEvent(
      event('commands_update', {
        commands: [{ name: 'model', description: 'Show model', inputHint: undefined }],
      }),
    );
    expect(currentHarnessCommands()).toHaveLength(1);
    expect(currentHarnessCommands()[0].name).toBe('model');
  });

  it('upserts one subagent card per id and keeps its final result', () => {
    handleHarnessEvent(event('turn_started'));
    handleHarnessEvent(
      event('subagent_update', {
        subagents: [
          {
            id: 'Foo',
            toolCallId: 'call_task',
            agent: 'sonic',
            title: 'Test investigator',
            task: 'Do it',
            status: 'running',
            recentTools: ['read_file'],
            recentOutput: ['half'],
            durationMs: 1200,
          },
        ],
      }),
    );
    let steps = currentHarnessDraft()?.agentSteps ?? [];
    expect(steps).toHaveLength(1);
    expect(steps[0].type).toBe('subagent');
    expect(steps[0].status).toBe('running');
    expect(steps[0].subagentId).toBe('Foo');
    expect(steps[0].subagentProfile).toBe('sonic');
    expect(steps[0].subagentName).toBe('Test investigator');
    expect(steps[0].subagentSteps?.length).toBe(2);

    handleHarnessEvent(
      event('subagent_update', {
        subagents: [
          {
            id: 'Foo',
            toolCallId: 'call_wait',
            agent: 'sonic',
            task: 'Do it',
            status: 'completed',
            recentTools: [],
            recentOutput: [],
            durationMs: 1500,
            result: 'BANANA',
          },
        ],
      }),
    );
    steps = currentHarnessDraft()?.agentSteps ?? [];
    expect(steps).toHaveLength(1);
    expect(steps[0].status).toBe('complete');
    expect(steps[0].duration).toBe(1500);
    expect(steps[0].subagentResult).toBe('BANANA');
  });
});

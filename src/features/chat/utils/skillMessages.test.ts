import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { SkillConfig } from '../../../core/types/skill';

// The store chain (ui-prefs / i18n) touches localStorage / document at module load while
// this suite runs in the node environment; mock the two domain modules the util consumes
// so only buildSkillMessages' own logic is under test.
const skillsState: Record<string, SkillConfig> = {};
vi.mock('../../../core/store/chat', () => ({
  setSkills: (m: Record<string, SkillConfig>) => {
    for (const k of Object.keys(skillsState)) delete skillsState[k];
    Object.assign(skillsState, m);
  },
  resolveAssistantSkills: (a: { skillIds?: string[] } | null | undefined) =>
    (a?.skillIds ?? []).map((id) => skillsState[id]).filter(Boolean),
}));
vi.mock('../../../core/i18n', () => ({ t: (key: string) => key }));

import { setSkills } from '../../../core/store/chat';
import { buildSkillMessages } from './skillMessages';
import type { Assistant } from '../../../core/store/types';

function skill(id: string, name: string, content: string, description = ''): SkillConfig {
  return { id, name, description, content };
}

/** Minimal assistant stub carrying only skillIds (the sole field buildSkillMessages reads). */
function mkAsst(skillIds: string[]): Assistant {
  return { skillIds } as unknown as Assistant;
}

describe('buildSkillMessages', () => {
  beforeEach(() => {
    setSkills({});
  });

  it('returns nothing when the assistant has no enabled skills', () => {
    expect(buildSkillMessages(null, true)).toEqual([]);
    expect(buildSkillMessages(mkAsst([]), true)).toEqual([]);
  });

  it('full inject (chat mode) inlines every enabled skill in order', () => {
    setSkills({
      s2: skill('s2', 'Beta', 'B-content'),
      s1: skill('s1', 'Alpha', 'A-content'),
    });
    const msgs = buildSkillMessages(mkAsst(['s1', 's2']), true);
    expect(msgs).toHaveLength(2);
    expect(msgs[0]).toEqual({ role: 'system', content: '[Skill: Alpha]\nA-content' });
    expect(msgs[1]).toEqual({ role: 'system', content: '[Skill: Beta]\nB-content' });
  });

  it('agent mode fully injects small skills and catalogs large ones', () => {
    const small = 'x'.repeat(800); // exactly the threshold: small
    const large = 'y'.repeat(801);
    setSkills({
      s1: skill('s1', 'Small', small),
      s2: skill('s2', 'Big', large, 'does big things'),
    });
    const msgs = buildSkillMessages(mkAsst(['s1', 's2']), false);
    // small skill fully injected + one catalog notice line
    expect(msgs).toHaveLength(2);
    expect(msgs[0].content).toBe('[Skill: Small]\n' + small);
    expect(msgs[1].content).toContain('Big: does big things');
    expect(msgs[1].content).not.toContain(large);
  });

  it('agent mode with only small skills emits no catalog notice', () => {
    setSkills({ s1: skill('s1', 'Small', 'tiny') });
    const msgs = buildSkillMessages(mkAsst(['s1']), false);
    expect(msgs).toHaveLength(1);
    expect(msgs[0].content).toBe('[Skill: Small]\ntiny');
  });
});

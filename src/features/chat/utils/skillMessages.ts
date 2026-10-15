// Skill system-message construction (token efficiency), extracted from ChatPage.

import { resolveAssistantSkills } from '../../../core/store/chat';
import { t } from '../../../core/i18n';
import type { Assistant } from '../../../core/store/types';

/**
 * Build Skill system messages (token efficiency):
 * - Chat mode (no read_skill tool): inject full skill content (previous behavior);
 * - Agent mode: small skills (<= 800 chars) are always fully injected (avoids the
 *   model missing them, keeps them usable); large skills inject only catalog lines
 *   (name + description) and are read in full on demand via the read_skill tool,
 *   so docx/pptx/xlsx-sized skills are not billed in full on every request.
 */
export function buildSkillMessages(
  asst: Assistant | undefined | null,
  fullInject: boolean,
): { role: 'system'; content: string }[] {
  const skills = resolveAssistantSkills(asst);
  if (skills.length === 0) return [];
  if (fullInject) {
    return skills.map((skill) => ({
      role: 'system' as const,
      content: '[Skill: ' + skill.name + ']\n' + skill.content,
    }));
  }
  const SMALL_SKILL_MAX_CHARS = 800;
  const small = skills.filter((s) => s.content.length <= SMALL_SKILL_MAX_CHARS);
  const large = skills.filter((s) => s.content.length > SMALL_SKILL_MAX_CHARS);
  const msgs = small.map((skill) => ({
    role: 'system' as const,
    content: '[Skill: ' + skill.name + ']\n' + skill.content,
  }));
  if (large.length > 0) {
    msgs.push({
      role: 'system' as const,
      content:
        t('chat.skillCatalogNotice') +
        '\n' +
        large.map((s) => '- ' + s.name + ': ' + (s.description || s.name)).join('\n'),
    });
  }
  return msgs;
}

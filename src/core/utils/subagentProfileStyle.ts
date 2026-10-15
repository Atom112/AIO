/**
 * Subagent profile style registry (shared): icon + role colors per built-in profile.
 *
 * Extracted from SubagentConversationView so every subagent surface (conversation
 * view, pull-up menu, settings page rows) renders the same role identity. Colors
 * come from the per-role CSS variables `--role-<id>-rgb` defined in src/index.css,
 * so this file has no literal colors for the rgba ratchet to count.
 */
import type { IconName } from '../../shared/components/Icon';

export interface SubagentProfileStyle {
  icon: IconName;
  color: string;
  bgColor: string;
  borderColor: string;
}

/** Builds a profile style from the per-role CSS var. */
const role = (id: string, icon: IconName, alpha = 0.9): SubagentProfileStyle => ({
  icon,
  color: `rgba(var(--role-${id}-rgb),${alpha})`,
  bgColor: `rgba(var(--role-${id}-rgb),0.1)`,
  borderColor: `rgba(var(--role-${id}-rgb),0.3)`,
});

const PROFILE_STYLES: Record<string, SubagentProfileStyle> = {
  explorer: role('explorer', 'search'),
  coder: role('coder', 'code'),
  reviewer: role('reviewer', 'review-search'),
  debugger: role('debugger', 'bug'),
  writer: role('writer', 'pen-line'),
  tester: role('tester', 'flask-check', 0.95),
  requirements: role('requirements', 'lightbulb'),
  architect: role('architect', 'compass'),
  vision: role('vision', 'eye'),
  general: role('general', 'sparkles'),
};

const DEFAULT_PROFILE: SubagentProfileStyle = {
  icon: 'sparkles',
  color: 'rgba(var(--role-default-rgb),0.75)',
  bgColor: 'rgba(var(--role-default-rgb),0.06)',
  borderColor: 'rgba(var(--role-default-rgb),0.2)',
};

/** profile id -> color/icon style (custom profiles fall back to the default look) */
export function getSubagentProfileStyle(profileId?: string): SubagentProfileStyle {
  return PROFILE_STYLES[profileId || ''] || DEFAULT_PROFILE;
}

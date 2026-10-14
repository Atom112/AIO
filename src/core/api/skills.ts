// Skills domain Tauri command wrappers (mirrors commands/skill.rs).
// Convention: components must never invoke these commands directly; always go through this module; signatures stay camelCase to match the backend.
import { invoke } from '@tauri-apps/api/core';
import type { DiscoveredNpxSkill, MarketSkill, SkillConfig } from '../types/skill';

export interface SkillMarketQuery {
  sort?: string | null;
  forceRefresh?: boolean;
}

/** List project-scoped skills (global list when projectId is null). */
export const listSkills = (projectId?: string | null) =>
  invoke<SkillConfig[]>('list_skills', { projectId: projectId ?? null });

/** Save (create or update) a skill. */
export const saveSkill = (skill: SkillConfig, projectId?: string | null) =>
  invoke<void>('save_skill', { skill, projectId: projectId ?? null });

/** Delete a skill. */
export const deleteSkill = (id: string, projectId?: string | null) =>
  invoke<void>('delete_skill', { id, projectId: projectId ?? null });

/** Scan the system for importable npx skill packages. */
export const discoverNpxSkills = () => invoke<DiscoveredNpxSkill[]>('discover_npx_skills');

/** Import an npx skill package. sourcePath = on-disk skill dir (skills CLI / skill-dir
 * sources are imported directly from it — their names are not npm packages). */
export const importNpxSkill = (
  packageName: string,
  projectId?: string | null,
  sourcePath?: string | null,
) =>
  invoke<SkillConfig>('import_npx_skill', {
    packageName,
    projectId: projectId ?? null,
    sourcePath: sourcePath ?? null,
  });

/** Refresh an imported npx skill (re-fetches package content). */
export const refreshNpxSkill = (id: string, projectId?: string | null) =>
  invoke<SkillConfig>('refresh_npx_skill', { id, projectId: projectId ?? null });

/** Browse the skill market. */
export const listSkillMarket = (query: SkillMarketQuery) =>
  invoke<MarketSkill[]>('list_skill_market', query as unknown as Record<string, unknown>);

/** Download a skill from the market into a project (or global). */
export const downloadMarketSkill = (skill: MarketSkill, projectId?: string | null) =>
  invoke<SkillConfig>('download_market_skill', {
    owner: skill.owner,
    repo: skill.repo,
    slug: skill.slug,
    projectId: projectId ?? null,
  });

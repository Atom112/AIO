// Skills 域 Tauri 命令封装（与 commands/skill.rs 对应）。
// 约定：组件不得直接 invoke 这些命令名，一律走本模块；签名与后端保持 camelCase。
import { invoke } from '@tauri-apps/api/core';
import type { DiscoveredNpxSkill, MarketSkill, SkillConfig } from '../types/skill';

export interface SkillMarketQuery {
  sort?: string | null;
  forceRefresh?: boolean;
}

/** 列出项目级 Skill（projectId 为空时列全局）。 */
export const listSkills = (projectId?: string | null) =>
  invoke<SkillConfig[]>('list_skills', { projectId: projectId ?? null });

/** 保存（新建或更新）一个 Skill。 */
export const saveSkill = (skill: SkillConfig, projectId?: string | null) =>
  invoke<void>('save_skill', { skill, projectId: projectId ?? null });

/** 删除一个 Skill。 */
export const deleteSkill = (id: string, projectId?: string | null) =>
  invoke<void>('delete_skill', { id, projectId: projectId ?? null });

/** 扫描系统上可导入的 npx skill 包。 */
export const discoverNpxSkills = () => invoke<DiscoveredNpxSkill[]>('discover_npx_skills');

/** 导入一个 npx skill 包。 */
export const importNpxSkill = (packageName: string, projectId?: string | null) =>
  invoke<SkillConfig>('import_npx_skill', { packageName, projectId: projectId ?? null });

/** 刷新已导入的 npx skill（重新拉取包内容）。 */
export const refreshNpxSkill = (id: string, projectId?: string | null) =>
  invoke<SkillConfig>('refresh_npx_skill', { id, projectId: projectId ?? null });

/** 浏览 Skill 市场。 */
export const listSkillMarket = (query: SkillMarketQuery) =>
  invoke<MarketSkill[]>('list_skill_market', query as unknown as Record<string, unknown>);

/** 从市场下载一个 Skill 到项目（或全局）。 */
export const downloadMarketSkill = (skill: MarketSkill, projectId?: string | null) =>
  invoke<SkillConfig>('download_market_skill', {
    owner: skill.owner,
    repo: skill.repo,
    slug: skill.slug,
    projectId: projectId ?? null,
  });

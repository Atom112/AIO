/** 可复用的助手系统指令模块。 */
export interface SkillConfig {
    id: string;
    name: string;
    description: string;
    content: string;
    sourceUrl?: string;
    sourceOwner?: string;
    sourceRepo?: string;
    sourceSlug?: string;
    installs?: number;
}

export interface MarketSkill {
    id: string;
    name: string;
    owner: string;
    repo: string;
    slug: string;
    description: string;
    sourceUrl: string;
    installs: number;
    installsLabel: string;
    weeklyInstalls: number[];
    category?: string;
}

export interface SkillMarketCategory {
    id: string;
    name: string;
    description: string;
    skillCount: number;
}

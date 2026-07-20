/** Skill 来源类型。 */
export type SkillSource = 'local' | 'market' | 'npx';

/** 可复用的助手系统指令模块。 */
export interface SkillConfig {
    id: string;
    name: string;
    description: string;
    content: string;
    /** Skill 来源。缺省为 local。 */
    source?: SkillSource;
    sourceUrl?: string;
    sourceOwner?: string;
    sourceRepo?: string;
    sourceSlug?: string;
    installs?: number;
    /** npx 包名（source = npx 时必填） */
    npxPackage?: string;
    /** npx 已安装版本号 */
    npxVersion?: string;
    /** npx 执行命令 */
    npxCommand?: string;
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

/** npx skill 发现结果：系统上检测到的可导入 Skill 包。 */
export interface DiscoveredNpxSkill {
    packageName: string;
    version: string;
    description: string;
    sourcePath: string;
    sourceType: string;       // "claude-skills-dir" | "global-npm"
    alreadyImported: boolean;
}

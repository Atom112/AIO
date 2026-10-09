/** Skill source type. */
export type SkillSource = 'local' | 'market' | 'npx';

/** Reusable assistant system-instruction module. */
export interface SkillConfig {
  id: string;
  name: string;
  description: string;
  content: string;
  /** Skill source. Defaults to local. */
  source?: SkillSource;
  sourceUrl?: string;
  sourceOwner?: string;
  sourceRepo?: string;
  sourceSlug?: string;
  installs?: number;
  /** npx package name (required when source = npx) */
  npxPackage?: string;
  /** Installed npx version */
  npxVersion?: string;
  /** npx execution command */
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
}

/** npx skill discovery result: importable skill packages detected on the system. */
export interface DiscoveredNpxSkill {
  packageName: string;
  version: string;
  description: string;
  sourcePath: string;
  sourceType: string; // "claude-skills-dir" | "global-npm"
  alreadyImported: boolean;
}

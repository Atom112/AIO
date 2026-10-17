/**
 * Harness tool logos — brand marks for the CLI tools on the harness page.
 *
 * Sources (user directive: prefer @lobehub/icons-static-svg):
 * - Color variants straight from the package (data URLs via svgToDataUrl, same
 *   pipeline as the provider logos): claude / gemini / qwen / deepseek (DSH).
 * - Brands that only ship a `currentColor` monochrome glyph (or a dark-surface
 *   color variant, e.g. codex-color's white spiral) get a badge treatment:
 *   the lobehub glyph in white on a navy rounded chip (static assets under
 *   public/icons/harness/, generated once from the package) so they read on
 *   both light and dark themes.
 * - Oh My Pi (omp) has no lobehub icon: hand-drawn violet π mark
 *   (public/icons/harness/omp.svg).
 *
 * Custom user-registered tools have no logo — the caller falls back to an
 * initials chip.
 */
import claudeColorSvg from '@lobehub/icons-static-svg/icons/claude-color.svg?raw';
import geminiColorSvg from '@lobehub/icons-static-svg/icons/gemini-color.svg?raw';
import qwenColorSvg from '@lobehub/icons-static-svg/icons/qwen-color.svg?raw';
import deepseekColorSvg from '@lobehub/icons-static-svg/icons/deepseek-color.svg?raw';
import { svgToDataUrl } from '../../../core/utils/modelLogo';

/** Built-in tool id → logo URL (data URL or public asset path). */
const BUILTIN_LOGOS: Record<string, string> = {
  claude: svgToDataUrl(claudeColorSvg),
  gemini: svgToDataUrl(geminiColorSvg),
  qwen: svgToDataUrl(qwenColorSvg),
  // DSH is DeepSeek's harness — the DeepSeek brand mark.
  dsh: svgToDataUrl(deepseekColorSvg),
  // Badge-wrapped monochrome glyphs (visible on both themes).
  codex: '/icons/harness/codex.svg',
  goose: '/icons/harness/goose.svg',
  opencode: '/icons/harness/opencode.svg',
  // Hand-drawn π (no lobehub icon for Oh My Pi).
  omp: '/icons/harness/omp.svg',
};

/** Logo URL for a harness tool id; null for custom tools (caller renders initials). */
export const getHarnessLogo = (toolId: string): string | null => BUILTIN_LOGOS[toolId] ?? null;

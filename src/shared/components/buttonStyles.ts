/**
 * Shared button style constants.
 *
 * Buttons of the same kind must reuse the constants in this file; do not hand-write
 * equivalent styles in components, so the same button kind stays consistent across all pages/components/modals.
 *
 * Usage:
 *   <button class={btnPrimaryOutline}>{t('common.confirm')}</button>
 *   Append extra layout classes (w-full / ml-auto / flex-1 etc.) at the call site:
 *   <button class={'w-full ' + btnPrimaryOutlineSm}>...</button>
 *
 * Semantic-color rule (AGENTS.md): buttons with a semantic color (confirm / close /
 * delete / test / start / stop ...) use the Outline family ONLY — the BORDER and the
 * TEXT carry the semantic color, the body stays neutral (bg-white/[0.06], same as
 * secondary). Solid fills (bg-pri) and tinted bodies (bg-danger/10 etc.) are banned
 * for text action buttons. Neutral actions use btnSecondary / btnSecondarySm.
 *
 * Size conventions (pick by scenario; do not mix within one scenario):
 *   - btnPrimaryOutline / btnSecondary        modal footers (confirm/create etc.)
 *   - *Sm / *OutlineSm                        inline actions on settings pages (save/test/start etc.)
 *   - btnBadge / btnBadgeBar                  small pill inline actions / toolbar alignment
 *   - btnGhost / btnIcon / btnClose           text buttons / icon buttons / modal close
 *   - btnTabMd / btnTabXs + tabActive/tabInactive  segmented controls / tabs
 */

/** Primary action button (confirm/create etc. in modal footers) — outline: primary border + text */
export const btnPrimaryOutline =
  'inline-flex items-center justify-center gap-1.5 px-4 py-2 rounded-lg text-sm font-medium border border-pri-50 bg-white/[0.06] text-pri hover:bg-pri-10 active:scale-95 transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer';

/** Primary action button (compact, inline actions on settings pages) — outline */
export const btnPrimaryOutlineSm =
  'inline-flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium border border-pri-50 bg-white/[0.06] text-pri hover:bg-pri-10 active:scale-95 transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer';

/** Secondary action button (cancel/later etc. in modal footers) */
export const btnSecondary =
  'inline-flex items-center justify-center gap-1.5 px-4 py-2 rounded-lg text-sm font-medium bg-white/[0.06] border border-white/10 text-white/60 hover:bg-white/[0.1] hover:text-white/85 active:scale-95 transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer';

/** Secondary action button (compact, inline actions on settings pages) */
export const btnSecondarySm =
  'inline-flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium bg-white/[0.06] border border-white/10 text-white/60 hover:bg-white/[0.1] hover:text-white/85 active:scale-95 transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer';

/** Success/approve action button (test connection, approve, start etc.) — outline: success border + text */
export const btnSuccessOutline =
  'inline-flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium border border-success/50 bg-white/[0.06] text-success hover:bg-success/10 active:scale-95 transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer';

/** Destructive action button (delete/remove/stop etc.) — outline: danger border + text */
export const btnDangerOutline =
  'inline-flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium border border-danger/50 bg-white/[0.06] text-danger hover:bg-danger/10 active:scale-95 transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer';

/** Security/credential action button (save-to-keychain etc.) — outline: warn border + text */
export const btnWarnOutline =
  'inline-flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium border border-warn/50 bg-white/[0.06] text-warn hover:bg-warn/10 active:scale-95 transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer';

/** Small pill inline action (height-matched to a p-1 container + btnTabXs segmented control bar, for inline toolbar alignment) */
export const btnBadgeBar =
  'inline-flex items-center justify-center gap-1 px-2.5 py-[7px] rounded-md text-xs font-medium bg-white/[0.06] border border-white/10 text-white/60 hover:bg-white/[0.1] hover:text-white/85 transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer';

/** Small pill inline action (tags/small buttons) */
export const btnBadge =
  'inline-flex items-center justify-center gap-1 px-2 py-1 rounded-md text-[11px] font-medium bg-white/[0.06] border border-white/10 text-white/60 hover:bg-white/[0.1] hover:text-white/85 transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer';

/** Ghost button (borderless text button) */
export const btnGhost =
  'inline-flex items-center justify-center bg-transparent border-none text-white/60 hover:text-white/90 transition-colors duration-200 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer';

/** Icon button (borderless square, toolbar/inline actions) */
export const btnIcon =
  'flex items-center justify-center bg-transparent border-none rounded-md transition-all duration-200 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer';

/** Modal top-right close button */
export const btnClose =
  'inline-flex items-center justify-center shrink-0 w-8 h-8 rounded-lg bg-transparent border-none cursor-pointer p-0 transition-all duration-200 text-white/40 hover:text-white hover:bg-danger/80';

/** Segmented control / tab: medium-size base class */
export const btnTabMd =
  'px-3 py-1.5 rounded-md text-sm transition-colors duration-200 cursor-pointer';

/** Segmented control / tab: small-size base class */
export const btnTabXs =
  'px-2.5 py-1 rounded-md text-xs transition-colors duration-200 cursor-pointer';

/** Segmented control / tab: selected state */
export const tabActive = 'bg-pri-20 text-white';

/** Segmented control / tab: unselected state */
export const tabInactive = 'text-white/50 hover:text-white/75';

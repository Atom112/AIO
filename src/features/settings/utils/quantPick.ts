/**
 * Quant-family ordering and default-pick rule for the local model market (pure functions).
 *
 * The market row's collapsed download button fetches exactly ONE quant family (a single
 * file, or every shard of one split GGUF) — never all variants. That makes the implicit
 * "which one" a product decision that must be predictable and must not silently land on a
 * low-quality tier.
 *
 * The previous rule was: among families whose name matched a whitelist
 * (q4_k_m / q4_k_s / q5_k_m / iq4_xs / ud-q4_k_xl) take the first by name, otherwise take
 * the first COMPLETE family by name. The fallback was wrong: in a repo that ships only
 * Q2_K / Q3_K_M / Q4_0 / Q6_K / Q8_0, alphabetic order picks **Q2_K** — the worst-quality
 * tier — and the collapsed row gave no hint about it.
 *
 * The rule now: among families whose shards are complete, prefer the one closest to the
 * Q4_K_M sweet spot — nearest level to 4 first, then the in-level variant
 * (K_M > K_XL > K_L > K_S > K > K_XS > _0), then the higher level on a tie (quality over
 * size when the choice is symmetric). The single "recommended" badge in the expanded list
 * marks this same family, so badge and one-click behaviour can never disagree.
 */

/** Parsed quant tier: `level` is the bits-per-weight tier (Q4 ⇒ 4), `style` the variant. */
interface ParsedQuant {
  level: number;
  style: number;
}

/**
 * Variant preference inside one level, best first. Lower is preferred.
 * 0 = K_M (the usual middle), 1 = K_XL/XXL (unsloth UD sizes), 2 = K_L, 3 = K_S,
 * 4 = bare K, 5 = XS/NL, 6 = legacy _0/_1, 7 = unknown suffix, 8 = self-describing
 * names (f16/bf16/f32/mxfp4).
 */
const STYLE_RULES: ReadonlyArray<readonly [RegExp, number]> = [
  [/^k_m$/, 0],
  [/^k_(?:xl|xxl)$/, 1],
  [/^k_l$/, 2],
  [/^k_s$/, 3],
  [/^k$/, 4],
  [/^(?:xs|xxs|nl)$/, 5],
  [/^[01]$/, 6],
];

/** Names that state the whole precision instead of a Q-tier. */
const BARE_LEVELS: Readonly<Record<string, number>> = {
  f32: 32,
  fp32: 32,
  f16: 16,
  fp16: 16,
  bf16: 16,
  mxfp4: 4,
  int8: 8,
  fp8: 8,
  int4: 4,
};

/**
 * Trailing quant token of a family id, allowing `-`, `_` and `.` before it and the
 * unsloth `UD-` prefix inside it (`…-UD-Q4_K_XL`, `…-Q4_K_M`, `…_q4_k_m`, `….Q4_K_M`).
 * Anchored at the end so a bigger family name never swallows the token.
 *
 * The continuation class excludes `-` and `_` on purpose: an id comes from a remote repo
 * listing, and letting the separator characters also match inside the repetition makes
 * `q0_0_0_0_…` ambiguous (two ways to split it) → exponential backtracking. CodeQL's
 * js/polynomial-redos flags exactly that, and unbounded repo names are untrusted input.
 */
const QUANT_TAIL_RE =
  /(?:^|[-_.])((?:ud[-_])?(?:iq|tq|q)\d{1,2}(?:[-_][a-z0-9]+)*|bf16|f16|f32|fp16|fp32|fp8|int8|int4|mxfp4)$/i;

/** Target tier of the one-click default: the size/quality sweet spot. */
const TARGET_LEVEL = 4;

/** Rank used when a name carries no recognisable quant token (sorted last). */
const UNKNOWN_KEY = [Number.POSITIVE_INFINITY, Number.POSITIVE_INFINITY, 0] as const;

/**
 * Parses one trailing token (`Q4_K_M`, `IQ4_XS`, `UD-Q4_K_XL`, `BF16`, …).
 *
 * Returns null for tokens that are not a quant tier — the caller walks further left,
 * which is what makes `ggml-model-q4_k_m` and `...-UD-Q4_K_XL` resolve correctly.
 */
function parseQuantToken(token: string): ParsedQuant | null {
  const lower = token.trim().toLowerCase();
  const bare = BARE_LEVELS[lower];
  if (bare !== undefined) return { level: bare, style: 8 };
  const m = /^(?:(?:ud|unsloth)[-_])?(iq|tq|q)?(\d{1,2})(?:_([a-z0-9_]+))?$/.exec(lower);
  if (!m) return null;
  const level = Number(m[2]);
  if (!Number.isFinite(level) || level <= 0 || level > 32) return null;
  const suffix = m[3] ?? '';
  const style = STYLE_RULES.find(([re]) => re.test(suffix))?.[1] ?? (suffix ? 7 : 8);
  return { level, style };
}

/** Trailing quant token of a family id, or null when the name carries none. */
function extractQuantToken(id: string): string | null {
  const m = QUANT_TAIL_RE.exec(id);
  return m ? m[1] : null;
}

/** Preference key of a family id: (distance to target tier, variant, higher tier first). */
function preferenceKey(id: string): readonly [number, number, number] {
  const parsed = extractQuantToken(id);
  if (!parsed) return UNKNOWN_KEY;
  const tier = parseQuantToken(parsed);
  if (!tier) return UNKNOWN_KEY;
  return [Math.abs(tier.level - TARGET_LEVEL), tier.style, -tier.level];
}

/**
 * Total preference order over quant-family ids: the default candidate comes first.
 * Used both for the expanded list's sort and for the default pick, so the top row of the
 * expanded list IS the family the collapsed button downloads.
 */
export function compareQuant(a: string, b: string): number {
  const ka = preferenceKey(a);
  const kb = preferenceKey(b);
  for (let i = 0; i < ka.length; i++) {
    if (ka[i] !== kb[i]) return ka[i] < kb[i] ? -1 : 1;
  }
  return a.localeCompare(b);
}

/**
 * The family the row's one-click download fetches: the most preferred family whose shards
 * are complete, or null when every family is incomplete (or the list is empty).
 *
 * `groups` may arrive in any order — the comparison is order-independent.
 */
export function pickDefaultQuant<T extends { id: string; complete: boolean }>(
  groups: readonly T[],
): T | null {
  let best: T | null = null;
  for (const g of groups) {
    if (!g.complete) continue;
    if (!best || compareQuant(g.id, best.id) < 0) best = g;
  }
  return best;
}

/**
 * Short label for the collapsed row (`DeepCoder-14B-Preview-Q4_K_M` → `Q4_K_M`), keeping
 * the original casing. Falls back to the id's last separator segment when no quant token
 * is found (a full file stem would blow up the row's right-hand control cluster).
 */
export function quantLabel(id: string): string {
  const token = extractQuantToken(id);
  if (token) return token;
  const segments = id.split(/[-_.]/);
  return segments[segments.length - 1] || id;
}

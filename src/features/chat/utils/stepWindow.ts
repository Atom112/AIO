/**
 * Historical-round step windowing (pure helpers).
 *
 * Why: an agent round can carry hundreds of steps (the dev machine's real case:
 * one round = 588 steps / 8MB payload), and the outer `max-height: 0` collapse
 * does NOT prevent mounting — every card's DOM was built on topic switch either
 * way. Extracted here so the window arithmetic is unit-testable.
 */

/** Steps mounted for an inactive round before the first "show earlier" click. */
export const STEP_WINDOW = 100;

/**
 * The id slice to render.
 *
 * - active rounds stream through the render queue: all ids pass through untouched;
 * - inactive rounds show the most recent `STEP_WINDOW + revealCount` steps, so
 *   each "show earlier" click mounts a bounded page instead of every card at once
 *   (the historic one-shot reveal mounted all 588 in a single frame → frame drops).
 */
export function windowedStepIds(
  ids: readonly string[],
  isActive: boolean,
  revealCount: number,
  stepWindow: number = STEP_WINDOW,
): readonly string[] {
  if (isActive) return ids;
  const limit = stepWindow + Math.max(0, revealCount);
  return ids.length <= limit ? ids : ids.slice(ids.length - limit);
}

/** How many older steps a "show earlier" click can still reveal (0 = fully shown). */
export function hiddenStepCount(
  ids: readonly string[],
  isActive: boolean,
  revealCount: number,
  stepWindow: number = STEP_WINDOW,
): number {
  if (isActive) return 0;
  return Math.max(0, ids.length - stepWindow - Math.max(0, revealCount));
}

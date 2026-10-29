/**
 * Tests for the historical-round step windowing helpers (pure).
 *
 * Context: one real round on the dev machine carries 588 steps / 8MB; the
 * pre-fix behavior mounted every card for collapsed rounds (topic-switch jank)
 * and revealed all remaining cards in a single frame on "show earlier"
 * (frame-rate collapse). The window must be bounded, page-incremental, and
 * never apply to the active streaming round.
 */
import { describe, expect, it } from 'vitest';
import { hiddenStepCount, windowedStepIds, STEP_WINDOW } from './stepWindow';

const ids = (n: number) => Array.from({ length: n }, (_, i) => `s${i}`);

describe('windowedStepIds', () => {
  it('passes the active round through untouched (streaming queue owns it)', () => {
    const all = ids(588);
    expect(windowedStepIds(all, true, 0)).toBe(all);
    expect(windowedStepIds(all, true, 999)).toBe(all);
  });

  it('caps an inactive round to the tail window', () => {
    const out = windowedStepIds(ids(588), false, 0);
    expect(out).toHaveLength(STEP_WINDOW);
    // The tail (most recent steps), not the head
    expect(out[0]).toBe(`s${588 - STEP_WINDOW}`);
    expect(out[out.length - 1]).toBe('s587');
  });

  it('reveals one bounded page per click instead of everything at once', () => {
    const all = ids(588);
    const page1 = windowedStepIds(all, false, 100);
    expect(page1).toHaveLength(200);
    const page2 = windowedStepIds(all, false, 200);
    expect(page2).toHaveLength(300);
    // Full reveal only once the budget covers every step
    expect(windowedStepIds(all, false, 488)).toHaveLength(588);
    expect(windowedStepIds(all, false, 1000)).toHaveLength(588);
  });

  it('handles short rounds and empty input', () => {
    expect(windowedStepIds(ids(5), false, 0)).toHaveLength(5);
    expect(windowedStepIds([], false, 0)).toHaveLength(0);
    // Negative reveal never shrinks the window below the base page
    expect(windowedStepIds(ids(300), false, -50)).toHaveLength(STEP_WINDOW);
  });
});

describe('hiddenStepCount', () => {
  it('reports the remaining older steps and counts down per page', () => {
    const all = ids(588);
    expect(hiddenStepCount(all, false, 0)).toBe(488);
    expect(hiddenStepCount(all, false, 100)).toBe(388);
    expect(hiddenStepCount(all, false, 488)).toBe(0);
    expect(hiddenStepCount(all, false, 1000)).toBe(0);
  });

  it('is zero for the active round (nothing is hidden while streaming)', () => {
    expect(hiddenStepCount(ids(588), true, 0)).toBe(0);
  });
});

import { describe, expect, it } from 'vitest';
import { compareQuant, pickDefaultQuant, quantLabel } from './quantPick';

/** Minimal FileGroup shape the rule needs. */
const g = (id: string, complete = true) => ({ id, complete });

/** Sorts ids by preference (default candidate first). */
const ordered = (ids: string[]) => [...ids].sort(compareQuant);

describe('compareQuant / pickDefaultQuant', () => {
  it('prefers Q4_K_M over every other common tier', () => {
    const ids = [
      'DeepCoder-14B-Preview-Q8_0',
      'DeepCoder-14B-Preview-Q6_K',
      'DeepCoder-14B-Preview-Q3_K_L',
      'DeepCoder-14B-Preview-Q4_K_M',
    ];
    expect(ordered(ids)[0]).toBe('DeepCoder-14B-Preview-Q4_K_M');
    expect(pickDefaultQuant(ids.map((id) => g(id)))?.id).toBe('DeepCoder-14B-Preview-Q4_K_M');
  });

  it('never falls back to the worst tier when no whitelist name exists', () => {
    // The historic regression: alphabetic order put Q2_K first and silently downloaded it.
    const ids = ['m-Q2_K', 'm-Q3_K_M', 'm-Q4_0', 'm-Q6_K', 'm-Q8_0'];
    expect(ordered(ids)[0]).toBe('m-Q4_0');
    expect(pickDefaultQuant(ids.map((id) => g(id)))?.id).toBe('m-Q4_0');
  });

  it('prefers the higher tier when the distance to Q4 is symmetric', () => {
    expect(pickDefaultQuant([g('m-Q3_K_M'), g('m-Q5_K_M')])?.id).toBe('m-Q5_K_M');
  });

  it('prefers K_M over the other variants of the same tier', () => {
    const ids = ['m-Q4_0', 'm-IQ4_XS', 'm-Q4_K_S', 'm-Q4_K_M'];
    expect(ordered(ids)).toEqual(['m-Q4_K_M', 'm-Q4_K_S', 'm-IQ4_XS', 'm-Q4_0']);
  });

  it('skips families whose shards are incomplete', () => {
    const groups = [g('m-Q4_K_M', false), g('m-Q6_K'), g('m-Q2_K')];
    expect(pickDefaultQuant(groups)?.id).toBe('m-Q6_K');
    expect(pickDefaultQuant([g('m-Q4_K_M', false), g('m-Q8_0', false)])).toBeNull();
    expect(pickDefaultQuant([])).toBeNull();
  });

  it('reads two-segment quant tokens (unsloth UD sizes, ggml naming)', () => {
    const ids = ['m-UD-Q4_K_XL', 'm-Q2_K', 'm-Q8_0'];
    expect(pickDefaultQuant(ids.map((id) => g(id)))?.id).toBe('m-UD-Q4_K_XL');
    expect(pickDefaultQuant([g('ggml-model-q4_k_m'), g('ggml-model-q8_0')])?.id).toBe(
      'ggml-model-q4_k_m',
    );
  });

  it('ranks names without a quant token last', () => {
    const ids = ['m-instruct', 'm-Q2_K'];
    expect(pickDefaultQuant(ids.map((id) => g(id)))?.id).toBe('m-Q2_K');
  });

  it('handles dotted and underscore-separated file stems', () => {
    const ids = ['Model.Q4_K_M', 'Model.Q2_K', 'Model.Q8_0'];
    expect(pickDefaultQuant(ids.map((id) => g(id)))?.id).toBe('Model.Q4_K_M');
    expect(pickDefaultQuant([g('model_q8_0'), g('model_q4_k_m')])?.id).toBe('model_q4_k_m');
  });
});

describe('quantLabel', () => {
  it('keeps the original casing of the trailing quant token', () => {
    expect(quantLabel('DeepCoder-14B-Preview-Q4_K_M')).toBe('Q4_K_M');
    expect(quantLabel('Qwen3-14B-IQ4_XS')).toBe('IQ4_XS');
    // The unsloth UD- prefix belongs to the tier name and stays in the label
    expect(quantLabel('model-UD-Q4_K_XL')).toBe('UD-Q4_K_XL');
    expect(quantLabel('model-bf16')).toBe('bf16');
    expect(quantLabel('Model.Q4_K_M')).toBe('Q4_K_M');
  });

  it('falls back to the last separator segment when no quant token is found', () => {
    expect(quantLabel('some-model-instruct')).toBe('instruct');
  });
});

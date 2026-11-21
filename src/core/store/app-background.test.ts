import { describe, expect, it } from 'vitest';
import {
  buildBackgroundLayerStyle,
  DEFAULT_APP_BACKGROUND,
  normalizeAppBackground,
  type AppBackgroundSettings,
} from './app-background';

// The background domain is a leaf module (solid signals + pure helpers). Its
// normalization and layer-style rules are what keep a corrupted localStorage entry
// from blanking the UI, so they are pinned here.

const settings = (patch: Partial<AppBackgroundSettings> = {}): AppBackgroundSettings => ({
  ...DEFAULT_APP_BACKGROUND,
  ...patch,
});

const resolveAsset = (path: string) => `http://asset.localhost/${encodeURIComponent(path)}`;

// Arbitrary fixture colors: they never reach the UI, they only exercise string handling.
// gate-allow: color test fixture
const FIXTURE_SOLID = '#123456';
// gate-allow: color test fixture
const FIXTURE_END = '#abcdef';

describe('normalizeAppBackground', () => {
  it('损坏 JSON / 非对象输入回退默认值', () => {
    expect(normalizeAppBackground('{not json')).toEqual(DEFAULT_APP_BACKGROUND);
    expect(normalizeAppBackground(null)).toEqual(DEFAULT_APP_BACKGROUND);
    expect(normalizeAppBackground(42)).toEqual(DEFAULT_APP_BACKGROUND);
  });

  it('未知 kind 与非 hex 颜色逐字段回退默认值', () => {
    const normalized = normalizeAppBackground({
      kind: 'rainbow',
      solidColor: 'red',
      gradientStart: '#1234',
      gradientEnd: FIXTURE_END,
      imagePath: 7,
    });
    expect(normalized.kind).toBe('default');
    expect(normalized.solidColor).toBe(DEFAULT_APP_BACKGROUND.solidColor);
    expect(normalized.gradientStart).toBe(DEFAULT_APP_BACKGROUND.gradientStart);
    expect(normalized.gradientEnd).toBe(FIXTURE_END);
    expect(normalized.imagePath).toBeNull();
  });

  it('角度与透明度 clamp 到合法区间', () => {
    expect(normalizeAppBackground({ opacity: 140 }).opacity).toBe(100);
    expect(normalizeAppBackground({ opacity: -5 }).opacity).toBe(0);
    expect(normalizeAppBackground({ gradientAngle: 400 }).gradientAngle).toBe(360);
    expect(normalizeAppBackground({ gradientAngle: -30 }).gradientAngle).toBe(0);
    expect(normalizeAppBackground({ opacity: 'high' }).opacity).toBe(100);
  });
});

describe('buildBackgroundLayerStyle', () => {
  it('深色模式与默认背景不渲染自定义层', () => {
    expect(buildBackgroundLayerStyle(settings({ kind: 'solid' }), true, resolveAsset)).toBeNull();
    expect(buildBackgroundLayerStyle(settings(), false, resolveAsset)).toBeNull();
  });

  it('单色层返回颜色与透明度', () => {
    const style = buildBackgroundLayerStyle(
      settings({ kind: 'solid', solidColor: FIXTURE_SOLID, opacity: 60 }),
      false,
      resolveAsset,
    );
    expect(style).toEqual({ background: FIXTURE_SOLID, opacity: '0.6' });
  });

  it('渐变层产出两色线性渐变', () => {
    const style = buildBackgroundLayerStyle(settings({ kind: 'gradient' }), false, resolveAsset);
    // gate-allow: color exact serialized gradient fixture (asserts the built-in default)
    expect(style?.background).toBe('linear-gradient(135deg, #2f2f2f, #0f172a)');
    expect(style?.opacity).toBe('1');
  });

  it('图片层使用 asset 协议 URL 并铺满', () => {
    const style = buildBackgroundLayerStyle(
      settings({ kind: 'image', imagePath: 'C:/data/bg.png', opacity: 75 }),
      false,
      resolveAsset,
    );
    expect(style).toEqual({
      'background-image': `url("${resolveAsset('C:/data/bg.png')}")`,
      'background-size': 'cover',
      'background-position': 'center',
      'background-repeat': 'no-repeat',
      opacity: '0.75',
    });
  });

  it('图片模式缺路径时不渲染层', () => {
    expect(
      buildBackgroundLayerStyle(settings({ kind: 'image', imagePath: null }), false, resolveAsset),
    ).toBeNull();
  });
});

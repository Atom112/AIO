import { beforeEach, describe, expect, it, vi } from 'vitest';

// ui-prefs reads localStorage at module load (theme/dark defaults), so a memory stub
// must exist before the dynamic import below.
const memory = new Map<string, string>();
vi.stubGlobal('localStorage', {
  getItem: (key: string) => memory.get(key) ?? null,
  setItem: (key: string, value: string) => void memory.set(key, value),
  removeItem: (key: string) => void memory.delete(key),
});

const {
  colorTargetsForKind,
  resolveColorTarget,
  primaryColorTargetForKind,
  loadColorEditTarget,
  persistColorEditTarget,
  normalizeThemeColor,
  DEFAULT_THEME_COLOR,
  normalizeAppFont,
  appFontStack,
  persistAppFont,
  normalizeAppFontSize,
  appFontSize,
  persistAppFontSize,
  appFontScale,
} = await import('./ui-prefs');

// The shared color picker must never point at a color the active background kind
// does not own — that is what keeps the ring in sync when the settings content
// switches (kind tab or target button).

beforeEach(() => {
  memory.clear();
});

describe('app font size', () => {
  it('normalizes unknown values to standard', () => {
    expect(normalizeAppFontSize('large')).toBe('large');
    expect(normalizeAppFontSize('xlarge')).toBe('xlarge');
    expect(normalizeAppFontSize('bogus')).toBe('standard');
    expect(normalizeAppFontSize(null)).toBe('standard');
  });

  it('persists the stop and exposes the matching scale', () => {
    persistAppFontSize('large');
    expect(appFontSize()).toBe('large');
    expect(memory.get('aio-app-font-size')).toBe('large');
    expect(appFontScale('small')).toBeLessThan(1);
    expect(appFontScale('standard')).toBe(1);
    expect(appFontScale('large')).toBeGreaterThan(1);
    expect(appFontScale('xlarge')).toBeGreaterThan(appFontScale('large'));
    persistAppFontSize('standard');
  });
});

describe('colorTargetsForKind', () => {
  it('每个背景模式只提供自己拥有的颜色目标', () => {
    expect(colorTargetsForKind('solid')).toEqual(['solid', 'theme']);
    expect(colorTargetsForKind('gradient')).toEqual(['gradientStart', 'gradientEnd', 'theme']);
    expect(colorTargetsForKind('default')).toEqual(['theme']);
    expect(colorTargetsForKind('image')).toEqual(['theme']);
  });
});

describe('primaryColorTargetForKind', () => {
  it('切到某个背景模式时默认编辑它自己的颜色', () => {
    expect(primaryColorTargetForKind('solid')).toBe('solid');
    expect(primaryColorTargetForKind('gradient')).toBe('gradientStart');
    expect(primaryColorTargetForKind('default')).toBe('theme');
    expect(primaryColorTargetForKind('image')).toBe('theme');
  });
});

describe('resolveColorTarget', () => {
  it('仍然可用的目标保持不变', () => {
    expect(resolveColorTarget('gradient', 'gradientEnd')).toBe('gradientEnd');
    expect(resolveColorTarget('solid', 'theme')).toBe('theme');
  });

  it('不可用的目标回退到该模式的主目标', () => {
    expect(resolveColorTarget('solid', 'gradientEnd')).toBe('solid');
    expect(resolveColorTarget('gradient', 'solid')).toBe('gradientStart');
    expect(resolveColorTarget('default', 'solid')).toBe('theme');
  });
});

describe('color target persistence', () => {
  it('默认返回主题色，保存后可读回', () => {
    expect(loadColorEditTarget()).toBe('theme');
    persistColorEditTarget('gradientEnd');
    expect(loadColorEditTarget()).toBe('gradientEnd');
  });

  it('损坏值回退主题色', () => {
    localStorage.setItem('aio-color-edit-target', 'bogus');
    expect(loadColorEditTarget()).toBe('theme');
  });
});

describe('normalizeThemeColor', () => {
  it('合法 hex 保留并转小写', () => {
    // gate-allow: color normalization fixture
    expect(normalizeThemeColor('#AABBCC')).toBe('#aabbcc');
  });

  it('旧版本写入的 CSS 变量与非法值回退默认柔雾蓝', () => {
    expect(normalizeThemeColor('var(--color-pri)')).toBe(DEFAULT_THEME_COLOR);
    expect(normalizeThemeColor('#12345')).toBe(DEFAULT_THEME_COLOR);
    expect(normalizeThemeColor(null)).toBe(DEFAULT_THEME_COLOR);
  });
});

describe('app font preference', () => {
  it('去空白与控制字符，空值表示系统默认', () => {
    expect(normalizeAppFont('  Microsoft YaHei  ')).toBe('Microsoft YaHei');
    expect(normalizeAppFont('Arial\n')).toBe('Arial');
    expect(normalizeAppFont('   ')).toBe('');
    expect(normalizeAppFont(null)).toBe('');
  });

  it('font-family 栈给字体名加引号并保留系统回退', () => {
    expect(appFontStack('Times New Roman')).toBe(
      '"Times New Roman", ui-sans-serif, system-ui, sans-serif',
    );
    expect(appFontStack('')).toBe('');
  });

  it('选择后持久化，恢复系统默认时清除存储', () => {
    persistAppFont('Microsoft YaHei');
    expect(localStorage.getItem('aio-app-font')).toBe('Microsoft YaHei');
    persistAppFont('');
    expect(localStorage.getItem('aio-app-font')).toBeNull();
  });
});

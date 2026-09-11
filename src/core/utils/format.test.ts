import { describe, expect, it } from 'vitest';
import { formatSize } from './format';

describe('formatSize', () => {
  it('非正数返回空串', () => {
    expect(formatSize(0)).toBe('');
    expect(formatSize(-5)).toBe('');
  });
  it('KB：整数，最小 1 KB', () => {
    expect(formatSize(512)).toBe('1 KB'); // 512B 向上取整到 1KB
    expect(formatSize(1536)).toBe('2 KB'); // 1.5KB → 2
    expect(formatSize(1024 * 999)).toBe('999 KB');
  });
  it('MB：1 位小数', () => {
    expect(formatSize(1024 * 1024)).toBe('1.0 MB');
    expect(formatSize(12.3 * 1024 * 1024)).toBe('12.3 MB');
  });
  it('GB：2 位小数', () => {
    expect(formatSize(2 * 1024 * 1024 * 1024)).toBe('2.00 GB');
  });
});

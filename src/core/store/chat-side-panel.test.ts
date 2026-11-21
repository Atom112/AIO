import { beforeEach, describe, expect, it } from 'vitest';

// File-diff tabs of the chat right-hand panel: opening dedupes by file, closing the
// active tab falls back to a neighbouring file and finally to the topic list — the
// rules that decide which diff (or the topic list) the panel shows.
const {
  diffPanelTabs,
  activeSidePanelTab,
  openDiffPanelTab,
  closeDiffPanelTab,
  resetDiffPanelTabs,
} = await import('./chat-side-panel');

const change = (filePath: string, diff = '@@ -1 +1 @@') => ({
  filePath,
  action: 'modify' as const,
  diff,
  summary: '+1 -0',
});

beforeEach(() => {
  resetDiffPanelTabs();
});

describe('diff panel tabs', () => {
  it('打开文件创建标签并激活最新一个', () => {
    openDiffPanelTab(change('a.ts'));
    openDiffPanelTab(change('b.ts'));
    expect(diffPanelTabs().map((tab) => tab.filePath)).toEqual(['a.ts', 'b.ts']);
    expect(activeSidePanelTab()).toBe(diffPanelTabs()[1].id);
  });

  it('同一文件再次打开复用标签并刷新内容', () => {
    openDiffPanelTab(change('a.ts', 'old'));
    const id = diffPanelTabs()[0].id;
    openDiffPanelTab(change('a.ts', 'new'), true);
    expect(diffPanelTabs()).toHaveLength(1);
    expect(diffPanelTabs()[0].id).toBe(id);
    expect(diffPanelTabs()[0].diff).toBe('new');
    expect(diffPanelTabs()[0].allReverted).toBe(true);
    expect(activeSidePanelTab()).toBe(id);
  });

  it('关闭激活标签后先切到右侧邻居，再回退到左侧', () => {
    openDiffPanelTab(change('a.ts'));
    openDiffPanelTab(change('b.ts'));
    openDiffPanelTab(change('c.ts'));
    const [a, b, c] = diffPanelTabs();
    openDiffPanelTab(change('a.ts'));
    closeDiffPanelTab(a.id);
    expect(activeSidePanelTab()).toBe(b.id);
    closeDiffPanelTab(b.id);
    expect(activeSidePanelTab()).toBe(c.id);
    closeDiffPanelTab(c.id);
    expect(activeSidePanelTab()).toBe('topics');
    expect(diffPanelTabs()).toEqual([]);
  });

  it('关闭非激活标签不改变当前选择', () => {
    openDiffPanelTab(change('a.ts'));
    openDiffPanelTab(change('b.ts'));
    const [a, b] = diffPanelTabs();
    closeDiffPanelTab(a.id);
    expect(activeSidePanelTab()).toBe(b.id);
    expect(diffPanelTabs().map((tab) => tab.filePath)).toEqual(['b.ts']);
  });
});

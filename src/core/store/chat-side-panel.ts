// Chat right-side panel tabs: the topic list plus file-change diffs opened from the chat.
//
// File-change bubbles keep living in the conversation; clicking one opens its diff in
// the right-hand panel as a tab ("文件1", "文件2", ... next to the "话题" tab) instead
// of expanding the diff inline in the message stream. Tabs are session-only: diffs can
// be large and belong to the conversation, so they are not persisted and are cleared
// when the user switches assistant/topic.

import { createSignal } from 'solid-js';
import type { FileChangeInfo } from './types';

/** One open diff tab in the right-hand panel. */
export interface DiffPanelTab {
  id: string;
  filePath: string;
  action: FileChangeInfo['action'];
  diff: string;
  summary: string;
  /** The whole round was reverted: the diff stays viewable but dimmed. */
  allReverted: boolean;
}

const [diffPanelTabs, setDiffPanelTabs] = createSignal<DiffPanelTab[]>([]);
/** Active right-panel tab: `'topics'` or the id of an open diff tab. */
export const [activeSidePanelTab, setActiveSidePanelTab] = createSignal<string>('topics');

/** Open files, in the order their tabs were created (tab labels are positional). */
export { diffPanelTabs };

let nextTabSeq = 0;

/**
 * Open (or refresh) the diff tab of a file and activate it.
 * The same file opened from a later round reuses its tab and shows the newest diff.
 * @param change file-change entry of the chat bubble
 * @param allReverted whether the whole round was reverted
 */
export const openDiffPanelTab = (change: FileChangeInfo, allReverted = false): void => {
  const prev = diffPanelTabs();
  const existing = prev.find((tab) => tab.filePath === change.filePath);
  if (existing) {
    setDiffPanelTabs(
      prev.map((tab) =>
        tab.id === existing.id
          ? {
              ...tab,
              action: change.action,
              diff: change.diff,
              summary: change.summary,
              allReverted,
            }
          : tab,
      ),
    );
    setActiveSidePanelTab(existing.id);
    return;
  }
  nextTabSeq += 1;
  const id = `diff-${nextTabSeq}`;
  setDiffPanelTabs([
    ...prev,
    {
      id,
      filePath: change.filePath,
      action: change.action,
      diff: change.diff,
      summary: change.summary,
      allReverted,
    },
  ]);
  setActiveSidePanelTab(id);
};

/**
 * Close a diff tab. Closing the active tab activates its right-hand neighbour,
 * then its left-hand one, and finally falls back to the topic list.
 * @param id tab id to close
 */
export const closeDiffPanelTab = (id: string): void => {
  const prev = diffPanelTabs();
  const index = prev.findIndex((tab) => tab.id === id);
  if (index < 0) return;
  setDiffPanelTabs(prev.filter((tab) => tab.id !== id));
  if (activeSidePanelTab() === id) {
    const fallback = prev[index + 1] ?? prev[index - 1];
    setActiveSidePanelTab(fallback && fallback.id !== id ? fallback.id : 'topics');
  }
};

/** Drop every diff tab and return the panel to the topic list (topic/assistant switch). */
export const resetDiffPanelTabs = (): void => {
  setDiffPanelTabs([]);
  setActiveSidePanelTab('topics');
};

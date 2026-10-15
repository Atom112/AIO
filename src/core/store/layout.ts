// Chat layout persistence (P3 前端打磨：localStorage 裸调收编).
//
// 此前组件层（ChatPage / chatCommands / TopicSidebar）直接读写 localStorage——
// 持久化键与格式散落三处，任何键改名都要逐一追。现在本模块是布局持久化的
// 唯一入口（core/store 域拥有持久化；组件只消费域 API），与 ui-prefs 的
// 主题/推理档位/联网搜索同一层。

type PanelSide = 'left' | 'right';

const WIDTH_KEY: Record<PanelSide, string> = {
  left: 'chat-left-panel-width',
  right: 'chat-right-panel-width',
};
const COLLAPSED_KEY: Record<PanelSide, string> = {
  left: 'left-collapsed',
  right: 'right-collapsed',
};
const FOLD_KEY = 'topic-fold-state';

/** localStorage 可能被策略禁用/抛配额错——读降级 null、写吞错（与 ui-prefs 同策略）。 */
const safeGet = (key: string): string | null => {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
};
const safeSet = (key: string, value: string): void => {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* restricted storage: session-only behavior */
  }
};

/** Panel width percentage; invalid/absent falls back to the default 18%. */
export const loadPanelWidth = (side: PanelSide): number => {
  const v = Number(safeGet(WIDTH_KEY[side]));
  return Number.isFinite(v) && v > 0 ? v : 18;
};

export const savePanelWidth = (side: PanelSide, width: number): void => {
  safeSet(WIDTH_KEY[side], String(width));
};

/** Panel collapsed flag ('true' string format, matching the historic values). */
export const loadPanelCollapsed = (side: PanelSide): boolean =>
  safeGet(COLLAPSED_KEY[side]) === 'true';

export const savePanelCollapsed = (side: PanelSide, collapsed: boolean): void => {
  safeSet(COLLAPSED_KEY[side], String(collapsed));
};

/**
 * Topic fold state per assistant: { [assistantId]: string[] }.
 * Storage unavailable or corrupted → session-only folds (empty set).
 */
export const loadTopicFoldState = (assistantId: string): Set<string> => {
  try {
    const raw = safeGet(FOLD_KEY);
    if (!raw) return new Set();
    const parsed = JSON.parse(raw) as Record<string, string[]>;
    const ids = parsed[assistantId];
    return new Set(Array.isArray(ids) ? ids : []);
  } catch {
    return new Set();
  }
};

export const saveTopicFoldState = (assistantId: string, ids: Set<string>): void => {
  try {
    const raw = safeGet(FOLD_KEY);
    const parsed = raw ? (JSON.parse(raw) as Record<string, string[]>) : {};
    parsed[assistantId] = Array.from(ids);
    safeSet(FOLD_KEY, JSON.stringify(parsed));
  } catch {
    /* restricted storage: keep working session-only */
  }
};

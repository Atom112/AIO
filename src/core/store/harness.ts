/**
 * Harness workbench domain store: tool detection, workspaces, topics, the live
 * message list, in-flight assistant drafts (built from the harness event stream),
 * run states, pending approvals and the agent plan. Domain-module pattern
 * mirrors core/store/*: module-level signals, re-exported via store.ts barrel.
 */

import { createSignal } from 'solid-js';
import type { Message, AgentStep } from './types';
import type {
  HarnessApprovalRequest,
  HarnessConfigOption,
  HarnessPlanEntry,
  HarnessToolStatus,
  HarnessTopic,
  HarnessUsage,
  HarnessWorkspace,
} from '../api/harnessPayloads';

export type {
  HarnessApprovalRequest,
  HarnessConfigOption,
  HarnessPlanEntry,
  HarnessToolStatus,
  HarnessTopic,
  HarnessWorkspace,
};

/** A harness tool as detected (name/adapter/state/custom come from the backend). */
export type HarnessToolInfo = HarnessToolStatus;

/** Pending approval with session attribution merged from the event envelope. */
export interface PendingHarnessApproval extends HarnessApprovalRequest {
  sessionRowId: string;
  toolId: string;
}

/** Run state of one harness topic (keyed by session row id). */
export interface HarnessRunInfo {
  state: 'idle' | 'connecting' | 'running' | 'error';
  error?: string;
}

/** Terminal error of the last turn (keyed by session row id; cleared on next turn). */
export type HarnessTurnErrors = Record<string, string | undefined>;

/** In-flight assistant draft assembled by the harness event listeners. */
export interface HarnessDraft {
  content: string;
  reasoning: string;
  agentSteps: AgentStep[];
  usage: HarnessUsage;
  startedAt: number;
}

// ===== Signals =====

const [harnessTools, setHarnessTools] = createSignal<HarnessToolInfo[]>([]);
const [harnessWorkspaces, setHarnessWorkspaces] = createSignal<HarnessWorkspace[]>([]);
const [harnessTopics, setHarnessTopics] = createSignal<HarnessTopic[]>([]);
const [harnessMessages, setHarnessMessages] = createSignal<Message[]>([]);
const [harnessRunStates, setHarnessRunStates] = createSignal<Record<string, HarnessRunInfo>>({});
const [harnessTurnErrors, setHarnessTurnErrors] = createSignal<HarnessTurnErrors>({});
const [harnessDrafts, setHarnessDrafts] = createSignal<Record<string, HarnessDraft>>({});
const [harnessApprovals, setHarnessApprovals] = createSignal<PendingHarnessApproval[]>([]);
const [harnessPlan, setHarnessPlan] = createSignal<Record<string, HarnessPlanEntry[]>>({});
const [harnessConfig, setHarnessConfig] = createSignal<Record<string, HarnessConfigOption[]>>({});

// ===== Native-adapter preferences (model / thinking level per tool, persisted) =====

interface NativePrefs {
  model?: string;
  thinking?: string;
}

const NATIVE_PREFS_KEY = 'aio-harness-native-prefs-v1';

function loadNativePrefs(): Record<string, NativePrefs> {
  try {
    const raw = localStorage.getItem(NATIVE_PREFS_KEY);
    if (raw) return JSON.parse(raw);
  } catch {
    /* corrupted entry — fall through */
  }
  return {};
}

const [harnessNativePrefs, setHarnessNativePrefs] =
  createSignal<Record<string, NativePrefs>>(loadNativePrefs());

/** Set + persist one preference field for a tool. */
const saveHarnessNativePref = (toolId: string, key: 'model' | 'thinking', value: string) => {
  setHarnessNativePrefs((prev) => {
    const next = { ...prev, [toolId]: { ...prev[toolId], [key]: value } };
    try {
      localStorage.setItem(NATIVE_PREFS_KEY, JSON.stringify(next));
    } catch {
      /* restricted storage — session-only */
    }
    return next;
  });
};

const [currentHarnessToolId, setCurrentHarnessToolId] = createSignal<string | null>(null);
const [currentHarnessWorkspaceId, setCurrentHarnessWorkspaceId] = createSignal<string | null>(null);
const [currentHarnessTopicId, setCurrentHarnessTopicId] = createSignal<string | null>(null);

// ===== Selection persistence (localStorage, ui-prefs pattern) =====

const SELECTION_KEY = 'aio-harness-selection-v1';

function persistSelection() {
  try {
    localStorage.setItem(
      SELECTION_KEY,
      JSON.stringify({
        toolId: currentHarnessToolId(),
        workspaceId: currentHarnessWorkspaceId(),
        topicId: currentHarnessTopicId(),
      }),
    );
  } catch {
    /* storage unavailable — selection just stops persisting */
  }
}

function loadSelection(): {
  toolId: string | null;
  workspaceId: string | null;
  topicId: string | null;
} {
  try {
    const raw = localStorage.getItem(SELECTION_KEY);
    if (raw) return JSON.parse(raw);
  } catch {
    /* corrupted entry — fall through to defaults */
  }
  return { toolId: null, workspaceId: null, topicId: null };
}

// ===== Claude Code permission mode (global default; applied at spawn and
// switched live via harness_set_permission_mode when a session runs) =====

const MODE_KEY = 'aio-harness-claude-mode';
const loadClaudeMode = (): string => {
  try {
    return localStorage.getItem(MODE_KEY) || 'default';
  } catch {
    return 'default';
  }
};
const [harnessClaudeMode, setHarnessClaudeMode] = createSignal<string>(loadClaudeMode());

/** Set + persist the Claude permission-mode choice. */
const saveHarnessClaudeMode = (mode: string) => {
  setHarnessClaudeMode(mode);
  try {
    localStorage.setItem(MODE_KEY, mode);
  } catch {
    /* restricted storage — session-only choice */
  }
};

// ===== Selectors =====

/** The selected workspace row. */
const currentHarnessWorkspace = () =>
  harnessWorkspaces().find((w) => w.id === currentHarnessWorkspaceId());

/** Run info of the current topic. */
const currentHarnessRun = (): HarnessRunInfo =>
  harnessRunStates()[currentHarnessTopicId() ?? ''] ?? { state: 'idle' };

/** Draft of the current topic (undefined when no turn is in flight). */
const currentHarnessDraft = () => harnessDrafts()[currentHarnessTopicId() ?? ''];

/** Terminal error of the current topic's last turn. */
const currentHarnessTurnError = () => harnessTurnErrors()[currentHarnessTopicId() ?? ''];

/** Approvals belonging to the current topic. */
const currentHarnessApprovals = () => {
  const topicId = currentHarnessTopicId();
  if (!topicId) return [];
  return harnessApprovals().filter((a) => a.sessionRowId === topicId);
};

/** Config options reported by the agent for the current topic. */
const currentHarnessConfigOptions = () => harnessConfig()[currentHarnessTopicId() ?? ''] ?? [];

/** Persisted model preference for a tool. */
const harnessNativeModel = (toolId: string) => harnessNativePrefs()[toolId]?.model;

/** Persisted thinking-level preference for a tool. */
const harnessNativeThinking = (toolId: string) => harnessNativePrefs()[toolId]?.thinking;

// ===== Mutators (used by the event listeners + page actions) =====

const selectHarnessTool = (id: string | null) => {
  setCurrentHarnessToolId(id);
  setCurrentHarnessWorkspaceId(null);
  setCurrentHarnessTopicId(null);
  persistSelection();
};

const selectHarnessWorkspace = (id: string | null) => {
  setCurrentHarnessWorkspaceId(id);
  setCurrentHarnessTopicId(null);
  persistSelection();
};

const selectHarnessTopic = (id: string | null) => {
  setCurrentHarnessTopicId(id);
  persistSelection();
};

/** Restore the persisted selection on mount (after data loads). */
const restoreHarnessSelection = () => loadSelection();

export {
  harnessTools,
  setHarnessTools,
  harnessWorkspaces,
  setHarnessWorkspaces,
  harnessTopics,
  setHarnessTopics,
  harnessMessages,
  setHarnessMessages,
  harnessRunStates,
  setHarnessRunStates,
  harnessTurnErrors,
  setHarnessTurnErrors,
  harnessDrafts,
  setHarnessDrafts,
  harnessApprovals,
  setHarnessApprovals,
  harnessPlan,
  setHarnessPlan,
  harnessConfig,
  setHarnessConfig,
  currentHarnessToolId,
  selectHarnessTool,
  currentHarnessWorkspaceId,
  selectHarnessWorkspace,
  currentHarnessTopicId,
  selectHarnessTopic,
  currentHarnessWorkspace,
  currentHarnessRun,
  currentHarnessDraft,
  currentHarnessTurnError,
  currentHarnessApprovals,
  currentHarnessConfigOptions,
  harnessNativePrefs,
  saveHarnessNativePref,
  harnessNativeModel,
  harnessNativeThinking,
  restoreHarnessSelection,
  harnessClaudeMode,
  saveHarnessClaudeMode,
};

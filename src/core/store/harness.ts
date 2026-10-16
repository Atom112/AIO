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
  HarnessPlanEntry,
  HarnessToolStatus,
  HarnessTopic,
  HarnessUsage,
  HarnessWorkspace,
} from '../api/harnessPayloads';

export type {
  HarnessApprovalRequest,
  HarnessPlanEntry,
  HarnessToolStatus,
  HarnessTopic,
  HarnessWorkspace,
};

/** A harness tool merged from detection + the display-name table. */
export interface HarnessToolInfo extends HarnessToolStatus {
  name: string;
}

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

/** Workspaces under the currently selected tool. */
const currentHarnessWorkspaces = () => {
  const toolId = currentHarnessToolId();
  if (!toolId) return [];
  return harnessWorkspaces().filter((w) => w.toolId === toolId);
};

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
  currentHarnessToolId,
  selectHarnessTool,
  currentHarnessWorkspaceId,
  selectHarnessWorkspace,
  currentHarnessTopicId,
  selectHarnessTopic,
  currentHarnessWorkspaces,
  currentHarnessWorkspace,
  currentHarnessRun,
  currentHarnessDraft,
  currentHarnessTurnError,
  currentHarnessApprovals,
  restoreHarnessSelection,
  harnessClaudeMode,
  saveHarnessClaudeMode,
};

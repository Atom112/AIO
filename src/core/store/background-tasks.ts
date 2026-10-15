// Global background-task registry: long-running app-side async jobs (memory
// fact extraction / consolidation, index rebuilds, code indexing, model
// downloads) surfaced through the chat status-bar pull-up menu.
//
// Fed by lifecycle/progress events (backgroundTaskListeners); one registry
// entry per (kind, scope) — e.g. `memory-extract:<project root>` or
// `model-download:<model id>`. Terminal entries auto-expire after a few
// seconds so the menu stays a live view of "what is running right now";
// running entries whose events stopped (missed terminal, dead job) are swept
// after a long TTL instead of lingering forever.
import { createSignal } from 'solid-js';

export type BackgroundTaskKind =
  | 'memory-extract'
  | 'memory-consolidate'
  | 'reindex'
  | 'code-index'
  | 'model-download'
  | 'gguf-download';

export type BackgroundTaskStatus = 'running' | 'done' | 'error' | 'paused' | 'cancelled';

export interface BackgroundTask {
  /** Registry key: `${kind}:${scope}` */
  id: string;
  kind: BackgroundTaskKind;
  status: BackgroundTaskStatus;
  /** Raw scope identity (project root path or model id); the menu resolves display names. */
  scope: string;
  /** Numeric progress when the job reports one (index counters / download bytes). */
  progress?: { done: number; total: number };
  /** Free-form detail line (downloaded file name). */
  detail?: string;
  error?: string;
  startedAt: number;
  endedAt?: number;
  /** Timestamp of the last received event; stale running entries are swept. */
  lastEventAt: number;
}

/** Terminal entries vanish after this delay (the "已下载" moment). */
const TERMINAL_TTL_MS = 10_000;
/** A running entry with no events for this long is assumed dead (missed terminal). */
const RUNNING_STALE_MS = 30 * 60_000;

const [backgroundTasks, setBackgroundTasks] = createSignal<BackgroundTask[]>([]);
export { backgroundTasks };

let sweepTimer: ReturnType<typeof setInterval> | undefined;

function ensureSweeper() {
  if (sweepTimer !== undefined) return;
  sweepTimer = setInterval(() => {
    const now = Date.now();
    setBackgroundTasks((prev) => {
      const next = prev.filter(
        (t) =>
          (t.status === 'running' && now - t.lastEventAt < RUNNING_STALE_MS) ||
          now - (t.endedAt ?? t.lastEventAt) < TERMINAL_TTL_MS,
      );
      return next.length === prev.length ? prev : next;
    });
    // Idle sweeper self-stops; the next mutation restarts it.
    if (backgroundTasks().length === 0 && sweepTimer !== undefined) {
      clearInterval(sweepTimer);
      sweepTimer = undefined;
    }
  }, 1_000);
}

type TaskPatch = Partial<Omit<BackgroundTask, 'id' | 'kind' | 'scope'>>;

function upsert(id: string, kind: BackgroundTaskKind, scope: string, patch: TaskPatch): void {
  const now = Date.now();
  ensureSweeper();
  setBackgroundTasks((prev) => {
    const idx = prev.findIndex((t) => t.id === id);
    if (idx < 0) {
      return [
        ...prev,
        { id, kind, scope, status: 'running', startedAt: now, lastEventAt: now, ...patch },
      ];
    }
    const next = prev.slice();
    next[idx] = { ...next[idx], ...patch, lastEventAt: now };
    return next;
  });
}

/** A job started (or re-started) — creates/refreshes its running entry. */
export function startBackgroundTask(
  kind: BackgroundTaskKind,
  scope: string,
  detail?: string,
): void {
  upsert(`${kind}:${scope}`, kind, scope, {
    status: 'running',
    detail,
    error: undefined,
    endedAt: undefined,
  });
}

/** Progress heartbeat — refreshes the running entry (creating one if the job has no start event). */
export function updateBackgroundTaskProgress(
  kind: BackgroundTaskKind,
  scope: string,
  progress: { done: number; total: number },
  detail?: string,
): void {
  upsert(`${kind}:${scope}`, kind, scope, {
    status: 'running',
    progress,
    detail,
    error: undefined,
    endedAt: undefined,
  });
}

/** Terminal state — closes the entry (ignored for unknown ids: no start was ever seen). */
export function finishBackgroundTask(
  kind: BackgroundTaskKind,
  scope: string,
  status: Exclude<BackgroundTaskStatus, 'running'>,
  error?: string,
): void {
  const id = `${kind}:${scope}`;
  setBackgroundTasks((prev) => {
    const idx = prev.findIndex((t) => t.id === id);
    if (idx < 0) return prev;
    const now = Date.now();
    const next = prev.slice();
    next[idx] = { ...next[idx], status, error, endedAt: now, lastEventAt: now };
    return next;
  });
}

/** Test helper: drop every entry and stop the sweeper. */
export function resetBackgroundTasks(): void {
  setBackgroundTasks([]);
  if (sweepTimer !== undefined) {
    clearInterval(sweepTimer);
    sweepTimer = undefined;
  }
}

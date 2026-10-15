// Background-task event listeners: feed the global background-task registry
// (core/store/background-tasks) from the lifecycle/progress events the backend
// emits for app-side async jobs. Registered once from the root Layout next to
// the chat stream listeners — downloads and memory jobs run regardless of which
// page is open, so page-scoped listeners would miss them.
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { EVENTS } from '../../../core/api/events';
import {
  finishBackgroundTask,
  startBackgroundTask,
  updateBackgroundTaskProgress,
} from '../../../core/store/background-tasks';

/** `memory-extract-progress` / `memory-consolidate-progress` payload (status carries the lifecycle). */
interface MemoryJobPayload {
  status: string;
  project: string;
  error?: string;
}

/** `reindex-progress` / `code-index-progress` payload: progress or terminal (status). */
interface IndexJobPayload {
  project?: string;
  done?: number;
  total?: number;
  status?: string;
  error?: string;
}

/** `model-download-progress` / `local-model-download-progress` payload: progress or terminal (done). */
interface DownloadJobPayload {
  id?: string;
  file?: string;
  downloaded?: number;
  total?: number;
  done?: boolean;
  status?: string;
  error?: string;
}

/**
 * Background-task listeners: registered once from the root Layout, alive for the whole app.
 * Returns the unlisten promises (Layout never unmounts; kept for symmetry).
 */
export function registerBackgroundTaskListeners(): Promise<UnlistenFn>[] {
  const memoryJob = (kind: 'memory-extract' | 'memory-consolidate') =>
    listen<MemoryJobPayload>(
      kind === 'memory-extract' ? EVENTS.memoryExtractProgress : EVENTS.memoryConsolidateProgress,
      (e) => {
        const { status, project, error } = e.payload;
        if (!project) return;
        if (status === 'running') {
          startBackgroundTask(kind, project);
        } else if (status === 'error') {
          finishBackgroundTask(kind, project, 'error', error);
        } else {
          finishBackgroundTask(kind, project, 'done');
        }
      },
    );

  const indexJob = (
    event: typeof EVENTS.reindexProgress | typeof EVENTS.codeIndexProgress,
    kind: 'reindex' | 'code-index',
  ) =>
    listen<IndexJobPayload>(event, (e) => {
      const { project, done, total, status, error } = e.payload;
      if (!project) return; // legacy payloads without identity cannot be keyed
      if (status === 'done' || status === 'error') {
        finishBackgroundTask(kind, project, status === 'error' ? 'error' : 'done', error);
      } else if (done !== undefined && total !== undefined) {
        updateBackgroundTaskProgress(kind, project, { done, total });
      }
    });

  const downloadJob = (
    event: typeof EVENTS.modelDownloadProgress | typeof EVENTS.localModelDownloadProgress,
    kind: 'model-download' | 'gguf-download',
  ) =>
    listen<DownloadJobPayload>(event, (e) => {
      const { id, file, downloaded, total, done, status, error } = e.payload;
      if (!id) return;
      if (done) {
        // Terminal: status extends the legacy `done` contract (ok implied when absent)
        const s =
          status === 'error' || status === 'paused' || status === 'cancelled' ? status : 'done';
        finishBackgroundTask(kind, id, s, error);
      } else if (file !== undefined && total !== undefined && downloaded !== undefined) {
        updateBackgroundTaskProgress(kind, id, { done: downloaded, total }, file);
      }
    });

  return [
    memoryJob('memory-extract'),
    memoryJob('memory-consolidate'),
    indexJob(EVENTS.reindexProgress, 'reindex'),
    indexJob(EVENTS.codeIndexProgress, 'code-index'),
    downloadJob(EVENTS.modelDownloadProgress, 'model-download'),
    downloadJob(EVENTS.localModelDownloadProgress, 'gguf-download'),
  ];
}

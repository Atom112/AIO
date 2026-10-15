import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// The background-task registry is a leaf module (solid signals only); its lifecycle
// rules — one entry per kind:scope, terminal entries expire, stale running entries
// are swept, unknown terminals ignored — underpin the status-bar pull-up menu.
const {
  backgroundTasks,
  startBackgroundTask,
  updateBackgroundTaskProgress,
  finishBackgroundTask,
  finishBackgroundTaskByKey,
  heartbeatBackgroundTask,
  resetBackgroundTasks,
} = await import('./background-tasks');

beforeEach(() => {
  vi.useFakeTimers();
  resetBackgroundTasks();
});

afterEach(() => {
  vi.useRealTimers();
  resetBackgroundTasks();
});

describe('background task registry', () => {
  it('start 创建 running 条目，同 kind:scope 再次 start 重置为 running', () => {
    startBackgroundTask('memory-extract', 'D:/proj');
    expect(backgroundTasks()).toHaveLength(1);
    expect(backgroundTasks()[0]).toMatchObject({
      id: 'memory-extract:D:/proj',
      status: 'running',
    });

    finishBackgroundTask('memory-extract', 'D:/proj', 'done');
    expect(backgroundTasks()[0].status).toBe('done');

    // A re-started job (new turn extraction) flips the same entry back to running
    startBackgroundTask('memory-extract', 'D:/proj');
    expect(backgroundTasks()[0].status).toBe('running');
    expect(backgroundTasks()[0].endedAt).toBeUndefined();
  });

  it('progress 更新计数并创建缺失条目（下载无 start 事件）', () => {
    updateBackgroundTaskProgress(
      'model-download',
      'bge-m3',
      { done: 1024, total: 4096 },
      'model.onnx',
    );
    expect(backgroundTasks()).toHaveLength(1);
    expect(backgroundTasks()[0]).toMatchObject({
      id: 'model-download:bge-m3',
      status: 'running',
      detail: 'model.onnx',
      progress: { done: 1024, total: 4096 },
    });

    updateBackgroundTaskProgress('model-download', 'bge-m3', { done: 2048, total: 4096 });
    expect(backgroundTasks()[0].progress?.done).toBe(2048);
  });

  it('未知 id 的终态事件被忽略（无 start 时不产生条目）', () => {
    finishBackgroundTask('reindex', 'D:/other', 'done');
    expect(backgroundTasks()).toHaveLength(0);
  });

  it('终态条目 10 秒后自动清除', () => {
    startBackgroundTask('reindex', 'D:/proj');
    finishBackgroundTask('reindex', 'D:/proj', 'done');
    expect(backgroundTasks()).toHaveLength(1);

    vi.advanceTimersByTime(9_000);
    expect(backgroundTasks()).toHaveLength(1);

    vi.advanceTimersByTime(1_500);
    expect(backgroundTasks()).toHaveLength(0);
  });

  it('运行中条目 30 分钟无事件视为死亡并清除（错过终态兜底）', () => {
    startBackgroundTask('memory-consolidate', 'D:/proj');
    vi.advanceTimersByTime(29 * 60_000);
    expect(backgroundTasks()).toHaveLength(1);

    vi.advanceTimersByTime(2 * 60_000);
    expect(backgroundTasks()).toHaveLength(0);
  });

  it('shell 命令按运行 id 键控：heartbeat 创建/刷新 elapsed，终态按 key 关闭', () => {
    heartbeatBackgroundTask(
      'shell-command',
      'shell-command:uuid-1',
      'cargo build --release',
      5_000,
    );
    expect(backgroundTasks()).toHaveLength(1);
    expect(backgroundTasks()[0]).toMatchObject({
      id: 'shell-command:uuid-1',
      kind: 'shell-command',
      status: 'running',
      scope: 'cargo build --release',
      elapsedMs: 5_000,
    });

    heartbeatBackgroundTask(
      'shell-command',
      'shell-command:uuid-1',
      'cargo build --release',
      10_000,
    );
    expect(backgroundTasks()[0].elapsedMs).toBe(10_000);

    // Same command text, different run: a separate entry (concurrent runs never merge)
    heartbeatBackgroundTask(
      'shell-command',
      'shell-command:uuid-2',
      'cargo build --release',
      5_000,
    );
    expect(backgroundTasks()).toHaveLength(2);

    finishBackgroundTaskByKey('shell-command:uuid-1', 'done');
    expect(backgroundTasks()[0].status).toBe('done');
    expect(backgroundTasks()[1].status).toBe('running');
  });
});

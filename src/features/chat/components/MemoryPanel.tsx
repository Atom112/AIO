import { Component, createEffect, createSignal, For, Show } from 'solid-js';
import { t } from '../../../core/i18n';
import {
  memoryArchive,
  memoryClear,
  memoryDelete,
  memoryGetVersions,
  memoryList,
  memoryPrune,
  memoryReindex,
  memorySearch,
  memorySetPinned,
  memoryStats,
  memoryUpdate,
} from '../../../core/utils/memory';
import type { FactVersion, MemoryFact, MemoryStats } from '../../../core/types/memory';

interface MemoryPanelProps {
  show: boolean;
  projectId: string;
  onClose: () => void;
}

type Tab = 'active' | 'archived' | 'superseded' | 'all';

/**
 * 项目记忆面板：统计 / 搜索 / 列表 / 编辑 / 钉住 / 归档 / 删除 / 版本审计 / 索引维护。
 */
const MemoryPanel: Component<MemoryPanelProps> = (props) => {
  const [stats, setStats] = createSignal<MemoryStats | null>(null);
  const [facts, setFacts] = createSignal<MemoryFact[]>([]);
  const [tab, setTab] = createSignal<Tab>('active');
  const [query, setQuery] = createSignal('');
  const [expanded, setExpanded] = createSignal<Record<string, FactVersion[]>>({});
  const [notice, setNotice] = createSignal('');

  const loadStats = async () => {
    try {
      setStats(await memoryStats(props.projectId));
    } catch {
      // 记忆库未创建时忽略
    }
  };

  const loadList = async () => {
    try {
      const page = await memoryList(props.projectId, {
        status: tab() === 'all' ? undefined : tab(),
        limit: 100,
      });
      setFacts(page.facts);
    } catch {
      setFacts([]);
    }
  };

  createEffect(() => {
    if (props.show) {
      void loadStats();
      void loadList();
    }
  });

  const doSearch = async () => {
    const q = query().trim();
    if (!q) {
      await loadList();
      return;
    }
    try {
      const res = await memorySearch(props.projectId, q, undefined, 20);
      setFacts(
        res.map((r) => ({
          id: r.id,
          content: r.content,
          category: r.category,
          importance: 0,
          status: 'active' as const,
          createdAt: '',
          updatedAt: r.updatedAt ?? '',
          accessCount: 0,
          pinned: false,
        })),
      );
    } catch {
      setFacts([]);
    }
  };

  const editFact = async (f: MemoryFact) => {
    const next = window.prompt(t('project.memoryPanel.edit'), f.content);
    if (!next || next === f.content) return;
    try {
      await memoryUpdate(props.projectId, f.id, { content: next });
      await loadList();
      await loadStats();
    } catch {
      // 忽略编辑失败
    }
  };

  const togglePin = async (f: MemoryFact) => {
    try {
      await memorySetPinned(props.projectId, f.id, !f.pinned);
      await loadList();
    } catch {
      // 忽略
    }
  };

  const archiveFact = async (f: MemoryFact) => {
    try {
      await memoryArchive(props.projectId, f.id);
      await loadList();
      await loadStats();
    } catch {
      // 忽略
    }
  };

  const deleteFact = async (f: MemoryFact) => {
    if (!window.confirm(t('project.memoryPanel.confirmDelete'))) return;
    try {
      await memoryDelete(props.projectId, f.id);
      await loadList();
      await loadStats();
    } catch {
      // 忽略
    }
  };

  const toggleVersions = async (id: string) => {
    const cur = expanded();
    if (cur[id]) {
      const next = { ...cur };
      delete next[id];
      setExpanded(next);
      return;
    }
    try {
      const vs = await memoryGetVersions(props.projectId, id);
      setExpanded((prev) => ({ ...prev, [id]: vs }));
    } catch {
      // 忽略
    }
  };

  const reasonLabel = (r: string) => {
    if (r === 'update') return t('project.memoryPanel.reason.update');
    if (r === 'merge') return t('project.memoryPanel.reason.merge');
    if (r === 'supersede') return t('project.memoryPanel.reason.supersede');
    return t('project.memoryPanel.reason.create');
  };

  const formatSize = (n: number) => (n / (1024 * 1024)).toFixed(1);

  const reindex = async () => {
    try {
      const [done, failed] = await memoryReindex(props.projectId);
      setNotice(
        t('project.memoryPanel.reindexDone', { done: String(done), failed: String(failed) }),
      );
      await loadStats();
    } catch {
      // 忽略
    }
  };

  const prune = async () => {
    try {
      const n = await memoryPrune(props.projectId);
      setNotice(t('project.memoryPanel.pruneDone', { n: String(n) }));
      await loadList();
      await loadStats();
    } catch {
      // 忽略
    }
  };

  const clearAll = async () => {
    if (!window.confirm(t('project.memoryPanel.confirmClear'))) return;
    try {
      await memoryClear(props.projectId);
      await loadList();
      await loadStats();
    } catch {
      // 忽略
    }
  };

  const tabs: { key: Tab; label: string }[] = [
    { key: 'active', label: t('project.memoryPanel.tabActive') },
    { key: 'archived', label: t('project.memoryPanel.tabArchived') },
    { key: 'superseded', label: t('project.memoryPanel.tabSuperseded') },
    { key: 'all', label: t('project.memoryPanel.tabAll') },
  ];

  return (
    <Show when={props.show}>
      <div
        class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6"
        onClick={() => props.onClose()}
      >
        <div
          class="w-full max-w-2xl max-h-[80vh] flex flex-col rounded-xl border border-dark-100 bg-[#14161f] shadow-2xl overflow-hidden"
          onClick={(e) => e.stopPropagation()}
        >
          {/* 头部 */}
          <div class="flex items-center justify-between px-4 py-3 border-b border-white/5">
            <span class="text-sm font-semibold text-[#eee]">{t('project.memoryPanel.title')}</span>
            <button
              class="px-2 py-1 rounded text-xs text-white/50 hover:bg-white/5 cursor-pointer"
              onClick={() => props.onClose()}
            >
              ✕
            </button>
          </div>

          {/* 统计 */}
          <Show when={stats()}>
            <div
              class="px-4 py-2 text-[11px] border-b border-white/5"
              style={{ color: 'rgba(var(--text-base-rgb),0.45)' }}
            >
              {t('project.memoryPanel.stats', {
                total: String(stats()!.totalFacts),
                active: String(stats()!.activeFacts),
                archived: String(stats()!.archivedFacts),
                superseded: String(stats()!.supersededFacts),
                embedded: String(stats()!.embeddedFacts),
                size: formatSize(stats()!.dbSizeBytes),
              })}
            </div>
          </Show>

          {/* 搜索 + 标签 */}
          <div class="flex items-center gap-2 px-4 py-2 border-b border-white/5">
            <input
              class="flex-1 p-2 bg-dark-300 border border-dark-100 rounded-lg text-sm text-[#e0e0e0] focus:outline-none focus:border-white/20"
              value={query()}
              onInput={(e) => setQuery(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') void doSearch();
              }}
              placeholder={t('project.memoryPanel.searchPlaceholder')}
            />
            <button
              class="px-3 py-2 rounded-lg text-xs font-semibold bg-pri-20 text-pri border border-pri/30 cursor-pointer hover:bg-pri-30 transition-colors"
              onClick={() => void doSearch()}
            >
              {t('project.memoryPanel.search')}
            </button>
          </div>

          <div class="flex items-center gap-1 px-4 py-1.5 border-b border-white/5">
            <For each={tabs}>
              {(tb) => (
                <button
                  class={
                    tab() === tb.key
                      ? 'px-2.5 py-1 rounded-md text-[11px] font-semibold bg-pri-20 text-pri cursor-pointer'
                      : 'px-2.5 py-1 rounded-md text-[11px] text-white/50 hover:bg-white/5 cursor-pointer'
                  }
                  onClick={() => {
                    setTab(tb.key);
                    setQuery('');
                  }}
                >
                  {tb.label}
                </button>
              )}
            </For>
          </div>

          {/* 列表 */}
          <div class="flex-1 overflow-y-auto px-4 py-2 scrollbar-thin">
            <For each={facts()}>
              {(f) => (
                <div class="py-2 border-b border-white/5 last:border-0">
                  <div class="flex items-start justify-between gap-3">
                    <div class="flex-1 min-w-0">
                      <div class="text-[13px] text-white/85 leading-relaxed whitespace-pre-wrap break-words">
                        {f.content}
                      </div>
                      <div
                        class="flex items-center gap-2 mt-1 text-[10px]"
                        style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}
                      >
                        <span class="px-1 py-0.5 rounded bg-pri-20 text-pri">{f.category}</span>
                        <Show when={f.pinned}>
                          <span class="text-yellow-400/80">{t('project.memoryPanel.pinned')}</span>
                        </Show>
                        <Show when={f.updatedAt}>
                          <span>{f.updatedAt}</span>
                        </Show>
                      </div>
                    </div>
                    <div class="flex items-center gap-1 shrink-0">
                      <button
                        class="px-2 py-1 rounded-md text-[11px] text-white/60 border border-dark-100 bg-dark-300 hover:bg-dark-200 cursor-pointer"
                        onClick={() => void editFact(f)}
                      >
                        {t('project.memoryPanel.edit')}
                      </button>
                      <button
                        class="px-2 py-1 rounded-md text-[11px] text-white/60 border border-dark-100 bg-dark-300 hover:bg-dark-200 cursor-pointer"
                        onClick={() => void togglePin(f)}
                      >
                        {f.pinned ? t('project.memoryPanel.unpin') : t('project.memoryPanel.pin')}
                      </button>
                      <Show when={f.status === 'active'}>
                        <button
                          class="px-2 py-1 rounded-md text-[11px] text-white/60 border border-dark-100 bg-dark-300 hover:bg-dark-200 cursor-pointer"
                          onClick={() => void archiveFact(f)}
                        >
                          {t('project.memoryPanel.archive')}
                        </button>
                      </Show>
                      <button
                        class="px-2 py-1 rounded-md text-[11px] text-white/60 border border-dark-100 bg-dark-300 hover:bg-dark-200 cursor-pointer"
                        onClick={() => void deleteFact(f)}
                      >
                        {t('project.memoryPanel.delete')}
                      </button>
                      <button
                        class="px-2 py-1 rounded-md text-[11px] text-white/60 border border-dark-100 bg-dark-300 hover:bg-dark-200 cursor-pointer"
                        onClick={() => void toggleVersions(f.id)}
                      >
                        {t('project.memoryPanel.versions')}
                      </button>
                    </div>
                  </div>
                  <Show when={expanded()[f.id]}>
                    <div class="mt-2 ml-1 flex flex-col gap-1 rounded-lg bg-dark-300/60 p-2 text-[11px]">
                      <For each={expanded()[f.id]}>
                        {(v) => (
                          <div style={{ color: 'rgba(var(--text-base-rgb),0.55)' }}>
                            <span class="text-pri/80">
                              v{v.version} {reasonLabel(v.reason)}
                            </span>
                            <span class="ml-2">{v.createdAt}</span>
                            <Show when={v.contentBefore && v.contentAfter}>
                              <div class="mt-0.5 line-clamp-3">
                                {v.contentBefore} → {v.contentAfter}
                              </div>
                            </Show>
                          </div>
                        )}
                      </For>
                    </div>
                  </Show>
                </div>
              )}
            </For>
            <Show when={facts().length === 0}>
              <div
                class="py-8 text-center text-xs"
                style={{ color: 'rgba(var(--text-base-rgb),0.3)' }}
              >
                {query().trim()
                  ? t('project.memoryPanel.noResult')
                  : t('project.memoryPanel.empty')}
              </div>
            </Show>
          </div>

          {/* 底部 */}
          <div class="flex items-center gap-2 px-4 py-2.5 border-t border-white/5">
            <Show when={notice()}>
              <span class="flex-1 text-[11px] text-pri/80">{notice()}</span>
            </Show>
            <button
              class="px-2 py-1 rounded-md text-[11px] text-white/60 border border-dark-100 bg-dark-300 hover:bg-dark-200 cursor-pointer"
              onClick={() => void reindex()}
            >
              {t('project.memoryPanel.reindex')}
            </button>
            <button
              class="px-2 py-1 rounded-md text-[11px] text-white/60 border border-dark-100 bg-dark-300 hover:bg-dark-200 cursor-pointer"
              onClick={() => void prune()}
            >
              {t('project.memoryPanel.prune')}
            </button>
            <button
              class="px-2.5 py-1.5 rounded-md text-[11px] text-red-400/90 border border-red-400/20 hover:bg-red-400/10 cursor-pointer"
              onClick={() => void clearAll()}
            >
              {t('project.memoryPanel.clear')}
            </button>
          </div>
        </div>
      </div>
    </Show>
  );
};

export default MemoryPanel;

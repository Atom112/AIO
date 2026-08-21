import { Component, createEffect, createSignal, For, Show } from 'solid-js';
import { listen } from '@tauri-apps/api/event';
import { t } from '../../../core/i18n';
import {
  embeddingSaveApiKey,
  embeddingTest,
  memoryArchive,
  memoryClear,
  memoryCodeIndex,
  memoryCodeSearch,
  memoryCodeStatus,
  memoryExport,
  memoryGetStatus,
  memoryImport,
  memoryDelete,
  memoryGetVersions,
  memoryList,
  memoryPrune,
  memoryReindex,
  memorySearch,
  memorySetPinned,
  memoryStats,
  memoryUpdate,
  memoryUpdateEmbedding,
} from '../../../core/utils/memory';
import type {
  CodeChunkHit,
  CodeIndexStatus,
  EmbeddingTestResult,
  FactVersion,
  MemoryEmbeddingConfig,
  MemoryFact,
  MemoryStats,
} from '../../../core/types/memory';
import {
  btnBadge,
  btnGhost,
  btnPrimarySm,
  btnSecondarySm,
  btnDanger,
  btnTabMd,
  btnTabXs,
  tabActive,
  tabInactive,
} from '../../../shared/components/buttonStyles';

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
  const [codeStatus, setCodeStatus] = createSignal<CodeIndexStatus | null>(null);
  const [codeQuery, setCodeQuery] = createSignal('');
  const [codeHits, setCodeHits] = createSignal<CodeChunkHit[]>([]);
  const [indexing, setIndexing] = createSignal(false);
  // 项目级嵌入模型（由 memory_get_status 提供，切换保存在项目记忆库 meta）
  const [embedding, setEmbedding] = createSignal<MemoryEmbeddingConfig | null>(null);
  const [editingEmbedding, setEditingEmbedding] = createSignal(false);
  const [ebProvider, setEbProvider] = createSignal<'local' | 'online'>('local');
  const [ebModel, setEbModel] = createSignal('');
  const [ebApiUrl, setEbApiUrl] = createSignal('');
  const [ebApiKey, setEbApiKey] = createSignal('');
  const [ebTesting, setEbTesting] = createSignal(false);
  const [ebTestResult, setEbTestResult] = createSignal<EmbeddingTestResult | null>(null);
  const [ebSaving, setEbSaving] = createSignal(false);
  const [reindexProgress, setReindexProgress] = createSignal<{
    done: number;
    total: number;
  } | null>(null);

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
      void loadCodeStatus();
      void loadEmbedding();
    }
  });

  const loadEmbedding = async () => {
    try {
      const st = await memoryGetStatus(props.projectId);
      setEmbedding(st.embedding);
    } catch {
      // 记忆库未创建时忽略
    }
  };

  const loadCodeStatus = async () => {
    try {
      setCodeStatus(await memoryCodeStatus(props.projectId));
    } catch {
      setCodeStatus(null);
    }
  };

  const doCodeIndex = async () => {
    setIndexing(true);
    try {
      const res = await memoryCodeIndex(props.projectId, false);
      setNotice(`${t('project.memoryPanel.codeStatus')}: ${res.chunks} / ${res.files}`);
      await loadCodeStatus();
    } catch {
      setNotice(t('project.memoryPanel.codeIndexFail'));
    } finally {
      setIndexing(false);
    }
  };

  const doCodeSearch = async () => {
    const q = codeQuery().trim();
    if (!q) return;
    try {
      setCodeHits(await memoryCodeSearch(props.projectId, q, 10));
    } catch {
      setCodeHits([]);
    }
  };

  const doExport = async () => {
    try {
      const json = await memoryExport(props.projectId);
      await navigator.clipboard.writeText(json);
      setNotice(t('project.memoryPanel.exportDone'));
    } catch {
      setNotice(t('project.memoryPanel.exportFail'));
    }
  };

  const doImport = async () => {
    const text = window.prompt(t('project.memoryPanel.importPrompt'));
    if (!text) return;
    try {
      const n = await memoryImport(props.projectId, text);
      setNotice(t('project.memoryPanel.importDone', { n: String(n) }));
      await loadList();
      await loadStats();
    } catch {
      setNotice(t('project.memoryPanel.importFail'));
    }
  };

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

  // 当前嵌入模型的展示文案（provider · model · dims）
  const embeddingLabel = () => {
    const e = embedding();
    if (!e) return t('app.memory.providerLocal');
    const kind =
      e.provider === 'online' ? t('app.memory.providerOnline') : t('app.memory.providerLocal');
    const model = e.model || (e.provider === 'online' ? '-' : t('app.memory.localModel'));
    return t('project.memoryPanel.embeddingCurrent', {
      kind,
      model,
      dims: String(e.dimensions),
    });
  };

  const openEmbeddingEditor = () => {
    const cur: MemoryEmbeddingConfig = embedding() ?? {
      provider: 'local',
      model: 'all-MiniLM-L6-v2',
      apiUrl: '',
      dimensions: 384,
      enabled: true,
    };
    setEbProvider(cur.provider === 'online' ? 'online' : 'local');
    setEbModel(cur.model);
    setEbApiUrl(cur.apiUrl);
    setEbApiKey('');
    setEbTestResult(null);
    setEditingEmbedding(true);
  };

  const testEmbedding = async () => {
    setEbTesting(true);
    try {
      setEbTestResult(
        await embeddingTest({
          provider: ebProvider(),
          model: ebModel(),
          apiUrl: ebApiUrl(),
          dimensions: 384,
          enabled: true,
          apiKey: ebApiKey() || undefined,
        }),
      );
    } catch {
      setEbTestResult(null);
    } finally {
      setEbTesting(false);
    }
  };

  const saveEmbedding = async () => {
    if (ebProvider() === 'online' && (!ebApiUrl().trim() || !ebModel().trim())) {
      setNotice(t('project.memoryPanel.embeddingNeedOnline'));
      return;
    }
    // 切换会清空本项目全部向量并重新嵌入（事实文本保留），再次确认
    if (!window.confirm(t('project.memoryPanel.embeddingSwitchConfirm'))) return;
    setEbSaving(true);
    let unlisten: (() => void) | undefined;
    try {
      if (ebProvider() === 'online') {
        await embeddingSaveApiKey(ebApiKey());
      }
      unlisten = await listen('reindex-progress', (ev) => {
        const p = ev.payload as { done?: number; total?: number };
        setReindexProgress({ done: p.done ?? 0, total: p.total ?? 0 });
      });
      const st = await memoryUpdateEmbedding(props.projectId, {
        provider: ebProvider(),
        model: ebModel(),
        apiUrl: ebApiUrl(),
        dimensions: 384,
        enabled: true,
      });
      setEmbedding(st.embedding);
      setNotice(t('project.memoryPanel.embeddingSaved'));
      setEditingEmbedding(false);
      await loadStats();
      await loadList();
    } catch {
      setNotice(t('project.memoryPanel.embeddingSaveFail'));
    } finally {
      unlisten?.();
      setReindexProgress(null);
      setEbSaving(false);
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
            <button class={btnBadge} onClick={() => props.onClose()}>
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

          {/* 嵌入模型（项目级；切换会清空本项目全部向量并重新嵌入） */}
          <div class="flex items-center justify-between gap-2 px-4 py-2 border-b border-white/5">
            <div class="min-w-0 text-[11px]" style={{ color: 'rgba(var(--text-base-rgb),0.6)' }}>
              <span class="mr-1">{t('project.memoryPanel.embeddingTitle')}:</span>
              <span>{embeddingLabel()}</span>
              <Show when={reindexProgress()}>
                <span class="ml-2 text-pri">
                  {t('project.memoryPanel.reindexing', {
                    done: String(reindexProgress()!.done),
                    total: String(reindexProgress()!.total),
                  })}
                </span>
              </Show>
            </div>
            <Show
              when={!editingEmbedding()}
              fallback={
                <button class={btnGhost} onClick={() => setEditingEmbedding(false)}>
                  {t('common.cancel')}
                </button>
              }
            >
              <button class={btnBadge} onClick={openEmbeddingEditor}>
                {t('project.memoryPanel.embeddingSwitch')}
              </button>
            </Show>
          </div>

          <Show when={editingEmbedding()}>
            <div class="flex flex-col gap-2 px-4 py-2 border-b border-white/5">
              <div class="flex gap-2">
                <button
                  class={`${btnTabMd} ${ebProvider() === 'local' ? tabActive : tabInactive}`}
                  onClick={() => setEbProvider('local')}
                >
                  {t('app.memory.providerLocal')}
                </button>
                <button
                  class={`${btnTabMd} ${ebProvider() === 'online' ? tabActive : tabInactive}`}
                  onClick={() => setEbProvider('online')}
                >
                  {t('app.memory.providerOnline')}
                </button>
              </div>
              <Show when={ebProvider() === 'local'}>
                <div class="text-[11px]" style={{ color: 'rgba(var(--text-base-rgb),0.5)' }}>
                  {t('app.memory.localModelDesc')}
                </div>
              </Show>
              <Show when={ebProvider() === 'online'}>
                <input
                  class="w-full p-2 bg-dark-300 border border-dark-100 rounded-lg text-sm text-[#e0e0e0] placeholder:text-white/25 focus:outline-none focus:border-pri-30"
                  placeholder={t('app.memory.model')}
                  value={ebModel()}
                  onInput={(e) => setEbModel(e.currentTarget.value)}
                />
                <input
                  class="w-full p-2 bg-dark-300 border border-dark-100 rounded-lg text-sm text-[#e0e0e0] placeholder:text-white/25 focus:outline-none focus:border-pri-30"
                  placeholder={t('app.memory.apiUrl')}
                  value={ebApiUrl()}
                  onInput={(e) => setEbApiUrl(e.currentTarget.value)}
                />
                <input
                  class="w-full p-2 bg-dark-300 border border-dark-100 rounded-lg text-sm text-[#e0e0e0] placeholder:text-white/25 focus:outline-none focus:border-pri-30"
                  type="password"
                  placeholder={t('app.memory.apiKey')}
                  value={ebApiKey()}
                  onInput={(e) => setEbApiKey(e.currentTarget.value)}
                />
              </Show>
              <Show when={ebTestResult()}>
                <div
                  class="text-[11px]"
                  style={{ color: ebTestResult()!.ok ? '#7cd9a0' : '#ff8a8a' }}
                >
                  {ebTestResult()!.ok
                    ? t('app.memory.testOk', {
                        dims: String(ebTestResult()!.dimensions),
                        ms: String(ebTestResult()!.latencyMs),
                      })
                    : t('app.memory.testFail', { error: ebTestResult()!.error ?? '' })}
                </div>
              </Show>
              <div class="flex items-center justify-end gap-2">
                <button
                  class={btnSecondarySm}
                  disabled={ebTesting()}
                  onClick={() => void testEmbedding()}
                >
                  {ebTesting() ? t('app.memory.testing') : t('app.memory.test')}
                </button>
                <button
                  class={btnPrimarySm}
                  disabled={ebSaving()}
                  onClick={() => void saveEmbedding()}
                >
                  {ebSaving() ? t('project.memoryPanel.embeddingSaving') : t('app.memory.save')}
                </button>
              </div>
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
            <button class={btnPrimarySm} onClick={() => void doSearch()}>
              {t('project.memoryPanel.search')}
            </button>
          </div>

          <div class="flex items-center gap-1 px-4 py-1.5 border-b border-white/5">
            <For each={tabs}>
              {(tb) => (
                <button
                  class={`${btnTabXs} ${tab() === tb.key ? tabActive : tabInactive}`}
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

          {/* 代码索引与检索 */}
          <div class="flex items-center gap-2 px-4 py-2 border-b border-white/5">
            <button class={btnPrimarySm} disabled={indexing()} onClick={() => void doCodeIndex()}>
              {indexing() ? '...' : t('project.memoryPanel.codeIndex')}
            </button>
            <input
              class="flex-1 p-2 bg-dark-300 border border-dark-100 rounded-lg text-xs text-[#e0e0e0] focus:outline-none focus:border-white/20"
              value={codeQuery()}
              onInput={(e) => setCodeQuery(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') void doCodeSearch();
              }}
              placeholder={t('project.memoryPanel.codeSearch')}
            />
            <button class={btnSecondarySm} onClick={() => void doCodeSearch()}>
              {t('project.memoryPanel.codeSearch')}
            </button>
          </div>
          <Show when={codeStatus()}>
            <div
              class="px-4 py-1 text-[10px] border-b border-white/5"
              style={{ color: 'rgba(var(--text-base-rgb),0.35)' }}
            >
              {t('project.memoryPanel.codeStatus', {
                chunks: String(codeStatus()!.chunks),
                files: String(codeStatus()!.files),
              })}
            </div>
          </Show>
          <Show when={codeHits().length > 0}>
            <div class="px-4 py-1.5 border-b border-white/5 flex flex-col gap-1">
              <For each={codeHits()}>
                {(h) => (
                  <div class="text-[11px]" style={{ color: 'rgba(var(--text-base-rgb),0.6)' }}>
                    <span class="text-pri/80">
                      {h.filePath}:{h.startLine}-{h.endLine}
                    </span>
                    <div class="text-white/60 line-clamp-2 whitespace-pre-wrap">{h.content}</div>
                  </div>
                )}
              </For>
            </div>
          </Show>

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
                      <button class={btnBadge} onClick={() => void editFact(f)}>
                        {t('project.memoryPanel.edit')}
                      </button>
                      <button class={btnBadge} onClick={() => void togglePin(f)}>
                        {f.pinned ? t('project.memoryPanel.unpin') : t('project.memoryPanel.pin')}
                      </button>
                      <Show when={f.status === 'active'}>
                        <button class={btnBadge} onClick={() => void archiveFact(f)}>
                          {t('project.memoryPanel.archive')}
                        </button>
                      </Show>
                      <button class={btnBadge} onClick={() => void deleteFact(f)}>
                        {t('project.memoryPanel.delete')}
                      </button>
                      <button class={btnBadge} onClick={() => void toggleVersions(f.id)}>
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
            <button class={btnBadge} onClick={() => void reindex()}>
              {t('project.memoryPanel.reindex')}
            </button>
            <button class={btnBadge} onClick={() => void prune()}>
              {t('project.memoryPanel.prune')}
            </button>
            <button class={btnBadge} onClick={() => void doExport()}>
              {t('project.memoryPanel.export')}
            </button>
            <button class={btnBadge} onClick={() => void doImport()}>
              {t('project.memoryPanel.import')}
            </button>
            <button class={btnDanger} onClick={() => void clearAll()}>
              {t('project.memoryPanel.clear')}
            </button>
          </div>
        </div>
      </div>
    </Show>
  );
};

export default MemoryPanel;

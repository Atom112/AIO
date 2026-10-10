import { Component, createEffect, createSignal, For, Show } from 'solid-js';
import { listen } from '@tauri-apps/api/event';
import { pickOpenPath, pickSavePath } from '../../../core/api/dialog';
import { t } from '../../../core/i18n';
import Icon from '../../../shared/components/Icon';
import Dropdown from '../../../shared/components/Dropdown';
import MemoryRerankSection from './MemoryRerankSection';
import { formatSize } from '../../../core/utils/format';
import { EVENTS } from '../../../core/api/events';
import {
  embeddingModelList,
  embeddingTest,
  memoryArchive,
  memoryClear,
  memoryCodeIndex,
  memoryCodeSearch,
  memoryCodeStatus,
  memoryConsolidate,
  memoryExportFile,
  memoryGetStatus,
  memoryGraphExtract,
  memoryImportFile,
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
  DEFAULT_EMBED_MODEL_ID,
} from '../../../core/utils/memory';
import type {
  CodeChunkHit,
  CodeIndexStatus,
  ConsolidationReport,
  EmbedderStatus,
  EmbeddingModelInfo,
  EmbeddingTestResult,
  FactVersion,
  GraphExtractReport,
  MemoryEmbeddingConfig,
  MemoryFact,
  MemoryStats,
} from '../../../core/types/memory';
import {
  btnClose,
  btnSecondarySm,
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
 * Project memory panel: stats / search / list / edit / pin / archive / delete / version audit / index maintenance.
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
  // Project-level embedding model (provided by memory_get_status; the choice is persisted in the project memory DB meta)
  const [embedding, setEmbedding] = createSignal<MemoryEmbeddingConfig | null>(null);
  // Actual embedder state for the project (resolved provider/model/dims; may differ from the config)
  const [embedderStatus, setEmbedderStatus] = createSignal<EmbedderStatus | null>(null);
  const [editingEmbedding, setEditingEmbedding] = createSignal(false);
  const [ebModel, setEbModel] = createSignal('');
  const [ebTesting, setEbTesting] = createSignal(false);
  const [ebTestResult, setEbTestResult] = createSignal<EmbeddingTestResult | null>(null);
  const [ebSaving, setEbSaving] = createSignal(false);
  // Matryoshka target dims (0 = model default full dims; only text-embedding-3/bge-m3 and the like support truncation)
  const [ebDims, setEbDims] = createSignal(0);
  const [reindexProgress, setReindexProgress] = createSignal<{
    done: number;
    total: number;
  } | null>(null);
  const [rrModels, setRrModels] = createSignal<EmbeddingModelInfo[]>([]);
  // Locally available embedding models (kind=embed, downloaded, globally enabled; from Embedding model management)
  const embedModels = () => {
    const list = rrModels().filter((m) => m.kind === 'embed' && m.downloaded && m.enabled);
    return list.length > 0 ? list : rrModels().filter((m) => m.kind === 'embed' && m.downloaded);
  };
  const loadStats = async () => {
    try {
      setStats(await memoryStats(props.projectId));
    } catch {
      // Ignore when the memory DB has not been created yet
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
      void loadRerankModels();
    }
  });

  const loadEmbedding = async () => {
    try {
      const st = await memoryGetStatus(props.projectId);
      setEmbedding(st.embedding);
      setEmbedderStatus(st.embedder);
      // Rerank config is loaded by the MemoryRerankSection child component itself
    } catch {
      // Ignore when the memory DB has not been created yet
    }
  };

  const loadRerankModels = async () => {
    try {
      setRrModels(await embeddingModelList());
    } catch {
      setRrModels([]);
    }
  };

  // ---- Sleep-time consolidation (manual trigger) + last-7-days quality probe ----
  const [consolidating, setConsolidating] = createSignal(false);

  const consolidate = async () => {
    setConsolidating(true);
    try {
      const r: ConsolidationReport = await memoryConsolidate(props.projectId);
      setNotice(
        t('project.memoryPanel.consolidateDone', {
          clusters: String(r.clusters),
          arbitrated: String(r.arbitrated),
          merged: String(r.merged),
          updated: String(r.updated),
          superseded: String(r.superseded),
        }),
      );
      void loadList();
      void loadStats();
    } catch {
      // Ignore
    } finally {
      setConsolidating(false);
    }
  };

  // ---- Lightweight knowledge-graph extraction ----
  const [graphExtracting, setGraphExtracting] = createSignal(false);

  const extractGraph = async () => {
    setGraphExtracting(true);
    try {
      const r: GraphExtractReport = await memoryGraphExtract(props.projectId);
      setNotice(
        t('project.memoryPanel.graphExtractDone', {
          processed: String(r.factsProcessed),
          relations: String(r.relations),
          failed: String(r.failed),
        }),
      );
    } catch {
      // Ignore
    } finally {
      setGraphExtracting(false);
    }
  };

  const judgeDistLabel = () => {
    const d = stats()?.judgeDistribution7d;
    if (!d) return '';
    const parts: string[] = [];
    if ((d.merge ?? 0) > 0) parts.push(`${t('project.memoryPanel.reason.merge')} ${d.merge}`);
    if ((d.update ?? 0) > 0) parts.push(`${t('project.memoryPanel.reason.update')} ${d.update}`);
    if ((d.supersede ?? 0) > 0)
      parts.push(`${t('project.memoryPanel.reason.supersede')} ${d.supersede}`);
    if ((d.keep_separate ?? 0) > 0)
      parts.push(`${t('project.memoryPanel.qKeep')} ${d.keep_separate}`);
    return parts.join(' · ');
  };

  const sourceRefLabel = (s: string) => {
    try {
      const parsed = JSON.parse(s) as { messageIds?: number[] };
      if (Array.isArray(parsed.messageIds) && parsed.messageIds.length > 0) {
        return t('project.memoryPanel.sourceMsgs', { ids: parsed.messageIds.join(',') });
      }
    } catch {
      // Ignore non-JSON (legacy format)
    }
    return '';
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

  /** Short hash (FNV-1a 32-bit → 8 hex chars): identifies the project in export file names.
   *  Repeated exports of the same project keep a stable file name; different projects never collide. */
  const projectHash8 = (id: string): string => {
    let h = 0x811c9dc5;
    for (let i = 0; i < id.length; i++) {
      h ^= id.charCodeAt(i);
      h = Math.imul(h, 0x01000193);
    }
    return (h >>> 0).toString(16).padStart(8, '0');
  };

  const doExport = async () => {
    // Export path via the backend save dialog (the chosen path becomes session-approved;
    // the write command refuses any path the backend dialog never returned)
    let path: string | null = null;
    try {
      path = await pickSavePath(`aio-memory-${projectHash8(props.projectId)}.json`, {
        name: 'JSON',
        extensions: ['json'],
      });
    } catch {
      /* user cancelled */
    }
    if (!path) return;
    try {
      await memoryExportFile(props.projectId, path);
      setNotice(t('project.memoryPanel.exportDone', { path }));
    } catch (e) {
      setNotice(t('project.memoryPanel.exportFail', { error: String(e) }));
    }
  };

  const doImport = async () => {
    // Import file via the backend open dialog (session-approved read path + size cap)
    let path: string | null = null;
    try {
      path = await pickOpenPath({ name: 'JSON', extensions: ['json'] });
    } catch {
      /* user cancelled */
    }
    if (!path) return;
    try {
      const n = await memoryImportFile(props.projectId, path);
      setNotice(t('project.memoryPanel.importDone', { n: String(n) }));
      await loadList();
      await loadStats();
    } catch (e) {
      setNotice(t('project.memoryPanel.importFail', { error: String(e) }));
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
      // Ignore edit failure
    }
  };

  const togglePin = async (f: MemoryFact) => {
    try {
      await memorySetPinned(props.projectId, f.id, !f.pinned);
      await loadList();
    } catch {
      // Ignore
    }
  };

  const archiveFact = async (f: MemoryFact) => {
    try {
      await memoryArchive(props.projectId, f.id);
      await loadList();
      await loadStats();
    } catch {
      // Ignore
    }
  };

  const deleteFact = async (f: MemoryFact) => {
    if (!window.confirm(t('project.memoryPanel.confirmDelete'))) return;
    try {
      await memoryDelete(props.projectId, f.id);
      await loadList();
      await loadStats();
    } catch {
      // Ignore
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
      // Ignore
    }
  };

  const reasonLabel = (r: string) => {
    if (r === 'update') return t('project.memoryPanel.reason.update');
    if (r === 'merge') return t('project.memoryPanel.reason.merge');
    if (r === 'supersede') return t('project.memoryPanel.reason.supersede');
    return t('project.memoryPanel.reason.create');
  };

  const reindex = async () => {
    try {
      const [done, failed] = await memoryReindex(props.projectId);
      setNotice(
        t('project.memoryPanel.reindexDone', { done: String(done), failed: String(failed) }),
      );
      await loadStats();
    } catch {
      // Ignore
    }
  };

  // Display label for the current embedding model (provider · model · dims), from the resolved embedder.
  // Since v0.8 only local embedding models are supported; the built-in one is fixed to all-MiniLM-L6-v2 · 384 dims.
  const embeddingLabel = () => {
    const st = embedderStatus();
    const kind = t('app.memory.providerLocal');
    if (st && st.available) {
      return t('project.memoryPanel.embeddingCurrent', {
        kind,
        model: st.model || '-',
        dims: String(st.dimensions),
      });
    }
    const cfg = embedding();
    const model = cfg?.model || DEFAULT_EMBED_MODEL_ID;
    return t('project.memoryPanel.embeddingCurrent', { kind, model, dims: '-' });
  };

  const openEmbeddingEditor = () => {
    const cur: MemoryEmbeddingConfig = embedding() ?? {
      provider: 'local',
      model: DEFAULT_EMBED_MODEL_ID,
      apiUrl: '',
      dimensions: 384,
      enabled: true,
    };
    const sel = embedModels().find((m) => m.id === cur.model)
      ? cur.model
      : (embedModels()[0]?.id ?? '');
    setEbModel(sel);
    // Prefilling dims only makes sense when the selected model matches the current one; after switching models the old dims are meaningless. Empty = model default
    setEbDims(sel === cur.model && cur.dimensions && cur.dimensions !== 384 ? cur.dimensions : 0);
    setEbTestResult(null);
    setEditingEmbedding(true);
  };

  const testEmbedding = async () => {
    setEbTesting(true);
    try {
      setEbTestResult(
        await embeddingTest({
          provider: 'local',
          model: ebModel(),
          apiUrl: '',
          dimensions: ebDims(),
          enabled: true,
        }),
      );
    } catch {
      setEbTestResult(null);
    } finally {
      setEbTesting(false);
    }
  };

  /** Effective dims for the editor's current selection (empty input = the selected model's default full dims) */
  const ebEffectiveDims = () => {
    if (ebDims() > 0) return ebDims();
    return embedModels().find((m) => m.id === ebModel().trim())?.dims ?? 0;
  };

  /** Whether this is a real switch that triggers clearing and rebuilding the vectors (model or effective dims changed) */
  const ebConfigChanged = () => {
    const cur = embedding();
    // No current config (status failed to load): treat as changed and let the confirm dialog guard it
    if (!cur) return true;
    return cur.model !== ebModel().trim() || cur.dimensions !== ebEffectiveDims();
  };

  const saveEmbedding = async () => {
    if (!ebModel().trim()) {
      setNotice(t('project.memoryPanel.embeddingNeedModel'));
      return;
    }
    // Confirm only on a "real switch" when the DB already has embedded content: first-time setup on
    // an empty DB and re-saving an unchanged config skip the destructive warning (backend early-exits on no change).
    if (ebConfigChanged() && (stats()?.embeddedFacts ?? 0) > 0) {
      if (!window.confirm(t('project.memoryPanel.embeddingSwitchConfirm'))) return;
    }
    setEbSaving(true);
    let unlisten: (() => void) | undefined;
    try {
      unlisten = await listen(EVENTS.reindexProgress, (ev) => {
        const p = ev.payload as { done?: number; total?: number };
        setReindexProgress({ done: p.done ?? 0, total: p.total ?? 0 });
      });
      const st = await memoryUpdateEmbedding(props.projectId, {
        provider: 'local',
        model: ebModel(),
        apiUrl: '',
        // Send effective dims (not the raw input 0) so the backend's no-change early-exit works:
        // an empty dims input must not be misjudged as a switch that needlessly clears all vectors.
        dimensions: ebEffectiveDims(),
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
      // Ignore
    }
  };

  const clearAll = async () => {
    if (!window.confirm(t('project.memoryPanel.confirmClear'))) return;
    try {
      await memoryClear(props.projectId);
      await loadList();
      await loadStats();
    } catch {
      // Ignore
    }
  };

  const tabs: { key: Tab; label: string }[] = [
    { key: 'active', label: t('project.memoryPanel.tabActive') },
    { key: 'archived', label: t('project.memoryPanel.tabArchived') },
    { key: 'superseded', label: t('project.memoryPanel.tabSuperseded') },
    { key: 'all', label: t('project.memoryPanel.tabAll') },
  ];

  /** Section divider (the panel is a single-column flat layout; thin lines separate sections) */
  const SectionDivider = () => (
    <div class="h-px shrink-0" style={{ background: 'var(--border-dim)' }} />
  );

  return (
    <Show when={props.show}>
      <div
        class="fixed inset-0 z-[1000] flex items-center justify-center bg-black/60 backdrop-blur-[12px] rounded-lg transition-all duration-200 ease-out"
        onClick={() => props.onClose()}
      >
        <div
          class="rounded-lg bg-dark-500 text-[rgba(var(--text-base-rgb),0.88)] p-6 w-[92%] max-w-[640px] max-h-[90vh] overflow-y-auto flex flex-col gap-4 transition-all duration-500 ease-out transform"
          style={{
            background: 'var(--panel-bg)',
            'backdrop-filter': 'blur(var(--acrylic-blur))',
            border: '1px solid var(--acrylic-border)',
          }}
          onClick={(e) => e.stopPropagation()}
        >
          {/* Header */}
          <div class="flex justify-between items-center border-b border-[rgba(var(--text-base-rgb),0.27)] pb-3">
            <h2 class="m-0 text-xl">{t('project.memoryPanel.title')}</h2>
            <button class={btnClose} onClick={() => props.onClose()} aria-label={t('common.close')}>
              <Icon name="x" size={18} class="stroke-[2.5]" />
            </button>
          </div>

          {/* Stats */}
          <Show when={stats()}>
            <div class="text-[11px]" style={{ color: 'rgba(var(--text-base-rgb),0.45)' }}>
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

          {/* Self-evolution quality probe (last 7 days) + consolidation (sleep-time consolidation manual trigger; runs daily automatically) */}
          <Show when={stats()}>
            <div class="flex flex-col gap-1.5">
              <div class="flex items-center justify-between gap-3">
                <label class="text-[10px] text-white/45 uppercase tracking-[1.5px] font-semibold">
                  {t('project.memoryPanel.qTitle')}
                </label>
                <div class="flex items-center gap-1.5">
                  <button
                    class={btnSecondarySm}
                    disabled={graphExtracting()}
                    onClick={() => void extractGraph()}
                  >
                    {graphExtracting() ? '...' : t('project.memoryPanel.graphExtract')}
                  </button>
                  <button
                    class={btnSecondarySm}
                    disabled={consolidating()}
                    onClick={() => void consolidate()}
                  >
                    {consolidating() ? '...' : t('project.memoryPanel.consolidate')}
                  </button>
                </div>
              </div>
              <div class="text-[11px]" style={{ color: 'rgba(var(--text-base-rgb),0.5)' }}>
                {t('project.memoryPanel.qSummary', {
                  queries: String(stats()!.retrievalQueries7d),
                  hits: String(stats()!.retrievalHits7d),
                  dedup: String(stats()!.dedupTriggers7d),
                  extracted: String(stats()!.extracted7d),
                })}
                <Show when={judgeDistLabel()}>
                  <span class="ml-2">
                    {t('project.memoryPanel.qJudge', { dist: judgeDistLabel() })}
                  </span>
                </Show>
              </div>
            </div>
            <SectionDivider />
          </Show>

          {/* Embedding model (project-level; switching clears and re-embeds all of this project's vectors) */}
          <div class="flex flex-col gap-1.5">
            <label class="text-[10px] text-white/45 uppercase tracking-[1.5px] font-semibold">
              {t('project.memoryPanel.embeddingTitle')}
            </label>
            <div class="flex items-center justify-between gap-3">
              <div class="min-w-0 text-[11px]" style={{ color: 'rgba(var(--text-base-rgb),0.6)' }}>
                {embeddingLabel()}
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
                  <button class={btnSecondarySm} onClick={() => setEditingEmbedding(false)}>
                    {t('common.cancel')}
                  </button>
                }
              >
                <button class={btnSecondarySm} onClick={openEmbeddingEditor}>
                  {t('project.memoryPanel.embeddingSwitch')}
                </button>
              </Show>
            </div>
            <Show when={editingEmbedding()}>
              <div class="flex flex-col gap-2 rounded-lg border border-dark-100 p-2.5">
                <div class="text-[11px]" style={{ color: 'rgba(var(--text-base-rgb),0.5)' }}>
                  {t('project.memoryPanel.embeddingPickLocal')}
                </div>
                <Dropdown
                  class="w-full"
                  triggerClass="h-7"
                  value={ebModel()}
                  onChange={setEbModel}
                  options={
                    embedModels().length > 0
                      ? embedModels().map((m) => ({
                          value: m.id,
                          label: `${m.id} · ${m.dims} dims`,
                        }))
                      : [
                          {
                            value: '',
                            label: t('project.memoryPanel.embeddingNoModels'),
                            disabled: true,
                          },
                        ]
                  }
                />
                <input
                  class="w-full p-2.5 bg-dark-300 border border-dark-100 rounded-lg text-[rgba(var(--text-base-rgb),0.88)] text-sm placeholder:text-white/25 focus:outline-none focus:border-pri-50"
                  type="number"
                  min={0}
                  placeholder={t('project.memoryPanel.embeddingDims')}
                  value={ebDims() > 0 ? String(ebDims()) : ''}
                  onInput={(e) => setEbDims(Number(e.currentTarget.value) || 0)}
                />
                <Show when={ebTestResult()}>
                  <div
                    class="text-[11px]"
                    style={{
                      color: ebTestResult()!.ok ? 'var(--color-success)' : 'var(--color-danger)',
                    }}
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
                    class={btnSecondarySm}
                    disabled={ebSaving()}
                    onClick={() => void saveEmbedding()}
                  >
                    {ebSaving() ? t('project.memoryPanel.embeddingSaving') : t('app.memory.save')}
                  </button>
                </div>
              </div>
            </Show>
          </div>

          <SectionDivider />

          {/* Rerank (project-level; second-stage rerank after RRF) — extracted into the MemoryRerankSection child component */}
          <MemoryRerankSection
            projectId={props.projectId}
            rrModels={rrModels}
            onNotice={setNotice}
          />

          <SectionDivider />

          {/* Search */}
          <div class="flex flex-col gap-1.5">
            <label class="text-[10px] text-white/45 uppercase tracking-[1.5px] font-semibold">
              {t('project.memoryPanel.search')}
            </label>
            <div class="flex items-center gap-2">
              <input
                class="flex-1 px-3 py-1.5 bg-dark-300 border border-dark-100 rounded-lg text-[rgba(var(--text-base-rgb),0.88)] text-xs placeholder:text-white/25 focus:outline-none focus:border-pri-50"
                value={query()}
                onInput={(e) => setQuery(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') void doSearch();
                }}
                placeholder={t('project.memoryPanel.searchPlaceholder')}
              />
              <button class={btnSecondarySm} onClick={() => void doSearch()}>
                {t('project.memoryPanel.search')}
              </button>
            </div>
          </div>

          {/* Memory entries */}
          <div class="flex flex-col gap-1.5">
            <div class="flex items-center gap-1">
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
            {/* Entry list scrolls independently with a 240px cap so many entries don't consume the panel's own scroll space */}
            <div class="flex flex-col rounded-lg border border-dark-100 p-2.5 max-h-[240px] overflow-y-auto scrollbar-thin">
              <For each={facts()}>
                {(f) => (
                  <div class="rounded-md px-2.5 py-2 transition-colors hover:bg-white/5">
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
                          <Show when={f.sourceRefs && sourceRefLabel(f.sourceRefs)}>
                            <span class="opacity-70">{sourceRefLabel(f.sourceRefs!)}</span>
                          </Show>
                          <Show when={f.pinned}>
                            <span class="text-yellow-400/80">
                              {t('project.memoryPanel.pinned')}
                            </span>
                          </Show>
                          <Show when={f.updatedAt}>
                            <span>{f.updatedAt}</span>
                          </Show>
                        </div>
                      </div>
                      <div class="flex items-center gap-1 shrink-0">
                        <button class={btnSecondarySm} onClick={() => void editFact(f)}>
                          {t('project.memoryPanel.edit')}
                        </button>
                        <button class={btnSecondarySm} onClick={() => void togglePin(f)}>
                          {f.pinned ? t('project.memoryPanel.unpin') : t('project.memoryPanel.pin')}
                        </button>
                        <Show when={f.status === 'active'}>
                          <button class={btnSecondarySm} onClick={() => void archiveFact(f)}>
                            {t('project.memoryPanel.archive')}
                          </button>
                        </Show>
                        <button class={btnSecondarySm} onClick={() => void deleteFact(f)}>
                          {t('project.memoryPanel.delete')}
                        </button>
                        <button class={btnSecondarySm} onClick={() => void toggleVersions(f.id)}>
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
          </div>

          <SectionDivider />

          {/* Code index and search */}
          <div class="flex flex-col gap-1.5">
            <label class="text-[10px] text-white/45 uppercase tracking-[1.5px] font-semibold">
              {t('project.memoryPanel.codeIndex')}
            </label>
            <div class="flex items-center gap-2">
              <button
                class={btnSecondarySm}
                disabled={indexing()}
                onClick={() => void doCodeIndex()}
              >
                {indexing() ? '...' : t('project.memoryPanel.codeIndex')}
              </button>
              <input
                class="flex-1 px-3 py-1.5 bg-dark-300 border border-dark-100 rounded-lg text-[rgba(var(--text-base-rgb),0.88)] text-xs placeholder:text-white/25 focus:outline-none focus:border-pri-50"
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
              <div class="text-[10px]" style={{ color: 'rgba(var(--text-base-rgb),0.35)' }}>
                {t('project.memoryPanel.codeStatus', {
                  chunks: String(codeStatus()!.chunks),
                  files: String(codeStatus()!.files),
                })}
              </div>
            </Show>
            <Show when={codeHits().length > 0}>
              <div class="flex flex-col gap-1">
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
          </div>

          <SectionDivider />

          {/* Footer actions */}
          <div class="flex items-center gap-2">
            <Show when={notice()}>
              <span class="flex-1 text-[11px] text-pri/80">{notice()}</span>
            </Show>
            <button class={btnSecondarySm} onClick={() => void reindex()}>
              {t('project.memoryPanel.reindex')}
            </button>
            <button class={btnSecondarySm} onClick={() => void prune()}>
              {t('project.memoryPanel.prune')}
            </button>
            <button class={btnSecondarySm} onClick={() => void doExport()}>
              {t('project.memoryPanel.export')}
            </button>
            <button class={btnSecondarySm} onClick={() => void doImport()}>
              {t('project.memoryPanel.import')}
            </button>
            <button class={btnSecondarySm} onClick={() => void clearAll()}>
              {t('project.memoryPanel.clear')}
            </button>
          </div>
        </div>
      </div>
    </Show>
  );
};

export default MemoryPanel;

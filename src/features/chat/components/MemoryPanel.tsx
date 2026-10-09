import { Component, createEffect, createSignal, For, Show } from 'solid-js';
import { listen } from '@tauri-apps/api/event';
import { open as openDialog, save as saveDialog } from '@tauri-apps/plugin-dialog';
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
  // 项目级嵌入器实际状态（实际解析到的 provider/model/维度，与配置可能不一致）
  const [embedderStatus, setEmbedderStatus] = createSignal<EmbedderStatus | null>(null);
  const [editingEmbedding, setEditingEmbedding] = createSignal(false);
  const [ebModel, setEbModel] = createSignal('');
  const [ebTesting, setEbTesting] = createSignal(false);
  const [ebTestResult, setEbTestResult] = createSignal<EmbeddingTestResult | null>(null);
  const [ebSaving, setEbSaving] = createSignal(false);
  // Matryoshka 目标维度（0 = 模型默认全维；仅 text-embedding-3/bge-m3 等支持裁剪）
  const [ebDims, setEbDims] = createSignal(0);
  const [reindexProgress, setReindexProgress] = createSignal<{
    done: number;
    total: number;
  } | null>(null);
  const [rrModels, setRrModels] = createSignal<EmbeddingModelInfo[]>([]);
  // 本地可用的嵌入模型（kind=embed 且已下载且全局启用；来自「Embedding 模型管理」）
  const embedModels = () => {
    const list = rrModels().filter((m) => m.kind === 'embed' && m.downloaded && m.enabled);
    return list.length > 0 ? list : rrModels().filter((m) => m.kind === 'embed' && m.downloaded);
  };
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
      void loadRerankModels();
    }
  });

  const loadEmbedding = async () => {
    try {
      const st = await memoryGetStatus(props.projectId);
      setEmbedding(st.embedding);
      setEmbedderStatus(st.embedder);
      // 重排配置由 MemoryRerankSection 子组件自行加载
    } catch {
      // 记忆库未创建时忽略
    }
  };

  const loadRerankModels = async () => {
    try {
      setRrModels(await embeddingModelList());
    } catch {
      setRrModels([]);
    }
  };

  // ---- Phase 2：sleep-time 合并整理（手动触发）+ 近 7 天质量探针 ----
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
      // 忽略
    } finally {
      setConsolidating(false);
    }
  };

  // ---- Phase 3：轻量知识图谱抽取（P3-3）----
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
      // 忽略
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
      // 非 JSON（旧格式）忽略
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

  const doExport = async () => {
    // 用户经系统保存对话框指定导出路径（后端写盘，fs 权限无需放宽）
    let path: string | null = null;
    try {
      path = await saveDialog({
        defaultPath: 'aio-memory.json',
        filters: [{ name: 'JSON', extensions: ['json'] }],
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
    // 用户经系统打开对话框选择导入文件（后端读盘）
    let path: string | null = null;
    try {
      const picked = await openDialog({
        multiple: false,
        filters: [{ name: 'JSON', extensions: ['json'] }],
      });
      path = typeof picked === 'string' ? picked : null;
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

  // 当前嵌入模型的展示文案（provider · model · dims）：以实际解析到的嵌入器为准。
  // 自 v0.8 起仅支持本地嵌入模型；内置模型固定为 all-MiniLM-L6-v2 · 384 维。
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
    // 维度预填仅在选中模型与当前一致时有意义；换选模型后旧维度无参考价值，留空 = 模型默认
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

  /** 编辑器当前选择的有效维度（输入留空 = 所选模型的默认全维） */
  const ebEffectiveDims = () => {
    if (ebDims() > 0) return ebDims();
    return embedModels().find((m) => m.id === ebModel().trim())?.dims ?? 0;
  };

  /** 是否为会触发向量清空重建的真实切换（模型或有效维度变化） */
  const ebConfigChanged = () => {
    const cur = embedding();
    // 无当前配置（状态加载失败）时按变化处理，交给确认弹窗保护
    if (!cur) return true;
    return cur.model !== ebModel().trim() || cur.dimensions !== ebEffectiveDims();
  };

  const saveEmbedding = async () => {
    if (!ebModel().trim()) {
      setNotice(t('project.memoryPanel.embeddingNeedModel'));
      return;
    }
    // 仅在「真实切换」且「库中已有嵌入内容」时确认：空库首次配置与原样重存
    // 都不触发破坏性提醒（原样重存时后端对无变化配置也会直接早退，不做任何操作）
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
        // 发送有效维度（而非原始输入 0）：让后端「无变化早退」判定真正生效，
        // 避免 dims 留空时被误判为切换而无谓清空重建全部向量
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

  /** 板块分隔线（面板为单列平铺布局，各板块间以细线分隔） */
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
          {/* 头部 */}
          <div class="flex justify-between items-center border-b border-[rgba(var(--text-base-rgb),0.27)] pb-3">
            <h2 class="m-0 text-xl">{t('project.memoryPanel.title')}</h2>
            <button class={btnClose} onClick={() => props.onClose()} aria-label={t('common.close')}>
              <Icon name="x" size={18} class="stroke-[2.5]" />
            </button>
          </div>

          {/* 统计 */}
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

          {/* 自进化质量探针（近 7 天）+ 合并整理（sleep-time consolidation 手动触发；每日自动执行） */}
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

          {/* 嵌入模型（项目级；切换会清空本项目全部向量并重新嵌入） */}
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

          {/* 重排（项目级；RRF 后二阶段精排）—— 已抽为 MemoryRerankSection 子组件 */}
          <MemoryRerankSection
            projectId={props.projectId}
            rrModels={rrModels}
            onNotice={setNotice}
          />

          <SectionDivider />

          {/* 检索 */}
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

          {/* 记忆条目 */}
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
            {/* 条目列表内独立滚动：上限 240px，避免过多条目占满面板本身的滚动空间 */}
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

          {/* 代码索引与检索 */}
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

          {/* 底部操作 */}
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

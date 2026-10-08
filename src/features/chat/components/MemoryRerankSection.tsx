// 项目级重排（rerank）配置子组件 —— 从 MemoryPanel 拆出。
//
// 拥有自己的 rr* 信号与全部重排逻辑（启用/停用、编辑、测试、模型下载/删除）。
// 与父组件的共享：rrModels / embedModels（嵌入模型清单，父组件持有）+ onNotice（提示区）+
// onModelsChanged（模型清单刷新）经 props 传入。

import { createSignal, onMount, Show } from 'solid-js';
import { listen } from '@tauri-apps/api/event';
import { EVENTS } from '../../../core/api/events';
import { t } from '../../../core/i18n';
import {
  embeddingModelDelete,
  embeddingModelDownload,
  memoryGetStatus,
  memoryUpdateRerank,
  rerankSaveApiKey,
  rerankTest,
  DEFAULT_ONLINE_RERANK_MODEL_ID,
  DEFAULT_RERANK_MODEL_ID,
} from '../../../core/utils/memory';
import type {
  EmbeddingModelInfo,
  MemoryRerankConfig,
  RerankTestResult,
} from '../../../core/types/memory';
import {
  btnBadge,
  btnDangerBadge,
  btnGhost,
  btnPrimarySm,
  btnSecondarySm,
  btnTabMd,
  tabActive,
  tabInactive,
} from '../../../shared/components/buttonStyles';

export interface MemoryRerankSectionProps {
  projectId: string;
  /** 本地可用嵌入模型清单（父组件持有，kind=embed 且已下载） */
  embedModels: () => EmbeddingModelInfo[];
  /** 重排/嵌入模型清单信号（父组件持有） */
  rrModels: () => EmbeddingModelInfo[];
  /** 设置提示消息 */
  onNotice: (msg: string) => void;
  /** 刷新共享模型清单 */
  onModelsChanged: () => Promise<void>;
}

const MemoryRerankSection = (props: MemoryRerankSectionProps) => {
  // 项目级重排配置（由 memory_get_status 提供，存于项目记忆库 meta；未加载时用默认值，
  // 保证「启用/停用」「配置/下载」入口在记忆未启用或 status 失败时依然可见可操作）
  const [rerank, setRerank] = createSignal<MemoryRerankConfig>({
    enabled: false,
    provider: 'local',
    model: DEFAULT_RERANK_MODEL_ID,
    apiUrl: '',
    apiKey: undefined,
  });

  // 子组件自行加载项目级重排配置（父组件不再耦合 rerank 状态）
  onMount(async () => {
    try {
      const st = await memoryGetStatus(props.projectId);
      setRerank(st.rerank);
    } catch {
      // 记忆库未创建时保留默认值，入口仍可见可操作
    }
  });
  const [editingRerank, setEditingRerank] = createSignal(false);
  const [rrProvider, setRrProvider] = createSignal<'local' | 'online'>('local');
  const [rrModel, setRrModel] = createSignal('');
  const [rrApiUrl, setRrApiUrl] = createSignal('');
  const [rrApiKey, setRrApiKey] = createSignal('');
  const [rrTesting, setRrTesting] = createSignal(false);
  const [rrTestResult, setRrTestResult] = createSignal<RerankTestResult | null>(null);
  const [rrSaving, setRrSaving] = createSignal(false);
  const [rrDownloading, setRrDownloading] = createSignal(false);
  // 下载进度（model-download-progress 事件驱动；total 未知时为 null 仅显示状态）
  const [rrDownloadProgress, setRrDownloadProgress] = createSignal<{
    file: string;
    downloaded: number;
    total: number;
  } | null>(null);

  const rerankLocalModel = () => props.rrModels().find((m) => m.kind === 'rerank');

  const openRerankEditor = () => {
    const r = rerank();
    setRrProvider(r?.provider === 'online' ? 'online' : 'local');
    setRrModel(r?.model ?? '');
    setRrApiUrl(r?.apiUrl ?? '');
    setRrApiKey('');
    setRrTestResult(null);
    setEditingRerank(true);
  };

  const toggleRerank = async () => {
    const r = rerank();
    if (!r) return;
    const next = !r.enabled;
    if (next) {
      // 启用前说明性能开销（磁盘/内存/CPU 延迟），用户知情确认
      if (!window.confirm(t('project.memoryPanel.rerankEnableConfirm'))) return;
      if (r.provider === 'online' && !r.apiUrl.trim()) {
        props.onNotice(t('project.memoryPanel.rerankNeedUrl'));
        return;
      }
      // 本地重排需先下载模型（一键下载见「配置」区）
      if (r.provider === 'local' && rerankLocalModel()?.downloaded === false) {
        props.onNotice(t('project.memoryPanel.rerankNeedModelFirst'));
        return;
      }
    }
    try {
      const st = await memoryUpdateRerank(props.projectId, { ...r, enabled: next });
      setRerank(st.rerank);
      props.onNotice(t('project.memoryPanel.rerankSaved'));
    } catch {
      props.onNotice(t('project.memoryPanel.rerankSaveFail'));
    }
  };
  const saveRerank = async () => {
    if (rrProvider() === 'online' && !rrApiUrl().trim()) {
      props.onNotice(t('project.memoryPanel.rerankNeedUrl'));
      return;
    }
    setRrSaving(true);
    try {
      if (rrProvider() === 'online') {
        await rerankSaveApiKey(rrApiKey());
      }
      const r = rerank();
      const st = await memoryUpdateRerank(props.projectId, {
        enabled: r?.enabled ?? false,
        provider: rrProvider(),
        model:
          rrProvider() === 'online'
            ? rrModel() || DEFAULT_ONLINE_RERANK_MODEL_ID
            : DEFAULT_RERANK_MODEL_ID,
        apiUrl: rrApiUrl(),
        apiKey: undefined,
      });
      setRerank(st.rerank);
      props.onNotice(t('project.memoryPanel.rerankSaved'));
      setEditingRerank(false);
    } catch {
      props.onNotice(t('project.memoryPanel.rerankSaveFail'));
    } finally {
      setRrSaving(false);
    }
  };

  const testRerank = async () => {
    setRrTesting(true);
    try {
      const res = await rerankTest({
        enabled: true,
        provider: rrProvider(),
        model:
          rrProvider() === 'online'
            ? rrModel() || DEFAULT_ONLINE_RERANK_MODEL_ID
            : DEFAULT_RERANK_MODEL_ID,
        apiUrl: rrApiUrl(),
        apiKey: rrApiKey() || undefined,
      });
      setRrTestResult(res);
    } catch {
      setRrTestResult(null);
    } finally {
      setRrTesting(false);
    }
  };
  const downloadRerankModel = async () => {
    setRrDownloading(true);
    setRrDownloadProgress(null);
    let unlisten: (() => void) | undefined;
    try {
      unlisten = await listen(EVENTS.modelDownloadProgress, (ev) => {
        const p = ev.payload as {
          id?: string;
          file?: string;
          downloaded?: number;
          total?: number;
          done?: boolean;
        };
        if (p?.done) {
          setRrDownloadProgress(null);
          return;
        }
        if (p?.file !== undefined) {
          setRrDownloadProgress({
            file: p.file,
            downloaded: p.downloaded ?? 0,
            total: p.total ?? 0,
          });
        }
      });
      const outcome = await embeddingModelDownload(DEFAULT_RERANK_MODEL_ID);
      await props.onModelsChanged();
      if (outcome.status !== 'ok') return; // 已暂停/取消：不自动启用，不打完成提示
      // 一键下载即用：下载完成后若重排尚未启用，自动启用（在线 Key/URL 不在此列）
      const r = rerank();
      if (r && r.provider === 'local' && !r.enabled) {
        const st = await memoryUpdateRerank(props.projectId, { ...r, enabled: true });
        setRerank(st.rerank);
        props.onNotice(t('project.memoryPanel.rerankDownloadEnabled'));
      } else if (outcome.info.downloaded) {
        props.onNotice(t('project.memoryPanel.rerankDownloaded'));
      }
    } catch {
      props.onNotice(t('project.memoryPanel.rerankTestFail', { error: 'download' }));
    } finally {
      unlisten?.();
      setRrDownloading(false);
      setRrDownloadProgress(null);
    }
  };

  // 下载进度计算：total 未知（Content-Length 缺失）时为 0，仅显示状态文案
  const rrDownloadPct = () => {
    const p = rrDownloadProgress();
    if (!p || p.total <= 0) return 0;
    return Math.min(100, Math.round((p.downloaded / p.total) * 100));
  };

  const rrDownloadLabel = () => {
    const p = rrDownloadProgress();
    if (!p) return t('project.memoryPanel.rerankDownloading');
    if (p.total <= 0) return t('project.memoryPanel.rerankDownloading');
    return t('project.memoryPanel.rerankDownloadingPct', {
      file: p.file,
      pct: String(rrDownloadPct()),
    });
  };

  const deleteRerankModel = async () => {
    if (!window.confirm(t('project.memoryPanel.rerankDeleteConfirm'))) return;
    try {
      await embeddingModelDelete(DEFAULT_RERANK_MODEL_ID);
      await props.onModelsChanged();
    } catch {
      // 忽略
    }
  };
  const rerankLabel = () => {
    const r = rerank();
    if (!r) return '';
    const provider =
      r.provider === 'online' ? t('app.memory.providerOnline') : t('app.memory.providerLocal');
    let status = r.enabled
      ? t('project.memoryPanel.rerankStatusOn')
      : t('project.memoryPanel.rerankStatusOff');
    const local = props.rrModels().find((m) => m.kind === 'rerank');
    if (r.provider === 'local' && r.enabled && local?.downloaded === false) {
      status = t('project.memoryPanel.rerankStatusNeedModel');
    }
    return t('project.memoryPanel.rerankCurrent', { provider, model: r.model || '-', status });
  };
  return (
    <>
      {' '}
      {/* 重排（项目级；RRF 后二阶段精排，未启用/失败时回退原序） */}
      <div class="flex flex-col gap-1.5">
        <div class="flex items-center justify-between gap-3">
          <label class="text-[10px] text-white/45 uppercase tracking-[1.5px] font-semibold">
            {t('project.memoryPanel.rerankTitle')}
          </label>
          <button class={btnBadge} onClick={() => void toggleRerank()}>
            {rerank().enabled
              ? t('project.memoryPanel.rerankDisable')
              : t('project.memoryPanel.rerankEnable')}
          </button>
        </div>
        <div class="flex items-center justify-between gap-3">
          <div class="min-w-0 text-[11px]" style={{ color: 'rgba(var(--text-base-rgb),0.6)' }}>
            {rerankLabel()}
          </div>
          <Show
            when={!editingRerank()}
            fallback={
              <button class={btnGhost} onClick={() => setEditingRerank(false)}>
                {t('common.cancel')}
              </button>
            }
          >
            <button class={btnBadge} onClick={openRerankEditor}>
              {t('project.memoryPanel.rerankConfigure')}
            </button>
          </Show>
        </div>
        <div class="text-[11px]" style={{ color: 'rgba(var(--text-base-rgb),0.45)' }}>
          {t('project.memoryPanel.rerankDesc')}
        </div>
        <Show when={editingRerank()}>
          <div class="flex flex-col gap-2 rounded-lg border border-dark-100 p-2.5">
            <div class="flex gap-2">
              <button
                class={`${btnTabMd} ${rrProvider() === 'local' ? tabActive : tabInactive}`}
                onClick={() => setRrProvider('local')}
              >
                {t('app.memory.providerLocal')}
              </button>
              <button
                class={`${btnTabMd} ${rrProvider() === 'online' ? tabActive : tabInactive}`}
                onClick={() => setRrProvider('online')}
              >
                {t('app.memory.providerOnline')}
              </button>
            </div>
            <Show when={rrProvider() === 'local'}>
              <div class="flex items-center justify-between gap-2">
                <div class="text-[11px]" style={{ color: 'rgba(var(--text-base-rgb),0.5)' }}>
                  {DEFAULT_RERANK_MODEL_ID}
                </div>
                <Show when={rerankLocalModel()}>
                  <button
                    class={btnSecondarySm}
                    disabled={rrDownloading()}
                    onClick={() => void downloadRerankModel()}
                  >
                    {rrDownloading()
                      ? t('project.memoryPanel.rerankDownloading')
                      : rerankLocalModel()!.downloaded
                        ? t('project.memoryPanel.rerankDownloaded')
                        : t('project.memoryPanel.rerankDownload')}
                  </button>
                </Show>
              </div>
              {/* 下载进度条：model-download-progress 事件驱动（分块 downloaded/total） */}
              <Show when={rrDownloadProgress()}>
                <div class="flex flex-col gap-1">
                  <div class="h-1 w-full overflow-hidden rounded bg-white/10">
                    <div
                      class="h-1 rounded bg-pri transition-[width] duration-200"
                      style={{ width: `${rrDownloadPct()}%` }}
                    />
                  </div>
                  <div
                    class="text-right text-[10px]"
                    style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}
                  >
                    {rrDownloadLabel()}
                  </div>
                </div>
              </Show>
              <div class="flex items-center justify-end gap-2">
                <Show when={rerankLocalModel()?.downloaded}>
                  <button class={btnDangerBadge} onClick={() => void deleteRerankModel()}>
                    {t('project.memoryPanel.rerankDelete')}
                  </button>
                </Show>
                <button
                  class={btnSecondarySm}
                  disabled={rrTesting()}
                  onClick={() => void testRerank()}
                >
                  {rrTesting() ? '...' : t('project.memoryPanel.rerankTest')}
                </button>
              </div>
            </Show>
            <Show when={rrProvider() === 'online'}>
              <input
                class="w-full p-2.5 bg-dark-300 border border-dark-100 rounded-lg text-[rgba(var(--text-base-rgb),0.88)] text-sm placeholder:text-white/25 focus:outline-none focus:border-pri-50"
                placeholder={t('app.memory.model')}
                value={rrModel()}
                onInput={(e) => setRrModel(e.currentTarget.value)}
              />
              <input
                class="w-full p-2.5 bg-dark-300 border border-dark-100 rounded-lg text-[rgba(var(--text-base-rgb),0.88)] text-sm placeholder:text-white/25 focus:outline-none focus:border-pri-50"
                placeholder={t('app.memory.apiUrl')}
                value={rrApiUrl()}
                onInput={(e) => setRrApiUrl(e.currentTarget.value)}
              />
              <input
                class="w-full p-2.5 bg-dark-300 border border-dark-100 rounded-lg text-[rgba(var(--text-base-rgb),0.88)] text-sm placeholder:text-white/25 focus:outline-none focus:border-pri-50"
                type="password"
                placeholder={t('app.memory.apiKey')}
                value={rrApiKey()}
                onInput={(e) => setRrApiKey(e.currentTarget.value)}
              />
              <div class="flex items-center justify-end gap-2">
                <button
                  class={btnSecondarySm}
                  disabled={rrTesting()}
                  onClick={() => void testRerank()}
                >
                  {rrTesting() ? '...' : t('app.memory.test')}
                </button>
                <button
                  class={btnPrimarySm}
                  disabled={rrSaving()}
                  onClick={() => void saveRerank()}
                >
                  {rrSaving() ? '...' : t('app.memory.save')}
                </button>
              </div>
            </Show>
            <Show when={rrTestResult()}>
              <div
                class="text-[11px]"
                style={{
                  color: rrTestResult()!.ok ? 'var(--color-success)' : 'var(--color-danger)',
                }}
              >
                {rrTestResult()!.ok
                  ? t('project.memoryPanel.rerankTestOk', {
                      ms: String(rrTestResult()!.latencyMs),
                    })
                  : t('project.memoryPanel.rerankTestFail', {
                      error: rrTestResult()!.error ?? '',
                    })}
              </div>
            </Show>
          </div>
        </Show>
      </div>
    </>
  );
};

export default MemoryRerankSection;

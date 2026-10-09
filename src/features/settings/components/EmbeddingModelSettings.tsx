/**
 * EmbeddingModelSettings — 「Embedding 模型管理」设置子页面。
 *
 * 全局管理本地嵌入与重排模型（与供应商设置类似）：
 * - 嵌入模型：安装目录可自定义（默认 <app_data_dir>/models/embed）；主流开源 ONNX 嵌入模型的下载安装 / 删除 / 启用停用
 * - 重排模型：本地交叉编码器（bge-reranker 系列 / mxbai），下载目录固定 <app_data_dir>/models/rerank
 * - 内置嵌入模型（all-MiniLM-L6-v2）随应用打包，开箱即用、不可删除
 *
 * 项目级 RAG 记忆设置（模型选择）仍在「项目记忆面板」中，本页只管全局模型。
 */
import { Component, createMemo, createSignal, For, Show, onMount } from 'solid-js';
import { listen } from '@tauri-apps/api/event';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import Icon from '../../../shared/components/Icon';
import { btnDangerBadge, btnSecondarySm } from '../../../shared/components/buttonStyles';
import { t } from '../../../core/i18n';
import {
  embeddingModelCancelDownload,
  embeddingModelDelete,
  embeddingModelDownload,
  embeddingModelList,
  embeddingModelPauseDownload,
  embeddingModelSetEnabled,
  embeddingModelsGetDir,
  embeddingModelsSetDir,
} from '../../../core/utils/memory';
import type { EmbeddingModelInfo } from '../../../core/types/memory';
import { formatSize } from '../../../core/utils/format';
import { EVENTS } from '../../../core/api/events';

const EmbeddingModelSettings: Component = () => {
  const [models, setModels] = createSignal<EmbeddingModelInfo[]>([]);
  const [installDir, setInstallDir] = createSignal('');
  const [dirInput, setDirInput] = createSignal('');
  const [savingDir, setSavingDir] = createSignal(false);
  const [notice, setNotice] = createSignal('');
  // 下载进度（model-download-progress 事件驱动；按模型 id 记录）
  const [progress, setProgress] = createSignal<
    Record<string, { file: string; downloaded: number; total: number }>
  >({});
  const [downloading, setDownloading] = createSignal<Record<string, boolean>>({});
  // 暂停中的模型（保留 .part 断点，显示「继续下载」）
  const [pausedIds, setPausedIds] = createSignal<Record<string, boolean>>({});

  const embedModels = createMemo(() => models().filter((m) => m.kind === 'embed'));
  const rerankModels = createMemo(() => models().filter((m) => m.kind === 'rerank'));

  const load = async () => {
    try {
      const [list, dir] = await Promise.all([embeddingModelList(), embeddingModelsGetDir()]);
      setModels(list);
      setInstallDir(dir);
      setDirInput(dir);
    } catch {
      setNotice(t('settings.embeddingModels.loadFail'));
    }
  };

  onMount(() => void load());

  const pickDir = async () => {
    try {
      const dir = await openDialog({ directory: true, multiple: false });
      if (dir && typeof dir === 'string') setDirInput(dir);
    } catch {
      /* user cancelled */
    }
  };

  const saveDir = async () => {
    setSavingDir(true);
    try {
      const effective = await embeddingModelsSetDir(dirInput());
      setInstallDir(effective);
      setDirInput(effective);
      setNotice(t('settings.embeddingModels.dirSaved'));
      await load();
    } catch {
      setNotice(t('settings.embeddingModels.dirSaveFail'));
    } finally {
      setSavingDir(false);
    }
  };

  const resetDir = async () => {
    setDirInput('');
    await saveDir();
  };

  const toggleEnabled = async (m: EmbeddingModelInfo) => {
    try {
      const next = await embeddingModelSetEnabled(m.id, !m.enabled);
      setModels((prev) => prev.map((x) => (x.id === next.id ? next : x)));
    } catch {
      setNotice(t('settings.embeddingModels.toggleFail'));
    }
  };

  const download = async (m: EmbeddingModelInfo) => {
    setDownloading((prev) => ({ ...prev, [m.id]: true }));
    setPausedIds((prev) => ({ ...prev, [m.id]: false }));
    setProgress((prev) => ({ ...prev, [m.id]: { file: '', downloaded: 0, total: 0 } }));
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
        if (!p || p.id !== m.id) return;
        if (p.done) {
          setProgress((prev) => {
            const next = { ...prev };
            delete next[m.id];
            return next;
          });
          return;
        }
        if (p.file !== undefined) {
          setProgress((prev) => ({
            ...prev,
            [m.id]: { file: p.file!, downloaded: p.downloaded ?? 0, total: p.total ?? 0 },
          }));
        }
      });
      const outcome = await embeddingModelDownload(m.id);
      await load();
      if (outcome.status === 'ok') {
        setNotice(t('settings.embeddingModels.downloaded', { name: m.id }));
      } else if (outcome.status === 'paused') {
        setPausedIds((prev) => ({ ...prev, [m.id]: true }));
        setNotice(t('settings.embeddingModels.downloadPaused', { name: m.id }));
      } else {
        setNotice(t('settings.embeddingModels.downloadCancelled', { name: m.id }));
      }
    } catch {
      setNotice(t('settings.embeddingModels.downloadFail', { name: m.id }));
    } finally {
      unlisten?.();
      setDownloading((prev) => ({ ...prev, [m.id]: false }));
      setProgress((prev) => {
        const next = { ...prev };
        delete next[m.id];
        return next;
      });
    }
  };

  const pauseDownload = (m: EmbeddingModelInfo) => {
    void embeddingModelPauseDownload(m.id);
  };

  const cancelDownload = async (m: EmbeddingModelInfo) => {
    const wasActive = downloading()[m.id];
    try {
      await embeddingModelCancelDownload(m.id);
    } catch {
      /* ignore */
    }
    // 无进行中的下载（已暂停）：命令已直接清理断点文件，本地重置暂停态；
    // 进行中：download() 的 promise 会以 cancelled 结局返回，由其统一收尾
    if (!wasActive) {
      setPausedIds((prev) => ({ ...prev, [m.id]: false }));
      setNotice(t('settings.embeddingModels.downloadCancelled', { name: m.id }));
    }
  };

  const removeModel = async (m: EmbeddingModelInfo) => {
    if (!window.confirm(t('settings.embeddingModels.deleteConfirm', { name: m.id }))) return;
    try {
      await embeddingModelDelete(m.id);
      await load();
      setNotice(t('settings.embeddingModels.deleted', { name: m.id }));
    } catch {
      setNotice(t('settings.embeddingModels.deleteFail', { name: m.id }));
    }
  };

  const pct = (m: EmbeddingModelInfo) => {
    const p = progress()[m.id];
    if (!p || p.total <= 0) return 0;
    return Math.min(100, Math.round((p.downloaded / p.total) * 100));
  };

  const progressLabel = (m: EmbeddingModelInfo) => {
    const p = progress()[m.id];
    if (!p) return '';
    if (p.total <= 0) return t('settings.embeddingModels.downloading');
    return t('settings.embeddingModels.downloadingPct', {
      file: p.file,
      pct: String(pct(m)),
    });
  };

  /** 模型行渲染（嵌入/重排模型共用）：下载/暂停/取消/删除/启停开关 + 进度条 */
  const renderModelRow = (m: EmbeddingModelInfo, index: () => number) => {
    const dlProgress = () => progress()[m.id];
    const isDownloading = () => downloading()[m.id];
    const isPaused = () => pausedIds()[m.id];
    return (
      <div
        class="flex items-center gap-3 px-3 py-2.5 rounded-[10px] border border-white/[0.05] bg-white/[0.02] animate-row-in"
        style={{ 'animation-delay': (index() + 1) * 30 + 'ms' }}
      >
        <div
          class="flex items-center justify-center w-9 h-9 rounded-lg shrink-0"
          style={{
            background: m.bundled
              ? 'rgba(var(--primary-rgb),0.15)'
              : 'rgba(var(--text-base-rgb),0.06)',
            color: m.bundled ? 'var(--primary-color)' : 'rgba(var(--text-base-rgb),0.5)',
          }}
        >
          <Icon name={m.bundled ? 'check-circle' : 'model'} size={18} />
        </div>
        <div class="grow min-w-0">
          <div class="flex items-center gap-2">
            <span class="text-[13px] text-white/85 font-medium font-mono truncate">{m.id}</span>
            <Show when={m.bundled}>
              <span class="px-1 py-0.5 rounded bg-pri-20 text-pri text-[10px] shrink-0">
                {t('settings.embeddingModels.bundled')}
              </span>
            </Show>
          </div>
          <div class="text-[10px] mt-0.5" style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}>
            {m.label}
            <Show when={m.dims > 0}>
              <span class="ml-1.5 opacity-70">{m.dims} dims</span>
            </Show>
            <Show when={formatSize(m.sizeBytes)}>
              <span class="ml-1.5 opacity-70">· {formatSize(m.sizeBytes)}</span>
            </Show>
          </div>
          <Show when={dlProgress()}>
            <div class="flex flex-col gap-1 mt-1.5">
              <div class="h-1 w-full overflow-hidden rounded bg-white/10">
                <div
                  class="h-1 rounded bg-pri transition-[width] duration-200"
                  style={{ width: pct(m) + '%' }}
                />
              </div>
              <div
                class="text-right text-[10px]"
                style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}
              >
                {progressLabel(m)}
              </div>
            </div>
          </Show>
        </div>
        <div class="flex items-center gap-1.5 shrink-0">
          <Show
            when={m.downloaded}
            fallback={
              <div class="flex items-center gap-1.5">
                <Show
                  when={!isDownloading()}
                  fallback={
                    <>
                      <button type="button" class={btnSecondarySm} onClick={() => pauseDownload(m)}>
                        <Icon name="pause" size={12} />
                        {t('settings.embeddingModels.pause')}
                      </button>
                      <button
                        type="button"
                        class={btnSecondarySm}
                        onClick={() => void cancelDownload(m)}
                      >
                        <Icon name="x" size={12} />
                        {t('common.cancel')}
                      </button>
                    </>
                  }
                >
                  <>
                    <button type="button" class={btnSecondarySm} onClick={() => void download(m)}>
                      <Icon name={isPaused() ? 'play' : 'download'} size={12} />
                      {isPaused()
                        ? t('settings.embeddingModels.resume')
                        : t('settings.embeddingModels.download')}
                    </button>
                    <Show when={isPaused()}>
                      <button
                        type="button"
                        class={btnSecondarySm}
                        onClick={() => void cancelDownload(m)}
                      >
                        <Icon name="x" size={12} />
                        {t('common.cancel')}
                      </button>
                    </Show>
                  </>
                </Show>
              </div>
            }
          >
            <span class="text-[11px] text-green-400/70">
              {t('settings.embeddingModels.installed')}
            </span>
            <Show when={!m.bundled}>
              <button
                type="button"
                class={btnDangerBadge}
                disabled={isDownloading()}
                onClick={() => void removeModel(m)}
              >
                <Icon name="trash" size={11} />
                {t('settings.embeddingModels.delete')}
              </button>
            </Show>
          </Show>
          <button
            type="button"
            class="relative inline-flex items-center h-[22px] w-[40px] rounded-full bg-white/[0.08] border border-white/[0.08] cursor-pointer shrink-0 focus:outline-none"
            style={{
              transition:
                'background 0.3s cubic-bezier(0.4, 0, 0.2, 1), border-color 0.3s ease, box-shadow 0.3s ease',
              ...(m.enabled
                ? {
                    background: 'rgba(var(--primary-rgb), 0.7)',
                    'border-color': 'rgba(var(--primary-rgb), 0.5)',
                    'box-shadow': '0 0 12px rgba(var(--primary-rgb), 0.35)',
                  }
                : {}),
            }}
            onClick={() => void toggleEnabled(m)}
            title={
              m.enabled
                ? t('settings.embeddingModels.disable')
                : t('settings.embeddingModels.enable')
            }
          >
            <span
              class="inline-block h-4 w-4 rounded-full bg-white transition-transform duration-300 shadow-[0_2px_6px_rgba(0,0,0,0.4)]"
              style={{ transform: m.enabled ? 'translateX(21px)' : 'translateX(3px)' }}
            />
          </button>
        </div>
      </div>
    );
  };

  /** 行渲染组件：把 For 的 index 访问器以 JSX 属性传递（solid/reactivity 要求 JSX 内使用） */
  const ModelRowItem: Component<{ m: EmbeddingModelInfo; index: () => number }> = (p) =>
    renderModelRow(p.m, p.index);

  return (
    <div class="space-y-1">
      <div class="pb-3 border-b border-[var(--border-dim)] animate-row-in">
        <div class="flex items-center justify-between gap-3">
          <h2 class="text-lg text-white font-semibold m-0">
            {t('settings.embeddingModels.title')}
          </h2>
          <Show when={notice()}>
            <span class="text-[11px] text-pri/80">{notice()}</span>
          </Show>
        </div>
      </div>

      {/* 安装目录 */}
      <div
        class="bg-white/[0.03] rounded-xl border border-[var(--border-dim)] p-4 animate-row-in"
        style={{ 'animation-delay': '30ms' }}
      >
        <div class="flex items-center justify-between gap-3 mb-2">
          <div class="min-w-0">
            <div class="text-sm text-white font-medium">
              {t('settings.embeddingModels.dirTitle')}
            </div>
            <div class="text-[11px] mt-0.5" style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}>
              {t('settings.embeddingModels.dirDesc')}
            </div>
          </div>
          <Icon name="folder" size={18} class="text-white/25 shrink-0" />
        </div>
        <div class="flex items-center gap-2">
          <input
            type="text"
            class="flex-1 min-w-0 px-3 py-2 bg-black/25 border border-white/[0.08] rounded-lg text-[rgba(var(--text-base-rgb),0.88)] text-xs placeholder:text-white/25 focus:outline-none focus:border-pri-50"
            placeholder={installDir()}
            value={dirInput()}
            onInput={(e) => setDirInput(e.currentTarget.value)}
          />
          <button type="button" class={btnSecondarySm} onClick={() => void pickDir()}>
            {t('settings.embeddingModels.browse')}
          </button>
          <button
            type="button"
            class={btnSecondarySm}
            disabled={savingDir()}
            onClick={() => void resetDir()}
          >
            {t('settings.embeddingModels.resetDir')}
          </button>
          <button
            type="button"
            class={btnSecondarySm}
            disabled={savingDir() || dirInput() === installDir()}
            onClick={() => void saveDir()}
          >
            {t('common.save')}
          </button>
        </div>
      </div>

      {/* 嵌入模型列表 */}
      <div
        class="bg-white/[0.03] rounded-xl border border-[var(--border-dim)] p-4 animate-row-in"
        style={{ 'animation-delay': '60ms' }}
      >
        <div class="flex items-center justify-between mb-3">
          <div class="min-w-0">
            <div class="text-sm text-white font-medium">
              {t('settings.embeddingModels.listTitle')}
            </div>
            <div class="text-[11px] mt-0.5" style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}>
              {t('settings.embeddingModels.listDesc')}
            </div>
          </div>
        </div>

        <div class="flex flex-col gap-1.5">
          <For each={embedModels()}>{(m, i) => <ModelRowItem m={m} index={i} />}</For>
        </div>
      </div>

      {/* 重排模型列表 */}
      <div
        class="bg-white/[0.03] rounded-xl border border-[var(--border-dim)] p-4 animate-row-in"
        style={{ 'animation-delay': '90ms' }}
      >
        <div class="flex items-center justify-between mb-3">
          <div class="min-w-0">
            <div class="text-sm text-white font-medium">
              {t('settings.embeddingModels.rerankTitle')}
            </div>
            <div class="text-[11px] mt-0.5" style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}>
              {t('settings.embeddingModels.rerankDesc')}
            </div>
          </div>
        </div>

        <div class="flex flex-col gap-1.5">
          <For each={rerankModels()}>{(m, i) => <ModelRowItem m={m} index={i} />}</For>
        </div>
      </div>
    </div>
  );
};

export default EmbeddingModelSettings;

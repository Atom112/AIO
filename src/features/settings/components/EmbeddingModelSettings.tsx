/**
 * EmbeddingModelSettings - "Embedding models" settings sub-page.
 *
 * Global management of local embedding and rerank models (similar to provider settings):
 * - Embedding models: customizable install directory (default <app_data_dir>/models/embed); download/install, delete, enable/disable for popular open-source ONNX embedding models
 * - Rerank models: local cross-encoders (bge-reranker family / mxbai), download directory fixed at <app_data_dir>/models/rerank
 * - Bundled embedding model (all-MiniLM-L6-v2) ships with the app: ready out of the box, cannot be deleted
 *
 * Project-level RAG memory settings (model selection) stay in the project memory panel; this page only manages global models.
 */
import { Component, createMemo, createSignal, For, Show, onMount } from 'solid-js';
import { listen } from '@tauri-apps/api/event';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import Icon, { type IconName } from '../../../shared/components/Icon';
import Switch from '../../../shared/components/Switch';
import {
  btnDangerOutline,
  btnPrimaryOutlineSm,
  btnSecondarySm,
} from '../../../shared/components/buttonStyles';
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
} from '../../../core/api/memory';
import type { EmbeddingModelInfo } from '../../../core/types/memory';
import { formatSize } from '../../../core/utils/format';
import { EVENTS } from '../../../core/api/events';

const EmbeddingModelSettings: Component = () => {
  const [models, setModels] = createSignal<EmbeddingModelInfo[]>([]);
  const [installDir, setInstallDir] = createSignal('');
  const [dirInput, setDirInput] = createSignal('');
  const [savingDir, setSavingDir] = createSignal(false);
  const [notice, setNotice] = createSignal('');
  // Download progress (driven by model-download-progress events; keyed by model id)
  const [progress, setProgress] = createSignal<
    Record<string, { file: string; downloaded: number; total: number }>
  >({});
  const [downloading, setDownloading] = createSignal<Record<string, boolean>>({});
  // Paused models (keep the .part resume data; show a "resume download" button)
  const [pausedIds, setPausedIds] = createSignal<Record<string, boolean>>({});

  const embedModels = createMemo(() => models().filter((m) => m.kind === 'embed'));
  const rerankModels = createMemo(() => models().filter((m) => m.kind === 'rerank'));

  /** Transient feedback line (auto-clears; sits on its own row under the page title) */
  let noticeTimer = 0;
  const showNotice = (msg: string) => {
    setNotice(msg);
    window.clearTimeout(noticeTimer);
    noticeTimer = window.setTimeout(() => setNotice(''), 4000);
  };

  const load = async () => {
    try {
      const [list, dir] = await Promise.all([embeddingModelList(), embeddingModelsGetDir()]);
      setModels(list);
      setInstallDir(dir);
      setDirInput(dir);
    } catch {
      showNotice(t('settings.embeddingModels.loadFail'));
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
      showNotice(t('settings.embeddingModels.dirSaved'));
      await load();
    } catch {
      showNotice(t('settings.embeddingModels.dirSaveFail'));
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
      showNotice(t('settings.embeddingModels.toggleFail'));
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
        showNotice(t('settings.embeddingModels.downloaded', { name: m.id }));
      } else if (outcome.status === 'paused') {
        setPausedIds((prev) => ({ ...prev, [m.id]: true }));
        showNotice(t('settings.embeddingModels.downloadPaused', { name: m.id }));
      } else {
        showNotice(t('settings.embeddingModels.downloadCancelled', { name: m.id }));
      }
    } catch {
      showNotice(t('settings.embeddingModels.downloadFail', { name: m.id }));
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
    // No active download (paused): the command already removed the partial file, so reset the paused state locally;
    // if a download is in progress, download()'s promise resolves as cancelled and handles cleanup itself
    if (!wasActive) {
      setPausedIds((prev) => ({ ...prev, [m.id]: false }));
      showNotice(t('settings.embeddingModels.downloadCancelled', { name: m.id }));
    }
  };

  const removeModel = async (m: EmbeddingModelInfo) => {
    if (!window.confirm(t('settings.embeddingModels.deleteConfirm', { name: m.id }))) return;
    try {
      await embeddingModelDelete(m.id);
      await load();
      showNotice(t('settings.embeddingModels.deleted', { name: m.id }));
    } catch {
      showNotice(t('settings.embeddingModels.deleteFail', { name: m.id }));
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

  /** Model row renderer (shared by embedding/rerank models): download/pause/cancel/delete, enable toggle + progress bar */
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
          class="flex items-center justify-center w-8 h-8 rounded-lg shrink-0"
          style={{
            background: m.bundled
              ? 'rgba(var(--primary-rgb),0.15)'
              : 'rgba(var(--text-base-rgb),0.06)',
            color: m.bundled ? 'var(--primary-color)' : 'rgba(var(--text-base-rgb),0.5)',
          }}
        >
          <Icon name={m.kind === 'rerank' ? 'arrow-down-up' : 'embedding'} size={16} />
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
                    <button
                      type="button"
                      class={btnPrimaryOutlineSm}
                      onClick={() => void download(m)}
                    >
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
            <span class="inline-flex items-center gap-1 text-[11px] text-success">
              <Icon name="check" size={10} />
              {t('settings.embeddingModels.installed')}
            </span>
            <Show when={!m.bundled}>
              <button
                type="button"
                class={btnDangerOutline}
                disabled={isDownloading()}
                onClick={() => void removeModel(m)}
              >
                <Icon name="trash" size={11} />
                {t('settings.embeddingModels.delete')}
              </button>
            </Show>
          </Show>
          <Switch
            variant="glow"
            checked={m.enabled}
            onChange={() => void toggleEnabled(m)}
            label={
              m.enabled
                ? t('settings.embeddingModels.disable')
                : t('settings.embeddingModels.enable')
            }
          />
        </div>
      </div>
    );
  };

  /** Row renderer component: passes For's index accessor as a JSX prop (solid/reactivity requires it to be used in JSX) */
  const ModelRowItem: Component<{ m: EmbeddingModelInfo; index: () => number }> = (p) =>
    renderModelRow(p.m, p.index);

  /** Section header: icon chip + title + description (shared by the three cards) */
  const SectionHeader: Component<{ icon: IconName; title: string; desc: string }> = (p) => (
    <div class="flex items-center gap-3 mb-3">
      <div
        class="w-8 h-8 rounded-lg flex items-center justify-center shrink-0 border"
        style={{
          background: 'rgba(var(--text-base-rgb),0.05)',
          'border-color': 'rgba(var(--text-base-rgb),0.12)',
          color: 'rgba(var(--text-base-rgb),0.55)',
        }}
      >
        <Icon name={p.icon} size={15} />
      </div>
      <div class="min-w-0">
        <div class="text-sm text-white font-medium">{p.title}</div>
        <div class="text-[11px] mt-0.5" style={{ color: 'rgba(var(--text-base-rgb),0.4)' }}>
          {p.desc}
        </div>
      </div>
    </div>
  );

  return (
    <div class="space-y-1">
      <div class="pb-3 border-b border-[var(--border-dim)] animate-row-in">
        <h2 class="text-lg text-white font-semibold m-0">{t('settings.embeddingModels.title')}</h2>
      </div>

      {/* Transient feedback (own row, auto-clears after 4s) */}
      <Show when={notice()}>
        <div class="text-[11px] px-1 text-pri/80 animate-row-in">{notice()}</div>
      </Show>

      {/* Install directory */}
      <div
        class="bg-white/[0.03] rounded-xl border border-[var(--border-dim)] p-4 animate-row-in"
        style={{ 'animation-delay': '30ms' }}
      >
        <SectionHeader
          icon="folder"
          title={t('settings.embeddingModels.dirTitle')}
          desc={t('settings.embeddingModels.dirDesc')}
        />
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
            class={btnPrimaryOutlineSm}
            disabled={savingDir() || dirInput() === installDir()}
            onClick={() => void saveDir()}
          >
            {t('common.save')}
          </button>
        </div>
      </div>

      {/* Embedding model list */}
      <div
        class="bg-white/[0.03] rounded-xl border border-[var(--border-dim)] p-4 animate-row-in"
        style={{ 'animation-delay': '60ms' }}
      >
        <SectionHeader
          icon="embedding"
          title={t('settings.embeddingModels.listTitle')}
          desc={t('settings.embeddingModels.listDesc')}
        />
        <div class="flex flex-col gap-1.5">
          <For each={embedModels()}>{(m, i) => <ModelRowItem m={m} index={i} />}</For>
        </div>
      </div>

      {/* Rerank model list */}
      <div
        class="bg-white/[0.03] rounded-xl border border-[var(--border-dim)] p-4 animate-row-in"
        style={{ 'animation-delay': '90ms' }}
      >
        <SectionHeader
          icon="arrow-down-up"
          title={t('settings.embeddingModels.rerankTitle')}
          desc={t('settings.embeddingModels.rerankDesc')}
        />
        <div class="flex flex-col gap-1.5">
          <For each={rerankModels()}>{(m, i) => <ModelRowItem m={m} index={i} />}</For>
        </div>
      </div>
    </div>
  );
};

export default EmbeddingModelSettings;

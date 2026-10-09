// Project-level rerank configuration subcomponent.
//
// Since v0.8.1 only local rerank models are supported (manifest kind=rerank entries):
// - Model download/delete/global enable-disable lives in Settings -> Embedding model management -> Rerank models
// - This component handles project-level enable/disable + model selection (downloaded and globally enabled models) + testing + saving
// Shared with the parent via props: rrModels (model list signal) + onNotice (notice area).

import { createSignal, onMount, Show } from 'solid-js';
import { t } from '../../../core/i18n';
import { memoryGetStatus, memoryUpdateRerank, rerankTest } from '../../../core/utils/memory';
import type {
  EmbeddingModelInfo,
  MemoryRerankConfig,
  RerankTestResult,
} from '../../../core/types/memory';
import { btnSecondarySm } from '../../../shared/components/buttonStyles';
import Dropdown from '../../../shared/components/Dropdown';
import Switch from '../../../shared/components/Switch';

export interface MemoryRerankSectionProps {
  projectId: string;
  /** Rerank/embedding model list signal (owned by the parent) */
  rrModels: () => EmbeddingModelInfo[];
  /** Show a notice message */
  onNotice: (msg: string) => void;
}

const MemoryRerankSection = (props: MemoryRerankSectionProps) => {
  // Project-level rerank config (served by memory_get_status, stored in the project memory store meta; defaults before load,
  // so the enable/disable and configure entry points stay visible and usable when memory is disabled or status fails)
  const [rerank, setRerank] = createSignal<MemoryRerankConfig>({
    enabled: false,
    provider: 'local',
    model: '',
  });

  // The subcomponent loads project-level rerank config itself (the parent no longer couples to rerank state)
  onMount(async () => {
    try {
      const st = await memoryGetStatus(props.projectId);
      setRerank(st.rerank);
    } catch {
      // Keep defaults when the memory store is not created; the entry point stays visible and usable
    }
  });
  const [editingRerank, setEditingRerank] = createSignal(false);
  const [rrModel, setRrModel] = createSignal('');
  const [rrTesting, setRrTesting] = createSignal(false);
  const [rrTestResult, setRrTestResult] = createSignal<RerankTestResult | null>(null);
  const [rrSaving, setRrSaving] = createSignal(false);

  // Selectable models: downloaded and globally enabled (download/delete/enable-disable managed in settings)
  const rerankModels = () =>
    props.rrModels().filter((m) => m.kind === 'rerank' && m.downloaded && m.enabled);
  // Info of the currently configured model (may be not downloaded/disabled - used for status display and pre-enable checks)
  const currentModelInfo = () =>
    props.rrModels().find((m) => m.kind === 'rerank' && m.id === rerank()?.model);

  const openRerankEditor = () => {
    const r = rerank();
    const selectable = rerankModels();
    setRrModel(
      selectable.some((m) => m.id === r?.model) ? (r?.model ?? '') : (selectable[0]?.id ?? ''),
    );
    setRrTestResult(null);
    setEditingRerank(true);
  };

  const toggleRerank = async (next: boolean) => {
    const r = rerank();
    if (!r) return;
    if (next) {
      // Explain the perf cost (memory/CPU latency) before enabling; require informed user confirmation
      if (!window.confirm(t('project.memoryPanel.rerankEnableConfirm'))) return;
      // Local rerank needs the model downloaded first (download/global toggle lives in settings)
      if (currentModelInfo()?.downloaded === false || rerankModels().length === 0) {
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
    if (!rrModel().trim()) {
      props.onNotice(t('project.memoryPanel.rerankNeedModelFirst'));
      return;
    }
    setRrSaving(true);
    try {
      const r = rerank();
      const st = await memoryUpdateRerank(props.projectId, {
        enabled: r?.enabled ?? false,
        provider: 'local',
        model: rrModel(),
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
        provider: 'local',
        model: rrModel(),
      });
      setRrTestResult(res);
    } catch {
      setRrTestResult(null);
    } finally {
      setRrTesting(false);
    }
  };

  const rerankLabel = () => {
    const r = rerank();
    if (!r) return '';
    const provider = t('app.memory.providerLocal');
    let status = r.enabled
      ? t('project.memoryPanel.rerankStatusOn')
      : t('project.memoryPanel.rerankStatusOff');
    if (r.enabled && currentModelInfo()?.downloaded === false) {
      status = t('project.memoryPanel.rerankStatusNeedModel');
    }
    return t('project.memoryPanel.rerankCurrent', { provider, model: r.model || '-', status });
  };
  return (
    <>
      {' '}
      {/* Rerank (project-level; second-stage refinement after RRF; falls back to original order when disabled or failing) */}
      <div class="flex flex-col gap-1.5">
        <div class="flex items-center justify-between gap-3">
          <label class="text-[10px] text-white/45 uppercase tracking-[1.5px] font-semibold">
            {t('project.memoryPanel.rerankTitle')}
          </label>
          <Switch
            checked={rerank().enabled}
            label={t('project.memoryPanel.rerankTitle')}
            onChange={(enabled) => void toggleRerank(enabled)}
          />
        </div>
        <div class="flex items-center justify-between gap-3">
          <div class="min-w-0 text-[11px]" style={{ color: 'rgba(var(--text-base-rgb),0.6)' }}>
            {rerankLabel()}
          </div>
          <Show
            when={!editingRerank()}
            fallback={
              <button class={btnSecondarySm} onClick={() => setEditingRerank(false)}>
                {t('common.cancel')}
              </button>
            }
          >
            <button class={btnSecondarySm} onClick={openRerankEditor}>
              {t('project.memoryPanel.rerankConfigure')}
            </button>
          </Show>
        </div>
        <Show when={editingRerank()}>
          {/* Reserve the same height as when models exist so the empty state does not collapse */}
          <div class="flex flex-col rounded-lg border border-dark-100 p-2.5 min-h-[110px]">
            <Show
              when={rerankModels().length > 0}
              fallback={
                <div
                  class="flex-1 flex items-center justify-center text-[11px] text-center"
                  style={{ color: 'rgba(var(--text-base-rgb),0.45)' }}
                >
                  {t('project.memoryPanel.rerankNeedModelFirst')}
                </div>
              }
            >
              <div class="flex flex-col gap-2">
                <div class="text-[11px]" style={{ color: 'rgba(var(--text-base-rgb),0.5)' }}>
                  {t('project.memoryPanel.rerankPick')}
                </div>
                <Dropdown
                  class="w-full"
                  triggerClass="h-7"
                  value={rrModel()}
                  onChange={setRrModel}
                  options={rerankModels().map((m) => ({ value: m.id, label: m.id }))}
                />
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
                <div class="flex items-center justify-end gap-2">
                  <button
                    class={btnSecondarySm}
                    disabled={rrTesting()}
                    onClick={() => void testRerank()}
                  >
                    {rrTesting() ? '...' : t('project.memoryPanel.rerankTest')}
                  </button>
                  <button
                    class={btnSecondarySm}
                    disabled={rrSaving()}
                    onClick={() => void saveRerank()}
                  >
                    {rrSaving() ? '...' : t('app.memory.save')}
                  </button>
                </div>
              </div>
            </Show>
          </div>
        </Show>
      </div>
    </>
  );
};

export default MemoryRerankSection;

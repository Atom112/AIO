// 项目级重排（rerank）配置子组件 —— 从 MemoryPanel 拆出。
//
// 自 v0.8.1 起仅支持本地重排模型（manifest kind=rerank 条目）：
// - 模型的下载/删除/全局启停在「设置 → Embedding 模型管理 → 重排模型」中管理
// - 本组件负责：项目级启用/停用 + 模型选择（已下载且全局启用的模型）+ 测试 + 保存
// 与父组件的共享：rrModels（模型清单信号）+ onNotice（提示区）经 props 传入。

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
  /** 重排/嵌入模型清单信号（父组件持有） */
  rrModels: () => EmbeddingModelInfo[];
  /** 设置提示消息 */
  onNotice: (msg: string) => void;
}

const MemoryRerankSection = (props: MemoryRerankSectionProps) => {
  // 项目级重排配置（由 memory_get_status 提供，存于项目记忆库 meta；未加载时用默认值，
  // 保证「启用/停用」「配置」入口在记忆未启用或 status 失败时依然可见可操作）
  const [rerank, setRerank] = createSignal<MemoryRerankConfig>({
    enabled: false,
    provider: 'local',
    model: '',
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
  const [rrModel, setRrModel] = createSignal('');
  const [rrTesting, setRrTesting] = createSignal(false);
  const [rrTestResult, setRrTestResult] = createSignal<RerankTestResult | null>(null);
  const [rrSaving, setRrSaving] = createSignal(false);

  // 可选模型：已下载且全局启用（下载/删除/启停在设置页管理）
  const rerankModels = () =>
    props.rrModels().filter((m) => m.kind === 'rerank' && m.downloaded && m.enabled);
  // 当前配置模型的信息（可能未下载/已停用——用于状态展示与启用前检查）
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
      // 启用前说明性能开销（内存/CPU 延迟），用户知情确认
      if (!window.confirm(t('project.memoryPanel.rerankEnableConfirm'))) return;
      // 本地重排需先下载模型（下载/全局启停入口在设置页）
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
      {/* 重排（项目级；RRF 后二阶段精排，未启用/失败时回退原序） */}
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
          <div class="flex flex-col gap-2 rounded-lg border border-dark-100 p-2.5">
            <div class="text-[11px]" style={{ color: 'rgba(var(--text-base-rgb),0.5)' }}>
              {t('project.memoryPanel.rerankPick')}
            </div>
            <Show
              when={rerankModels().length > 0}
              fallback={
                <div class="text-[11px]" style={{ color: 'rgba(var(--text-base-rgb),0.45)' }}>
                  {t('project.memoryPanel.rerankNeedModelFirst')}
                </div>
              }
            >
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
            </Show>
          </div>
        </Show>
      </div>
    </>
  );
};

export default MemoryRerankSection;

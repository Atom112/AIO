/**
 * Workspace instruction-file editor — each CLI tool reads a convention file
 * from the workspace root (CLAUDE.md / AGENTS.md / GEMINI.md / QWEN.md /
 * .goosehints); this modal edits it in place so the harness agent picks up the
 * project instructions without leaving AIO. Atomic write on save.
 */
import { Component, createSignal, onMount, Show } from 'solid-js';
import Icon from '../../../shared/components/Icon';
import { btnClose, btnPrimaryOutline, btnSecondary } from '../../../shared/components/buttonStyles';
import {
  harnessInstructionRead,
  harnessInstructionWrite,
  type HarnessInstruction,
} from '../../../core/api/harness';
import { t } from '../../../core/i18n';

interface HarnessInstructionModalProps {
  toolId: string;
  toolName: string;
  workspacePath: string;
  workspaceName: string;
  onClose: () => void;
}

const HarnessInstructionModal: Component<HarnessInstructionModalProps> = (props) => {
  const [data, setData] = createSignal<HarnessInstruction | null>(null);
  const [content, setContent] = createSignal('');
  const [savedTick, setSavedTick] = createSignal(false);
  const [error, setError] = createSignal('');

  onMount(async () => {
    try {
      const d = await harnessInstructionRead(props.toolId, props.workspacePath);
      setData(d);
      setContent(d.content);
    } catch (e) {
      setError(String(e));
    }
  });

  const save = async () => {
    setError('');
    try {
      await harnessInstructionWrite(props.toolId, props.workspacePath, content());
      setSavedTick(true);
      setTimeout(() => setSavedTick(false), 1500);
      const d = data();
      if (d) setData({ ...d, exists: true, content: content() });
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div
      class="fixed inset-0 z-[1000] bg-black/25 flex items-center justify-center"
      onClick={() => props.onClose()}
    >
      <div
        class="rounded-xl px-6 py-5 max-w-[640px] w-[92%] relative"
        style={{
          background: 'var(--dialog-bg)',
          'backdrop-filter': 'var(--popup-blur)',
          border: '1px solid var(--border-dim)',
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <button class={`absolute top-3 right-3 ${btnClose}`} onClick={() => props.onClose()}>
          <Icon name="x" class="w-4 h-4" />
        </button>

        <div class="text-sm font-medium text-white/85 mb-1">
          {t('harness.instruction.title', { name: props.toolName })}
        </div>
        <div class="text-[11px] text-white/40 mb-3 truncate" title={data()?.path}>
          {props.workspaceName} · {data()?.path.split(/[\\/]/).pop() ?? ''}
          {data() && !data()!.exists ? ` · ${t('harness.instruction.willCreate')}` : ''}
        </div>

        <Show
          when={data()}
          fallback={<div class="text-xs text-white/40">{t('common.loading')}</div>}
        >
          <textarea
            class="w-full rounded-lg bg-[rgba(0,0,0,0.2)] border border-[var(--border-dim)] px-3 py-2 text-xs text-white placeholder:text-white/25 outline-none focus:border-pri-50 transition-colors resize-y min-h-[260px] font-mono leading-relaxed"
            placeholder={t('harness.instruction.placeholder')}
            value={content()}
            onInput={(e) => setContent(e.currentTarget.value)}
          />
        </Show>

        <Show when={error()}>
          <div class="text-[11px] text-danger mt-2 break-all">{error()}</div>
        </Show>

        <div class="flex items-center justify-end gap-2 mt-4">
          <button class={btnSecondary} onClick={() => props.onClose()}>
            {t('common.cancel')}
          </button>
          <button class={btnPrimaryOutline} onClick={() => void save()}>
            {savedTick() ? t('common.saved') : t('common.save')}
          </button>
        </div>
      </div>
    </div>
  );
};

export default HarnessInstructionModal;

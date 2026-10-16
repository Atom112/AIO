/**
 * Add/edit custom ACP agent modal — registers any ACP-speaking CLI as a harness
 * tool (command + argv). Acrylic modal per AGENTS.md (dialog tokens).
 * The user authors the command line themselves (same trust model as MCP stdio).
 */
import { Component, createSignal, Show } from 'solid-js';
import Icon from '../../../shared/components/Icon';
import { btnClose, btnPrimaryOutline, btnSecondary } from '../../../shared/components/buttonStyles';
import { harnessSaveCustomAgent } from '../../../core/api/harness';
import { t } from '../../../core/i18n';
import type { HarnessCustomAgent } from '../../../core/api/harnessPayloads';

interface HarnessCustomAgentModalProps {
  /** Existing agent being edited (null = create). */
  agent?: HarnessCustomAgent | null;
  onClose: () => void;
  onSaved: (agent: HarnessCustomAgent) => void;
}

const HarnessCustomAgentModal: Component<HarnessCustomAgentModalProps> = (props) => {
  // Snapshot the editing target once at mount — the modal owns its draft state;
  // live-updating the fields from a mutating `agent` prop is not a use case.
  const initial = () => props.agent ?? null;
  const [name, setName] = createSignal(initial()?.name ?? '');
  const [command, setCommand] = createSignal(initial()?.command ?? '');
  const [args, setArgs] = createSignal((initial()?.args ?? []).join('\n'));
  const [error, setError] = createSignal('');
  const [busy, setBusy] = createSignal(false);

  const submit = async () => {
    const n = name().trim();
    const c = command().trim();
    if (!n || !c) {
      setError(t('harness.customAgent.required'));
      return;
    }
    setBusy(true);
    try {
      const saved = await harnessSaveCustomAgent({
        id: props.agent?.id ?? '',
        name: n,
        command: c,
        args: args()
          .split('\n')
          .map((l) => l.trim())
          .filter((l) => l.length > 0),
      });
      props.onSaved(saved);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const inputClass =
    'w-full rounded-lg bg-[rgba(0,0,0,0.2)] border border-[var(--border-dim)] px-3 py-1.5 text-xs text-white placeholder:text-white/25 outline-none focus:border-pri-50 transition-colors';

  return (
    <div
      class="fixed inset-0 z-[1000] bg-black/25 flex items-center justify-center"
      onClick={() => props.onClose()}
    >
      <div
        class="rounded-xl px-6 py-5 max-w-[420px] w-[90%] relative"
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

        <div class="text-sm font-medium text-white/85 mb-4">{t('harness.customAgent.title')}</div>

        <input
          class={`${inputClass} mb-2`}
          placeholder={t('harness.customAgent.namePlaceholder')}
          value={name()}
          onInput={(e) => setName(e.currentTarget.value)}
        />
        <input
          class={`${inputClass} mb-2`}
          placeholder={t('harness.customAgent.commandPlaceholder')}
          value={command()}
          onInput={(e) => setCommand(e.currentTarget.value)}
        />
        <textarea
          class={`${inputClass} resize-none mb-2`}
          rows={3}
          placeholder={t('harness.customAgent.argsPlaceholder')}
          value={args()}
          onInput={(e) => setArgs(e.currentTarget.value)}
        />

        <Show when={error()}>
          <div class="text-[11px] text-danger mb-3 break-all">{error()}</div>
        </Show>

        <div class="flex items-center justify-end gap-2">
          <button class={btnSecondary} onClick={() => props.onClose()}>
            {t('common.cancel')}
          </button>
          <button class={btnPrimaryOutline} disabled={busy()} onClick={() => void submit()}>
            {t('harness.customAgent.save')}
          </button>
        </div>
      </div>
    </div>
  );
};

export default HarnessCustomAgentModal;

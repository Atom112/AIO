/**
 * Harness approval bubble — the CLI agent paused on a permission request (ACP
 * `session/request_permission`). Renders the tool call (name / arguments /
 * diff preview) and the options the agent offered, then forwards the chosen
 * option id back through harness_respond_approval. Visual language mirrors
 * ToolApprovalBubble.
 */
import { Component, createSignal, For, Show } from 'solid-js';
import Icon from '../../../shared/components/Icon';
import DiffView from '../../../shared/components/DiffView';
import { harnessRespondApproval } from '../../../core/api/harness';
import { setHarnessApprovals, harnessApprovals } from '../../../core/store/harness';
import { t } from '../../../core/i18n';
import type { PendingHarnessApproval } from '../../../core/store/harness';

interface HarnessApprovalBubbleProps {
  approval: PendingHarnessApproval;
}

/** Option kind → [button class, label]. Unknown kinds render as neutral with the agent's label. */
const optionClass = (kind: string): string => {
  if (kind.startsWith('allow')) {
    return 'border-success/50 text-success hover:bg-success/10';
  }
  if (kind.startsWith('reject')) {
    return 'border-danger/50 text-danger hover:bg-danger/10';
  }
  return 'border-white/15 text-white/60 hover:bg-white/[0.08]';
};

const optionLabel = (kind: string, name: string): string => {
  switch (kind) {
    case 'allow_once':
      return t('harness.approval.allowOnce');
    case 'allow_always':
      return t('harness.approval.allowAlways');
    case 'reject_once':
      return t('harness.approval.rejectOnce');
    case 'reject_always':
      return t('harness.approval.rejectAlways');
    default:
      return name;
  }
};

const HarnessApprovalBubble: Component<HarnessApprovalBubbleProps> = (props) => {
  const [answering, setAnswering] = createSignal(false);

  const answer = async (optionId: string) => {
    if (answering()) return;
    setAnswering(true);
    const requestId = props.approval.requestId;
    // Remove optimistically; an expired event is a no-op then.
    setHarnessApprovals(harnessApprovals().filter((a) => a.requestId !== requestId));
    try {
      await harnessRespondApproval(requestId, optionId);
    } catch (e) {
      console.error('[harness] respond approval failed:', e);
    } finally {
      setAnswering(false);
    }
  };

  const argsText = () => {
    const raw = props.approval.arguments;
    if (raw == null) return null;
    if (typeof raw === 'string') return raw;
    try {
      return JSON.stringify(raw, null, 2);
    } catch {
      return String(raw);
    }
  };

  return (
    <div class="rounded-[10px] px-3.5 py-3 m-2 bg-[rgba(var(--surface-alt-bg),0.95)] border border-[rgba(var(--primary-rgb),0.15)] shadow-[0_4px_20px_rgba(0,0,0,0.3)]">
      {/* Header */}
      <div class="flex items-center gap-2 mb-2">
        <Icon name="alert-triangle" size={13} class="text-warn shrink-0" />
        <span class="text-[13px] font-medium text-white/85 truncate flex-1">
          {t('harness.approval.title')}
        </span>
        <Show when={props.approval.toolName}>
          <span class="shrink-0 text-[11px] px-1.5 py-0.5 rounded bg-[rgba(var(--text-base-rgb),0.08)] text-white/60 font-mono">
            {props.approval.toolName}
          </span>
        </Show>
      </div>

      {/* Title */}
      <Show when={props.approval.title}>
        <div class="text-xs text-white/70 mb-1.5 break-words">{props.approval.title}</div>
      </Show>

      {/* Arguments */}
      <Show when={argsText()}>
        <pre
          class="mb-1.5 max-h-[180px] overflow-auto rounded-lg bg-[rgba(0,0,0,0.2)] border border-[var(--border-dim)] p-2.5 text-[11px] leading-4 text-white/70 whitespace-pre-wrap break-all"
          style={{ 'font-family': 'ui-monospace,monospace' }}
        >
          {argsText()}
        </pre>
      </Show>

      {/* Diff preview */}
      <Show when={props.approval.previewDiff}>
        <div class="mb-1.5">
          <DiffView
            changes={[
              {
                filePath: props.approval.filePath ?? '',
                action: 'modify' as const,
                diff: props.approval.previewDiff!,
                summary: '',
              },
            ]}
          />
        </div>
      </Show>

      {/* Options */}
      <div class="flex items-center gap-2 flex-wrap mt-2">
        <For each={props.approval.options}>
          {(opt) => (
            <button
              class={`inline-flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium bg-white/[0.06] active:scale-95 transition-all duration-200 cursor-pointer border ${optionClass(opt.kind)}`}
              disabled={answering()}
              onClick={() => void answer(opt.optionId)}
            >
              {optionLabel(opt.kind, opt.name)}
            </button>
          )}
        </For>
      </div>
    </div>
  );
};

export default HarnessApprovalBubble;

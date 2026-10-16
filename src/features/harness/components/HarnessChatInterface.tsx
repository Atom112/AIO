/**
 * Harness conversation column: status bar (tool / workspace / run state / stop /
 * token counts), error strip, message list (history + live draft), pending
 * approval bubbles and the input bar. Visual language mirrors ChatInterface.
 */
import { Component, For, Show } from 'solid-js';
import Icon from '../../../shared/components/Icon';
import Dropdown from '../../../shared/components/Dropdown';
import {
  currentHarnessApprovals,
  currentHarnessDraft,
  currentHarnessRun,
  currentHarnessToolId,
  currentHarnessTopicId,
  currentHarnessTurnError,
  currentHarnessWorkspace,
  harnessClaudeMode,
  harnessMessages,
  harnessTopics,
  harnessTools,
  harnessTurnErrors,
  saveHarnessClaudeMode,
  setHarnessTurnErrors,
} from '../../../core/store/harness';
import { formatNumber, t } from '../../../core/i18n';
import { harnessSetPermissionMode } from '../../../core/api/harness';
import HarnessMessageList from './HarnessMessageList';
import HarnessApprovalBubble from './HarnessApprovalBubble';
import HarnessPlanStrip from './HarnessPlanStrip';

interface HarnessChatInterfaceProps {
  inputMessage: string;
  setInputMessage: (v: string) => void;
  isBusy: boolean;
  onSend: () => void;
  onStop: () => void;
  onOpenAddWorkspace: () => void;
}

const HarnessChatInterface: Component<HarnessChatInterfaceProps> = (props) => {
  const run = currentHarnessRun;
  const tool = () => harnessTools().find((x) => x.id === currentHarnessToolId());
  const workspace = currentHarnessWorkspace;
  const topic = () => harnessTopics().find((x) => x.id === currentHarnessTopicId());
  const hasTopic = () => !!currentHarnessTopicId();

  /** Run-state chip label. */
  const stateLabel = () => {
    switch (run().state) {
      case 'connecting':
        return t('harness.state.connecting');
      case 'running':
        return t('harness.state.running');
      case 'error':
        return t('harness.state.error');
      default:
        return t('harness.state.idle');
    }
  };

  /** Token/cost readout from the live draft usage (or the last assistant message). */
  const usage = () => currentHarnessDraft()?.usage;
  const lastAssistant = () => {
    const msgs = harnessMessages();
    for (let i = msgs.length - 1; i >= 0; i--) {
      if (msgs[i].role === 'assistant') return msgs[i];
    }
    return undefined;
  };
  const contextTokens = () => usage()?.contextTokens ?? lastAssistant()?.contextTokens;
  const inputTokens = () => usage()?.inputTokens ?? lastAssistant()?.inputTokens;
  const outputTokens = () => usage()?.outputTokens ?? lastAssistant()?.outputTokens;

  const handleInputKey = (e: KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      if (!props.isBusy) props.onSend();
    }
  };

  return (
    <div class="flex flex-col h-full min-w-0 rounded-xl overflow-hidden relative">
      {/* ===== Status bar ===== */}
      <div
        class="flex items-center gap-2 px-3 h-11 border-b shrink-0"
        style={{ 'border-color': 'var(--border-dim)' }}
      >
        <Icon name="terminal" size={13} class="text-white/60 shrink-0" />
        <span class="text-[13px] font-medium text-white/85 truncate max-w-[160px]">
          {tool()?.name ?? t('harness.chat.noTool')}
        </span>
        <Show when={workspace()}>
          <span class="text-white/25">/</span>
          <span class="text-xs text-white/50 truncate max-w-[180px]" title={workspace()!.path}>
            {workspace()!.name}
          </span>
        </Show>
        <Show when={topic()}>
          <span class="text-white/25">/</span>
          <span class="text-xs text-white/50 truncate flex-1">{topic()!.title}</span>
        </Show>

        <div class="ml-auto flex items-center gap-2 shrink-0">
          {/* Claude Code permission-mode selector (native adapter only) */}
          <Show when={tool()?.id === 'claude' && tool()?.adapter === 'claude-native'}>
            <Dropdown
              variant="ghost"
              class="w-[128px]"
              value={harnessClaudeMode()}
              onChange={(mode) => {
                saveHarnessClaudeMode(mode);
                const sid = currentHarnessTopicId();
                if (sid) void harnessSetPermissionMode(sid, mode).catch(console.error);
              }}
              options={[
                { value: 'default', label: t('harness.mode.default') },
                { value: 'acceptEdits', label: t('harness.mode.acceptEdits') },
                { value: 'plan', label: t('harness.mode.plan') },
                { value: 'dontAsk', label: t('harness.mode.dontAsk') },
                { value: 'bypassPermissions', label: t('harness.mode.bypassPermissions') },
              ]}
            />
          </Show>

          <Show when={contextTokens() || inputTokens() || outputTokens()}>
            <span
              class="text-[11px] text-white/35 whitespace-nowrap"
              style={{ 'font-family': 'ui-monospace,monospace' }}
            >
              <Show when={inputTokens()}>
                <span title={t('chat.inputTokens')}>
                  {'↗ ' + formatNumber(inputTokens()!)}
                </span>{' '}
              </Show>
              <Show when={outputTokens()}>
                <span title={t('chat.outputTokens')}>
                  {'↘ ' + formatNumber(outputTokens()!)}
                </span>{' '}
              </Show>
              <Show when={contextTokens()}>
                <span title={t('harness.usage.context')}>
                  {'◔ ' + formatNumber(contextTokens()!)}
                </span>
              </Show>
            </span>
          </Show>

          {/* Run state chip */}
          <Show
            when={run().state !== 'idle'}
            fallback={<span class="text-[11px] text-white/30">{stateLabel()}</span>}
          >
            <span
              class={`flex items-center gap-1.5 text-[11px] px-2 py-[3px] rounded-full border ${
                run().state === 'error'
                  ? 'border-danger/40 text-danger'
                  : run().state === 'running'
                    ? 'border-pri-40 text-pri'
                    : 'border-warn/40 text-warn'
              }`}
            >
              <Show when={run().state !== 'error'}>
                <Icon name="spinner" size={10} class="animate-spin" />
              </Show>
              {stateLabel()}
            </span>
          </Show>

          {/* Stop button */}
          <Show when={run().state === 'running' || run().state === 'connecting'}>
            <button
              class="flex items-center gap-1.5 px-2.5 py-1 rounded-md text-xs font-medium border border-danger/50 bg-white/[0.06] text-danger hover:bg-danger/10 active:scale-95 transition-all duration-200 cursor-pointer"
              onClick={() => props.onStop()}
            >
              <Icon name="stop" size={10} />
              {t('harness.chat.stop')}
            </button>
          </Show>
        </div>
      </div>

      {/* ===== Plan strip (agent task board) ===== */}
      <HarnessPlanStrip />

      {/* ===== Error strip ===== */}
      <Show when={currentHarnessTurnError()}>
        <div class="flex items-start gap-2 px-3 py-2 border-b text-xs text-danger bg-danger/[0.06]">
          <Icon name="alert-triangle" size={13} class="shrink-0 mt-[1px]" />
          <span class="break-all flex-1">{currentHarnessTurnError()}</span>
          <button
            class="shrink-0 text-white/30 hover:text-white/70 transition-colors"
            title={t('common.close')}
            onClick={() => {
              const sid = currentHarnessTopicId();
              if (!sid) return;
              const next = { ...harnessTurnErrors() };
              delete next[sid];
              setHarnessTurnErrors(next);
            }}
          >
            <Icon name="x" size={12} />
          </button>
        </div>
      </Show>

      {/* ===== Message list ===== */}
      <HarnessMessageList />

      {/* ===== Approvals + input ===== */}
      <div class="shrink-0 border-t" style={{ 'border-color': 'var(--border-dim)' }}>
        <For each={currentHarnessApprovals()}>
          {(approval) => <HarnessApprovalBubble approval={approval} />}
        </For>

        <Show
          when={hasTopic()}
          fallback={
            <div class="px-4 py-6 text-center text-xs text-white/35">
              {tool() ? t('harness.chat.noTopic') : t('harness.chat.selectTool')}
              <Show when={!tool() || !workspace()}>
                <div class="mt-2">
                  <button
                    class="text-xs text-pri hover:underline cursor-pointer"
                    onClick={() => props.onOpenAddWorkspace()}
                  >
                    {t('harness.chat.addWorkspaceHint')}
                  </button>
                </div>
              </Show>
            </div>
          }
        >
          <div class="flex items-end gap-2 px-3 py-2.5">
            <textarea
              class="flex-1 resize-none bg-white/[0.04] border border-white/[0.08] rounded-lg px-3 py-2 text-sm text-white placeholder:text-white/25 outline-none focus:border-pri-50 transition-colors max-h-[160px]"
              rows={1}
              placeholder={t('harness.chat.inputPlaceholder')}
              value={props.inputMessage}
              disabled={props.isBusy}
              onInput={(e) => props.setInputMessage(e.currentTarget.value)}
              onKeyDown={handleInputKey}
            />
            <Show
              when={!props.isBusy}
              fallback={
                <button
                  class="flex items-center justify-center w-[34px] h-[34px] rounded-lg border border-danger/50 bg-white/[0.06] text-danger hover:bg-danger/10 active:scale-95 transition-all duration-200 cursor-pointer"
                  onClick={() => props.onStop()}
                  title={t('harness.chat.stop')}
                >
                  <Icon name="stop" size={12} />
                </button>
              }
            >
              <button
                class="flex items-center justify-center w-[34px] h-[34px] rounded-lg bg-pri text-white hover:brightness-110 active:scale-95 transition-all duration-200 cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
                disabled={!props.inputMessage.trim()}
                onClick={() => props.onSend()}
                title={t('harness.chat.send')}
              >
                <Icon name="send" size={14} />
              </button>
            </Show>
          </div>
        </Show>
      </div>
    </div>
  );
};

export default HarnessChatInterface;

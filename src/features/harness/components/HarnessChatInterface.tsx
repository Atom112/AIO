/**
 * Harness conversation column — strict replica of the chat page's ChatInterface
 * column structure and visual language:
 * - Column root: rounded-[12px] p-[15px] pb-5 (the chat page's padded panel);
 * - Top action bar: single-line title (tool / workspace / topic);
 * - Bottom wrapper: the chat status-bar frame (rounded-2xl surface tint)
 *   hosting the plan strip, error row, approval bubbles and the status row
 *   (run-state chip + token readouts), followed by the chat-style input
 *   container (rounded-xl dark tint box) with the morphing send/stop button
 *   exactly like the chat page's input area;
 * - The Claude permission-mode dropdown sits in the input bottom-left slot
 *   (the chat page's AgentModeSelector position).
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
import { getHarnessLogo } from '../utils/harnessLogos';
import HarnessMessageList from './HarnessMessageList';
import HarnessApprovalBubble from './HarnessApprovalBubble';
import HarnessPlanStrip from './HarnessPlanStrip';
import HarnessConfigControls from './HarnessConfigControls';

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

  /** Title parts for the top bar: tool / workspace / topic. */
  const titleParts = () => {
    const parts: string[] = [];
    if (tool()) parts.push(tool()!.name);
    if (workspace()) parts.push(workspace()!.name);
    if (topic()) parts.push(topic()!.title);
    return parts;
  };

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

  /** Brand logo of the selected tool (custom tools → null → terminal icon). */
  const logoUrl = () => (tool() ? getHarnessLogo(tool()!.id) : null);

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
    if (e.key === 'Enter' && !e.ctrlKey && !e.shiftKey) {
      e.preventDefault();
      if (!props.isBusy) props.onSend();
    }
  };

  const dismissError = () => {
    const sid = currentHarnessTopicId();
    if (!sid) return;
    const next = { ...harnessTurnErrors() };
    delete next[sid];
    setHarnessTurnErrors(next);
  };

  return (
    <div class="flex flex-col flex-grow items-stretch rounded-[12px] box-border overflow-hidden p-[15px] pb-5 relative h-full min-w-0">
      {/* ===== Top action bar (chat page style: logo + single-line title) ===== */}
      <div class="flex items-center justify-between px-1 pb-3 shrink-0">
        <span class="flex items-center gap-1.5 min-w-0">
          <Show
            when={logoUrl()}
            fallback={
              <Icon
                name="terminal"
                size={14}
                class="shrink-0"
                style={{ color: 'rgba(var(--text-base-rgb),0.6)' }}
              />
            }
          >
            <img
              src={logoUrl()!}
              alt={tool()?.name ?? ''}
              class="w-4 h-4 shrink-0 object-contain rounded-[3px]"
            />
          </Show>
          <span
            class="text-sm font-medium truncate"
            style={{ color: 'rgba(var(--text-base-rgb),0.7)' }}
            title={titleParts().join(' / ')}
          >
            {titleParts().join(' / ') || t('harness.chat.noTool')}
          </span>
        </span>
      </div>

      {/* ===== Messages (flex-grow; the hint shows when no topic is selected) ===== */}
      <Show
        when={hasTopic()}
        fallback={
          <div class="flex-grow flex items-center justify-center min-h-0">
            <div class="text-center text-xs text-white/35 max-w-[420px] leading-5">
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
          </div>
        }
      >
        <HarnessMessageList />
      </Show>

      {/* ===== Bottom wrapper: status-bar frame + input (chat page structure) ===== */}
      <div class="bg-transparent flex flex-col relative w-full z-10">
        <Show when={hasTopic()}>
          {/* Status bar frame (chat page's rounded-2xl surface tint) */}
          <div
            class="flex flex-col rounded-2xl text-xs"
            style={{
              background: 'rgba(var(--surface-bg), 0.5)',
              border: '1px solid rgba(var(--border-rgb), 0.15)',
            }}
          >
            {/* Plan strip (agent task board) */}
            <HarnessPlanStrip />

            {/* Error row (dismissable) */}
            <Show when={currentHarnessTurnError()}>
              <div
                class="flex items-start gap-2 px-3 py-1.5 border-b text-[11px]"
                style={{
                  'border-color': 'rgba(var(--border-rgb), 0.15)',
                  color: 'var(--color-danger)',
                }}
              >
                <Icon name="alert-triangle" size={12} class="shrink-0 mt-[1px]" />
                <span class="break-all flex-1">{currentHarnessTurnError()}</span>
                <button
                  class="shrink-0 text-white/30 hover:text-white/70 transition-colors cursor-pointer border-none bg-transparent p-0"
                  title={t('common.close')}
                  onClick={dismissError}
                >
                  <Icon name="x" size={11} />
                </button>
              </div>
            </Show>

            {/* Approval bubbles (waiting on the user) */}
            <For each={currentHarnessApprovals()}>
              {(approval) => <HarnessApprovalBubble approval={approval} />}
            </For>

            {/* Status row (chat page's px-3 h-8) */}
            <div class="flex items-center gap-2 px-3 h-8">
              {/* Run-state chip */}
              <Show
                when={run().state !== 'idle'}
                fallback={
                  <span class="text-[11px]" style={{ color: 'rgba(var(--text-base-rgb),0.35)' }}>
                    {stateLabel()}
                  </span>
                }
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

              <div class="ml-auto flex items-center gap-2 shrink-0">
                {/* Token readouts */}
                <Show when={contextTokens() || inputTokens() || outputTokens()}>
                  <span
                    class="text-[11px] whitespace-nowrap"
                    style={{
                      color: 'rgba(var(--text-base-rgb),0.4)',
                      'font-family': 'ui-monospace,monospace',
                    }}
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
              </div>
            </div>
          </div>

          {/* Input container (chat page's rounded-xl dark tint box) */}
          <div
            class="rounded-xl box-border flex flex-col gap-[10px] mt-[3px] p-[10px] transition-all duration-200 w-full"
            style={{ background: 'rgba(0,0,0,0.25)', border: '1px solid var(--border-dim)' }}
          >
            <textarea
              class="bg-transparent border-none text-white font-inherit text-base leading-relaxed min-h-[40px] max-h-[20vh] outline-none overflow-y-hidden px-[5px] pb-[5px] resize-none w-full focus:overflow-y-auto"
              placeholder={t('harness.chat.inputPlaceholder')}
              value={props.inputMessage}
              onInput={(e) => props.setInputMessage(e.currentTarget.value)}
              onKeyDown={handleInputKey}
            />

            <div
              class="flex items-center justify-between border-t pt-2"
              style={{ 'border-color': 'var(--border-dim)' }}
            >
              <div class="flex items-center gap-2">
                {/* Session config controls (model / thinking level) — the
                    AgentModeSelector slot; rendered from the agent-reported
                    option set, so controls appear only when the tool exposes
                    them (ACP agents) or is a native adapter (claude/codex). */}
                <HarnessConfigControls isBusy={props.isBusy} />
                {/* Claude permission mode */}
                <Show when={tool()?.id === 'claude' && tool()?.adapter === 'claude-native'}>
                  <Dropdown
                    variant="ghost"
                    class="w-[128px]"
                    menuUp
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
              </div>

              <div class="flex items-center gap-2">
                {/* Send / stop: the chat page's morphing control */}
                <button
                  class={`flex items-center justify-center border-none rounded-lg cursor-pointer h-8 w-8 transition-all duration-100 hover:opacity-90 hover:scale-105 active:scale-95 ${
                    props.isBusy ? 'bg-[var(--color-danger)] text-white' : 'text-white'
                  }`}
                  style={!props.isBusy ? { background: 'rgba(var(--primary-rgb),0.3)' } : {}}
                  onClick={() => (props.isBusy ? props.onStop() : props.onSend())}
                  title={props.isBusy ? t('harness.chat.stop') : t('harness.chat.send')}
                >
                  <Show
                    when={props.isBusy}
                    fallback={<Icon src="/icons/app-logo/send.svg" class="w-[18px] h-[18px]" />}
                  >
                    <Icon src="/icons/app-logo/stop-square.svg" class="w-[18px] h-[18px]" />
                  </Show>
                </button>
              </div>
            </div>
          </div>
        </Show>
      </div>
    </div>
  );
};

export default HarnessChatInterface;

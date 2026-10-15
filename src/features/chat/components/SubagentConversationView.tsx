/**
 * Standalone conversation view for a subagent.
 *
 * Entered from the pull-up menu at the right of the status bar (or the compact card in the main agent timeline); replaces the whole conversation column.
 * The message stream reuses the main topic's UI styles:
 * - Task issued by the main agent -> right user bubble (primary background, same as main topic user messages)
 * - Each subagent turn narration + its tool executions -> left assistant bubble (avatar + text background, same as main topic assistant messages)
 * - Final report after completion -> last left assistant bubble (full Markdown)
 * - Header: back button top-left + subagent identity and status + stop button
 *
 * Data source: type='subagent' steps in agentSteps of the parent topic's assistant messages
 * (subagentSteps / subagentResult persist with the message and remain viewable after restart).
 */
import {
  Component,
  createSignal,
  createMemo,
  createEffect,
  Show,
  For,
  Index,
  onCleanup,
  type JSX,
} from 'solid-js';
import Icon from '../../../shared/components/Icon';
import Markdown from '../../../shared/components/Markdown';
import ThinkBlock from '../../../shared/components/ThinkBlock';
import { t } from '../../../core/i18n';
import {
  activeSubagentView,
  setActiveSubagentView,
  findSubagentStep,
  type SubagentStep,
  type Topic,
} from '../../../core/store/store';
import { stopSubagent } from '../../../core/api/chat';
import { btnGhost, btnIcon } from '../../../shared/components/buttonStyles';
import { getSubagentProfileStyle } from '../../../core/utils/subagentProfileStyle';

// ---- Profile configuration (shared by the compact card and the pull-up menu) ----
// The style registry itself lives in core/utils/subagentProfileStyle.ts (shared with
// the settings page rows); this module keeps the i18n label + duration formatters.

/** profile id -> display name (i18n) */
export function subagentProfileLabel(profileId?: string): string {
  switch (profileId) {
    case 'explorer':
      return t('agent.profile.explorer');
    case 'coder':
      return t('agent.profile.coder');
    case 'general':
      return t('agent.profile.general');
    case 'requirements':
      return t('agent.profile.requirements');
    case 'vision':
      return t('agent.profile.vision');
    default:
      return t('agent.profile.subagent');
  }
}

/** milliseconds -> duration text */
export function formatSubagentDuration(ms: number): string {
  if (ms < 50) return '0.1s';
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds.toFixed(1)}s`;
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  return t('agent.step.duration', { mins, secs });
}

// ---- Tool execution row inside bubbles (visually matches the tool cards of the main topic's AgentProcessBlock) ----
const ToolRow: Component<{ step: SubagentStep }> = (props) => (
  <div
    class="flex items-start gap-1.5 px-2 py-1 rounded-md text-[11px]"
    style={{
      background: 'rgba(var(--text-base-rgb),0.03)',
      'border-left': '2px solid rgba(240,160,64,0.55)',
    }}
  >
    <Icon name="wrench" size={11} class="mt-[2px] shrink-0 opacity-70" />
    <div class="flex-1 min-w-0">
      <span class="font-medium text-white/60 break-all">{props.step.toolName}</span>
      <Show when={props.step.summary}>
        <div class="font-mono text-[10px] text-white/40 leading-relaxed break-words mt-0.5">
          {props.step.summary}
        </div>
      </Show>
    </div>
  </div>
);

/** One conversation bubble: a subagent turn's thinking + narration text + tools executed within that turn */
interface SubagentTurn {
  /** Owning LLM round (undefined for legacy persisted data without the round field) */
  round?: number;
  /** Streamed reasoning (chain of thought), rendered as a collapsible ThinkBlock */
  thinking?: SubagentStep;
  text: string;
  tools: SubagentStep[];
}

interface SubagentConversationViewProps {
  /** The main agent's current topic (subagent steps live in agentSteps of its assistant messages) */
  topic: Topic | null;
}

const SubagentConversationView: Component<SubagentConversationViewProps> = (props) => {
  const step = createMemo(() => findSubagentStep(props.topic, activeSubagentView()));

  // Fallback: auto-return to the main conversation when the message is deleted or topic data disappears
  createEffect(() => {
    if (activeSubagentView() && !step()) setActiveSubagentView(null);
  });

  const profileStyle = () => getSubagentProfileStyle(step()?.subagentProfile);
  const displayName = () => step()?.subagentName || subagentProfileLabel(step()?.subagentProfile);
  const taskText = () => step()?.subagentTask || '';
  const resultText = () => step()?.subagentResult || '';
  const isRunning = () => step()?.status === 'running';
  const isError = () => step()?.status === 'error';
  const isComplete = () => step()?.status === 'complete';

  // Live timer (ticks while running; freezes to duration when complete)
  const [elapsedMs, setElapsedMs] = createSignal(0);
  createEffect(() => {
    const s = step();
    if (s && s.status === 'running' && s.timestamp) {
      const intervalId = setInterval(() => {
        if (step()?.status !== 'running') {
          clearInterval(intervalId);
          return;
        }
        setElapsedMs(Date.now() - s.timestamp);
      }, 100);
      onCleanup(() => clearInterval(intervalId));
    } else if (s && s.timestamp) {
      setElapsedMs((s.duration ?? 0) || Date.now() - s.timestamp);
    }
  });

  // Message-stream grouping (harness round 11, live-streaming aware):
  // NEW data (steps carry `round`): each round = one turn { thinking?, text, tools[] } —
  //   thinking streams as a collapsible reasoning block, content as live narration text,
  //   the round-end tool_call summary is deduped against the streamed content.
  // LEGACY data (no round field): the pre-streaming shape — a no-toolName tool_call step
  //   opens a narration bubble; thinking/content/streaming steps are not rendered.
  const turns = createMemo<SubagentTurn[]>(() => {
    const steps = step()?.subagentSteps || [];
    const out: SubagentTurn[] = [];
    let cur: SubagentTurn | null = null;
    const newTurn = (round?: number): SubagentTurn => {
      cur = { round, thinking: undefined, text: '', tools: [] };
      out.push(cur);
      return cur;
    };
    const ensureForRound = (round: number): SubagentTurn =>
      cur && cur.round === round ? cur : newTurn(round);
    const legacy = steps.every((s) => s.round === undefined);
    for (const s of steps) {
      if (legacy) {
        if (s.type === 'content' || s.type === 'streaming' || s.type === 'thinking') continue;
        if (!s.toolName) {
          newTurn(undefined);
          cur!.text = s.summary === '[工具调用]' ? '' : s.summary || '';
        } else {
          if (!cur) newTurn(undefined);
          cur!.tools.push(s);
        }
        continue;
      }
      // Round-aware path (live-streamed subagents)
      const r = s.round;
      if (s.type === 'streaming') continue; // heartbeats / giant-arg progress
      if (s.type === 'thinking') {
        ensureForRound(r!).thinking = s;
      } else if (s.type === 'content') {
        const turn = ensureForRound(r!);
        turn.text = (turn.text || '') + (s.summary || '');
      } else if (s.toolName) {
        if (!cur) newTurn(r);
        cur!.tools.push(s);
      } else {
        // Round-end narration summary: dedupe against the streamed text of the same round
        const curTurn = cur as SubagentTurn | null; // closures reassign cur; TS CFA can't track
        const dup = !!curTurn && curTurn.round === r && !!curTurn.text;
        if (dup) continue;
        newTurn(r);
        cur!.text = s.summary === '[工具调用]' ? '' : s.summary || '';
      }
    }
    return out;
  });

  // Stop a background subagent individually (only fire-and-forget ones are findable in the registry;
  // awaited subagents stop together with the whole turn, so clicking here is an idempotent no-op)
  const [stopping, setStopping] = createSignal(false);
  const handleStop = async () => {
    const id = step()?.subagentId;
    if (!id || stopping()) return;
    setStopping(true);
    try {
      await stopSubagent(id);
    } finally {
      setStopping(false);
    }
  };

  // ---- Auto-scroll (reuses the main conversation pattern) ----
  let bodyRef: HTMLDivElement | undefined;
  // Content wrapper layer: the element the stream-following ResizeObserver observes (bodyRef is a flex-1 fixed-height
  // scroll box; observing it fires no RO on content growth, while the wrapper's height truly grows with the bubble content).
  let contentRef: HTMLDivElement | undefined;
  const [autoScroll, setAutoScroll] = createSignal(true);
  let userScrolledUp = false;
  let suppressScroll = false;

  const isAtBottom = () => {
    if (!bodyRef) return true;
    const el = bodyRef;
    return el.scrollHeight - el.scrollTop - el.clientHeight < 50;
  };

  const snapToBottom = () => {
    if (!bodyRef) return;
    const el = bodyRef;
    const target = Math.max(0, el.scrollHeight - el.clientHeight);
    // Only suppress the echoed scroll event when the position actually changes: when already at the bottom (the common
    // stream-following case) no event fires, and leaving suppressScroll set would swallow the user's next scroll event
    if (el.scrollTop < target) {
      suppressScroll = true;
      el.scrollTop = target;
    }
  };

  const handleWheel = (e: WheelEvent) => {
    if (e.deltaY < 0) {
      userScrolledUp = true;
      suppressScroll = false;
      setAutoScroll(false);
    }
  };

  const handleScroll = () => {
    if (suppressScroll) {
      suppressScroll = false;
      return;
    }
    const atBottom = isAtBottom();
    setAutoScroll(atBottom);
    if (atBottom) userScrolledUp = false;
  };

  createEffect(() => {
    if (contentRef) {
      const observer = new ResizeObserver(() => {
        if (!userScrolledUp && autoScroll()) snapToBottom();
      });
      observer.observe(contentRef);
      if (bodyRef) observer.observe(bodyRef);
      onCleanup(() => observer.disconnect());
    }
  });

  // Snap to bottom when entering the view
  createEffect(() => {
    if (step() && bodyRef) {
      queueMicrotask(() => {
        if (!userScrolledUp) snapToBottom();
      });
    }
  });

  // ---- Bubble rendering helpers (reuse the main topic assistant message styles) ----
  const avatar = () => {
    const ps = profileStyle();
    return (
      <div
        class="flex flex-shrink-0 items-center justify-center w-9 h-9 rounded-full"
        style={{
          background: ps.bgColor,
          border: `1px solid ${ps.borderColor}`,
          'box-shadow': '0 2px 6px rgba(0,0,0,0.25)',
        }}
      >
        <Icon name={ps.icon} size={15} style={{ color: ps.color }} />
      </div>
    );
  };

  /** Left assistant bubble row (avatar + text-background bubble, same style as main topic assistant messages) */
  const agentRow = (children: JSX.Element) => (
    <div class="flex items-start gap-2 mb-3">
      {avatar()}
      <div class="flex flex-col items-start max-w-[75%]">
        <div
          class="rounded-[10px] rounded-tl-[2px] leading-relaxed max-w-full px-[14px] py-[10px] break-words text-white"
          style={{
            background: 'rgba(var(--text-base-rgb),0.04)',
            border: '1px solid rgba(var(--text-base-rgb),0.13)',
            'backdrop-filter': 'blur(6px)',
          }}
        >
          {children}
        </div>
      </div>
    </div>
  );

  return (
    <div class="flex flex-col flex-grow items-stretch rounded-[12px] box-border overflow-hidden p-[15px] pb-5 relative h-full">
      {/* Header: back button top-left + subagent identity + stop */}
      <div
        class="flex items-center gap-2 px-3 h-10 shrink-0 border-b"
        style={{ 'border-color': 'var(--border-dim)' }}
      >
        <button
          type="button"
          class={btnGhost + ' px-2 py-1 rounded-md text-xs shrink-0'}
          onClick={() => setActiveSubagentView(null)}
        >
          <Icon name="arrow-left" size={13} />
          <span class="ml-1">{t('agent.subagent.back')}</span>
        </button>

        <span class="w-px h-4 shrink-0" style={{ background: 'rgba(var(--text-base-rgb),0.12)' }} />

        <div class="flex items-center gap-2 min-w-0 flex-1">
          <span
            class="flex items-center justify-center w-5 h-5 rounded shrink-0"
            style={{
              background: profileStyle().bgColor,
              border: `1px solid ${profileStyle().borderColor}`,
              color: profileStyle().color,
            }}
          >
            <Icon
              name={profileStyle().icon}
              size={12}
              classList={{ 'animate-spin': isRunning() }}
            />
          </span>
          <span class="text-[13px] font-semibold text-white/85 truncate shrink-0">
            {displayName()}
          </span>
          <span
            class="text-[10px] px-1.5 py-0.5 rounded font-medium shrink-0"
            style={{
              color: profileStyle().color,
              background: 'rgba(var(--text-base-rgb),0.05)',
              border: `1px solid ${profileStyle().borderColor}`,
            }}
          >
            {subagentProfileLabel(step()?.subagentProfile)}
          </span>
          <Show when={step()}>
            <span class="font-mono text-[10px] text-white/25 shrink-0">
              {formatSubagentDuration(elapsedMs())}
            </span>
          </Show>
          <Show when={isRunning()}>
            <span
              class="w-1.5 h-1.5 rounded-full animate-pulse shrink-0"
              style={{ background: profileStyle().color }}
            />
          </Show>
          <Show when={isError()}>
            <Icon name="x" size={11} class="text-red-400/70 shrink-0" />
          </Show>
        </div>

        <Show when={isRunning() && step()?.subagentId}>
          <button
            type="button"
            class={btnIcon + ' w-6 h-6 shrink-0 text-white/40 hover:text-red-400/80'}
            title={t('agent.subagent.stop')}
            aria-label={t('agent.subagent.stop')}
            onClick={() => void handleStop()}
          >
            <Icon name="stop" size={10} classList={{ 'animate-pulse': stopping() }} />
          </button>
        </Show>
      </div>

      {/* Message stream: same layout as the main topic (right user bubbles + left assistant bubbles) */}
      <div
        ref={bodyRef}
        class="flex-1 overflow-y-auto min-h-0"
        onScroll={handleScroll}
        onWheel={handleWheel}
      >
        <div ref={contentRef} class="pt-3 pb-[15px]">
          <Show when={step()}>
            {/* Task issued by the main agent: right user bubble */}
            <Show when={taskText()}>
              <div class="flex items-start gap-2 mb-3 justify-end pr-[10px]">
                <div class="flex flex-col items-end max-w-[75%]">
                  <div
                    class="rounded-[10px] rounded-tr-[2px] leading-relaxed max-w-full px-[14px] py-[10px] break-words text-white"
                    style={{
                      background: 'rgba(var(--primary-rgb),0.08)',
                      border: '1px solid rgba(var(--primary-rgb),0.16)',
                      'backdrop-filter': 'blur(6px)',
                    }}
                  >
                    <div class="mt-1">
                      <Markdown content={taskText()} />
                    </div>
                  </div>
                </div>
              </div>
            </Show>

            {/* Subagent per-turn thinking + narration + tool executions: left assistant bubble.
                <Index> (position-keyed), not <For>: the turns memo rebuilds turn OBJECTS on every
                streamed delta, and <For> compares by reference — it would destroy/recreate every
                bubble per token, replaying ThinkBlock's entrance animation (the "flicker") and
                resetting its inner scroll. Index keeps each row's DOM alive across rebuilds. */}
            <Index each={turns()}>
              {(turn) =>
                agentRow(
                  <>
                    <Show
                      when={turn().text || turn().tools.length > 0 || turn().thinking}
                      fallback={
                        <div class="flex items-center gap-2 py-1 text-white/50 italic text-[14px] select-none">
                          <Icon name="spinner" size={14} class="animate-spin opacity-50" />
                          <span class="animate-pulse">{t('agent.sub.working')}</span>
                        </div>
                      }
                    >
                      {/* Streamed chain of thought (live while running, collapsible after) */}
                      <Show when={turn().thinking?.summary}>
                        <ThinkBlock
                          content={turn().thinking!.summary || ''}
                          isStreaming={turn().thinking?.status === 'running'}
                        />
                      </Show>
                      <Show when={turn().text}>
                        <div class="mt-1 text-[13px]">
                          <Markdown content={turn().text} />
                        </div>
                      </Show>
                      <Show when={turn().tools.length > 0}>
                        <div
                          class="flex flex-col gap-1"
                          classList={{ 'mt-2': !!turn().text || !!turn().thinking?.summary }}
                        >
                          <For each={turn().tools}>{(tool) => <ToolRow step={tool} />}</For>
                        </div>
                      </Show>
                    </Show>
                  </>,
                )
              }
            </Index>

            {/* Working: trailing bubble (executing tools / waiting for the next turn's output) */}
            <Show when={isRunning()}>
              {agentRow(
                <div class="flex items-center gap-2 py-1 text-white/50 italic text-[14px] select-none">
                  <Icon name="spinner" size={14} class="animate-spin opacity-50" />
                  <span class="animate-pulse">{t('agent.sub.working')}</span>
                </div>,
              )}
            </Show>

            {/* Error: red-text assistant bubble */}
            <Show when={isError()}>
              {agentRow(
                <div
                  class="text-[13px] leading-relaxed break-words"
                  style={{ color: 'rgba(224,85,85,0.75)' }}
                >
                  {resultText() || t('agent.sub.error')}
                </div>,
              )}
            </Show>

            {/* Final report after completion: last assistant bubble (full Markdown) */}
            <Show when={isComplete() && resultText()}>
              {agentRow(
                <div class="mt-1 text-[13px] text-white/85">
                  <Markdown content={resultText()} />
                </div>,
              )}
            </Show>
          </Show>
        </div>
      </div>
    </div>
  );
};

export default SubagentConversationView;

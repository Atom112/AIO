/**
 * Collapsible block for the agent work process.
 *
 * Two render modes:
 * - New mode (with agentSteps): timeline cards, one per step (thinking / tool call / interim summary)
 * - Old mode (no agentSteps): legacy-data fallback, shows the reasoning + interimContent + toolCalls blocks
 *
 * Interaction:
 * - Running steps: auto-expanded with a pulsing left border
 * - Completed steps: collapsed by default (header only), click to expand individually
 * - Outer toggle button: collapse / expand the whole step list
 */
import DiffView from '../../../shared/components/DiffView';
import {
  Component,
  createSignal,
  createMemo,
  Show,
  For,
  onCleanup,
  createEffect,
  onMount,
  untrack,
} from 'solid-js';
import type { AgentStep, ToolCallDisplay } from '../../../core/store/store';
import { setActiveSubagentView } from '../../../core/store/store';
import Icon, { type IconName } from '../../../shared/components/Icon';
import ToolCallBubble from './ToolCallBubble';
import Markdown from '../../../shared/components/Markdown';
import { getSubagentProfileStyle, subagentProfileLabel } from './SubagentConversationView';
import { stopSubagent } from '../../../core/api/chat';
import { t } from '../../../core/i18n';

interface AgentProcessBlockProps {
  /** Native chain-of-thought text from the model (old mode) */
  reasoning?: string;
  /** Tool calls produced during this round (old mode) */
  toolCalls?: ToolCallDisplay[];
  /** Accumulated interim summary text from intermediate rounds (old mode) */
  interimContent?: string;
  /** Whether the agent is still working */
  isActive: boolean;
  /** Agent round start timestamp (ms) */
  startTime?: number;
  /** Agent work-process timeline (new mode) */
  agentSteps?: AgentStep[];
}

// ---- Step type configuration ----
const STEP_CONFIG: Record<AgentStep['type'], { icon: IconName; borderColor: string }> = {
  thinking: { icon: 'brain', borderColor: 'rgba(var(--primary-rgb),0.55)' },
  tool_call: { icon: 'wrench', borderColor: 'rgba(240,160,64,0.55)' },
  content: { icon: 'document', borderColor: 'rgba(156,163,175,0.35)' },
  subagent: { icon: 'sparkles', borderColor: 'rgba(140,120,220,0.55)' },
};

function stepTypeLabel(type: AgentStep['type']): string {
  switch (type) {
    case 'thinking':
      return t('agent.step.thinking');
    case 'tool_call':
      return t('agent.step.toolCall');
    case 'content':
      return t('agent.step.content');
    case 'subagent':
      return t('agent.step.subagent');
  }
}

// Step IDs that have already played their entrance animation; prevents replaying when the agentSteps array is rebuilt
const animatedStepIds = new Set<string>();

// Render-buffer state (indexed by round startTime).
// Streaming updates to agentSteps create new message object references; the parent <For> compares by
// reference and destroys/recreates AgentProcessBlock — every tool event triggers a remount. So render
// progress, the pending-dequeue queue and the dequeue timer must live at module level; otherwise queue
// state is lost on remount and new steps render immediately (bypassing the buffer).
interface RoundRenderQueue {
  /** Step ids already dequeued (rendered) */
  renderedIds: Set<string>;
  /** Step ids waiting to be dequeued (arrival order preserved) */
  pendingIds: string[];
  /** Dequeue timer; non-null means the queue is dequeuing one by one */
  dequeueTimer: number | null;
}
const perRoundQueue = new Map<string, RoundRenderQueue>();

// Dequeue-notification subscribers (indexed by round): the component may remount while the dequeue
// timer is alive; the timer notifies only the active instance via this Map, never destroyed ones.
const queueSubscribers = new Map<string, (id: string) => void>();

function getRoundQueue(roundKey: string): RoundRenderQueue {
  let q = perRoundQueue.get(roundKey);
  if (!q) {
    q = { renderedIds: new Set(), pendingIds: [], dequeueTimer: null };
    perRoundQueue.set(roundKey, q);
  }
  return q;
}

function getStepBorderColor(step: AgentStep): string {
  if (step.type === 'subagent') return STEP_CONFIG[step.type].borderColor;
  if (step.type !== 'tool_call') return STEP_CONFIG[step.type].borderColor;
  if (step.status === 'running') return 'rgba(240,160,64,0.55)';
  if (step.status === 'error') return 'rgba(224,85,85,0.55)';
  return 'rgba(76,175,144,0.55)';
}
function formatDuration(ms: number): string {
  if (ms < 50) return '0.1s';
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds.toFixed(1)}s`;
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  return t('agent.step.duration', { mins, secs });
}

function stepToolLabel(step: AgentStep): string {
  if (step.type === 'subagent') return step.subagentProfile || 'subagent';
  const name = step.toolCall?.function?.name || 'unknown';
  // Truncate overly long tool names
  return name.length > 28 ? name.slice(0, 26) + '…' : name;
}

function stepStatusIcon(step: AgentStep): IconName | null {
  if (step.type === 'subagent')
    return step.status === 'running' ? 'spinner' : step.status === 'error' ? 'x' : 'check';
  if (step.type !== 'tool_call') return null;
  if (step.status === 'running') return 'spinner';
  if (step.status === 'error') return 'x';
  return 'check';
}

// ---- Single step card component ----
const StepCard: Component<{
  step: AgentStep;
  isActive: boolean;
  expanded: boolean;
  onToggle: () => void;
}> = (props) => {
  const cfg = createMemo(() => STEP_CONFIG[props.step.type]);
  const durationMs = createMemo(
    () =>
      props.step.duration ??
      (props.step.status === 'running' ? Date.now() - props.step.timestamp : undefined),
  );

  // Entrance-animation dedupe: apply the animation class only on the step's first render
  const stepId = untrack(() => props.step.id);
  const isNew = !animatedStepIds.has(stepId);
  if (isNew) {
    animatedStepIds.add(stepId);
  }

  // The entrance-animation class is present from the first frame (animation "backwards" starts at
  // the initial keyframe) and is removed afterwards so is-running stepCardPulse wins (shorthand
  // animation properties override each other).
  let entranceRef: HTMLDivElement | undefined;

  // Round-11 feedback: the running thinking step's body is a capped scroll box but had
  // no follow logic — streamed reasoning grew below the fold. Follow the bottom while
  // the step runs; a wheel-up pauses, scrolling back to the bottom resumes.
  const thinkingBodyRef: { current: HTMLDivElement | undefined } = { current: undefined };
  let thinkingFollowPaused = false;
  createEffect(() => {
    const step = props.step;
    const thinkingText = step.thinkingText; // track reasoning growth
    const el = thinkingBodyRef.current;
    void thinkingText;
    if (step.status === 'running' && step.type === 'thinking' && el && !thinkingFollowPaused) {
      el.scrollTop = el.scrollHeight;
    }
  });

  // Lock the expanded state during the entrance animation: a fast running → complete flip would
  // collapse the body height and visually squash the animation; real expanded returns once it ends.
  const [entranceDone, setEntranceDone] = createSignal(!isNew);
  const effectiveExpanded = () => !entranceDone() || props.expanded;

  onMount(() => {
    if (!isNew || !entranceRef) return;
    // Total entrance duration ~0.9s (0.1s delay + 0.8s expand-width); unlock expanded afterwards
    const timer = setTimeout(() => {
      setEntranceDone(true);
    }, 920);
    onCleanup(() => clearTimeout(timer));
  });

  // Stop a background subagent individually (only fire-and-forget ones are found in the registry;
  // awaited subagents stop with the whole round, so this click is an idempotent no-op)
  const [stopping, setStopping] = createSignal(false);
  const handleStopSubagent = async () => {
    const id = props.step.subagentId;
    if (!id || stopping()) return;
    setStopping(true);
    try {
      await stopSubagent(id);
    } finally {
      setStopping(false);
    }
  };

  // Harness round 11 (live subagent progress): the running subagent's latest streamed
  // sub-step — a streaming narration tail, a "write_file: 12KB" argument-growth line, or
  // the newest tool row. Keeps the timeline card informative during multi-minute subagents.
  const latestSubagentProgress = createMemo(() => {
    if (props.step.type !== 'subagent' || props.step.status !== 'running') return undefined;
    const steps = props.step.subagentSteps || [];
    for (let i = steps.length - 1; i >= 0; i--) {
      const s = steps[i];
      const text =
        s.type === 'streaming'
          ? s.summary
          : s.type === 'content'
            ? tailOf(s.summary || '', 60)
            : s.toolName
              ? `${s.toolName} ✓`
              : undefined;
      if (text) return text;
    }
    return undefined;
  });
  const tailOf = (text: string, max: number) =>
    text.length > max ? `…${text.slice(text.length - max)}` : text;

  return (
    <div
      ref={entranceRef}
      class="rounded-md overflow-hidden bg-white/[0.015] border border-white/[0.04] hover:bg-white/[0.03]"
      classList={{
        'is-running': props.step.status === 'running',
        'is-error': props.step.status === 'error',
        'is-expanded': effectiveExpanded(),
      }}
      style={{
        '--step-border-color': getStepBorderColor(props.step),
        'border-left': `2px solid var(--step-border-color, var(--border-dim))`,
        transition: 'border-color 0.3s, background 0.2s',
        ...(props.step.status === 'running'
          ? {
              background: 'rgba(var(--primary-rgb), 0.03)',
              animation: 'stepCardPulse 2s ease-in-out infinite',
            }
          : {}),
        ...(props.step.status === 'error' ? { 'border-left-color': 'rgba(224, 85, 85, 0.5)' } : {}),
        ...(isNew && !entranceDone()
          ? {
              animation:
                'stepSlideIn 0.6s cubic-bezier(0.22, 0.61, 0.36, 1) backwards, expand-width 0.8s cubic-bezier(0.25, 0.46, 0.45, 0.94) 0.1s backwards',
            }
          : {}),
      }}
    >
      <div class="flex items-center w-full">
        <button
          type="button"
          class="flex items-center flex-1 min-w-0 gap-1.5 px-2.5 py-1.5 cursor-pointer select-none bg-transparent border-none text-white/45 text-[11px] transition-colors duration-200 hover:text-white/70"
          onClick={() => {
            if (props.step.type === 'subagent') {
              // Subagent: the whole card is the entry point; clicking opens its standalone conversation view (inner steps are never expanded)
              if (props.step.subagentId) setActiveSubagentView(props.step.subagentId);
            } else {
              props.onToggle();
            }
          }}
          disabled={props.step.status === 'running' && props.step.type !== 'subagent'}
        >
          <span class="flex items-center justify-center w-4 h-4 rounded bg-white/[0.05] shrink-0">
            <Icon
              name={cfg().icon}
              size={12}
              class="w-3 h-3"
              classList={{ 'is-spinning': props.step.status === 'running' }}
              style={
                props.step.type === 'subagent'
                  ? { color: getSubagentProfileStyle(props.step.subagentProfile).color }
                  : undefined
              }
            />
          </span>
          <span class="font-medium flex-none">{stepTypeLabel(props.step.type)}</span>
          {props.step.type === 'tool_call' && (
            <span class="truncate text-white/55 flex-initial min-w-0">
              · {stepToolLabel(props.step)}
            </span>
          )}
          {props.step.type === 'subagent' && (
            <span class="truncate text-white/55 flex-initial min-w-0">
              · {subagentProfileLabel(props.step.subagentProfile)}
            </span>
          )}
          {props.step.type === 'subagent' && props.step.subagentTask && (
            <span class="truncate text-white/30 flex-1 min-w-0">{props.step.subagentTask}</span>
          )}
          {/* Live progress line (harness round 11): latest streamed sub-step of a running
              subagent — narration tail / tool-arg growth, so a multi-minute subagent no
              longer shows as a silent "thinking" card. */}
          {props.step.type === 'subagent' &&
            props.step.status === 'running' &&
            latestSubagentProgress() && (
              <span class="truncate text-white/40 flex-1 min-w-0 tabular-nums">
                {latestSubagentProgress()}
              </span>
            )}
          {(props.step.type === 'tool_call' || props.step.type === 'subagent') &&
            stepStatusIcon(props.step) && (
              <span class="flex items-center justify-center shrink-0">
                <Icon
                  name={stepStatusIcon(props.step)!}
                  size={10}
                  classList={{ 'animate-spin': props.step.status === 'running' }}
                />
              </span>
            )}
          {durationMs() !== undefined && (
            <span class="font-mono text-white/25 text-[10px] flex-none ml-auto">
              {formatDuration(durationMs()!)}
            </span>
          )}
          <span
            class="flex items-center justify-center shrink-0 text-white/20 transition-[transform,color] duration-200"
            aria-hidden="true"
          >
            <Show
              when={props.step.type === 'subagent'}
              fallback={
                <Icon
                  name="arrow-left"
                  size={10}
                  class="-rotate-90 transition-transform duration-200"
                />
              }
            >
              <Icon name="chevron-right" size={10} />
            </Show>
          </span>
        </button>
        <Show
          when={
            props.step.type === 'subagent' &&
            props.step.status === 'running' &&
            props.step.subagentId
          }
        >
          <button
            type="button"
            class="flex items-center justify-center w-6 h-6 mr-2 rounded shrink-0 bg-transparent border-none cursor-pointer text-white/40 hover:text-red-400/80 transition-colors"
            title={t('agent.subagent.stop')}
            aria-label={t('agent.subagent.stop')}
            onClick={(e) => {
              e.stopPropagation();
              void handleStopSubagent();
            }}
          >
            <Icon name="stop" size={10} classList={{ 'animate-pulse': stopping() }} />
          </button>
        </Show>
      </div>

      {/* Body */}
      <div
        class="overflow-hidden opacity-0 max-h-0 px-2.5"
        style={(() => {
          const expanded = effectiveExpanded();
          const running = props.step.status === 'running';
          // Subagent cards have no expandable body (clicking the card opens the standalone view), so it never expands
          const open = (expanded || running) && props.step.type !== 'subagent';
          return {
            transition:
              'max-height 0.25s cubic-bezier(0.4, 0, 0.2, 1), opacity 0.18s, padding 0.25s',
            ...(open
              ? { 'max-height': '800px', opacity: 1, 'padding-top': '2px', 'padding-bottom': '8px' }
              : {}),
          };
        })()}
      >
        {/* Thinking step: raw text */}
        {props.step.type === 'thinking' && props.step.thinkingText && (
          <div
            ref={(el) => {
              thinkingBodyRef.current = el;
              // StepCard instances persist across status updates (stable slot in the
              // visibleIds list), so a plain assignment suffices; clear on disposal.
              onCleanup(() => {
                if (thinkingBodyRef.current === el) thinkingBodyRef.current = undefined;
              });
            }}
            onWheel={(e) => {
              if (e.deltaY < 0) thinkingFollowPaused = true;
            }}
            onScroll={(e) => {
              const el = e.currentTarget;
              if (el.scrollHeight - el.scrollTop - el.clientHeight < 50)
                thinkingFollowPaused = false;
            }}
            class="text-[12px] leading-relaxed italic whitespace-pre-wrap break-words text-white/50 px-2 py-1.5 rounded bg-black/[0.12] max-h-[300px] overflow-y-auto"
          >
            {props.step.thinkingText}
          </div>
        )}

        {/* Tool-call step: reuses ToolCallBubble */}
        {props.step.type === 'tool_call' && props.step.toolCall && (
          <div class="text-[12px]">
            <ToolCallBubble
              toolCall={props.step.toolCall}
              state={
                (props.step.toolCall.state ||
                  (props.step.status === 'running'
                    ? 'calling'
                    : props.step.status === 'error'
                      ? 'error'
                      : 'success')) as 'calling' | 'success' | 'error'
              }
              result={props.step.toolCall.result}
              error={props.step.toolCall.error}
            />

            {/* File-change diff preview */}
            {props.step.type === 'tool_call' && props.step.toolCall?.fileChanges?.length && (
              <div class="mt-1.5">
                <DiffView changes={props.step.toolCall.fileChanges} />
              </div>
            )}
          </div>
        )}

        {/* Content step: interim summary rendered as Markdown */}
        {props.step.type === 'content' && props.step.contentText && (
          <div class="text-[13px] leading-relaxed break-words text-white/65 p-1">
            <Markdown content={props.step.contentText} />
          </div>
        )}
      </div>
    </div>
  );
};

// ---- Main component ----
const AgentProcessBlock: Component<AgentProcessBlockProps> = (props) => {
  const [userExpanded, setUserExpanded] = createSignal<boolean | null>(null);
  const [expandedSteps, setExpandedSteps] = createSignal<Set<string>>(new Set());

  // ---- Timeline auto-scroll (same pattern as ChatInterface) ----
  let stepsContainerRef: HTMLDivElement | undefined;
  const [autoScrollSteps, setAutoScrollSteps] = createSignal(true);
  let userScrolledUpSteps = false; // Plain variable: ResizeObserver callback must see it synchronously
  let suppressScrollSteps = false; // Plain variable: suppresses programmatic scroll events

  const isStepsAtBottom = () => {
    if (!stepsContainerRef) return true;
    const el = stepsContainerRef;
    return el.scrollHeight - el.scrollTop - el.clientHeight < 50;
  };

  const snapStepsToBottom = () => {
    if (!stepsContainerRef) return;
    suppressScrollSteps = true;
    const el = stepsContainerRef;
    const target = Math.max(0, el.scrollHeight - el.clientHeight);
    if (el.scrollTop < target) {
      el.scrollTop = target;
    }
  };

  const handleStepsWheel = (e: WheelEvent) => {
    if (e.deltaY < 0) {
      userScrolledUpSteps = true;
      suppressScrollSteps = false;
      setAutoScrollSteps(false);
    }
  };

  const handleStepsScroll = () => {
    if (suppressScrollSteps) {
      suppressScrollSteps = false;
      return;
    }
    const atBottom = isStepsAtBottom();
    setAutoScrollSteps(atBottom);
    if (atBottom) userScrolledUpSteps = false;
  };

  // Force-expand the whole block while working; let the user control it once done
  const isExpanded = () => {
    if (props.isActive) return true;
    return userExpanded() ?? false;
  };

  // Live timer for total elapsed time
  const [elapsedMs, setElapsedMs] = createSignal(0);

  // Derive frozen elapsed time from agentSteps data (unaffected by remounts; avoids Date.now() drift)
  const frozenElapsed = (): number => {
    if (!props.startTime) return 0;
    const steps = props.agentSteps;
    if (steps && steps.length > 0) {
      const last = steps[steps.length - 1];
      if (last.status === 'complete' && last.duration !== undefined) {
        return last.timestamp + last.duration - props.startTime;
      }
      if (last.status !== 'running') {
        return last.timestamp - props.startTime;
      }
    }
    return 0;
  };

  createEffect(() => {
    if (props.isActive && props.startTime) {
      const intervalId = setInterval(() => {
        if (!props.isActive) {
          clearInterval(intervalId);
          return;
        }
        setElapsedMs(Date.now() - props.startTime!);
      }, 100);
      onCleanup(() => clearInterval(intervalId));
    } else if (!props.isActive && props.startTime) {
      setElapsedMs(frozenElapsed());
    }
  });

  onCleanup(() => {});

  // Streaming scroll: snap to bottom as content grows (ResizeObserver, fires only on size changes)
  createEffect(() => {
    if (props.isActive && stepsContainerRef) {
      const observer = new ResizeObserver(() => {
        if (!userScrolledUpSteps && autoScrollSteps()) {
          snapStepsToBottom();
        }
      });
      observer.observe(stepsContainerRef);
      onCleanup(() => observer.disconnect());
    }
  });

  const toggleOuter = () => {
    if (props.isActive) return;
    setUserExpanded((p) => !(p ?? false));
  };

  const toggleStep = (stepId: string) => {
    setExpandedSteps((prev) => {
      const next = new Set(prev);
      if (next.has(stepId)) next.delete(stepId);
      else next.add(stepId);
      return next;
    });
  };

  const isStepExpanded = (step: AgentStep) => {
    if (step.status === 'running') return true;
    return expandedSteps().has(step.id);
  };

  // Decide which mode to use
  const hasSteps = () => !!(props.agentSteps && props.agentSteps.length > 0);
  const hasOldContent = () =>
    !!props.reasoning?.trim() ||
    !!props.interimContent?.trim() ||
    !!(props.toolCalls && props.toolCalls.length > 0);
  const hasContent = () => hasSteps() || hasOldContent();

  // Timeline steps: exclude the last content step (it is the final reply, already shown in the Markdown bubble below)
  const timelineSteps = () => {
    const steps = props.agentSteps;
    if (!steps || steps.length === 0) return [];
    const last = steps[steps.length - 1];
    if (last.type === 'content') return steps.slice(0, -1);
    return steps;
  };

  // ---- Render buffer: the agent may create 7-8 tool-call steps nearly simultaneously; rendering
  // them directly makes the animations push each other around into visual chaos. New steps enter a
  // queue and are dequeued one by one at a fixed interval for an orderly streaming entrance.
  //
  // Key design: streaming updates to agentSteps create new message object references; the parent
  // <For> compares by reference and destroys/recreates AgentProcessBlock — every tool event
  // triggers a remount. Therefore the queue's three elements (renderedIds / pendingIds /
  // dequeueTimer) all live in the module-level perRoundQueue and are restored as-is after a
  // remount: already-rendered steps restore visibleIds directly (avoiding a flash of re-rendering
  // from scratch), pending steps keep queuing, and an in-flight timer is not interrupted by the
  // remount (onCleanup must not clear it).
  //
  // Note: the component may have been remounted several times while a timer callback is alive, so
  // setVisibleIds does not capture the component instance's setter directly; it is recorded in the
  // module-level subscribers, registered/deregistered by the currently active instance, and the
  // timer notifies only the newest instance.
  const DEQUEUE_INTERVAL = 150;
  const roundKey = untrack(() => String(props.startTime ?? 0));
  const queue = getRoundQueue(roundKey);

  const [visibleIds, setVisibleIds] = createSignal<string[]>([...queue.renderedIds]);

  const stepById = (id: string): AgentStep | undefined => timelineSteps().find((s) => s.id === id);

  // Dequeue-render subscriber: after a remount only the newest instance receives dequeue notifications; the old DOM was already destroyed by <For>.
  const subscriber = (id: string) =>
    setVisibleIds((prev) => (prev.includes(id) ? prev : [...prev, id]));
  queueSubscribers.set(roundKey, subscriber);
  onCleanup(() => {
    // Unregister only if this instance is still the subscriber (a newer instance overwrote it on remount)
    if (queueSubscribers.get(roundKey) === subscriber) queueSubscribers.delete(roundKey);
  });

  const flushQueue = () => {
    if (queue.pendingIds.length === 0) {
      queue.dequeueTimer = null;
      return;
    }
    const id = queue.pendingIds.shift()!;
    queue.renderedIds.add(id);
    queueSubscribers.get(roundKey)?.(id);
    queue.dequeueTimer = window.setTimeout(flushQueue, DEQUEUE_INTERVAL);
  };

  // Watch source steps: while active, new steps enqueue and render one by one; when inactive, sync the full list directly
  createEffect(() => {
    const steps = timelineSteps();
    if (!props.isActive) {
      // Inactive (historical data / finished): render everything at once, clear the queue and free this round's queue state
      if (queue.dequeueTimer) {
        clearTimeout(queue.dequeueTimer);
        queue.dequeueTimer = null;
      }
      queue.pendingIds = [];
      queue.renderedIds.clear();
      perRoundQueue.delete(roundKey);
      steps.forEach((s) => queue.renderedIds.add(s.id));
      setVisibleIds(steps.map((s) => s.id));
      return;
    }
    // Active: find step ids neither rendered nor queued and enqueue them (after a remount restore,
    // pendingIds remain valid — no duplicate enqueueing and no bypassing the queue on instance reset).
    // The first element also waits one dequeue interval: enqueueing starts the timer, which drives
    // the dequeue cadence uniformly, so every element (including the first) enters with consistent
    // timing and an intact animation.
    const newIds = steps
      .filter((s) => !queue.renderedIds.has(s.id) && !queue.pendingIds.includes(s.id))
      .map((s) => s.id);
    if (newIds.length > 0) {
      queue.pendingIds.push(...newIds);
      if (queue.dequeueTimer === null) {
        queue.dequeueTimer = window.setTimeout(flushQueue, DEQUEUE_INTERVAL);
      }
    }
  });

  return (
    <Show when={hasContent()}>
      <div class="my-2" classList={{ 'is-open': isExpanded(), 'is-active': props.isActive }}>
        {/* Outer header */}
        <button
          type="button"
          class="flex items-center w-full gap-2 px-3 py-2 cursor-pointer select-none bg-transparent border-none text-white/50 text-xs transition-colors duration-200 hover:text-white/75"
          onClick={toggleOuter}
          aria-expanded={isExpanded()}
        >
          <span class="font-medium flex-none">
            {props.isActive ? t('agent.status.working') : t('agent.status.worked')}
          </span>
          <span class="font-mono text-white/30 text-[11px] flex-none ml-0.5">
            {props.startTime ? formatDuration(elapsedMs()) : ''}
          </span>
          <span
            class="flex items-center justify-center ml-auto w-5 h-5 rounded-full bg-white/[0.08] text-white/40 transition-colors duration-200"
            aria-hidden="true"
          >
            <Icon
              name="arrow-left"
              size={16}
              classList={{ '-rotate-90': !isExpanded(), 'rotate-90': isExpanded() }}
              style={{ transition: 'transform 0.2s' }}
            />
          </span>
        </button>

        {/* Body: scrolling lives on the outer wrapper; inner content lays out normally without compression */}
        <div
          class="overflow-hidden transition-[max-height] duration-300 ease-in-out"
          style={isExpanded() ? 'max-height: 620px' : 'max-height: 0'}
        >
          <div
            ref={stepsContainerRef}
            class="overflow-y-auto"
            style={{ 'max-height': '620px' }}
            onScroll={handleStepsScroll}
            onWheel={handleStepsWheel}
          >
            {/* === New mode: timeline cards === */}
            <Show when={hasSteps()}>
              <div class="flex flex-col gap-2 px-3">
                <For each={visibleIds()}>
                  {(id) => {
                    // Use a memo to fetch the latest step object: status updates (running→complete)
                    // show live, while <For>'s each is a stable id list so the DOM is not rebuilt
                    const step = createMemo(() => stepById(id));
                    return (
                      <Show when={step()}>
                        <StepCard
                          step={step()!}
                          isActive={props.isActive}
                          expanded={isStepExpanded(step()!)}
                          onToggle={() => toggleStep(id)}
                        />
                      </Show>
                    );
                  }}
                </For>
              </div>
            </Show>

            {/* === Old mode (fallback): flat three blocks === */}
            <Show when={!hasSteps() && hasOldContent()}>
              <Show when={props.reasoning && props.reasoning.trim()}>
                <div class="mb-2">
                  <div class="flex items-center text-[11px] font-medium mb-1 text-white/40">
                    <Icon name="brain" size={11} class="mr-1.5 opacity-60" />
                    {t('agent.step.thinking')}
                  </div>
                  <div class="text-[12.5px] leading-relaxed italic whitespace-pre-wrap break-words text-white/55 px-2.5 py-2 rounded-md bg-black/15 border-l-2 border-l-white/[0.08] max-h-[360px] overflow-y-auto">
                    {props.reasoning}
                  </div>
                </div>
              </Show>

              <Show when={props.interimContent && props.interimContent.trim()}>
                <div class="mb-2">
                  <div class="flex items-center text-[11px] font-medium mb-1 text-white/40">
                    <Icon name="document" size={11} class="mr-1.5 opacity-60" />
                    {t('agent.step.interim')}
                  </div>
                  <div class="text-[12.5px] leading-relaxed italic whitespace-pre-wrap break-words text-white/55 px-2.5 py-2 rounded-md bg-black/15 border-l-2 border-l-white/[0.08] max-h-[360px] overflow-y-auto">
                    <Markdown content={props.interimContent!} />
                  </div>
                </div>
              </Show>

              <Show when={props.toolCalls && props.toolCalls.length > 0}>
                <div class="agent-process-toolcalls">
                  <For each={props.toolCalls}>
                    {(tc) => (
                      <ToolCallBubble
                        toolCall={tc}
                        state={tc.state ?? 'calling'}
                        result={tc.result}
                        error={tc.error}
                      />
                    )}
                  </For>
                </div>
              </Show>
            </Show>
          </div>
        </div>
      </div>
    </Show>
  );
};

export default AgentProcessBlock;

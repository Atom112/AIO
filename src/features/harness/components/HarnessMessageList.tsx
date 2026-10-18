/**
 * Harness message list — renders the loaded history plus the live assistant
 * draft with the chat pipeline's own leaf components (Markdown / ThinkBlock /
 * AgentProcessBlock), so harness turns look exactly like native agent turns.
 *
 * Paged rendering (the chat page's proven pattern): only the most recent
 * window mounts; an IntersectionObserver sentinel at the top loads earlier
 * pages with scroll anchoring. Switching to a long agent session no longer
 * mounts every Markdown bubble at once (the reported switch wait).
 */
import {
  Component,
  createEffect,
  createMemo,
  createSignal,
  For,
  onCleanup,
  onMount,
  Show,
} from 'solid-js';
import Markdown from '../../../shared/components/Markdown';
import ThinkBlock from '../../../shared/components/ThinkBlock';
import AgentProcessBlock from '../../chat/components/AgentProcessBlock';
import { btnGhost } from '../../../shared/components/buttonStyles';
import {
  currentHarnessDraft,
  currentHarnessRun,
  currentHarnessTopicId,
  harnessMessages,
} from '../../../core/store/harness';
import { formatNumber, t } from '../../../core/i18n';
import type { Message } from '../../../core/store/types';

/** Plain-text content for the Markdown renderer (multimodal parts are not rendered here). */
const bubbleContentText = (msg: Message): string =>
  typeof msg.content === 'string' ? msg.content : '';

const HarnessMessageList: Component = () => {
  let scrollRef: HTMLDivElement | undefined;
  /** Follow the bottom while streaming; wheel-up pauses (ThinkBlock's pattern). */
  let followPaused = false;
  const handleWheel = (e: WheelEvent) => {
    if (e.deltaY < 0) followPaused = true;
  };
  const handleScroll = () => {
    const el = scrollRef;
    if (!el) return;
    if (el.scrollHeight - el.scrollTop - el.clientHeight < 50) {
      // M-12: back at the bottom — an append-pinned window (grown to keep the
      // viewport anchored while the user read history) trims to the tail again.
      if (followPaused) followPaused = false;
      trimPinnedWindow();
    }
  };
  const scrollToBottom = () => {
    const el = scrollRef;
    if (!el || followPaused) return;
    el.scrollTop = el.scrollHeight;
  };

  // ---- Paged message rendering (chat page's ChatInterface pattern) ----
  const PAGE_SIZE = 60;
  const [visibleCount, setVisibleCount] = createSignal(PAGE_SIZE);
  // ---- M-12: append-direction window pinning (see ChatInterface's counterpart).
  // While the user is scrolled up reading history, an append would slide the tail
  // window's start forward and swap the content under their viewport; grow the
  // window by the append delta so its top stays anchored to the same message.
  // Returning to the bottom trims back to the tail (in handleScroll, where the
  // transition is observable; the rAF snap compensates the unmounted-above
  // content so the view stays glued to the bottom). ----
  let pinnedGrowth = 0;
  let lastLen = 0;
  const trimPinnedWindow = () => {
    if (pinnedGrowth === 0) return;
    pinnedGrowth = 0;
    setVisibleCount(PAGE_SIZE);
    requestAnimationFrame(() => {
      const el = scrollRef;
      if (el) el.scrollTop = el.scrollHeight - el.clientHeight;
    });
  };
  // Reset the window on every topic switch
  createEffect(() => {
    void currentHarnessTopicId();
    setVisibleCount(PAGE_SIZE);
    pinnedGrowth = 0;
    lastLen = 0;
  });
  /** Absolute index of the first rendered message (the tail window start). */
  const windowStart = createMemo(() => Math.max(0, harnessMessages().length - visibleCount()));
  createEffect(() => {
    const len = harnessMessages().length;
    const delta = len - lastLen;
    lastLen = len;
    if (delta > 0 && followPaused && windowStart() > 0) {
      pinnedGrowth += delta;
      setVisibleCount((c) => c + delta);
    }
  });
  let topSentinelRef: HTMLDivElement | undefined;
  let loadingEarlier = false;
  const loadEarlier = () => {
    if (loadingEarlier || windowStart() === 0) return;
    loadingEarlier = true;
    const el = scrollRef;
    const prevHeight = el?.scrollHeight ?? 0;
    const prevTop = el?.scrollTop ?? 0;
    setVisibleCount((c) => c + PAGE_SIZE);
    // Scroll anchoring: keep the viewport on the same content after prepending
    requestAnimationFrame(() => {
      if (el) el.scrollTop = prevTop + (el.scrollHeight - prevHeight);
      loadingEarlier = false;
    });
  };
  onMount(() => {
    const el = scrollRef;
    if (el) el.scrollTop = el.scrollHeight;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((en) => en.isIntersecting)) loadEarlier();
      },
      { root: scrollRef, rootMargin: '160px' },
    );
    if (topSentinelRef) io.observe(topSentinelRef);
    onCleanup(() => io.disconnect());
  });

  // Follow while a draft streams or messages append; also re-follow on topic switch.
  createEffect(() => {
    void harnessMessages().length;
    void currentHarnessDraft()?.content;
    void currentHarnessDraft()?.agentSteps?.length;
    scrollToBottom();
  });
  createEffect(() => {
    const topicId = currentHarnessTopicId();
    if (topicId) {
      followPaused = false;
      scrollToBottom();
    }
  });

  const draft = () => currentHarnessDraft();
  const isActive = () =>
    currentHarnessRun().state === 'running' || currentHarnessRun().state === 'connecting';
  const empty = () => harnessMessages().length === 0 && !draft();

  /** Render one message bubble (user right / assistant left, chat visual language). */
  const Bubble: Component<{ msg: Message; live?: boolean }> = (props) => {
    const msg = () => props.msg;
    return (
      <div
        class={`flex gap-3 w-full mb-3 ${msg().role === 'assistant' ? 'justify-start items-start' : 'justify-end items-start pr-[10px]'}`}
      >
        <div
          class={`flex flex-col max-w-[75%] ${msg().role === 'assistant' ? 'items-start' : 'items-end'}`}
        >
          <div
            class={`rounded-[10px] leading-relaxed max-w-full min-h-[1.5em] px-[14px] py-[10px] break-words ${
              msg().role === 'assistant'
                ? 'rounded-tl-[2px] text-white'
                : 'rounded-tr-[2px] text-white'
            }`}
            style={{
              background:
                msg().role === 'assistant'
                  ? 'rgba(var(--text-base-rgb),0.04)'
                  : 'rgba(var(--primary-rgb),0.08)',
              border:
                msg().role === 'assistant'
                  ? '1px solid rgba(var(--text-base-rgb),0.13)'
                  : '1px solid rgba(var(--primary-rgb),0.16)',
              'backdrop-filter': 'blur(6px)',
            }}
          >
            <Show when={msg().role === 'assistant' && msg().reasoning}>
              <ThinkBlock content={msg().reasoning!} isStreaming={!!props.live && isActive()} />
            </Show>
            <Show
              when={
                msg().role === 'assistant' &&
                Array.isArray(msg().agentSteps) &&
                msg().agentSteps!.length > 0
              }
            >
              <AgentProcessBlock
                agentSteps={msg().agentSteps}
                isActive={!!props.live && isActive()}
                startTime={msg().agentStartTime}
              />
            </Show>
            <Show when={msg().content}>
              <div class="markdown-body text-sm">
                <Markdown content={bubbleContentText(msg())} />
              </div>
            </Show>
          </div>
        </div>
      </div>
    );
  };

  return (
    <div
      ref={scrollRef}
      class="flex-grow overflow-y-auto pb-[15px] z-[1] min-h-0"
      onWheel={handleWheel}
      onScroll={handleScroll}
    >
      {/* Load-earlier sentinel: always mounted so the IntersectionObserver keeps a
          stable target; the button is only shown when older pages exist */}
      <div ref={topSentinelRef} />
      <Show when={windowStart() > 0}>
        <div class="flex justify-center py-2">
          <button
            class={btnGhost + ' text-xs'}
            onClick={loadEarlier}
            title={t('chat.loadEarlierTitle')}
          >
            {t('chat.loadEarlier', { count: formatNumber(windowStart()) })}
          </button>
        </div>
      </Show>
      <For each={harnessMessages().slice(windowStart())}>{(msg) => <Bubble msg={msg} />}</For>
      <Show when={draft()}>
        <Bubble
          msg={{
            id: '__harness_draft__',
            role: 'assistant',
            content: draft()!.content,
            reasoning: draft()!.reasoning || undefined,
            agentSteps: draft()!.agentSteps,
            agentStartTime: draft()!.startedAt,
          }}
          live
        />
      </Show>
      <Show when={empty()}>
        <div class="flex-1 flex items-center justify-center">
          <div class="text-xs text-white/30 text-center max-w-[420px] leading-5">
            {t('harness.chat.emptyHint')}
          </div>
        </div>
      </Show>
    </div>
  );
};

export default HarnessMessageList;

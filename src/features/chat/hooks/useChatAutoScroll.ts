/**
 * Chat auto-scroll engine (阶段4 第2项拆分切片：ChatInterface 按 useChatAutoScroll 路径).
 *
 * Verbatim extraction of the scroll subsystem from ChatInterface:
 * - follow-bottom during streaming (instant snaps, capped by an echo-compare
 *   that distinguishes programmatic scrolls from user scrolls);
 * - smooth scroll (rAF, easeOutCubic) for non-streaming growth, never upward;
 * - topic-switch snap: content-visibility placeholders lay out at 100px, so one
 *   assignment misses the real bottom — progressive correction runs over
 *   multiple rAF frames until the position settles (guard cap 40);
 * - wheel-up cancels following with zero latency; scrolling back to the
 *   bottom (50px threshold) resumes it.
 */
import { createEffect, on, onCleanup, createSignal, type Accessor } from 'solid-js';

/** Deps the ChatInterface component provides. */
export interface AutoScrollDeps {
  /** Scroll container element (the fixed-height flex-grow box). */
  scrollContainerRef: Accessor<HTMLDivElement | undefined>;
  /** Content wrapper element observed for streaming growth. */
  streamContentRef: Accessor<HTMLDivElement | undefined>;
  /** Active topic id (topic-switch detection). */
  topicId: Accessor<string | undefined>;
  /** Whether the AI is generating (follow during, one final smooth scroll at end). */
  isThinking: Accessor<boolean>;
  /** History length (growth detection for the non-streaming smooth path). */
  historyLength: Accessor<number>;
}

export function useChatAutoScroll(deps: AutoScrollDeps) {
  const { scrollContainerRef, streamContentRef, topicId, isThinking, historyLength } = deps;

  /** Auto-follow enabled (drives the scroll-to-bottom button visibility too). */
  const [autoScroll, setAutoScroll] = createSignal(true);
  /** Plain mutable flag: the next rAF frame sees it immediately, stopping follow-bottom with zero latency */
  let userScrolledUp = false;
  /** In-flight smooth-scroll rAF id (cancelled on wheel-up / teardown) */
  let smoothScrollRAF: number | undefined;
  /** Programmatic-scroll echo guard (pending count + last written scrollTop) */
  let pendingSnaps = 0;
  let lastProgrammaticTop = -1;
  /** History-length baseline for growth detection */
  let lastHistoryLen = 0;

  /** Whether the container is scrolled to the bottom (50px threshold) */
  const isAtBottom = () => {
    const el = scrollContainerRef();
    if (!el) return true;
    return el.scrollHeight - el.scrollTop - el.clientHeight < 50;
  };

  /** Wheel scroll up: stop auto-scroll immediately and cancel any in-flight smooth scroll */
  const handleWheel = (e: WheelEvent) => {
    if (e.deltaY < 0) {
      userScrolledUp = true;
      if (smoothScrollRAF !== undefined) {
        cancelAnimationFrame(smoothScrollRAF);
        smoothScrollRAF = undefined;
      }
      pendingSnaps = 0;
      setAutoScroll(false);
    }
  };

  /** Scroll event: distinguish user scrolling from programmatic scrolling (position-echo comparison; see lastProgrammaticTop) */
  const handleScroll = () => {
    const el = scrollContainerRef();
    if (pendingSnaps > 0) {
      pendingSnaps -= 1;
      if (el && Math.abs(el.scrollTop - lastProgrammaticTop) < 2) {
        return;
      }
      // Position mismatch: user drag between programmatic scrolls; falls through to the user logic below (count already decremented)
    }
    const atBottom = isAtBottom();
    setAutoScroll(atBottom);
    // User manually scrolled back to the bottom: clear the scroll-up flag and resume auto-follow
    if (atBottom) {
      userScrolledUp = false;
    }
  };

  /** Snap to the bottom instantly (used during streaming; scrolls down only) */
  const snapToBottom = () => {
    const el = scrollContainerRef();
    if (!el) return;
    const target = Math.max(0, el.scrollHeight - el.clientHeight);
    if (el.scrollTop < target) {
      pendingSnaps += 1;
      el.scrollTop = target;
      lastProgrammaticTop = target;
    }
  };

  /** Smooth-scroll to the bottom (rAF animation, for non-streaming cases) */
  const smoothScrollToBottom = () => {
    const el = scrollContainerRef();
    if (!el) return;
    const target = Math.max(0, el.scrollHeight - el.clientHeight);
    const start = el.scrollTop;
    const distance = target - start;
    // Target not above the current position: never scroll upward (prevents a brief content collapse from snapping the view back to the top)
    if (target <= start) {
      el.scrollTop = target;
      lastProgrammaticTop = target;
      return;
    }
    if (Math.abs(distance) < 2) {
      el.scrollTop = target;
      lastProgrammaticTop = target;
      return;
    }
    if (smoothScrollRAF !== undefined) cancelAnimationFrame(smoothScrollRAF);
    const duration = 300;
    const startTime = performance.now();
    const animate = (now: number) => {
      const cur = scrollContainerRef();
      if (!cur || !autoScroll()) {
        smoothScrollRAF = undefined;
        return;
      }
      const elapsed = now - startTime;
      const progress = Math.min(elapsed / duration, 1);
      const eased = 1 - Math.pow(1 - progress, 3);
      pendingSnaps += 1;
      cur.scrollTop = start + distance * eased;
      lastProgrammaticTop = cur.scrollTop;
      if (progress < 1) {
        smoothScrollRAF = requestAnimationFrame(animate);
      } else {
        smoothScrollRAF = undefined;
      }
    };
    smoothScrollRAF = requestAnimationFrame(animate);
  };

  /** Snap to the latest conversation after a topic switch or component remount.
   *  Message rows use content-visibility: auto (contain-intrinsic-size: auto 100px); unrendered
   *  rows lay out as 100px placeholders, so one scrollTop assignment only reaches the placeholder
   *  bottom. Progressive rendering expands about one viewport per frame, so correction continues
   *  over multiple frames until the position settles (guard cap 60; short ones settle in 2-3).
   *  Aborts on user scroll-up; the first snap runs in a microtask (DOM committed, not
   *  yet painted), avoiding a one-frame flash at the wrong position.
   *  Round-11 feedback fix: the loop used to exit on "two frames at the same scrollTop",
   *  which happens early while placeholder rows are still collapsed — late height
   *  expansion then drifted the view off the bottom with no further correction
   *  ("auto-scroll fails / lands at the top of history"). Now it exits only when the
   *  view is genuinely AT the bottom AND stable for two consecutive frames. */
  const snapToLatestOnTopicSwitch = () => {
    queueMicrotask(() => {
      if (!scrollContainerRef() || userScrolledUp) return;
      snapToBottom();
      let lastTop = -1;
      let guard = 0;
      let settledFrames = 0;
      const step = () => {
        const el = scrollContainerRef();
        if (!el || userScrolledUp || guard >= 60) return;
        snapToBottom();
        const cur = el.scrollTop;
        if (isAtBottom() && cur === lastTop) {
          settledFrames += 1;
          if (settledFrames >= 2) return; // at the bottom and stable — done
        } else {
          settledFrames = 0;
        }
        lastTop = cur;
        guard += 1;
        requestAnimationFrame(step);
      };
      requestAnimationFrame(step);
    });
  };

  /** On topic switch: reset auto-scroll and the history-length baseline, then snap to the latest conversation */
  createEffect(
    on(topicId, () => {
      setAutoScroll(true);
      userScrolledUp = false;
      lastHistoryLen = historyLength();
      // Show the latest conversation right after a topic switch / remount (return from a
      // subagent view): the length-watching effect only scrolls when len > lastHistoryLen,
      // never on the switch path; without a snap the view stays clamped at the topic's start.
      snapToLatestOnTopicSwitch();
    }),
  );

  /** Generation state change: reset auto-scroll on start; do one final smooth scroll on end */
  createEffect(
    on(isThinking, (thinking, prev) => {
      if (thinking && !prev) {
        // User sent a new message; generation starting
        setAutoScroll(true);
        userScrolledUp = false;
      } else if (!thinking && prev) {
        // Generation just ended: content may have just finished rendering; one final scroll, then stop
        const el = scrollContainerRef();
        if (autoScroll() && el) {
          requestAnimationFrame(() => {
            if (autoScroll() && scrollContainerRef() && !isThinking()) {
              smoothScrollToBottom();
            }
          });
        }
      }
    }),
  );

  /** Non-streaming case: smooth-scroll only when the message count grows (new messages), avoiding repeated re-snapping from post-generation reflow */
  createEffect(() => {
    const len = historyLength();
    if (isThinking()) {
      lastHistoryLen = len;
      return;
    }
    if (!autoScroll() || !scrollContainerRef()) {
      lastHistoryLen = len;
      return;
    }
    if (len > lastHistoryLen) {
      requestAnimationFrame(() => {
        if (autoScroll() && !isThinking() && scrollContainerRef()) {
          smoothScrollToBottom();
        }
      });
    }
    lastHistoryLen = len;
  });

  /** While streaming: ResizeObserver follows content growth to keep the view at the bottom.
   *  Observes the content wrapper (streamContentRef) because the scroll container is a
   *  fixed-height flex-grow box: content growth changes scrollHeight but not its box size, so
   *  RO on the container never fires past the initial observe() callback and streaming growth
   *  goes unfollowed (one snap at start and end only). The wrapper grows with content, firing
   *  on every expansion; the container is observed too (window resize / input-area height). */
  createEffect(() => {
    const content = streamContentRef();
    if (!isThinking() || !content) return;
    const observer = new ResizeObserver(() => {
      if (!userScrolledUp && autoScroll()) {
        snapToBottom();
      }
    });
    observer.observe(content);
    const container = scrollContainerRef();
    if (container) observer.observe(container);
    onCleanup(() => observer.disconnect());
  });
  /** Cancel the smooth-scroll animation on component teardown */
  onCleanup(() => {
    if (smoothScrollRAF !== undefined) {
      cancelAnimationFrame(smoothScrollRAF);
    }
  });

  /** Click handler for the scroll-to-bottom button: resume following and smooth-scroll down. */
  const scrollBackToBottom = () => {
    userScrolledUp = false;
    setAutoScroll(true);
    smoothScrollToBottom();
  };

  return { autoScroll, handleWheel, handleScroll, scrollBackToBottom };
}

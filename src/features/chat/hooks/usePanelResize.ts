/**
 * Panel drag-resize logic for the chat three-column layout (P3/阶段4 第2项拆分切片：
 * ChatPage 按 useChatXxx 钩子路径拆分，本钩子承接全部拖拽/幕帘/吸附逻辑).
 *
 * Left panel ("curtain" layout):
 * - Drag tracks the cursor 1:1 in BOTH directions, snapping only on release;
 * - "Curtain" reveal width: while narrowing, content is progressively clipped by
 *   the panel's left edge instead of reflowing (no empty transition frame);
 * - Release snap: pinned at minimum / inside the deadzone → collapse via the CSS
 *   transition; otherwise easeOutCubic-animate to >= 15% and remember (rAF).
 *
 * Right panel: plain percentage drag while expanded.
 */
import { createSignal, onCleanup, onMount, type Accessor } from 'solid-js';

/** Deps the ChatPage component provides (persisted widths/collapse state stay in the component). */
export interface PanelResizeDeps {
  /** Layout root ref (clientWidth = percent basis; offsetWidth for the right panel). */
  chatPageRef: Accessor<HTMLDivElement | undefined>;
  isLeftCollapsed: Accessor<boolean>;
  /** Single entry point that sets + persists the left collapse flag (with width fallback). */
  setLeftCollapsed: (collapsed: boolean) => void;
  leftPanelWidth: Accessor<number>;
  setLeftPanelWidth: (v: number) => void;
  rightPanelWidth: Accessor<number>;
  setRightPanelWidth: (v: number) => void;
  isRightCollapsed: Accessor<boolean>;
}

/** Collapsed rail width of the left panel (kept in sync with ProjectSidebar). */
const LEFT_RAIL_WIDTH_PX = 54;
const LEFT_MAX_WIDTH_PCT = 30;
/** Release deadzone (px): dragging out less than this counts as not really opened → collapse on release. */
const LEFT_RELEASE_DEADZONE_PX = 12;

export function usePanelResize(deps: PanelResizeDeps) {
  const {
    chatPageRef,
    isLeftCollapsed,
    setLeftCollapsed,
    leftPanelWidth,
    setLeftPanelWidth,
    setRightPanelWidth,
  } = deps;

  const [isResizing, setIsResizing] = createSignal(false);
  /** Live width preview (%) while dragging: rendered during tracking, cleared on release to fall back to the remembered leftPanelWidth */
  const [leftDragWidth, setLeftDragWidth] = createSignal<number | null>(null);
  /**
   * "Curtain" layout width of the expanded content (px, = content-box width of the 15% minimal panel).
   * While narrowing, content neither reflows nor fades — it is progressively clipped by the panel's left
   * edge (curtain reveal), avoiding an empty transition frame with both content sets invisible.
   */
  const [leftRevealMinPx, setLeftRevealMinPx] = createSignal(0);
  /** rAF id of the release snap animation: cancelled when a new drag starts so it never fights the user's gesture */
  let leftSnapRaf: number | undefined;

  // Curtain-reveal target width: content-box width of the 15% minimal panel (asymmetric padding 15 left
  // + 21 right, plus a 2px safety margin so content never overflows at 15%); recomputed on resize;
  // ProjectSidebar infers container width from -38
  onMount(() => {
    const updateRevealMin = () => {
      const base = chatPageRef()?.clientWidth ?? window.innerWidth;
      setLeftRevealMinPx(Math.max(base * 0.15 - 38, 120));
    };
    updateRevealMin();
    window.addEventListener('resize', updateRevealMin);
    onCleanup(() => window.removeEventListener('resize', updateRevealMin));
    onCleanup(() => {
      if (leftSnapRaf !== undefined) cancelAnimationFrame(leftSnapRaf);
    });
  });

  /** Plain percentage drag (right panel): reports the cursor position as % of the layout width. */
  const attachResizeDrag = (e: MouseEvent, onDragMove: (pct: number) => void) => {
    e.preventDefault();
    setIsResizing(true);

    const totalW = chatPageRef()?.offsetWidth ?? window.innerWidth;
    const handleMove = (moveEvent: MouseEvent) => {
      onDragMove((moveEvent.clientX / totalW) * 100);
    };

    const stopResize = () => {
      setIsResizing(false);
      document.removeEventListener('mousemove', handleMove);
      document.removeEventListener('mouseup', stopResize);
    };

    document.addEventListener('mousemove', handleMove);
    document.addEventListener('mouseup', stopResize);
  };

  /**
   * Drag-resize the left panel: strict 1:1 cursor tracking in both expand and collapse directions; snap only on release.
   *
   * - Tracking origin: map the panel's "currently occupied pixel width" onto the cursor axis — the rail
   *   width when collapsed, the DOM-measured rendered width when expanded (the handle sits at the
   *   panel's right edge, so measuring guarantees zero first-frame displacement and no jump).
   *   Width delta = cursor delta, strictly 1:1;
   * - Percentage conversion uses `clientWidth` (content width), matching how CSS `width:%` resolves;
   * - Minimum width: tracking width never drops below the collapsed rail width, so the handle cannot
   *   be dragged to the far left and content never overflows;
   * - Release snap: pinned at the minimum or still inside the deadzone -> collapse; otherwise land at >= 15% and remember.
   */
  const startLeftResize = (e: MouseEvent) => {
    // user re-grabbed the handle: cancel the in-flight snap animation, the gesture takes over
    if (leftSnapRaf !== undefined) {
      cancelAnimationFrame(leftSnapRaf);
      leftSnapRaf = undefined;
    }
    const cssBase = chatPageRef()?.clientWidth ?? window.innerWidth;
    const wasCollapsed = isLeftCollapsed();
    const doCollapse = LEFT_RAIL_WIDTH_PX / cssBase; // narrowest tracking width (%)
    // panel's current occupied pixel width: rail width when collapsed; measured when expanded (falls back to the remembered width if unmeasurable)
    const handleEl = e.currentTarget instanceof Element ? e.currentTarget : null;
    const measuredPx = handleEl?.parentElement?.getBoundingClientRect().width ?? 0;
    const basePx = wasCollapsed
      ? LEFT_RAIL_WIDTH_PX
      : measuredPx > 0
        ? measuredPx
        : (leftPanelWidth() / 100) * cssBase;
    // cursor axis zero point: cursor X minus this equals the width the panel should occupy
    const anchorX = e.clientX - basePx;
    setIsResizing(true);
    e.preventDefault();

    const handleMove = (moveEvent: MouseEvent) => {
      const pct = ((moveEvent.clientX - anchorX) / cssBase) * 100;
      setLeftDragWidth(Math.min(Math.max(pct, doCollapse), LEFT_MAX_WIDTH_PCT));
    };

    const stopResize = () => {
      document.removeEventListener('mousemove', handleMove);
      document.removeEventListener('mouseup', stopResize);
      const dragged = leftDragWidth();
      if (dragged === null) {
        setIsResizing(false);
        return; // pressed without dragging: keep as-is
      }
      const draggedPx = (dragged / 100) * cssBase;
      if (draggedPx <= LEFT_RAIL_WIDTH_PX + LEFT_RELEASE_DEADZONE_PX) {
        // pinned at the minimum / never really opened: snap back to the collapsed rail (via the CSS collapse transition)
        setLeftDragWidth(null);
        setIsResizing(false);
        setLeftCollapsed(true);
        return;
      }
      // Opened: ease the preview width to the snap target (>= 15%) with rAF.
      // Mask/padding/opacity follow each frame's width, so the ending has no hard clip or jump;
      // isResizing stays true (CSS transitions disabled) during it to avoid double smoothing.
      const target = Math.max(dragged, 15);
      const durationMs = 250;
      const start = performance.now();
      const step = (now: number) => {
        const t = Math.min((now - start) / durationMs, 1);
        const eased = 1 - Math.pow(1 - t, 3); // easeOutCubic
        setLeftDragWidth(dragged + (target - dragged) * eased);
        if (t < 1) {
          leftSnapRaf = requestAnimationFrame(step);
          return;
        }
        leftSnapRaf = undefined;
        setLeftPanelWidth(target);
        setLeftCollapsed(false);
        setLeftDragWidth(null);
        setIsResizing(false);
      };
      leftSnapRaf = requestAnimationFrame(step);
    };

    document.addEventListener('mousemove', handleMove);
    // listen for mouseup immediately: a press-and-release on the handle without any mousemove still completes the snap
    document.addEventListener('mouseup', stopResize);
  };

  /** Drag-resize the right panel (original behavior: only available when expanded). */
  const startResizeRight = (e: MouseEvent) => {
    attachResizeDrag(e, (pct) => {
      setRightPanelWidth(Math.min(Math.max(100 - pct, 15), 30));
    });
  };

  return {
    isResizing,
    leftDragWidth,
    leftRevealMinPx,
    startLeftResize,
    startResizeRight,
  };
}

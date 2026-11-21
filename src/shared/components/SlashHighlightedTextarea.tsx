/**
 * Chat input with an inline slash-command highlight.
 *
 * A `<textarea>` cannot color a substring, so the visible text is painted by a
 * mirror layer behind a transparent-text textarea. Both layers share the same
 * font, padding and wrapping; a recognized leading command token is rendered in
 * the theme color while the rest keeps the normal text color. The mirror follows
 * the textarea's vertical scroll and its client width (so a visible scrollbar does
 * not change the wrap point).
 */

import {
  Component,
  For,
  createEffect,
  createMemo,
  createSignal,
  on,
  onCleanup,
  onMount,
} from 'solid-js';

export interface SlashTokenRange {
  /** Start index of the recognized command token in the input value. */
  start: number;
  /** End index (exclusive). */
  end: number;
}

interface SlashHighlightedTextareaProps {
  value: string;
  onInput: (value: string) => void;
  onKeyDown?: (event: KeyboardEvent) => void;
  placeholder: string;
  /** Return the recognized command token range, or null when there is none. */
  commandRange: (value: string) => SlashTokenRange | null;
  /** Receives the underlying textarea (used by SlashCommandMenu positioning). */
  textareaRef?: (el: HTMLTextAreaElement) => void;
  /** Extra textarea classes. */
  class?: string;
}

interface Segment {
  text: string;
  command: boolean;
}

const SlashHighlightedTextarea: Component<SlashHighlightedTextareaProps> = (props) => {
  const [scrollTop, setScrollTop] = createSignal(0);
  const [contentWidth, setContentWidth] = createSignal(0);
  const [composing, setComposing] = createSignal(false);
  let textarea: HTMLTextAreaElement | undefined;

  const segments = createMemo<Segment[]>(() => {
    const text = props.value;
    const range = props.commandRange(text);
    if (!range) return [{ text, command: false }];

    const start = Math.max(0, Math.min(text.length, range.start));
    const end = Math.max(start, Math.min(text.length, range.end));
    if (end <= start) return [{ text, command: false }];

    return [
      { text: text.slice(0, start), command: false },
      { text: text.slice(start, end), command: true },
      { text: text.slice(end), command: false },
    ].filter((segment) => segment.text.length > 0);
  });

  /** Match the mirror to the textarea's content box (excludes a visible scrollbar). */
  const syncWidth = () => {
    const width = textarea?.clientWidth ?? 0;
    if (width > 0) setContentWidth(width);
  };

  onMount(() => {
    if (!textarea) return;
    syncWidth();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(syncWidth);
    observer.observe(textarea);
    onCleanup(() => observer.disconnect());
  });

  // A shorter value (or a programmatic fill) can reset the textarea's scrollTop
  // without firing `scroll`; re-read it after layout so the mirror never stays
  // translated past the new content.
  createEffect(
    on(
      () => props.value,
      () => {
        requestAnimationFrame(() => {
          if (textarea) setScrollTop(textarea.scrollTop);
        });
      },
    ),
  );

  return (
    <div class="relative w-full">
      <div
        aria-hidden="true"
        class="absolute inset-0 overflow-hidden pointer-events-none select-none"
        classList={{ 'opacity-0': composing() }}
      >
        <div
          class="whitespace-pre-wrap break-words font-inherit text-base leading-relaxed text-white px-[5px] pb-[5px]"
          style={{
            transform: `translateY(-${scrollTop()}px)`,
            width: contentWidth() > 0 ? `${contentWidth()}px` : undefined,
          }}
        >
          <For each={segments()}>
            {(segment) => (
              <span
                data-slash-command={segment.command ? '' : undefined}
                style={segment.command ? { color: 'rgb(var(--primary-rgb))' } : undefined}
              >
                {segment.text}
              </span>
            )}
          </For>
        </div>
      </div>

      <textarea
        ref={(el) => {
          textarea = el;
          props.textareaRef?.(el);
        }}
        class={`relative z-10 bg-transparent border-none placeholder:text-white/25 font-inherit text-base leading-relaxed min-h-[40px] max-h-[20vh] outline-none overflow-y-hidden px-[5px] pb-[5px] resize-none w-full focus:overflow-y-auto ${props.class ?? ''}`}
        classList={{ 'text-transparent': !composing(), 'text-white': composing() }}
        style={{ 'caret-color': 'rgb(var(--primary-rgb))' }}
        placeholder={props.placeholder}
        value={props.value}
        onInput={(event) => props.onInput(event.currentTarget.value)}
        onKeyDown={(event) => props.onKeyDown?.(event)}
        onCompositionStart={() => setComposing(true)}
        onCompositionEnd={() => setComposing(false)}
        onScroll={(event) => {
          setScrollTop(event.currentTarget.scrollTop);
          syncWidth();
        }}
      />
    </div>
  );
};

export default SlashHighlightedTextarea;

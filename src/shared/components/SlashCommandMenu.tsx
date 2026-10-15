/**
 * @file SlashCommandMenu.tsx
 * @description Slash command popup menu — appears when `/` is typed in the chat input and injects prompt text into the conversation
 *
 * Usage: <SlashCommandMenu
 *   textareaRef={textareaRef}
 *   inputMessage={inputMessage}
 *   setInputMessage={setInputMessage}
 * />
 * Interaction:
 * - Opens when `/` is typed at the start of a line or after whitespace
 * - Fuzzy-filters in real time as typing continues
 * - ↑↓ to navigate, Enter to select and fill into the input, Escape to close
 * - On selection the query is replaced with the full command name; arguments are injected via $ARGUMENTS and handled by ChatPage
 */

import {
  Component,
  createSignal,
  createMemo,
  createEffect,
  onCleanup,
  For,
  Show,
  Setter,
} from 'solid-js';
import { Portal } from 'solid-js/web';
import {
  getSlashCommands,
  getCommandDisplayDescription,
  getCommandArgumentHint,
  type CommandAction,
} from '../../core/shortcuts';
import { t } from '../../core/i18n';

interface SlashCommandMenuProps {
  /** Textarea DOM ref (used to position the menu and read/write the caret) */
  textareaRef: HTMLTextAreaElement | undefined;
  /** Current textarea text */
  inputMessage: string;
  /** Textarea text setter */
  setInputMessage: Setter<string>;
}

// ---- matching utilities ----

/** Extract the slash command word currently being typed (the last /-prefixed word before the caret) */
function extractSlashQuery(
  text: string,
  cursorPos: number,
): { query: string; start: number; args: string } | null {
  const before = text.slice(0, cursorPos);

  // Find the last /-prefixed word (at line start or after whitespace)
  const slashMatch = before.match(/(?:^|[\s\n])(\/[^\s\n]*)$/);
  if (!slashMatch) return null;

  const fullMatch = slashMatch[0]; // e.g. " /review" or "/review"
  const cmdWithSlash = slashMatch[1]; // "/review"

  // Exclude URLs (e.g. https://...)
  if (cmdWithSlash.length > 1 && cmdWithSlash[1] === '/') return null;

  // Start offset of the slash in the original text
  const startIdx = cursorPos - fullMatch.length + (fullMatch[0] === '/' ? 0 : 1);

  // The user may have typed arguments after the command word; split at the first space
  const spaceIdx = cmdWithSlash.indexOf(' ');
  const query = spaceIdx > 0 ? cmdWithSlash.slice(0, spaceIdx) : cmdWithSlash;
  const args = spaceIdx > 0 ? cmdWithSlash.slice(spaceIdx + 1) : '';

  return { query, start: startIdx, args };
}

/** Simple fuzzy match: query characters appear in order within the target string */
function fuzzyMatch(query: string, target: string): boolean {
  const q = query.toLowerCase();
  const t = target.toLowerCase();
  let qi = 0;
  for (let ti = 0; ti < t.length && qi < q.length; ti++) {
    if (t[ti] === q[qi]) qi++;
  }
  return qi === q.length;
}

const SlashCommandMenu: Component<SlashCommandMenuProps> = (props) => {
  const [open, setOpen] = createSignal(false);
  const [query, setQuery] = createSignal('');
  const [selectedIndex, setSelectedIndex] = createSignal(0);
  const [menuStyle, setMenuStyle] = createSignal<Record<string, string>>({});
  const [slashStartIdx, setSlashStartIdx] = createSignal(0);
  const [slashArgs, setSlashArgs] = createSignal('');
  let menuRef: HTMLDivElement | undefined;
  let justSelected = false;

  // All slash commands
  const allSlashCommands = createMemo(() => getSlashCommands());

  // Fuzzy-filtered command list
  const filteredCommands = createMemo(() => {
    const q = query().slice(1); // strip the leading /
    if (!q) return allSlashCommands();
    return allSlashCommands().filter((cmd) => {
      // Match the command name (minus the / prefix) and the description
      const name = cmd.label.startsWith('/') ? cmd.label.slice(1) : cmd.label;
      return fuzzyMatch(q, name) || fuzzyMatch(q, getCommandDisplayDescription(cmd));
    });
  });

  // Update the menu position
  const updatePosition = () => {
    const ta = props.textareaRef;
    if (!ta) return;
    const rect = ta.getBoundingClientRect();
    setMenuStyle({
      left: `${rect.left}px`,
      bottom: `${window.innerHeight - rect.top + 8}px`,
      width: `${rect.width}px`,
    });
  };

  // Close the menu
  const close = () => {
    setOpen(false);
    setQuery('');
    setSelectedIndex(0);
  };

  // Select a command: place /commandName into the input (keeping arguments the user typed)
  const selectCommand = (cmd: CommandAction) => {
    const ta = props.textareaRef;
    if (!ta) return;

    const text = props.inputMessage;
    const start = slashStartIdx();
    const cursorPos = ta.selectionStart;

    // Keep arguments the user typed (/review src/main.rs → args = "src/main.rs")
    const args = slashArgs();
    const cmdText = args ? `${cmd.label} ${args}` : cmd.label;

    const before = text.slice(0, start);
    const after = text.slice(cursorPos);
    const newText = before + cmdText + after;

    props.setInputMessage(newText);

    // Move focus to the end of the command text
    const newCursorPos = before.length + cmdText.length;
    requestAnimationFrame(() => {
      ta.focus();
      ta.setSelectionRange(newCursorPos, newCursorPos);
    });

    justSelected = true;
    close();
  };
  const handleKeyDown = (e: KeyboardEvent) => {
    if (!open()) return;

    const cmds = filteredCommands();

    switch (e.key) {
      case 'Escape':
        e.preventDefault();
        e.stopPropagation();
        close();
        break;
      case 'ArrowDown':
        e.preventDefault();
        e.stopPropagation();
        setSelectedIndex((i) => Math.min(i + 1, Math.max(0, cmds.length - 1)));
        scrollToSelected();
        break;
      case 'ArrowUp':
        e.preventDefault();
        e.stopPropagation();
        setSelectedIndex((i) => Math.max(i - 1, 0));
        scrollToSelected();
        break;
      case 'Enter':
      case 'Tab': {
        e.preventDefault();
        e.stopPropagation();
        const idx = selectedIndex();
        if (idx >= 0 && idx < cmds.length) {
          selectCommand(cmds[idx]);
        }
        break;
      }
    }
  };

  // Register/unregister capture-phase listeners as the menu opens/closes
  createEffect(() => {
    if (open()) {
      document.addEventListener('keydown', handleKeyDown, true);
    } else {
      document.removeEventListener('keydown', handleKeyDown, true);
    }
  });

  onCleanup(() => {
    document.removeEventListener('keydown', handleKeyDown, true);
  });

  // Scroll the selected item into view (uses the data-selected attribute instead of brittle long CSS selectors)
  const scrollToSelected = () => {
    requestAnimationFrame(() => {
      const el = menuRef?.querySelector('[data-selected="true"]') as HTMLElement;
      el?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
    });
  };

  // ---- watch input changes to detect / triggers ----

  createEffect(() => {
    const text = props.inputMessage;
    const ta = props.textareaRef;
    if (!ta) return;

    const cursorPos = ta.selectionStart;
    const result = extractSlashQuery(text, cursorPos);

    if (result) {
      // Suppress reopening right after selection: if a command was just selected and the query is already a full command name, keep the menu closed
      if (justSelected) {
        const isExactCmd = allSlashCommands().some((c) => c.label === result.query);
        if (isExactCmd) return;
        // The user kept editing (the query is no longer a full command), allow the menu to reopen
        justSelected = false;
      }
      setQuery(result.query);
      setSlashStartIdx(result.start);
      setSlashArgs(result.args);
      setSelectedIndex(0);
      setOpen(true);
      updatePosition();
    } else {
      justSelected = false;
      close();
    }
  });

  // Track scroll/resize and update the menu position
  const onScrollOrResize = () => {
    if (open()) updatePosition();
  };

  // Simpler than a MutationObserver: re-attach inside a createEffect each time the menu opens
  createEffect(() => {
    if (open()) {
      window.addEventListener('scroll', onScrollOrResize, true);
      window.addEventListener('resize', onScrollOrResize);
    } else {
      window.removeEventListener('scroll', onScrollOrResize, true);
      window.removeEventListener('resize', onScrollOrResize);
    }
  });

  onCleanup(() => {
    window.removeEventListener('scroll', onScrollOrResize, true);
    window.removeEventListener('resize', onScrollOrResize);
  });

  // Close on outside click
  const onDocClick = (e: MouseEvent) => {
    if (open() && menuRef && !menuRef.contains(e.target as Node)) {
      close();
    }
  };

  createEffect(() => {
    if (open()) {
      document.addEventListener('mousedown', onDocClick);
    } else {
      document.removeEventListener('mousedown', onDocClick);
    }
  });

  onCleanup(() => {
    document.removeEventListener('mousedown', onDocClick);
  });

  // ---- rendering ----

  return (
    <Portal>
      <div
        ref={menuRef}
        class="fixed z-[9998] max-h-[280px] overflow-y-auto border border-white/[0.08] rounded-[10px] p-1.5 bg-[var(--popup-bg)] backdrop-blur-[24px] shadow-[0_0_0_1px_var(--border-dim),0_8px_32px_rgba(0,0,0,0.5)] transition-all duration-100 ease-out"
        style={{ ...menuStyle(), animation: 'slash-menu-in 0.1s ease-out' }}
        classList={{
          'invisible opacity-0 translate-y-1 pointer-events-none': !(
            open() && filteredCommands().length > 0
          ),
          'visible opacity-100 translate-y-0 pointer-events-auto':
            open() && filteredCommands().length > 0,
        }}
      >
        <For each={filteredCommands()}>
          {(cmd, idx) => {
            const isSelected = () => idx() === selectedIndex();
            return (
              <div
                data-selected={isSelected()}
                class={`flex items-center justify-between gap-3 px-2.5 py-[7px] rounded-md cursor-pointer transition-colors duration-[80ms] ${isSelected() ? 'bg-white/[0.10]' : 'hover:bg-white/[0.04]'}`}
                onClick={() => selectCommand(cmd)}
                onMouseEnter={() => setSelectedIndex(idx())}
              >
                <div class="flex items-center gap-1.5 min-w-0">
                  <span class="text-[13px] font-semibold text-white/85 whitespace-nowrap font-mono">
                    {cmd.label}
                  </span>
                  <Show when={getCommandArgumentHint(cmd)}>
                    <span class="text-[11px] text-white/30 italic whitespace-nowrap overflow-hidden text-ellipsis">
                      {getCommandArgumentHint(cmd)}
                    </span>
                  </Show>
                </div>
                <span class="text-[11px] text-white/30 whitespace-nowrap overflow-hidden text-ellipsis shrink text-right">
                  {getCommandDisplayDescription(cmd)}
                </span>
              </div>
            );
          }}
        </For>
        <div class="flex items-center justify-center gap-4 px-4 pt-2 pb-2.5 border-t border-t-white/[0.04] text-[10px] text-white/20">
          <span>
            <kbd>↑↓</kbd> {t('command.palette.hintNavigate')}
          </span>
          <span>
            <kbd>Enter</kbd> {t('common.select')}
          </span>
          <span>
            <kbd>Esc</kbd> {t('command.palette.hintClose')}
          </span>
        </div>
      </div>
    </Portal>
  );
};

export default SlashCommandMenu;

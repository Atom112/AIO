/**
 * @file CommandPalette.tsx
 * @description Command palette component — VS Code-style global command search and execution entry
 *
 * Trigger: Ctrl+K by default (customizable in settings)
 *
 * Features:
 * - Fuzzy search over all registered commands
 * - Grouped display by category
 * - Keyboard navigation (↑↓ to select, Enter to run, Escape to close)
 * - Shows each command's current shortcut
 *
 * Filtering algorithm:
 * - Chinese/English substring match (case-insensitive)
 * - Initialism match (e.g. "ts" matches "Toggle Sidebar")
 * - Multi-keyword, space-separated (every keyword must match)
 */

import {
  Component,
  createSignal,
  createMemo,
  createEffect,
  onMount,
  onCleanup,
  For,
  Show,
} from 'solid-js';
import {
  commandPaletteOpen,
  setCommandPaletteOpen,
  getRegisteredCommands,
  getShortcutKeys,
  executeCommand,
  registerCommand,
  unregisterCommand,
  formatShortcutForDisplay,
  getCommandDisplayLabel,
  getCommandDisplayDescription,
  type CommandAction,
  type CommandCategory,
} from '../../core/shortcuts';
import { t, type TranslationKey } from '../../core/i18n';

// ---- Category display config ----

const CATEGORY_LABELS: Record<CommandCategory, TranslationKey> = {
  navigation: 'command.category.navigation',
  chat: 'command.category.chat',
  sidebar: 'command.category.sidebar',
  global: 'command.category.global',
};

const CATEGORY_COLORS: Record<CommandCategory, string> = {
  navigation: 'rgba(var(--primary-rgb), 0.6)',
  chat: 'rgba(144, 200, 144, 0.6)',
  sidebar: 'rgba(200, 170, 120, 0.6)',
  global: 'rgba(180, 140, 200, 0.6)',
};

// ---- Fuzzy matching helpers ----

/** Check whether a single keyword matches a command. */
function matchWord(word: string, cmd: CommandAction): boolean {
  const w = word.toLowerCase();
  const label = getCommandDisplayLabel(cmd).toLowerCase();
  const desc = getCommandDisplayDescription(cmd).toLowerCase();

  // Substring match.
  if (label.includes(w) || desc.includes(w)) return true;

  // Initialism match (first letter of each word in the label).
  const initials = label
    .split(/\s+/)
    .map((s) => s[0])
    .join('')
    .toLowerCase();
  if (initials.includes(w)) return true;

  // (Removed a dead "pinyin initials" branch: mapping every label char to its
  // own first char is a no-op — the result equals the label itself, which the
  // substring match above already covers. Real pinyin-initial matching would
  // need a romanization table; until then, Chinese labels match by substring.)

  return false;
}

/** Check whether a query matches a command (space-separated keywords, all must match). */
function matchesQuery(query: string, cmd: CommandAction): boolean {
  if (!query.trim()) return true;
  const words = query.trim().split(/\s+/);
  return words.every((w) => matchWord(w, cmd));
}

// ---- Component ----

const CommandPalette: Component = () => {
  const [query, setQuery] = createSignal('');
  const [selectedIndex, setSelectedIndex] = createSignal(0);
  let inputRef: HTMLInputElement | undefined;
  let listRef: HTMLDivElement | undefined;

  // All registered commands.
  const allCommands = createMemo(() => getRegisteredCommands());

  // Filtered results.
  const filteredCommands = createMemo(() => {
    const q = query();
    return allCommands().filter((cmd) => !cmd.isSlashCommand && matchesQuery(q, cmd));
  });

  // Grouped results.
  const groupedCommands = createMemo(() => {
    const groups: {
      category: CommandCategory;
      label: string;
      color: string;
      items: CommandAction[];
    }[] = [];
    const seen = new Set<CommandCategory>();
    const cmds = filteredCommands();

    for (const cmd of cmds) {
      // Defensive: the registry may hold commands without category metadata (registerCommand
      // does not fill it for unknown ids). Fall back to global and warn so t(undefined) cannot crash the render tree.
      if (!CATEGORY_LABELS[cmd.category]) {
        console.warn(
          `[commands] 命令 ${cmd.id} 缺少元数据注册（shortcuts.ts 内置表），已兜底到 global 分组`,
        );
      }
      const category: CommandCategory =
        cmd.category && CATEGORY_LABELS[cmd.category] ? cmd.category : 'global';
      if (!seen.has(category)) {
        seen.add(category);
        groups.push({
          category,
          label: t(CATEGORY_LABELS[category]),
          color: CATEGORY_COLORS[category] || 'rgba(var(--text-base-rgb),0.4)',
          items: [],
        });
      }
      const group = groups.find((g) => g.category === category)!;
      group.items.push(cmd);
    }

    return groups;
  });

  // Flattened index (for keyboard navigation).
  const flatIndex = createMemo(() => {
    const result: CommandAction[] = [];
    for (const g of groupedCommands()) {
      for (const cmd of g.items) {
        result.push(cmd);
      }
    }
    return result;
  });

  // Reset state and focus the input when the palette opens.
  createEffect(() => {
    if (commandPaletteOpen()) {
      setQuery('');
      setSelectedIndex(0);
      // The DOM may not be rendered yet; wait a frame before focusing.
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          inputRef?.focus();
        });
      });
    }
  });

  // Reset the selected index when the query changes.
  createEffect(() => {
    query(); // track dependency
    setSelectedIndex(0);
  });

  // Close the palette.
  const close = () => {
    setCommandPaletteOpen(false);
  };

  // Execute a command and close.
  const execute = (actionId: string) => {
    close();
    setTimeout(() => executeCommand(actionId), 50);
  };

  // Scroll the selected item into view.
  const scrollToSelected = () => {
    requestAnimationFrame(() => {
      // FE-05: 属性选择器（class 串是非法/误配选择器，此前恒 null）
      const selected = listRef?.querySelector('[data-selected]') as HTMLElement;
      selected?.scrollIntoView({ block: 'nearest' });
    });
  };

  // Keyboard navigation.
  const onKeyDown = (e: KeyboardEvent) => {
    const flat = flatIndex();

    if (e.key === 'Escape') {
      e.preventDefault();
      close();
      return;
    }

    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setSelectedIndex((i) => Math.min(i + 1, Math.max(0, flat.length - 1)));
      scrollToSelected();
      return;
    }

    if (e.key === 'ArrowUp') {
      e.preventDefault();
      setSelectedIndex((i) => Math.max(i - 1, 0));
      scrollToSelected();
      return;
    }

    if (e.key === 'Enter') {
      e.preventDefault();
      const idx = selectedIndex();
      if (idx >= 0 && idx < flat.length) {
        execute(flat[idx].id);
      }
      return;
    }
  };

  // Close on overlay click.
  const onOverlayClick = (e: MouseEvent) => {
    if (e.target === e.currentTarget) {
      close();
    }
  };

  // ---- Lifecycle: register the command-palette command itself ----

  onMount(() => {
    registerCommand({
      id: 'command-palette',
      label: '命令面板',
      description: '打开命令面板，搜索并执行所有可用命令',
      category: 'navigation',
      defaultKeys: 'Ctrl+K',
      handler: () => {
        setCommandPaletteOpen(true);
      },
    });
  });

  onCleanup(() => {
    unregisterCommand('command-palette');
  });

  // ---- Render ----

  return (
    <Show when={commandPaletteOpen()}>
      {/* Overlay */}
      <div
        class="fixed inset-0 z-[9999] flex items-start justify-center pt-[18vh] bg-black/50"
        style={{
          'backdrop-filter': 'blur(6px)',
          animation: 'command-palette-fade-in 0.12s ease-out',
        }}
        onClick={onOverlayClick}
      >
        {/* Card */}
        <div
          class="w-[600px] max-h-[460px] flex flex-col border border-white/[0.08] rounded-[14px] overflow-hidden"
          style={{
            background: 'rgba(var(--surface-alt-bg), 0.95)',
            'box-shadow':
              '0 0 0 1px rgba(var(--text-base-rgb), 0.04), 0 16px 48px rgba(0, 0, 0, 0.5)',
            animation: 'command-palette-slide-in 0.15s cubic-bezier(0.16, 1, 0.3, 1)',
          }}
          onClick={(e) => e.stopPropagation()}
        >
          {/* Search input */}
          <div class="flex items-center gap-2.5 px-4 py-3.5 border-b border-b-white/[0.06]">
            <svg
              class="text-white/25 shrink-0"
              width="16"
              height="16"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <circle cx="11" cy="11" r="8" />
              <line x1="21" y1="21" x2="16.65" y2="16.65" />
            </svg>
            <input
              ref={inputRef}
              type="text"
              class="flex-1 bg-transparent border-none outline-none text-white/90 text-[15px] leading-[1.4]"
              placeholder={t('command.palette.placeholder')}
              value={query()}
              onInput={(e) => {
                setQuery(e.currentTarget.value);
              }}
              onKeyDown={onKeyDown}
            />
          </div>

          {/* Command list */}
          <div ref={listRef} class="flex-1 overflow-y-auto p-2 max-h-[340px]">
            <Show
              when={flatIndex().length > 0}
              fallback={
                <div class="py-8 px-4 text-center text-[13px] text-white/25">
                  {t('command.palette.empty')}
                </div>
              }
            >
              <For each={groupedCommands()}>
                {(group) => (
                  <div class="mb-1">
                    <div
                      class="pt-1.5 pb-1 px-3 text-[10px] font-semibold tracking-[0.06em] uppercase"
                      style={{ color: group.color }}
                    >
                      {group.label}
                    </div>
                    <For each={group.items}>
                      {(cmd) => {
                        const globalIdx = () => flatIndex().indexOf(cmd);
                        const isSelected = () => globalIdx() === selectedIndex();
                        const keys = () => getShortcutKeys(cmd.id);

                        return (
                          <div
                            // FE-05: data-selected 锚点——class 串当选择器恒 null，
                            // 键盘导航的 scrollIntoView 从未执行过
                            data-selected={isSelected() ? 'true' : undefined}
                            class={`flex items-center justify-between px-3 py-2 rounded-lg cursor-pointer transition-colors duration-100 gap-3${isSelected() ? ' selected' : ''}`}
                            onClick={() => execute(cmd.id)}
                            onMouseEnter={() => setSelectedIndex(globalIdx())}
                          >
                            <div class="flex flex-col min-w-0 flex-1">
                              <span class="text-[13px] font-medium text-white/85 leading-[1.3]">
                                {getCommandDisplayLabel(cmd)}
                              </span>
                              <span class="text-[11px] text-white/30 leading-[1.3] mt-px whitespace-nowrap overflow-hidden text-ellipsis">
                                {getCommandDisplayDescription(cmd)}
                              </span>
                            </div>
                            <Show when={keys()}>
                              <span class="text-[11px] font-medium text-white/35 bg-white/[0.05] border border-white/[0.06] rounded-[5px] px-[7px] py-0.5 whitespace-nowrap shrink-0 tracking-[0.02em]">
                                {formatShortcutForDisplay(keys())}
                              </span>
                            </Show>
                          </div>
                        );
                      }}
                    </For>
                  </div>
                )}
              </For>
            </Show>
          </div>

          {/* Footer hints */}
          <div class="flex items-center justify-center gap-4 px-4 pt-2 pb-2.5 border-t border-t-white/[0.04] text-[10px] text-white/20">
            <span>
              <kbd>↑↓</kbd> {t('command.palette.hintNavigate')}
            </span>
            <span>
              <kbd>Enter</kbd> {t('command.palette.hintRun')}
            </span>
            <span>
              <kbd>Esc</kbd> {t('command.palette.hintClose')}
            </span>
          </div>
        </div>
      </div>
    </Show>
  );
};

export default CommandPalette;

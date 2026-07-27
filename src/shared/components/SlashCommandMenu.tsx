/**
 * @file SlashCommandMenu.tsx
 * @description 斜杠命令浮动菜单 — 在聊天输入框中输入 `/` 时弹出，注入提示词到对话
 *
 * 用法：<SlashCommandMenu
 *   textareaRef={textareaRef}
 *   inputMessage={inputMessage}
 *   setInputMessage={setInputMessage}
 * />
 * 交互：
 * - 在输入框行首或空格后输入 `/` 时弹出
 * - 继续输入时实时模糊过滤
 * - ↑↓ 导航，Enter 选中填入输入框，Escape 关闭
 * - 选中后替换为完整命令名；参数通过 $ARGUMENTS 注入由 ChatPage 处理
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
  /** 输入框 DOM 引用（用于定位菜单和读写光标） */
  textareaRef: HTMLTextAreaElement | undefined;
  /** 输入框当前文本 */
  inputMessage: string;
  /** 输入框文本 setter */
  setInputMessage: Setter<string>;
}

// ---- 匹配工具 ----

/** 从文本中提取当前正在输入的斜杠命令词（光标位置之前的最后一个以 / 开头的词） */
function extractSlashQuery(
  text: string,
  cursorPos: number,
): { query: string; start: number; args: string } | null {
  const before = text.slice(0, cursorPos);

  // 找到最后一个以 / 开头的词（在行首或空格后）
  const slashMatch = before.match(/(?:^|[\s\n])(\/[^\s\n]*)$/);
  if (!slashMatch) return null;

  const fullMatch = slashMatch[0]; // 如 " /review" 或 "/review"
  const cmdWithSlash = slashMatch[1]; // "/review"

  // 排除 URL（如 https://...）
  if (cmdWithSlash.length > 1 && cmdWithSlash[1] === '/') return null;

  // 斜杠在原文中的起始位置
  const startIdx = cursorPos - fullMatch.length + (fullMatch[0] === '/' ? 0 : 1);

  // 用户可能在命令词后输入了参数，用第一个空格分割
  const spaceIdx = cmdWithSlash.indexOf(' ');
  const query = spaceIdx > 0 ? cmdWithSlash.slice(0, spaceIdx) : cmdWithSlash;
  const args = spaceIdx > 0 ? cmdWithSlash.slice(spaceIdx + 1) : '';

  return { query, start: startIdx, args };
}

/** 简单模糊匹配：查询字符按顺序出现在目标字符串中 */
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

  // 所有斜杠命令
  const allSlashCommands = createMemo(() => getSlashCommands());

  // 模糊过滤后的命令列表
  const filteredCommands = createMemo(() => {
    const q = query().slice(1); // 去掉开头的 /
    if (!q) return allSlashCommands();
    return allSlashCommands().filter((cmd) => {
      // 匹配命令名（去掉 / 前缀）和描述
      const name = cmd.label.startsWith('/') ? cmd.label.slice(1) : cmd.label;
      return fuzzyMatch(q, name) || fuzzyMatch(q, getCommandDisplayDescription(cmd));
    });
  });

  // 更新菜单位置
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

  // 关闭菜单
  const close = () => {
    setOpen(false);
    setQuery('');
    setSelectedIndex(0);
  };

  // 选中命令：在输入框放置 /commandName（保留用户已输入的参数）
  const selectCommand = (cmd: CommandAction) => {
    const ta = props.textareaRef;
    if (!ta) return;

    const text = props.inputMessage;
    const start = slashStartIdx();
    const cursorPos = ta.selectionStart;

    // 保留用户已输入的参数（/review src/main.rs → args = "src/main.rs"）
    const args = slashArgs();
    const cmdText = args ? `${cmd.label} ${args}` : cmd.label;

    const before = text.slice(0, start);
    const after = text.slice(cursorPos);
    const newText = before + cmdText + after;

    props.setInputMessage(newText);

    // 焦点移到命令文本末尾
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

  // 菜单打开/关闭时注册/注销捕获阶段监听
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

  // 滚动到选中项（使用 data-selected 属性避免脆弱的长 CSS 选择器）
  const scrollToSelected = () => {
    requestAnimationFrame(() => {
      const el = menuRef?.querySelector('[data-selected="true"]') as HTMLElement;
      el?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
    });
  };

  // ---- 监听输入变化，检测 / 触发 ----

  createEffect(() => {
    const text = props.inputMessage;
    const ta = props.textareaRef;
    if (!ta) return;

    const cursorPos = ta.selectionStart;
    const result = extractSlashQuery(text, cursorPos);

    if (result) {
      // 菜单选中后抑制重开：如果刚选中且当前查询是完整命令名，不弹菜单
      if (justSelected) {
        const isExactCmd = allSlashCommands().some((c) => c.label === result.query);
        if (isExactCmd) return;
        // 用户继续编辑了（查询已不是完整命令），允许菜单重新弹出
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

  // 监听滚动/窗口大小变化，更新菜单位置
  const onScrollOrResize = () => {
    if (open()) updatePosition();
  };

  // 使用 MutationObserver 以外，更简单的方式：在 createEffect 里每次打开时更新
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

  // 点击外部关闭
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

  // ---- 渲染 ----

  return (
    <Portal>
      <div
        ref={menuRef}
        class="fixed z-[9998] max-h-[280px] overflow-y-auto border border-white/[0.08] rounded-[10px] p-1.5 bg-[rgba(18,22,38,0.96)] shadow-[0_0_0_1px_var(--border-dim),0_8px_32px_rgba(0,0,0,0.5)] transition-all duration-100 ease-out"
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

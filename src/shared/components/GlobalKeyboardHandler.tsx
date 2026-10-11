/**
 * @file GlobalKeyboardHandler.tsx
 * @description Global keyboard shortcut listener component (no UI), mounted in Layout
 *
 * Responsibilities:
 * 1. Listen to document keydown events
 * 2. Normalize the key combo and look up the matching registered command
 * 3. Execute the command handler
 * 4. Escape priority: command palette → user-registered Escape commands (e.g. stop generation) → browser default
 *
 * Edge cases:
 * - When an input is focused, only whitelisted shortcuts such as Escape/Enter pass through
 * - When the command palette is open, Escape closes the palette first
 */

import { onMount, onCleanup } from 'solid-js';
import {
  normalizeKeyCombo,
  getActionByKeys,
  executeCommand,
  commandPaletteOpen,
  setCommandPaletteOpen,
  shortcutRecording,
  isEditableTarget,
  isInputPassthrough,
} from '../../core/shortcuts';

const GlobalKeyboardHandler = () => {
  const onKeyDown = (event: KeyboardEvent) => {
    // P3（录制互斥）：设置页正在录制快捷键时，全局处理器必须完全让位——
    // 录制器自己持有 capture 监听；此处再响应会双重触发（录制 Ctrl+K 的
    // 同时打开命令面板；Escape 被录成绑定的同时关闭面板）。
    if (shortcutRecording()) return;

    const combo = normalizeKeyCombo(event);
    if (!combo) return;

    // Escape priority chain: command palette > registered commands
    if (combo === 'Escape') {
      // Priority 1: close the command palette
      if (commandPaletteOpen()) {
        event.preventDefault();
        event.stopPropagation();
        setCommandPaletteOpen(false);
        return;
      }
      // Priority 2: run a registered Escape command (e.g. stop generation, close modal)
      // falls through to the lookup logic below
    }

    // Look up the matching command
    const actionId = getActionByKeys(combo);
    if (!actionId) return;

    // When an input is focused, only whitelisted shortcuts pass through
    const targetIsEditable = isEditableTarget(event.target);
    if (targetIsEditable && !isInputPassthrough(combo)) {
      return;
    }

    // Execute the command
    event.preventDefault();
    event.stopPropagation();
    executeCommand(actionId);
  };

  onMount(() => {
    document.addEventListener('keydown', onKeyDown, { capture: true });
  });

  onCleanup(() => {
    document.removeEventListener('keydown', onKeyDown, { capture: true });
  });

  // Pure logic component, renders no DOM
  return null;
};

export default GlobalKeyboardHandler;

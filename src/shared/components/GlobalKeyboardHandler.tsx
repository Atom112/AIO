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
  isEditableTarget,
  isInputPassthrough,
} from '../../core/shortcuts';

const GlobalKeyboardHandler = () => {
  const onKeyDown = (event: KeyboardEvent) => {
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

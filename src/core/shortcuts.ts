/**
 * @file shortcuts.ts
 * @description Core module of the shortcut system: command registry, shortcut binding management, and keyboard event normalization.
 *
 * Architecture:
 * - commandRegistry: declarative list of all registered commands (registered by components in onMount)
 * - shortcutBindings: user-defined shortcut map (actionId → keys), persisted to localStorage
 * - commandPaletteOpen: globally accessible signal for the command palette open state
 * - normalizeKeyCombo: normalizes a KeyboardEvent into a canonical form such as "Ctrl+K"
 */

import { createSignal } from 'solid-js';
import { t, type TranslationKey } from './i18n';

// ---- Type definitions ----

/** Command category */
export type CommandCategory = 'navigation' | 'chat' | 'sidebar' | 'global';

/** A single registrable command */
export interface CommandAction {
  id: string;
  label: string;
  description: string;
  category: CommandCategory;
  defaultKeys: string; // default shortcut, e.g. "Ctrl+K"
  // Action-style slash commands may receive argument text (e.g. `/compact 10000`);
  // a returned string is shown to the user as that command's feedback message (rendered by ChatPage)
  handler: (args?: string) => void | string | Promise<void | string>;
  /** Slash command: prompt body injected into the chat message (contains the $ARGUMENTS placeholder) */
  promptBody?: string;
  /** Slash command: argument placeholder hint text, e.g. "[file path]" */
  argumentHint?: string;
  /** Marks a slash command; appears in the input box / menu */
  isSlashCommand?: boolean;
}

// ---- Command registry ----

const commandMap = new Map<string, CommandAction>();

type CommandTranslation = {
  label?: TranslationKey;
  description: TranslationKey;
  prompt?: TranslationKey;
  argumentHint?: TranslationKey;
};

const COMMAND_TRANSLATIONS: Record<string, CommandTranslation> = {
  'command-palette': {
    label: 'command.commandPalette.label',
    description: 'command.commandPalette.description',
  },
  'toggle-left-sidebar': {
    label: 'command.toggleLeftSidebar.label',
    description: 'command.toggleLeftSidebar.description',
  },
  'toggle-right-sidebar': {
    label: 'command.toggleRightSidebar.label',
    description: 'command.toggleRightSidebar.description',
  },
  'go-to-settings': {
    label: 'command.goToSettings.label',
    description: 'command.goToSettings.description',
  },
  'go-to-chat': { label: 'command.goToChat.label', description: 'command.goToChat.description' },
  'new-topic': { label: 'command.newTopic.label', description: 'command.newTopic.description' },
  'new-assistant': {
    label: 'command.newAssistant.label',
    description: 'command.newAssistant.description',
  },
  'new-project': {
    label: 'command.newProject.label',
    description: 'command.newProject.description',
  },
  'toggle-web-search': {
    label: 'command.toggleWebSearch.label',
    description: 'command.toggleWebSearch.description',
  },
  'cycle-reasoning': {
    label: 'command.cycleReasoning.label',
    description: 'command.cycleReasoning.description',
  },
  'cycle-agent-mode': {
    label: 'command.cycleAgentMode.label',
    description: 'command.cycleAgentMode.description',
  },
  'stop-generation': {
    label: 'command.stopGeneration.label',
    description: 'command.stopGeneration.description',
  },
  'focus-input': {
    label: 'command.focusInput.label',
    description: 'command.focusInput.description',
  },
  'upload-file': {
    label: 'command.uploadFile.label',
    description: 'command.uploadFile.description',
  },
  'prev-assistant': {
    label: 'command.prevAssistant.label',
    description: 'command.prevAssistant.description',
  },
  'next-assistant': {
    label: 'command.nextAssistant.label',
    description: 'command.nextAssistant.description',
  },
  'prev-topic': { label: 'command.prevTopic.label', description: 'command.prevTopic.description' },
  'next-topic': { label: 'command.nextTopic.label', description: 'command.nextTopic.description' },
  'slash-clear': { description: 'slash.clear.description' },
  'slash-compact': { description: 'slash.compact.description' },
  'slash-review': {
    description: 'slash.review.description',
    prompt: 'slash.review.prompt',
    argumentHint: 'slash.review.hint',
  },
  'slash-explain': {
    description: 'slash.explain.description',
    prompt: 'slash.explain.prompt',
    argumentHint: 'slash.explain.hint',
  },
  'slash-fix': {
    description: 'slash.fix.description',
    prompt: 'slash.fix.prompt',
    argumentHint: 'slash.fix.hint',
  },
  'slash-optimize': {
    description: 'slash.optimize.description',
    prompt: 'slash.optimize.prompt',
    argumentHint: 'slash.optimize.hint',
  },
  'slash-translate': {
    description: 'slash.translate.description',
    prompt: 'slash.translate.prompt',
    argumentHint: 'slash.translate.hint',
  },
  'slash-summarize': {
    description: 'slash.summarize.description',
    prompt: 'slash.summarize.prompt',
    argumentHint: 'slash.summarize.hint',
  },
  'slash-search': { description: 'slash.search.description' },
  'slash-settings': { description: 'slash.settings.description' },
  'slash-help': { description: 'slash.help.description' },
  'slash-btw': { description: 'slash.btw.description', argumentHint: 'slash.btw.hint' },
};

export function getCommandDisplayLabel(command: CommandAction): string {
  const key = COMMAND_TRANSLATIONS[command.id]?.label;
  return key ? t(key) : command.label;
}

export function getCommandDisplayDescription(command: CommandAction): string {
  const key = COMMAND_TRANSLATIONS[command.id]?.description;
  return key ? t(key) : command.description;
}

export function getCommandArgumentHint(command: CommandAction): string | undefined {
  const key = COMMAND_TRANSLATIONS[command.id]?.argumentHint;
  return key ? t(key) : command.argumentHint;
}

function getCommandPromptBody(command: CommandAction): string | undefined {
  const key = COMMAND_TRANSLATIONS[command.id]?.prompt;
  return key ? t(key) : command.promptBody;
}

/** Command registry version signal - incremented on every register/unregister to drive createMemo recomputation */
const [commandVersion, setCommandVersion] = createSignal(0);

/** no-op placeholder handler */
const NOOP_HANDLER = () => {};

/** Register a command (components call this in onMount to override the handler) */
export function registerCommand(
  action: CommandAction | Pick<CommandAction, 'id' | 'handler'>,
): string {
  const id = action.id;
  if (commandMap.has(id)) {
    // already exists → update the handler only (keep metadata)
    const existing = commandMap.get(id)!;
    existing.handler = action.handler;
  } else {
    commandMap.set(id, action as CommandAction);
  }
  setCommandVersion((v) => v + 1);
  return id;
}

/** Unregister a command's handler (reset to no-op); metadata is kept for the settings page */
export function unregisterCommand(id: string): void {
  const existing = commandMap.get(id);
  if (existing) {
    existing.handler = NOOP_HANDLER;
    setCommandVersion((v) => v + 1);
  }
}

/** Get a read-only list of all registered commands (reactive: depends on commandVersion) */
export function getRegisteredCommands(): CommandAction[] {
  commandVersion(); // track the version so createMemo recomputes on register/unregister
  return Array.from(commandMap.values());
}

/** Get all slash commands (isSlashCommand === true) for the input box / menu */
export function getSlashCommands(): CommandAction[] {
  commandVersion();
  return Array.from(commandMap.values()).filter((c) => c.isSlashCommand === true);
}

/** Slash command resolution result */
export interface ResolvedSlashCommand {
  /** The matched command */
  command: CommandAction;
  /** Argument text after the command name (/review src/main.rs → "src/main.rs") */
  args: string;
  /** Text with $ARGUMENTS replaced in promptBody; set only when promptBody is non-empty */
  resolvedBody?: string;
}

/**
 * Resolve a slash command from user input
 * @returns the match, or null if the input does not start with / or no command matches
 */
export function resolveSlashCommand(input: string): ResolvedSlashCommand | null {
  const trimmed = input.trim();
  if (!trimmed.startsWith('/')) return null;

  const spaceIdx = trimmed.indexOf(' ');
  const cmdName = spaceIdx > 0 ? trimmed.slice(0, spaceIdx) : trimmed;
  const args = spaceIdx > 0 ? trimmed.slice(spaceIdx + 1).trim() : '';

  const allSlash = getSlashCommands();
  const cmd = allSlash.find((c) => c.label === cmdName);
  if (!cmd) return null;

  const result: ResolvedSlashCommand = { command: cmd, args };
  const promptBody = getCommandPromptBody(cmd);
  if (promptBody) {
    result.resolvedBody = promptBody.replace(/\$ARGUMENTS/g, args);
  }
  return result;
}

// ---- Shortcut binding management ----

const STORAGE_KEY = 'aio-shortcut-bindings';

/** Load custom shortcut bindings from localStorage */
function loadBindings(): Record<string, string> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return JSON.parse(raw) as Record<string, string>;
  } catch (e) {
    console.warn('[shortcuts] 加载快捷键绑定失败，使用默认值', e);
  }
  return {};
}

/** Persist shortcut bindings to localStorage */
function saveBindings(bindings: Record<string, string>): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(bindings));
  } catch (e) {
    console.warn('[shortcuts] 保存快捷键绑定失败', e);
  }
}

// reactive bindings signal
const [bindings, setBindings] = createSignal<Record<string, string>>(loadBindings());

/** Get the current shortcut of a command (user-defined first, otherwise the default) */
export function getShortcutKeys(actionId: string): string {
  const custom = bindings()[actionId];
  if (custom) return custom;
  const cmd = commandMap.get(actionId);
  return cmd?.defaultKeys ?? '';
}

/** Modify a shortcut binding and persist it */
export function setShortcutBinding(actionId: string, keys: string): void {
  setBindings((prev) => {
    const next = { ...prev };
    if (keys) {
      next[actionId] = keys;
    } else {
      delete next[actionId];
    }
    saveBindings(next);
    return next;
  });
}

/** Reset a command's shortcut to its default */
export function resetShortcutBinding(actionId: string): void {
  setBindings((prev) => {
    if (!(actionId in prev)) return prev; // already the default
    const next = { ...prev };
    delete next[actionId];
    saveBindings(next);
    return next;
  });
}

/** Reset all shortcuts to their defaults */
export function resetAllShortcutBindings(): void {
  setBindings({});
  saveBindings({});
}

/** Check whether a command's shortcut has been user-modified */
export function isShortcutModified(actionId: string): boolean {
  return actionId in bindings();
}

/** Reverse lookup: find the command ID for a shortcut string (used for conflict detection) */
export function getActionByKeys(keys: string): string | null {
  // check user-defined bindings first
  for (const [id, k] of Object.entries(bindings())) {
    if (k === keys) return id;
  }
  // then default bindings (skipping ones overridden by a custom binding)
  for (const [id, cmd] of commandMap) {
    if (id in bindings()) continue; // overridden by a custom binding, skip
    if (cmd.defaultKeys === keys) return id;
  }
  return null;
}

// ---- Command execution ----

/** Execute a command (look up by ID and call its handler) */
export function executeCommand(actionId: string): boolean {
  const cmd = commandMap.get(actionId);
  if (cmd) {
    try {
      cmd.handler();
    } catch (e) {
      console.error(`[shortcuts] 执行命令 "${actionId}" 时出错:`, e);
    }
    return true;
  }
  return false;
}

// ---- Command palette state ----

export const [commandPaletteOpen, setCommandPaletteOpen] = createSignal(false);

// ---- Keyboard event normalization ----

/** Modifier key names */
const MODIFIER_KEYS = ['Control', 'Alt', 'Shift', 'Meta'];

/**
 * Normalize a KeyboardEvent into a shortcut string
 * @returns e.g. "Ctrl+K", "Ctrl+Shift+N", "Escape", "F2"; null if not a valid shortcut
 */
export function normalizeKeyCombo(event: KeyboardEvent): string | null {
  // ignore bare modifier presses (Ctrl/Alt/Shift/Meta with no other key)
  if (MODIFIER_KEYS.includes(event.key)) return null;

  // ignore NumLock, CapsLock, and similar lock keys
  if (event.key === 'NumLock' || event.key === 'CapsLock' || event.key === 'ScrollLock')
    return null;

  // ignore IME composition input
  if (event.isComposing) return null;

  const parts: string[] = [];

  if (event.ctrlKey) parts.push('Ctrl');
  if (event.altKey) parts.push('Alt');
  if (event.shiftKey) parts.push('Shift');
  if (event.metaKey) parts.push('Meta');

  // main key name
  let mainKey = event.key;

  // normalize special key names
  if (mainKey === ' ') {
    mainKey = 'Space';
  } else if (mainKey.length === 1) {
    // single-character keys: uppercase letters; digits/symbols stay as-is
    if (mainKey >= 'a' && mainKey <= 'z') {
      mainKey = mainKey.toUpperCase();
    }
  }
  // other keys (ArrowUp, Enter, Escape, F1-F12, etc.) stay as-is

  parts.push(mainKey);
  return parts.join('+');
}

/**
 * Convert a shortcut string into a platform-readable form (for UI display)
 * - macOS: Ctrl/Alt/Shift/Meta are replaced with the ⌃⌥⇧⌘ symbols
 * - Windows/Linux: Meta is replaced with Win, everything else stays as-is
 */
export function formatShortcutForDisplay(keys: string): string {
  if (!keys) return keys;
  const isMac = /Mac/i.test(navigator.userAgent);
  if (isMac) {
    return keys
      .replace(/\bCtrl\b/g, '⌃')
      .replace(/\bAlt\b/g, '⌥')
      .replace(/\bShift\b/g, '⇧')
      .replace(/\bMeta\b/g, '⌘');
  }
  return keys.replace(/\bMeta\b/g, 'Win');
}

// ---- Predicate helpers ----

/**
 * Check whether focus is currently in an editable element (input/textarea/contenteditable)
 * Used to decide whether global shortcuts fire
 */
export function isEditableTarget(target: EventTarget | null): boolean {
  if (!target || !(target instanceof HTMLElement)) return false;
  const tag = target.tagName.toLowerCase();
  if (tag === 'input' || tag === 'textarea' || tag === 'select') return true;
  if (target.isContentEditable) return true;
  return false;
}

/**
 * Check whether a shortcut can still fire while an input box is focused
 * Only Escape (stop generation/close dialogs) and Enter (send message) may fire while typing
 */
export function isInputPassthrough(keys: string): boolean {
  return keys === 'Escape' || keys === 'Enter';
}

// ---- Module-level pre-registration: command metadata (with no-op handlers) ----
// These calls make every command's metadata visible on any page (e.g. the settings page).
// Components override the real handler via registerCommand in onMount
// and reset it to no-op via unregisterCommand in onCleanup.

registerCommand({
  id: 'command-palette',
  label: t('cmd.command-palette.label'),
  description: t('cmd.command-palette.description'),
  category: 'navigation',
  defaultKeys: 'Ctrl+K',
  handler: NOOP_HANDLER,
});
registerCommand({
  id: 'toggle-left-sidebar',
  label: t('cmd.toggle-left-sidebar.label'),
  description: t('cmd.toggle-left-sidebar.description'),
  category: 'navigation',
  defaultKeys: 'Ctrl+B',
  handler: NOOP_HANDLER,
});
registerCommand({
  id: 'toggle-right-sidebar',
  label: t('cmd.toggle-right-sidebar.label'),
  description: t('cmd.toggle-right-sidebar.description'),
  category: 'navigation',
  defaultKeys: 'Ctrl+Alt+B',
  handler: NOOP_HANDLER,
});
registerCommand({
  id: 'go-to-settings',
  label: t('cmd.go-to-settings.label'),
  description: t('cmd.go-to-settings.description'),
  category: 'navigation',
  defaultKeys: 'Ctrl+,',
  handler: NOOP_HANDLER,
});
registerCommand({
  id: 'go-to-chat',
  label: t('cmd.go-to-chat.label'),
  description: t('cmd.go-to-chat.description'),
  category: 'navigation',
  defaultKeys: 'Ctrl+1',
  handler: NOOP_HANDLER,
});
registerCommand({
  id: 'new-topic',
  label: t('cmd.new-topic.label'),
  description: t('cmd.new-topic.description'),
  category: 'chat',
  defaultKeys: 'Ctrl+N',
  handler: NOOP_HANDLER,
});
// new-project: ChatPage only overrides the handler via registerCommand({id, handler}).
// Its metadata must be pre-registered here like every other command: without a category,
// the command map stores an object that crashes CommandPalette grouping on t(undefined),
// taking down the whole render tree.
registerCommand({
  id: 'new-project',
  label: t('command.newProject.label'),
  description: t('command.newProject.description'),
  category: 'sidebar',
  defaultKeys: 'Ctrl+Shift+N',
  handler: NOOP_HANDLER,
});
registerCommand({
  id: 'new-assistant',
  label: t('cmd.new-assistant.label'),
  description: t('cmd.new-assistant.description'),
  category: 'chat',
  defaultKeys: 'Ctrl+Shift+N',
  handler: NOOP_HANDLER,
});
registerCommand({
  id: 'toggle-web-search',
  label: t('cmd.toggle-web-search.label'),
  description: t('cmd.toggle-web-search.description'),
  category: 'chat',
  defaultKeys: 'Ctrl+Shift+S',
  handler: NOOP_HANDLER,
});
registerCommand({
  id: 'cycle-reasoning',
  label: t('cmd.cycle-reasoning.label'),
  description: t('cmd.cycle-reasoning.description'),
  category: 'chat',
  defaultKeys: 'Ctrl+Shift+R',
  handler: NOOP_HANDLER,
});
registerCommand({
  id: 'cycle-agent-mode',
  label: t('cmd.cycle-agent-mode.label'),
  description: t('cmd.cycle-agent-mode.description'),
  category: 'chat',
  defaultKeys: 'Ctrl+Shift+M',
  handler: NOOP_HANDLER,
});
registerCommand({
  id: 'stop-generation',
  label: t('cmd.stop-generation.label'),
  description: t('cmd.stop-generation.description'),
  category: 'chat',
  defaultKeys: 'Escape',
  handler: NOOP_HANDLER,
});
registerCommand({
  id: 'focus-input',
  label: t('cmd.focus-input.label'),
  description: t('cmd.focus-input.description'),
  category: 'chat',
  defaultKeys: 'Ctrl+I',
  handler: NOOP_HANDLER,
});
registerCommand({
  id: 'upload-file',
  label: t('cmd.upload-file.label'),
  description: t('cmd.upload-file.description'),
  category: 'chat',
  defaultKeys: 'Ctrl+U',
  handler: NOOP_HANDLER,
});
registerCommand({
  id: 'prev-assistant',
  label: t('cmd.prev-assistant.label'),
  description: t('cmd.prev-assistant.description'),
  category: 'sidebar',
  defaultKeys: 'Ctrl+[',
  handler: NOOP_HANDLER,
});
registerCommand({
  id: 'next-assistant',
  label: t('cmd.next-assistant.label'),
  description: t('cmd.next-assistant.description'),
  category: 'sidebar',
  defaultKeys: 'Ctrl+]',
  handler: NOOP_HANDLER,
});
registerCommand({
  id: 'prev-topic',
  label: t('cmd.prev-topic.label'),
  description: t('cmd.prev-topic.description'),
  category: 'sidebar',
  defaultKeys: 'Ctrl+Shift+[',
  handler: NOOP_HANDLER,
});
registerCommand({
  id: 'next-topic',
  label: t('cmd.next-topic.label'),
  description: t('cmd.next-topic.description'),
  category: 'sidebar',
  defaultKeys: 'Ctrl+Shift+]',
  handler: NOOP_HANDLER,
});

// ---- Module-level pre-registration: slash commands (inject prompts into the conversation) ----

registerCommand({
  id: 'slash-clear',
  label: t('cmd.slash-clear.label'),
  description: t('cmd.slash-clear.description'),
  category: 'chat',
  defaultKeys: '',
  handler: NOOP_HANDLER,
  isSlashCommand: true,
  promptBody: '',
  argumentHint: undefined,
});

registerCommand({
  id: 'slash-compact',
  label: t('cmd.slash-compact.label'),
  description: t('cmd.slash-compact.description'),
  category: 'chat',
  defaultKeys: '',
  handler: NOOP_HANDLER,
  isSlashCommand: true,
  promptBody: '',
  argumentHint: 'slash.compact.hint',
});

registerCommand({
  id: 'slash-review',
  label: t('cmd.slash-review.label'),
  description: t('cmd.slash-review.description'),
  category: 'chat',
  defaultKeys: '',
  handler: NOOP_HANDLER,
  isSlashCommand: true,
  promptBody: '请审查当前项目的代码变更，找出潜在问题、bug 和改进建议。$ARGUMENTS',
  argumentHint: '[文件或目录]',
});

registerCommand({
  id: 'slash-explain',
  label: t('cmd.slash-explain.label'),
  description: t('cmd.slash-explain.description'),
  category: 'chat',
  defaultKeys: '',
  handler: NOOP_HANDLER,
  isSlashCommand: true,
  promptBody: '请解释以下代码的功能和逻辑：\n\n$ARGUMENTS',
  argumentHint: '[代码或文件]',
});

registerCommand({
  id: 'slash-fix',
  label: t('cmd.slash-fix.label'),
  description: t('cmd.slash-fix.description'),
  category: 'chat',
  defaultKeys: '',
  handler: NOOP_HANDLER,
  isSlashCommand: true,
  promptBody: '请修复以下问题，并说明原因和修改方案：\n\n$ARGUMENTS',
  argumentHint: '[问题描述]',
});

registerCommand({
  id: 'slash-optimize',
  label: t('cmd.slash-optimize.label'),
  description: t('cmd.slash-optimize.description'),
  category: 'chat',
  defaultKeys: '',
  handler: NOOP_HANDLER,
  isSlashCommand: true,
  promptBody: '请优化以下代码，提升性能或可读性，并说明优化点：\n\n$ARGUMENTS',
  argumentHint: '[代码]',
});

registerCommand({
  id: 'slash-translate',
  label: t('cmd.slash-translate.label'),
  description: t('cmd.slash-translate.description'),
  category: 'chat',
  defaultKeys: '',
  handler: NOOP_HANDLER,
  isSlashCommand: true,
  promptBody: '请将以下内容翻译成中文：\n\n$ARGUMENTS',
  argumentHint: '[文本]',
});

registerCommand({
  id: 'slash-summarize',
  label: t('cmd.slash-summarize.label'),
  description: t('cmd.slash-summarize.description'),
  category: 'chat',
  defaultKeys: '',
  handler: NOOP_HANDLER,
  isSlashCommand: true,
  promptBody: '请总结以下内容的关键要点：\n\n$ARGUMENTS',
  argumentHint: '[文本或文件]',
});

registerCommand({
  id: 'slash-search',
  label: t('cmd.slash-search.label'),
  description: t('cmd.slash-search.description'),
  category: 'chat',
  defaultKeys: '',
  handler: NOOP_HANDLER,
  isSlashCommand: true,
  promptBody: '',
  argumentHint: undefined,
});

registerCommand({
  id: 'slash-settings',
  label: t('cmd.slash-settings.label'),
  description: t('cmd.slash-settings.description'),
  category: 'navigation',
  defaultKeys: '',
  handler: NOOP_HANDLER,
  isSlashCommand: true,
  promptBody: '',
  argumentHint: undefined,
});

registerCommand({
  id: 'slash-help',
  label: t('cmd.slash-help.label'),
  description: t('cmd.slash-help.description'),
  category: 'chat',
  defaultKeys: '',
  handler: NOOP_HANDLER,
  isSlashCommand: true,
  promptBody: '',
  argumentHint: undefined,
});

registerCommand({
  id: 'slash-btw',
  label: t('cmd.slash-btw.label'),
  description: t('cmd.slash-btw.description'),
  category: 'chat',
  defaultKeys: '',
  handler: NOOP_HANDLER,
  isSlashCommand: true,
  promptBody: '',
  argumentHint: '[question]',
});

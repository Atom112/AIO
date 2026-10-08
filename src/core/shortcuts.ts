/**
 * @file shortcuts.ts
 * @description 快捷键系统核心模块：命令注册表、快捷键绑定管理、键盘事件标准化
 *
 * 架构说明：
 * - commandRegistry: 所有已注册命令的声明式列表（由各组件在 onMount 中注册）
 * - shortcutBindings: 用户自定义的快捷键映射（actionId → keys），持久化到 localStorage
 * - commandPaletteOpen: 命令面板开关状态信号（全局可访问）
 * - normalizeKeyCombo: 将 KeyboardEvent 标准化为 "Ctrl+K" 等规范格式
 */

import { createSignal } from 'solid-js';
import { t, type TranslationKey } from './i18n';

// ---- 类型定义 ----

/** 命令类别 */
export type CommandCategory = 'navigation' | 'chat' | 'sidebar' | 'global';

/** 单个可注册命令 */
export interface CommandAction {
  id: string;
  label: string;
  description: string;
  category: CommandCategory;
  defaultKeys: string; // 默认快捷键，如 "Ctrl+K"
  handler: (args?: string) => void; // Action 型斜杠命令可接收参数文本（如 `/compact 10000`）
  /** 斜杠命令：注入到聊天消息的提示词正文（含 $ARGUMENTS 占位符） */
  promptBody?: string;
  /** 斜杠命令：参数占位符提示文本，如 "[file path]" */
  argumentHint?: string;
  /** 标记为斜杠命令，出现在输入框 / 菜单中 */
  isSlashCommand?: boolean;
}

// ---- 命令注册表 ----

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

/** 命令注册表版本号信号 — 每次 register/unregister 自增，驱动 createMemo 重新计算 */
const [commandVersion, setCommandVersion] = createSignal(0);

/** no-op 占位处理器 */
const NOOP_HANDLER = () => {};

/** 注册一个命令（组件在 onMount 中调用以覆盖处理器） */
export function registerCommand(
  action: CommandAction | Pick<CommandAction, 'id' | 'handler'>,
): string {
  const id = action.id;
  if (commandMap.has(id)) {
    // 已存在 → 仅更新处理器（保留元数据）
    const existing = commandMap.get(id)!;
    existing.handler = action.handler;
  } else {
    commandMap.set(id, action as CommandAction);
  }
  setCommandVersion((v) => v + 1);
  return id;
}

/** 注销命令的处理器（重置为 no-op），保留元数据供设置页显示 */
export function unregisterCommand(id: string): void {
  const existing = commandMap.get(id);
  if (existing) {
    existing.handler = NOOP_HANDLER;
    setCommandVersion((v) => v + 1);
  }
}

/** 获取所有已注册命令的只读列表（响应式：依赖 commandVersion） */
export function getRegisteredCommands(): CommandAction[] {
  commandVersion(); // 追踪版本号，确保 createMemo 在注册/注销时重新计算
  return Array.from(commandMap.values());
}

/** 获取所有斜杠命令（isSlashCommand === true），用于输入框 / 菜单 */
export function getSlashCommands(): CommandAction[] {
  commandVersion();
  return Array.from(commandMap.values()).filter((c) => c.isSlashCommand === true);
}

/** 斜杠命令解析结果 */
export interface ResolvedSlashCommand {
  /** 匹配到的命令 */
  command: CommandAction;
  /** 命令名之后的参数文本（/review src/main.rs → "src/main.rs"） */
  args: string;
  /** promptBody 中 $ARGUMENTS 替换后的文本，仅当 promptBody 非空时有值 */
  resolvedBody?: string;
}

/**
 * 解析用户输入中的斜杠命令
 * @returns 匹配结果；若非 / 开头或无匹配命令返回 null
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

// ---- 快捷键绑定管理 ----

const STORAGE_KEY = 'aio-shortcut-bindings';

/** 从 localStorage 加载自定义快捷键绑定 */
function loadBindings(): Record<string, string> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return JSON.parse(raw) as Record<string, string>;
  } catch (e) {
    console.warn('[shortcuts] 加载快捷键绑定失败，使用默认值', e);
  }
  return {};
}

/** 持久化快捷键绑定到 localStorage */
function saveBindings(bindings: Record<string, string>): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(bindings));
  } catch (e) {
    console.warn('[shortcuts] 保存快捷键绑定失败', e);
  }
}

// 响应式绑定信号
const [bindings, setBindings] = createSignal<Record<string, string>>(loadBindings());

/** 获取某个命令的当前快捷键（用户自定义优先，否则默认） */
export function getShortcutKeys(actionId: string): string {
  const custom = bindings()[actionId];
  if (custom) return custom;
  const cmd = commandMap.get(actionId);
  return cmd?.defaultKeys ?? '';
}

/** 修改快捷键绑定并持久化 */
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

/** 重置某个命令的快捷键为默认值 */
export function resetShortcutBinding(actionId: string): void {
  setBindings((prev) => {
    if (!(actionId in prev)) return prev; // 已经是默认值
    const next = { ...prev };
    delete next[actionId];
    saveBindings(next);
    return next;
  });
}

/** 重置所有快捷键为默认值 */
export function resetAllShortcutBindings(): void {
  setBindings({});
  saveBindings({});
}

/** 检查某个命令的快捷键是否已被用户修改 */
export function isShortcutModified(actionId: string): boolean {
  return actionId in bindings();
}

/** 反向查找：根据快捷键字符串找到对应的命令 ID（用于冲突检测） */
export function getActionByKeys(keys: string): string | null {
  // 先查用户自定义绑定
  for (const [id, k] of Object.entries(bindings())) {
    if (k === keys) return id;
  }
  // 再查默认绑定（排除已被自定义覆盖的）
  for (const [id, cmd] of commandMap) {
    if (id in bindings()) continue; // 已被自定义覆盖，跳过
    if (cmd.defaultKeys === keys) return id;
  }
  return null;
}

// ---- 命令执行 ----

/** 执行命令（按 ID 查找并调用 handler） */
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

// ---- 命令面板状态 ----

export const [commandPaletteOpen, setCommandPaletteOpen] = createSignal(false);

// ---- 键盘事件标准化 ----

/** 修饰键名称列表 */
const MODIFIER_KEYS = ['Control', 'Alt', 'Shift', 'Meta'];

/**
 * 将 KeyboardEvent 标准化为快捷键字符串
 * @returns 如 "Ctrl+K", "Ctrl+Shift+N", "Escape", "F2"；如果不是有效快捷键则返回 null
 */
export function normalizeKeyCombo(event: KeyboardEvent): string | null {
  // 忽略纯修饰键按下（只有 Ctrl/Alt/Shift/Meta 没有其他键）
  if (MODIFIER_KEYS.includes(event.key)) return null;

  // 忽略数字锁定、大小写锁定等
  if (event.key === 'NumLock' || event.key === 'CapsLock' || event.key === 'ScrollLock')
    return null;

  // 忽略 IME 组合输入
  if (event.isComposing) return null;

  const parts: string[] = [];

  if (event.ctrlKey) parts.push('Ctrl');
  if (event.altKey) parts.push('Alt');
  if (event.shiftKey) parts.push('Shift');
  if (event.metaKey) parts.push('Meta');

  // 获取主键名
  let mainKey = event.key;

  // 规范化特殊键名
  if (mainKey === ' ') {
    mainKey = 'Space';
  } else if (mainKey.length === 1) {
    // 单字符键：字母大写，数字/符号保持原样
    if (mainKey >= 'a' && mainKey <= 'z') {
      mainKey = mainKey.toUpperCase();
    }
  }
  // 其他键（ArrowUp, Enter, Escape, F1-F12 等）保持原样

  parts.push(mainKey);
  return parts.join('+');
}

/**
 * 将快捷键字符串转换为平台可读形式（用于 UI 显示）
 * - macOS：将 Ctrl/Alt/Shift/Meta 替换为 ⌃⌥⇧⌘ 符号
 * - Windows/Linux：将 Meta 替换为 Win，其余保持原样
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

// ---- 条件判断工具 ----

/**
 * 判断当前焦点是否在可编辑元素中（input/textarea/contenteditable）
 * 用于决定是否触发全局快捷键
 */
export function isEditableTarget(target: EventTarget | null): boolean {
  if (!target || !(target instanceof HTMLElement)) return false;
  const tag = target.tagName.toLowerCase();
  if (tag === 'input' || tag === 'textarea' || tag === 'select') return true;
  if (target.isContentEditable) return true;
  return false;
}

/**
 * 判断该快捷键在输入框聚焦时是否仍然可以触发
 * 仅 Escape（停止生成/关闭弹窗）和 Enter（发送消息）允许在输入时触发
 */
export function isInputPassthrough(keys: string): boolean {
  return keys === 'Escape' || keys === 'Enter';
}

// ---- 模块级预注册：命令元数据（含 no-op 处理器） ----
// 这些调用确保所有命令的元数据在任何页面都可见（如设置页）。
// 各组件在 onMount 中通过 registerCommand 覆盖真实处理器，
// 在 onCleanup 中通过 unregisterCommand 重置为 no-op。

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
// new-project：ChatPage 经 registerCommand({id, handler}) 只覆盖处理器，
// 此前缺这条内置元数据注册 → 命令表存入无 category 的对象 →
// CommandPalette 分组时 t(undefined) 崩溃整个渲染树（项目设置弹窗打不开、
// 导航高亮失效）。元数据必须在此预注册（同其他命令）。
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

// ---- 模块级预注册：斜杠命令（注入提示词到对话） ----

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

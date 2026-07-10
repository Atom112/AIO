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

import { createSignal, createMemo } from 'solid-js';

// ---- 类型定义 ----

/** 命令类别 */
export type CommandCategory = 'navigation' | 'chat' | 'sidebar' | 'global';

/** 单个可注册命令 */
export interface CommandAction {
    id: string;
    label: string;
    description: string;
    category: CommandCategory;
    defaultKeys: string;         // 默认快捷键，如 "Ctrl+K"
    handler: () => void;
    /** 斜杠命令：注入到聊天消息的提示词正文（含 $ARGUMENTS 占位符） */
    promptBody?: string;
    /** 斜杠命令：参数占位符提示文本，如 "[file path]" */
    argumentHint?: string;
    /** 标记为斜杠命令，出现在输入框 / 菜单中 */
    isSlashCommand?: boolean;
}

// ---- 命令注册表 ----

const commandMap = new Map<string, CommandAction>();

/** 命令注册表版本号信号 — 每次 register/unregister 自增，驱动 createMemo 重新计算 */
const [commandVersion, setCommandVersion] = createSignal(0);

/** no-op 占位处理器 */
const NOOP_HANDLER = () => {};

/** 注册一个命令（组件在 onMount 中调用以覆盖处理器） */
export function registerCommand(action: CommandAction): string {
    const id = action.id;
    if (commandMap.has(id)) {
        // 已存在 → 仅更新处理器（保留元数据）
        const existing = commandMap.get(id)!;
        existing.handler = action.handler;
    } else {
        commandMap.set(id, { ...action });
    }
    setCommandVersion(v => v + 1);
    return id;
}

/** 注销命令的处理器（重置为 no-op），保留元数据供设置页显示 */
export function unregisterCommand(id: string): void {
    const existing = commandMap.get(id);
    if (existing) {
        existing.handler = NOOP_HANDLER;
        setCommandVersion(v => v + 1);
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
    return Array.from(commandMap.values()).filter(c => c.isSlashCommand === true);
}

/** 根据 ID 查找命令 */
export function getCommandById(id: string): CommandAction | undefined {
    return commandMap.get(id);
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
    setBindings(prev => {
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
    setBindings(prev => {
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

/** 修饰键在快捷键字符串中的显示顺序 */
const MODIFIER_ORDER = ['Ctrl', 'Alt', 'Shift', 'Meta'];

/** 修饰键映射：event.key → 显示名 */
const MODIFIER_MAP: Record<string, string> = {
    'Control': 'Ctrl',
    'Alt': 'Alt',
    'Shift': 'Shift',
    'Meta': 'Meta',
};

/**
 * 将 KeyboardEvent 标准化为快捷键字符串
 * @returns 如 "Ctrl+K", "Ctrl+Shift+N", "Escape", "F2"；如果不是有效快捷键则返回 null
 */
export function normalizeKeyCombo(event: KeyboardEvent): string | null {
    // 忽略纯修饰键按下（只有 Ctrl/Alt/Shift/Meta 没有其他键）
    if (MODIFIER_KEYS.includes(event.key)) return null;

    // 忽略数字锁定、大小写锁定等
    if (event.key === 'NumLock' || event.key === 'CapsLock' || event.key === 'ScrollLock') return null;

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
 * 解析快捷键字符串为可读形式（用于 UI 显示）
 * 目前直接返回原字符串，未来可加入平台适配（如 Mac 上将 Ctrl 显示为 ⌘）
 */
export function formatShortcutForDisplay(keys: string): string {
    return keys;
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

registerCommand({ id: 'command-palette',      label: '命令面板',       description: '打开命令面板，搜索并执行所有可用命令',           category: 'navigation', defaultKeys: 'Ctrl+K',        handler: NOOP_HANDLER });
registerCommand({ id: 'toggle-left-sidebar',  label: '切换左侧边栏',   description: '显示或隐藏助手列表侧边栏',                       category: 'navigation', defaultKeys: 'Ctrl+B',        handler: NOOP_HANDLER });
registerCommand({ id: 'toggle-right-sidebar', label: '切换右侧边栏',   description: '显示或隐藏话题列表侧边栏',                       category: 'navigation', defaultKeys: 'Ctrl+Alt+B',    handler: NOOP_HANDLER });
registerCommand({ id: 'go-to-settings',       label: '打开设置',       description: '导航到应用设置页面',                             category: 'navigation', defaultKeys: 'Ctrl+,',        handler: NOOP_HANDLER });
registerCommand({ id: 'go-to-chat',           label: '回到聊天',       description: '导航到聊天页面',                                 category: 'navigation', defaultKeys: 'Ctrl+1',        handler: NOOP_HANDLER });
registerCommand({ id: 'new-topic',            label: '新建话题',       description: '在当前助手中创建新的话题',                       category: 'chat',        defaultKeys: 'Ctrl+N',        handler: NOOP_HANDLER });
registerCommand({ id: 'new-assistant',        label: '新建助手',       description: '创建新的 AI 助手',                              category: 'chat',        defaultKeys: 'Ctrl+Shift+N', handler: NOOP_HANDLER });
registerCommand({ id: 'toggle-web-search',    label: '切换联网搜索',   description: '开启或关闭联网搜索功能',                         category: 'chat',        defaultKeys: 'Ctrl+Shift+S', handler: NOOP_HANDLER });
registerCommand({ id: 'cycle-reasoning',      label: '切换推理强度',   description: '循环切换推理深度：关闭 → 低 → 中 → 高',         category: 'chat',        defaultKeys: 'Ctrl+Shift+R', handler: NOOP_HANDLER });
registerCommand({ id: 'cycle-agent-mode',     label: '切换 Agent 模式',description: '循环切换 Agent 执行模式：对话 → 普通 → 自动 → 计划', category: 'chat',     defaultKeys: 'Ctrl+Shift+M', handler: NOOP_HANDLER });
registerCommand({ id: 'stop-generation',      label: '停止生成',       description: '停止当前 AI 回复的生成',                        category: 'chat',        defaultKeys: 'Escape',       handler: NOOP_HANDLER });
registerCommand({ id: 'focus-input',          label: '聚焦输入框',     description: '将光标聚焦到消息输入框',                         category: 'chat',        defaultKeys: 'Ctrl+I',       handler: NOOP_HANDLER });
registerCommand({ id: 'upload-file',          label: '上传文件',       description: '打开文件选择对话框上传文件',                     category: 'chat',        defaultKeys: 'Ctrl+U',       handler: NOOP_HANDLER });
registerCommand({ id: 'prev-assistant',       label: '上一个助手',     description: '切换到上一个助手',                               category: 'sidebar',     defaultKeys: 'Ctrl+[',       handler: NOOP_HANDLER });
registerCommand({ id: 'next-assistant',       label: '下一个助手',     description: '切换到下一个助手',                               category: 'sidebar',     defaultKeys: 'Ctrl+]',       handler: NOOP_HANDLER });
registerCommand({ id: 'prev-topic',           label: '上一个话题',     description: '切换到上一个话题',                               category: 'sidebar',     defaultKeys: 'Ctrl+Shift+[', handler: NOOP_HANDLER });
registerCommand({ id: 'next-topic',           label: '下一个话题',     description: '切换到下一个话题',                               category: 'sidebar',     defaultKeys: 'Ctrl+Shift+]', handler: NOOP_HANDLER });

// ---- 模块级预注册：斜杠命令（注入提示词到对话） ----

registerCommand({
    id: 'slash-clear', label: '/clear', description: '清空当前对话历史，开始新会话',
    category: 'chat', defaultKeys: '', handler: NOOP_HANDLER,
    isSlashCommand: true, promptBody: '', argumentHint: undefined,
});

registerCommand({
    id: 'slash-compact', label: '/compact', description: '压缩上下文，总结对话后继续',
    category: 'chat', defaultKeys: '', handler: NOOP_HANDLER,
    isSlashCommand: true, promptBody: '', argumentHint: undefined,
});

registerCommand({
    id: 'slash-review', label: '/review', description: '审查代码变更，找出潜在问题和改进建议',
    category: 'chat', defaultKeys: '', handler: NOOP_HANDLER,
    isSlashCommand: true,
    promptBody: '请审查当前项目的代码变更，找出潜在问题、bug 和改进建议。$ARGUMENTS',
    argumentHint: '[文件或目录]',
});

registerCommand({
    id: 'slash-explain', label: '/explain', description: '解释代码的功能和逻辑',
    category: 'chat', defaultKeys: '', handler: NOOP_HANDLER,
    isSlashCommand: true,
    promptBody: '请解释以下代码的功能和逻辑：\n\n$ARGUMENTS',
    argumentHint: '[代码或文件]',
});

registerCommand({
    id: 'slash-fix', label: '/fix', description: '修复代码问题或错误',
    category: 'chat', defaultKeys: '', handler: NOOP_HANDLER,
    isSlashCommand: true,
    promptBody: '请修复以下问题，并说明原因和修改方案：\n\n$ARGUMENTS',
    argumentHint: '[问题描述]',
});

registerCommand({
    id: 'slash-optimize', label: '/optimize', description: '优化代码性能或可读性',
    category: 'chat', defaultKeys: '', handler: NOOP_HANDLER,
    isSlashCommand: true,
    promptBody: '请优化以下代码，提升性能或可读性，并说明优化点：\n\n$ARGUMENTS',
    argumentHint: '[代码]',
});

registerCommand({
    id: 'slash-translate', label: '/translate', description: '将内容翻译成中文',
    category: 'chat', defaultKeys: '', handler: NOOP_HANDLER,
    isSlashCommand: true,
    promptBody: '请将以下内容翻译成中文：\n\n$ARGUMENTS',
    argumentHint: '[文本]',
});

registerCommand({
    id: 'slash-summarize', label: '/summarize', description: '总结内容的关键要点',
    category: 'chat', defaultKeys: '', handler: NOOP_HANDLER,
    isSlashCommand: true,
    promptBody: '请总结以下内容的关键要点：\n\n$ARGUMENTS',
    argumentHint: '[文本或文件]',
});

registerCommand({
    id: 'slash-search', label: '/search', description: '切换联网搜索状态',
    category: 'chat', defaultKeys: '', handler: NOOP_HANDLER,
    isSlashCommand: true, promptBody: '', argumentHint: undefined,
});

registerCommand({
    id: 'slash-settings', label: '/settings', description: '打开应用设置页面',
    category: 'navigation', defaultKeys: '', handler: NOOP_HANDLER,
    isSlashCommand: true, promptBody: '', argumentHint: undefined,
});

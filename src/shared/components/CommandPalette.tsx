/**
 * @file CommandPalette.tsx
 * @description 命令面板组件 — VS Code 风格的全局命令搜索与执行入口
 *
 * 触发方式：默认 Ctrl+K（可在设置中自定义）
 *
 * 功能：
 * - 模糊搜索所有已注册命令
 * - 按类别分组显示
 * - 键盘导航（↑↓ 选择，Enter 执行，Escape 关闭）
 * - 显示每个命令的当前快捷键
 *
 * 过滤算法：
 * - 中文/英文子串匹配（不区分大小写）
 * - 首字母缩写匹配（如 "ts" 匹配 "Toggle Sidebar"）
 * - 多关键词空格分隔（每个关键词都必须匹配）
 */

import { Component, createSignal, createMemo, createEffect, onMount, onCleanup, For, Show } from 'solid-js';
import {
    commandPaletteOpen,
    setCommandPaletteOpen,
    getRegisteredCommands,
    getShortcutKeys,
    executeCommand,
    registerCommand,
    unregisterCommand,
    formatShortcutForDisplay,
    type CommandAction,
    type CommandCategory,
} from '../../core/shortcuts';

// ---- 类别显示配置 ----

const CATEGORY_LABELS: Record<CommandCategory, string> = {
    navigation: '导航',
    chat: '聊天',
    sidebar: '侧边栏',
    global: '全局',
};

const CATEGORY_COLORS: Record<CommandCategory, string> = {
    navigation: 'rgba(124, 154, 191, 0.6)',
    chat: 'rgba(144, 200, 144, 0.6)',
    sidebar: 'rgba(200, 170, 120, 0.6)',
    global: 'rgba(180, 140, 200, 0.6)',
};

// ---- 模糊匹配工具 ----

/** 检查单个关键词是否匹配命令 */
function matchWord(word: string, cmd: CommandAction): boolean {
    const w = word.toLowerCase();
    const label = cmd.label.toLowerCase();
    const desc = cmd.description.toLowerCase();

    // 子串匹配
    if (label.includes(w) || desc.includes(w)) return true;

    // 首字母缩写匹配（取 label 中每个单词的首字母）
    const initials = label
        .split(/\s+/)
        .map(s => s[0])
        .join('')
        .toLowerCase();
    if (initials.includes(w)) return true;

    // 中文拼音首字母（取 label 每个字符 — 对中文有效）
    if (/^[a-z]+$/.test(w) && label.length >= w.length) {
        const pyAbbr = Array.from(label)
            .map(ch => ch[0]?.toLowerCase() ?? '')
            .join('');
        if (pyAbbr.includes(w)) return true;
    }

    return false;
}

/** 检查查询字符串是否匹配命令（空格分隔多关键词，全部匹配） */
function matchesQuery(query: string, cmd: CommandAction): boolean {
    if (!query.trim()) return true;
    const words = query.trim().split(/\s+/);
    return words.every(w => matchWord(w, cmd));
}

// ---- 组件 ----

const CommandPalette: Component = () => {
    const [query, setQuery] = createSignal('');
    const [selectedIndex, setSelectedIndex] = createSignal(0);
    let inputRef: HTMLInputElement | undefined;
    let listRef: HTMLDivElement | undefined;

    // 获取所有注册命令
    const allCommands = createMemo(() => getRegisteredCommands());

    // 过滤结果
    const filteredCommands = createMemo(() => {
        const q = query();
        return allCommands().filter(cmd => matchesQuery(q, cmd));
    });

    // 分组结果
    const groupedCommands = createMemo(() => {
        const groups: { category: CommandCategory; label: string; color: string; items: CommandAction[] }[] = [];
        const seen = new Set<CommandCategory>();
        const cmds = filteredCommands();

        for (const cmd of cmds) {
            if (!seen.has(cmd.category)) {
                seen.add(cmd.category);
                groups.push({
                    category: cmd.category,
                    label: CATEGORY_LABELS[cmd.category] || cmd.category,
                    color: CATEGORY_COLORS[cmd.category] || 'rgba(255,255,255,0.4)',
                    items: [],
                });
            }
            const group = groups.find(g => g.category === cmd.category)!;
            group.items.push(cmd);
        }

        return groups;
    });

    // 扁平化索引（用于键盘导航）
    const flatIndex = createMemo(() => {
        const result: CommandAction[] = [];
        for (const g of groupedCommands()) {
            for (const cmd of g.items) {
                result.push(cmd);
            }
        }
        return result;
    });

    // 面板打开时重置状态并聚焦输入框
    createEffect(() => {
        if (commandPaletteOpen()) {
            setQuery('');
            setSelectedIndex(0);
            // DOM 还未渲染时，等待一帧再聚焦
            requestAnimationFrame(() => {
                requestAnimationFrame(() => {
                    inputRef?.focus();
                });
            });
        }
    });

    // 搜索词变化时重置选中索引
    createEffect(() => {
        query(); // 追踪依赖
        setSelectedIndex(0);
    });

    // 关闭面板
    const close = () => {
        setCommandPaletteOpen(false);
    };

    // 执行命令并关闭
    const execute = (actionId: string) => {
        close();
        setTimeout(() => executeCommand(actionId), 50);
    };

    // 滚动到选中项
    const scrollToSelected = () => {
        requestAnimationFrame(() => {
            const selected = listRef?.querySelector('.command-palette-item.selected') as HTMLElement;
            selected?.scrollIntoView({ block: 'nearest' });
        });
    };

    // 键盘导航
    const onKeyDown = (e: KeyboardEvent) => {
        const flat = flatIndex();

        if (e.key === 'Escape') {
            e.preventDefault();
            close();
            return;
        }

        if (e.key === 'ArrowDown') {
            e.preventDefault();
            setSelectedIndex(i => Math.min(i + 1, Math.max(0, flat.length - 1)));
            scrollToSelected();
            return;
        }

        if (e.key === 'ArrowUp') {
            e.preventDefault();
            setSelectedIndex(i => Math.max(i - 1, 0));
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

    // 点击遮罩关闭
    const onOverlayClick = (e: MouseEvent) => {
        if (e.target === e.currentTarget) {
            close();
        }
    };

    // ---- 生命周期：注册 command-palette 命令自身 ----

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

    // ---- 渲染 ----

    return (
        <Show when={commandPaletteOpen()}>
            {/* 遮罩 */}
            <div class="command-palette-overlay" onClick={onOverlayClick}>
                {/* 卡片 */}
                <div class="command-palette-card" onClick={e => e.stopPropagation()}>
                    {/* 搜索框 */}
                    <div class="command-palette-input-wrapper">
                        <svg
                            class="command-palette-search-icon"
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
                            class="command-palette-input"
                            placeholder="输入命令名称搜索..."
                            value={query()}
                            onInput={e => {
                                setQuery(e.currentTarget.value);
                            }}
                            onKeyDown={onKeyDown}
                        />
                    </div>

                    {/* 命令列表 */}
                    <div ref={listRef} class="command-palette-list">
                        <Show
                            when={flatIndex().length > 0}
                            fallback={
                                <div class="command-palette-empty">
                                    没有找到匹配的命令
                                </div>
                            }
                        >
                            <For each={groupedCommands()}>
                                {(group) => (
                                    <div class="command-palette-group">
                                        <div
                                            class="command-palette-category"
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
                                                        class={`command-palette-item${isSelected() ? ' selected' : ''}`}
                                                        onClick={() => execute(cmd.id)}
                                                        onMouseEnter={() => setSelectedIndex(globalIdx())}
                                                    >
                                                        <div class="command-palette-item-left">
                                                            <span class="command-palette-item-label">
                                                                {cmd.label}
                                                            </span>
                                                            <span class="command-palette-item-desc">
                                                                {cmd.description}
                                                            </span>
                                                        </div>
                                                        <Show when={keys()}>
                                                            <span class="command-palette-item-shortcut">
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

                    {/* 底部提示 */}
                    <div class="command-palette-footer">
                        <span><kbd>↑↓</kbd> 导航</span>
                        <span><kbd>Enter</kbd> 执行</span>
                        <span><kbd>Esc</kbd> 关闭</span>
                    </div>
                </div>
            </div>
        </Show>
    );
};

export default CommandPalette;

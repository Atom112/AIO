/**
 * @file GlobalKeyboardHandler.tsx
 * @description 全局键盘快捷键监听组件（无 UI），挂载到 Layout 中
 *
 * 职责：
 * 1. 监听 document 的 keydown 事件
 * 2. 将按键标准化后查找对应的注册命令
 * 3. 执行命令处理器
 * 4. Escape 键优先级：命令面板 → 用户注册的 Escape 命令（如停止生成）→ 浏览器默认
 *
 * 边界条件：
 * - 输入框聚焦时，仅允许 Escape/Enter 等白名单快捷键通过
 * - 命令面板打开时，Escape 优先关闭面板
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

        // Escape 优先级链：命令面板 > 注册命令
        if (combo === 'Escape') {
            // 优先级 1：关闭命令面板
            if (commandPaletteOpen()) {
                event.preventDefault();
                event.stopPropagation();
                setCommandPaletteOpen(false);
                return;
            }
            // 优先级 2：执行注册的 Escape 命令（如停止生成、关闭弹窗）
            // 继续走下面的查找逻辑
        }

        // 查找匹配的命令
        const actionId = getActionByKeys(combo);
        if (!actionId) return;

        // 输入框聚焦时，仅白名单快捷键通过
        const targetIsEditable = isEditableTarget(event.target);
        if (targetIsEditable && !isInputPassthrough(combo)) {
            return;
        }

        // 执行命令
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

    // 纯逻辑组件，不渲染任何 DOM
    return null;
};

export default GlobalKeyboardHandler;

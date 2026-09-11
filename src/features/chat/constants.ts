// 聊天域共享常量 —— ChatPage 与其事件监听器 hook 共用（避免组件 ↔ hook 循环导入）。

/** 中断/取消导致的工具调用占位结果文本（标记给模型看，保证上下文可理解） */
export const TOOL_INTERRUPTED = '[Interrupted by user]';

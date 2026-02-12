/**
 * ============================================================================
 * 文件功能摘要
 * ============================================================================
 * 
 * 【核心功能】
 * 系统提示词（System Prompt）编辑模态框组件，用于为当前助手设置自定义提示词。
 * 提供多行文本编辑、取消/保存操作、进入/退出动画效果。
 * 
 * 【数据流流向】
 * ┌─────────────────────────────────────────────────────────────────────────┐
 * │  外部数据流入 (Props)                                                    │
 * │  ├── show: boolean ← 父组件控制模态框显示/隐藏                           │
 * │  ├── initialPrompt?: string ← 初始提示词内容（可选）                     │
 * │  ├── onSave?: (text: string) => void ← 保存回调，传出编辑后的文本        │
 * │  └── onClose: () => void ← 关闭回调                                      │
 * │                                                                          │
 * │  本地状态                                                                │
 * │  ├── promptText: 文本域当前内容（独立于父组件，编辑时本地管理）          │
 * │  └── isExiting: 控制退出动画状态                                         │
 * │                                                                          │
 * │  数据流出                                                                │
 * │  ├── onSave(promptText) → 保存时将本地文本回调给父组件                   │
 * │  └── onClose() → 关闭模态框（点击遮罩、关闭按钮、取消或保存后）          │
 * │                                                                          │
 * │  本地文件                                                                │
 * │  └── 导入 PromptModal.css 样式文件                                       │
 * └─────────────────────────────────────────────────────────────────────────┘
 * 
 * 【组件层级】
 * PromptModal (本组件)
 * ├── Show 条件渲染（props.show 控制）
 * ├── 遮罩层（点击关闭）
 * └── 模态框内容容器（带动画）
 *     ├── 头部（标题 + 关闭按钮）
 *     ├── 内容区（多行文本域）
 *     └── 底部（取消按钮 + 保存按钮）
 * 
 * 【设计特点】
 * - 受控组件：文本域值由本地 promptText Signal 管理
 * - 乐观编辑：编辑过程中不同步父组件，保存时才回调
 * - 状态重置：每次打开时从 initialPrompt 重新初始化
 * ============================================================================
 */

// SolidJS 核心 API
import { Component, createSignal, createEffect } from 'solid-js';
// SolidJS Web 专属组件：条件渲染（用于 Portal 场景，此处与常规 Show 用法一致）
import { Show } from 'solid-js/web';
// 本地样式文件
import './PromptModal.css';

/**
 * PromptModal 组件属性接口定义
 */
interface PromptModalProps {
    /** 控制模态框的显示/隐藏 */
    show: boolean;
    /** 初始提示词文本，打开模态框时自动填充到编辑区 */
    initialPrompt?: string;
    /**
     * 保存回调函数
     * @param text - 用户编辑后的提示词内容
     */
    onSave?: (text: string) => void;
    /** 关闭模态框回调，点击遮罩、关闭按钮、取消或保存后触发 */
    onClose: () => void;
}

/**
 * 系统提示词设置模态框组件
 * 
 * @component
 * @description 浮窗形式的提示词编辑器，支持多行文本输入。
 *              编辑状态本地管理，保存时才同步回父组件。
 * 
 * @param {PromptModalProps} props - 组件属性
 * @returns {JSX.Element} 提示词模态框 JSX 元素
 */
const PromptModal: Component<PromptModalProps> = (props) => {
    /**
     * 文本域内容状态
     * 
     * 设计说明：独立于父组件状态，避免每次输入都触发父组件重渲染。
     * 仅在保存时通过 onSave 回调将最终值传出。
     */
    const [promptText, setPromptText] = createSignal<string>('');

    /** 退出动画状态：true 时添加退出动画类名 */
    const [isExiting, setIsExiting] = createSignal(false);

    /**
     * 处理关闭模态框（带动画）
     * 
     * 动画流程：
     * 1. 设置 isExiting=true 触发 CSS 退出动画
     * 2. 等待 200ms（CSS 动画时长）
     * 3. 重置状态并调用父组件 onClose
     */
    const handleClose = () => {
        setIsExiting(true);
        setTimeout(() => {
            setIsExiting(false);
            props.onClose();
        }, 200);
    };

    /**
     * 监听模态框显示状态
     * 
     * 副作用逻辑：每当 props.show 变为 true（模态框打开）时，
     * 将 props.initialPrompt 同步到本地 promptText，确保每次打开都是最新初始值。
     * 
     * 数据流：props.initialPrompt → setPromptText → 文本域显示
     */
    createEffect(() => {
        if (props.show) {
            // 使用空字符串作为默认值，避免 undefined
            setPromptText(props.initialPrompt ?? '');
        }
    });

    /**
     * 处理保存操作
     * 
     * 数据流：
     * 1. 检查 onSave 回调是否存在
     * 2. 调用 onSave 传入当前编辑的 promptText
     * 3. 触发关闭流程（带动画）
     */
    const handleSave = () => {
        if (props.onSave) {
            props.onSave(promptText());
        }
        handleClose();
    };

    return (
        // 条件渲染：仅当 props.show 为 true 时渲染模态框
        <Show when={props.show}>
            {/* 
                遮罩层：全屏半透明背景
                点击遮罩本身（非内容区）时关闭模态框
                动态类名控制进入/退出动画
            */}
            <div
                classList={{ 
                    "modal-backdrop": true, 
                    "overlay-out": isExiting()  // 退出动画类
                }}
                class="overlay-in"  // 进入动画类
                onClick={(e) => e.target === e.currentTarget && handleClose()}
            >
                {/* 模态框内容容器：阻止冒泡防止点击内容区关闭 */}
                <div
                    classList={{ 
                        "modal-content": true, 
                        "animate-out": isExiting()  // 退出动画类
                    }}
                    class="animate-in"  // 进入动画类
                >
                    {/* 头部区域：标题和关闭按钮 */}
                    <div class="modal-header">
                        <h2>设置当前模型提示词</h2>
                        {/* 关闭按钮：点击触发带动画的关闭 */}
                        <button onClick={handleClose} class="close-button">
                            &times;  {/* HTML 实体：乘号，作为 X 图标 */}
                        </button>
                    </div>

                    {/* 内容区域：多行文本输入域 */}
                    <div class="modal-body">
                        <textarea
                            rows={8}  // 默认显示 8 行高度
                            value={promptText()}  // 受控组件：绑定本地状态
                            onInput={(e) => setPromptText(e.currentTarget.value)}  // 输入时更新本地状态
                            placeholder="例如：你是一个乐于助人的 AI 助手。"  // 空白时的提示文本
                        />
                    </div>

                    {/* 底部区域：操作按钮组 */}
                    <div class="modal-footer">
                        {/* 取消按钮：关闭模态框，不保存修改 */}
                        <button onClick={handleClose} class="btn-cancel">
                            取消
                        </button>
                        {/* 保存按钮：回调当前文本并关闭 */}
                        <button onClick={handleSave} class="btn-save">
                            保存
                        </button>
                    </div>
                </div>
            </div>
        </Show>
    );
};

// 默认导出组件
export default PromptModal;
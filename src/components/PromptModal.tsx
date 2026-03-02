import { Component, createSignal, createEffect } from 'solid-js';
import { Show } from 'solid-js/web';
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
        <Show when={props.show}>
            <div
                classList={{ 
                    "modal-backdrop": true, 
                    "overlay-out": isExiting()
                }}
                class="overlay-in"
                onClick={(e) => e.target === e.currentTarget && handleClose()}
            >
                <div
                    classList={{ 
                        "modal-content": true, 
                        "animate-out": isExiting()
                    }}
                    class="animate-in" 
                >
                    <div class="modal-header">
                        <h2>设置当前模型提示词</h2>
                        <button onClick={handleClose} class="close-button">
                            &times;
                        </button>
                    </div>

                    <div class="modal-body">
                        <textarea
                            rows={8}
                            value={promptText()}
                            onInput={(e) => setPromptText(e.currentTarget.value)}
                            placeholder="例如：你是一个乐于助人的 AI 助手。"
                        />
                    </div>

                    <div class="modal-footer">
                        <button onClick={handleClose} class="btn-cancel">
                            取消
                        </button>

                        <button onClick={handleSave} class="btn-save">
                            保存
                        </button>
                    </div>
                </div>
            </div>
        </Show>
    );
};

export default PromptModal;
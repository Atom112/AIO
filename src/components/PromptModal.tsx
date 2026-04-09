import { Component, createSignal, createEffect } from 'solid-js';
import { Show } from 'solid-js/web';

/**
 * PromptModal 组件属性接口
 *
 * @interface
 */
interface PromptModalProps {
    show: boolean;              //控制模态框显示/隐藏 
    initialPrompt?: string;     //初始提示词文本（可选）
    onSave?: (text: string) => void;    // 保存回调
    onClose: () => void;    // 关闭模态框回调
}

/**
 * 系统提示词设置模态框组件
 *
 * @param {PromptModalProps} props - 组件属性
 * @returns {JSX.Element}
 */
const PromptModal: Component<PromptModalProps> = (props) => {

    const [promptText, setPromptText] = createSignal<string>('');   //文本域内容状态
    const [isExiting, setIsExiting] = createSignal(false);          //退出动画状态
    const [isEntering, setIsEntering] = createSignal(true);         //入场动画状态

    /**
     * 处理关闭模态框并触发退出动画
     *
     * @returns {void}
     */
    const handleClose = () => {
        setIsExiting(true);
        setTimeout(() => {
            setIsExiting(false);
            props.onClose();
        }, 300);
    };

    /**
     * 打开模态框时同步初始提示词并触发入场动画
     *
     * @returns {void}
     */
    createEffect(() => {
        if (props.show) {
            setPromptText(props.initialPrompt ?? '');
            setIsEntering(true);
            setTimeout(() => setIsEntering(false), 0);
        }
    });

    /**
     * 保存提示词并关闭模态框
     *
     * @returns {void}
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
                    "opacity-0 pointer-events-none": isExiting() || isEntering(),
                    "opacity-100": !isExiting() && !isEntering()
                }}
                class="modal-overlay bg-black/60 z-[1000] rounded-lg transition-all duration-200 ease-out"
                onClick={(e) => e.target === e.currentTarget && handleClose()}
            >
                <div
                    classList={{
                        "scale-95 opacity-0": isExiting() || isEntering(),
                        "scale-100 opacity-100": !isExiting() && !isEntering()
                    }}
                    class="modal-panel bg-dark-500 text-[#e0e0e0] p-6 rounded-lg w-[90%] max-w-[500px] flex flex-col gap-4 transition-all duration-500 ease-out transform"
                >
                    <div class="flex justify-between items-center border-b border-[#444] pb-3">
                        <h2 class='m-0 text-xl'>设置当前模型提示词</h2>
                        <button onClick={handleClose} class="close-btn">
                            &times;
                        </button>
                    </div>
                        <textarea
                            rows={8}
                            value={promptText()}
                            onInput={(e) => setPromptText(e.currentTarget.value)}
                            placeholder="例如：你是一个乐于助人的 AI 助手。"
                            class="w-full p-2.5 bg-dark-300 border border-dark-100 rounded-lg text-[#e0e0e0] text-base font-mono resize-y box-border focus:outline-none focus:border-pri-50" 
                            style="font-family: 'JetBrains Mono', Consolas, Monaco, 'Courier New', monospace !important;"
                        />

                    <div class="flex justify-end gap-3">
                        <button onClick={handleClose} class="px-5 py-2.5 border-0 cursor-pointer font-bold bg-dark-100 text-[#e0e0e0] rounded-lg transition-all duration-200 hover:bg-dark-50">
                            取消
                        </button>

                        <button onClick={handleSave} class="px-5 py-2.5 border-0 cursor-pointer font-bold bg-pri text-black rounded-lg hover:scale-105 transition-all duration-200">
                            保存
                        </button>
                    </div>
                </div>
            </div>
        </Show>
    );
};

export default PromptModal;
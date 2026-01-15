import { Component, createSignal, createEffect } from 'solid-js';
import { Show } from 'solid-js/web';
import './PromptModal.css';

interface PromptModalProps {
  show: boolean;
  initialPrompt?: string;
  onSave?: (text: string) => void;
  onClose: () => void;
}

const PromptModal: Component<PromptModalProps> = (props) => {
  // 使用 createSignal 来管理 textarea 中的文本
  // 这样可以独立于父组件的状态，只在保存时才更新父组件
  const [promptText, setPromptText] = createSignal<string>('');

  // 使用 createEffect 监听 props.show 的变化
  // 当浮窗显示时（props.show 变为 true），用父组件传入的初始值来设置文本域的内容
  createEffect(() => {
    if (props.show) {
      setPromptText(props.initialPrompt ?? '');
    }
  });

  const handleSave = () => {
    // 调用父组件传来的 onSave 函数，并把当前文本作为参数
    if (props.onSave) {
      props.onSave(promptText());
    }
    // 调用父组件传来的 onClose 函数来关闭浮窗
    props.onClose();
  };

  const handleBackdropClick = (e: Event) => {
    // 确保是点击背景本身，而不是点击浮窗内容区域
    if (e.currentTarget === e.target) {
      props.onClose();
    }
  };

  return (
    <Show when={props.show}>
      <div class="modal-backdrop" onClick={handleBackdropClick}>
        <div class="modal-content">
          <div class="modal-header">
            <h2>设置当前模型提示词</h2>
            <button onClick={props.onClose} class="close-button">&times;</button>
          </div>
          <div class="modal-body">
            <textarea
              rows={8}
              value={promptText()}
              onInput={(e) => setPromptText((e.currentTarget as HTMLTextAreaElement).value)}
              placeholder="例如：你是一个乐于助人的 AI 助手。"
            />
          </div>
          <div class="modal-footer">
            <button onClick={props.onClose} class="btn-cancel">取消</button>
            <button onClick={handleSave} class="btn-save">保存</button>
          </div>
        </div>
      </div>
    </Show>
  );
};

export default PromptModal;
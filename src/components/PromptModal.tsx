import { Component, createSignal, createEffect } from 'solid-js';
import { Show } from 'solid-js/web';
import './PromptModal.css';

/**
 * PromptModal 组件属性接口定义
 */
interface PromptModalProps {
  /** 是否显示模态框 */
  show: boolean;
  /** 初始显示的提示词文本 */
  initialPrompt?: string;
  /** 点击保存时的回调函数，返回当前文本域内容 */
  onSave?: (text: string) => void;
  /** 关闭模态框的回调函数（取消或保存后触发） */
  onClose: () => void;
}

/**
 * 系统提示词设置模态框组件
 * 用于编辑助手的 System Prompt，通过浮窗形式呈现
 * 
 * @param props PromptModalProps
 */
const PromptModal: Component<PromptModalProps> = (props) => {
  /** 
   * 管理 TextArea 内部的文本状态
   * 独立于父组件状态，仅在执行保存操作时才同步回父组件
   */
  const [promptText, setPromptText] = createSignal<string>('');

  const [isExiting, setIsExiting] = createSignal(false);

  const handleClose = () => {
    setIsExiting(true);
    setTimeout(() => {
      setIsExiting(false);
      props.onClose();
    }, 200);
  };
  /**
   * 监听模态框显示状态
   * 每当浮窗打开（props.show 变为 true）时，同步父组件传入的初始提示词
   */
  createEffect(() => {
    if (props.show) {
      setPromptText(props.initialPrompt ?? '');
    }
  });

  /**
   * 处理保存逻辑
   * 触发 onSave 回调并关闭模态框
   */
  const handleSave = () => {
    if (props.onSave) {
      props.onSave(promptText());
    }
    handleClose();
  };


  return (
    <Show when={props.show}>
      {/* 遮罩背景 */}
      <div
        classList={{ "modal-backdrop": true, "overlay-out": isExiting() }}
        class="overlay-in"
        onClick={(e) => e.target === e.currentTarget && handleClose()}
      >
        <div
          classList={{ "modal-content": true, "animate-out": isExiting() }}
          class="animate-in"
        >

          {/* 头部：标题与关闭按钮 */}
          <div class="modal-header">
            <h2>设置当前模型提示词</h2>
            <button onClick={handleClose} class="close-button">&times;</button>
          </div>

          {/* 内容区：文本输入域 */}
          <div class="modal-body">
            <textarea
              rows={8}
              value={promptText()}
              onInput={(e) => setPromptText(e.currentTarget.value)}
              placeholder="例如：你是一个乐于助人的 AI 助手。"
            />
          </div>

          {/* 底部：操作按钮 */}
          <div class="modal-footer">
            <button onClick={handleClose} class="btn-cancel">取消</button>
            <button onClick={handleSave} class="btn-save">保存</button>
          </div>

        </div>

      </div>
    </Show>
  );
};

export default PromptModal;
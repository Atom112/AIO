import { Component, For, Show, Setter } from 'solid-js';
import Markdown from './Markdown';
import { Topic, globalUserAvatar, selectedModel } from '../store/store';
import { open } from '@tauri-apps/plugin-dialog';
import './ChatInterface.css';

interface ChatInterfaceProps {
    activeTopic: Topic | null;
    isChangingTopic: boolean;
    isThinking: boolean;
    isProcessing: boolean;
    isDragging: boolean;
    typingIndex: number | null;
    inputMessage: string;
    setInputMessage: Setter<string>;
    pendingFiles: { name: string, content: string, type: 'text' | 'image' }[];
    setPendingFiles: Setter<{ name: string, content: string, type: 'text' | 'image' }[]>;
    handleSendMessage: () => void;
    handleStopGeneration: () => void;
    handleFileUpload: (path: string, type: 'file' | 'image') => Promise<void>;
}

const ChatInterface: Component<ChatInterfaceProps> = (props) => {
    let textareaRef: HTMLTextAreaElement | undefined;

    /**
     * 根据模型名称获取对应的 Logo 图标路径
     */
    const getModelLogo = (modelName: string) => {
        const name = modelName.toLowerCase();
        if (name.includes('gpt')) return '/icons/openai.svg';
        if (name.includes('claude')) return '/icons/claude-color.svg';
        if (name.includes('grok')) return '/icons/grok.svg';
        if (name.includes('gemini')) return '/icons/gemini-color.svg';
        if (name.includes('deepseek')) return '/icons/deepseek-color.svg';
        if (name.includes('qwen') || name.includes('qwq')) return '/icons/qwen-color.svg';
        if (name.includes('kimi') || name.includes('moonshot')) return '/icons/moonshot.svg';
        if (name.includes('doubao')) return '/icons/doubao-color.svg';
        if (name.includes('glm')) return '/icons/zhipu-color.svg';
        return '/icons/ollama.svg';
    };

    return (
        <div class="chat-input-container">
            {/* 消息区域 */}
            <div class="chat-messages-area" classList={{ 'topic-switching': props.isChangingTopic }}>
                <Show when={props.activeTopic}>
                    <For each={props.activeTopic?.history}>
                        {(msg: any, index) => (
                            <div class={`message ${msg.role}`} style={{ "animation-delay": `${Math.min(index() * 0.03, 0.4)}s`, "animation-duration": props.typingIndex === index() ? "0.1s" : "0.35s" }}>
                                <div class="message-wrapper">
                                    <Show when={msg.role === 'assistant'}>
                                        <div class="chat-avatar-container ai">
                                            <img src={getModelLogo(msg.modelId || selectedModel()?.model_id || "")} alt="AI" class="chat-avatar-img" />
                                        </div>
                                    </Show>

                                    <div class="message-body">
                                        <div class="message-content" classList={{ 'typing': props.typingIndex === index() }}>
                                            {/* 文件卡片预览 */}
                                            <Show when={msg.role === 'user' && msg.displayFiles && msg.displayFiles.length > 0}>
                                                <For each={msg.displayFiles}>
                                                    {(file: any) => (
                                                        <div class="file-attachment-card">
                                                            <div class="file-icon-wrapper">
                                                                <svg fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                                                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
                                                                </svg>
                                                            </div>
                                                            <div class="file-info"><div class="file-name">{file.name}</div><div class="file-meta">已解析</div></div>
                                                        </div>
                                                    )}
                                                </For>
                                            </Show>
                                            {/* 文本内容渲染 */}
                                            <div class="message-text-part">
                                                <Markdown content={msg.role === 'user' && msg.displayText !== undefined ? msg.displayText : msg.content} />
                                            </div>
                                        </div>

                                        {/* 模型名称信息 */}
                                        <Show when={msg.role === 'assistant' && (msg.modelId || selectedModel()?.model_id)}>
                                            <div class="message-model-info">{msg.modelId || selectedModel()?.model_id}</div>
                                        </Show>

                                        {/* 快捷操作 */}
                                        <div class="message-actions">
                                            <button class="copy-bubble-button" onClick={(e) => {
                                                const text = msg.role === 'user' && msg.displayText !== undefined ? msg.displayText : msg.content;
                                                if (!text) return;
                                                navigator.clipboard.writeText(text).then(() => {
                                                    const btn = e.currentTarget;
                                                    const label = btn.querySelector('span');
                                                    if (label) {
                                                        const originalText = label.innerText;
                                                        btn.classList.add('copied'); label.innerText = '已复制';
                                                        setTimeout(() => { btn.classList.remove('copied'); label.innerText = originalText; }, 2000);
                                                    }
                                                });
                                            }}>
                                                <svg fill="none" viewBox="0 0 24 24" stroke="currentColor" style="width: 14px; height: 14px;"><path d="M8 5H6a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2v-1M8 5a2 2 0 002 2h2a2 2 0 002-2M8 5a2 2 0 012-2h2a2 2 0 012 2m0 0h2a2 2 0 012 2v3m2 4H10m0 0l3-3m-3 3l3 3" /></svg>
                                                <span>复制</span>
                                            </button>
                                        </div>
                                    </div>

                                    <Show when={msg.role === 'user'}>
                                        <div class="chat-avatar-container user">
                                            <img src={globalUserAvatar()} alt="User" class="chat-avatar-img" />
                                        </div>
                                    </Show>
                                </div>
                            </div>
                        )}
                    </For>
                    <Show when={props.isThinking}>
                        <div class="message assistant">
                            <div class="message-wrapper">
                                <div class="chat-avatar-container ai">
                                    <img src={getModelLogo(selectedModel()?.model_id || "")} class="chat-avatar-img" />
                                </div>
                                <div class="message-body">
                                    <div class="message-content" style="opacity: 0.6">AI 正在思考中...</div>
                                </div>
                            </div>
                        </div>
                    </Show>
                </Show>
            </div>

            {/* 状态指示 */}
            <Show when={props.isProcessing}><div class="loading-overlay">正在解析文件内容...</div></Show>

            {/* 待发送文件列表 */}
            <div class="file-tags-container">
                <For each={props.pendingFiles}>
                    {(file, i) => (
                        <div class="file-tag">
                            <Show when={file.type === 'image'} fallback={<span class="file-icon">📄</span>}>
                                <img src={file.content} style="width: 20px; height: 20px; object-fit: cover; margin-right: 5px; border-radius: 2px;" />
                            </Show>
                            {file.name}
                            <button onClick={() => props.setPendingFiles(p => p.filter((_, idx) => idx !== i()))}>×</button>
                        </div>
                    )}
                </For>
            </div>

            {/* 输入区域 */}
            <div class="chat-input-wrapper">
                <div class="chat-input-unified">
                    <textarea
                        ref={textareaRef}
                        class="unified-textarea"
                        placeholder="输入消息... (Ctrl + Enter 换行)"
                        value={props.inputMessage}
                        onInput={(e) => {
                            props.setInputMessage(e.currentTarget.value);
                            e.currentTarget.style.height = 'auto';
                            const newHeight = e.currentTarget.scrollHeight;
                            e.currentTarget.style.height = `${newHeight}px`;
                            if (newHeight > 200) { e.currentTarget.style.overflowY = 'auto'; e.currentTarget.style.height = '200px'; }
                            else { e.currentTarget.style.overflowY = 'hidden'; }
                        }}
                        onKeyDown={(e) => {
                            if (e.key === 'Enter' && !e.ctrlKey && !e.shiftKey) {
                                e.preventDefault();
                                props.handleSendMessage();
                                if (textareaRef) textareaRef.style.height = '40px';
                            }
                        }}
                    />
                    <div class="input-toolbar">
                        <div class="toolbar-left">
                            <button class="toolbar-icon-btn" title="上传文件" onClick={async () => {
                                const selected = await open({ multiple: true });
                                if (!selected) return; // 如果取消选择，直接返回

                                // 关键点：统一转换为数组
                                const paths = Array.isArray(selected) ? selected : [selected];

                                // 此时遍历 paths，里面的 path 自动就是 string 类型
                                for (const path of paths) {
                                    await props.handleFileUpload(path, 'file');
                                }
                            }}>
                                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15.5 5.5L8.5 12.5C7.39543 13.6046 7.39543 15.3954 8.5 16.5C9.60457 17.6046 11.3954 17.6046 12.5 16.5L19.5 9.5C21.1569 7.84315 21.1569 5.15685 19.5 3.5C17.8431 1.84315 15.1569 1.84315 13.5 3.5L6.5 10.5C4.29086 12.7091 4.29086 16.2909 6.5 18.5C8.70914 20.7091 12.2909 20.7091 14.5 18.5L20.5 12.5" /></svg>
                            </button>
                            <button class="toolbar-icon-btn" title="上传图片" onClick={async () => {
                                const selected = await open({
                                    multiple: true,
                                    filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp'] }]
                                });
                                if (!selected) return;

                                const paths = Array.isArray(selected) ? selected : [selected];
                                for (const path of paths) {
                                    await props.handleFileUpload(path, 'image');
                                }
                            }}>
                                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="3" width="18" height="18" rx="2" /><circle cx="8.5" cy="8.5" r="1.5" fill="currentColor" /><path d="M21 15L16 10L5 21" stroke-linecap="round" stroke-linejoin="round" /></svg>
                            </button>
                        </div>
                        <div class="toolbar-right">
                            <button classList={{ 'unified-send-btn': true, 'stop-state': props.isThinking }} onClick={() => props.isThinking ? props.handleStopGeneration() : props.handleSendMessage()}>
                                <Show when={props.isThinking} fallback={
                                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 2L11 13M22 2L15 22L11 13M11 13L2 9L22 2" /></svg>
                                }>
                                    <svg viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="6" width="12" height="12" rx="2" /></svg>
                                </Show>
                            </button>
                        </div>
                    </div>
                </div>
            </div>

            {/* 拖拽蒙层 */}
            <Show when={props.isDragging}>
                <div class="drag-drop-overlay">
                    <div class="drag-drop-content">
                        <div class="drag-icons">
                            <div class="drag-icon-card side"><svg width="24" height="24" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"></path></svg></div>
                            <div class="drag-icon-card center"><svg width="32" height="32" fill="none" stroke="currentColor" stroke-width="2.5" viewBox="0 0 24 24"><path d="M12 16V8m0 0l-3 3m3-3l3 3m-9 8h12"></path></svg></div>
                            <div class="drag-icon-card side"><svg width="24" height="24" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path d="M7 21h10a2 2 0 002-2V9.414a1 1 0 00-.293-.707l-5.414-5.414A1 1 0 0012.586 3H7a2 2 0 00-2 2v14a2 2 0 002 2z"></path></svg></div>
                        </div>
                        <h2>上传文件</h2>
                        <p>支持 PDF、Docx、pptx 和图片解析</p>
                    </div>
                </div>
            </Show>
        </div>
    );
};

export default ChatInterface;

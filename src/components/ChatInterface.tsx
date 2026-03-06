import { Component, For, Show, Setter } from 'solid-js';
import Markdown from './Markdown';
import { Topic, globalUserAvatar, selectedModel } from '../store/store';
import { open } from '@tauri-apps/plugin-dialog';

/**
 * 组件 Props 接口定义
 */
interface ChatInterfaceProps {
    /** 当前激活的话题对象，包含消息历史 */
    activeTopic: Topic | null;
    /** 话题切换中状态，用于触发动画效果 */
    isChangingTopic: boolean;
    /** AI 是否正在思考/生成回复 */
    isThinking: boolean;
    /** 是否正在后台处理文件解析 */
    isProcessing: boolean;
    /** 是否有文件正在被拖拽到聊天区域 */
    isDragging: boolean;
    /** 当前应用打字机效果的消息索引，null 表示无 */
    typingIndex: number | null;
    /** 输入框当前文本值 */
    inputMessage: string;
    /** 设置输入框文本的 Setter */
    setInputMessage: Setter<string>;
    /** 待发送文件列表（已选择但未发送） */
    pendingFiles: { name: string, content: string, type: 'text' | 'image' }[];
    /** 设置待发送文件列表的 Setter */
    setPendingFiles: Setter<{ name: string, content: string, type: 'text' | 'image' }[]>;
    /** 发送消息回调（包含文本和 pendingFiles） */
    handleSendMessage: () => void;
    /** 停止 AI 生成回调 */
    handleStopGeneration: () => void;
    /**
     * 文件上传处理回调
     * @param path - 文件绝对路径
     * @param type - 文件类型：'file' 通用文件 或 'image' 图片
     */
    handleFileUpload: (path: string, type: 'file' | 'image') => Promise<void>;
}

/**
 * 聊天界面组件
 * 
 * @component
 * @description 完整的聊天交互界面，包括消息渲染、输入控制、文件上传。
 *              支持 Markdown 渲染、代码复制、文件拖拽、自适应输入框。
 * 
 * @param {ChatInterfaceProps} props - 组件属性
 * @returns {JSX.Element} 聊天界面 JSX 元素
 */
const ChatInterface: Component<ChatInterfaceProps> = (props) => {
    /** 文本域 DOM 引用，用于重置高度 */
    let textareaRef: HTMLTextAreaElement | undefined;

    /**
     * 根据模型名称获取对应的品牌 Logo 路径
     * 
     * @param {string} modelName - 模型名称或 ID
     * @returns {string} Logo 图片的 URL 路径
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
        <div class="flex flex-col flex-grow items-stretch border border-[var(--primary-color)] shadow-[inset_0_0_20px_1px_var(--primary-30)] rounded-lg box-border overflow-hidden p-[15px] pb-5 relative h-full">
            {/* 消息展示区域 */}
            <div
                class={`flex-grow opacity-100 overflow-y-auto pb-[15px] transition-opacity duration-200 ease-out z-[1] ${props.isChangingTopic ? 'opacity-0' : 'opacity-100'}`}
            >
                <Show when={props.activeTopic}>
                    <For each={props.activeTopic?.history}>
                        {(msg: any, index) => (
                            <div
                                class={`message flex flex-col mb-3 pointer-events-auto 
                                        animate-message-in opacity-0
                                        ${msg.role === 'assistant' ? 'items-start' : 'items-end'}`}
                                style={{
                                    "animation-delay": `${index() * 0.03}s`,
                                    "animation-fill-mode": "forwards"
                                }}
                            >
                                <div class={`flex gap-3 w-full ${msg.role === 'assistant' ? 'justify-start items-start' : 'justify-end items-start'}`}>
                                    <Show when={msg.role === 'assistant'}>
                                        <div class="flex flex-shrink-0 items-center justify-center w-9 h-9 rounded-full bg-[#dfdfdf] border border-[var(--primary-20)] shadow-[0_2px_6px_rgba(0,0,0,0.15)] overflow-hidden">
                                            <img
                                                src={getModelLogo(msg.modelId || selectedModel()?.model_id || "")}
                                                alt="AI"
                                                class="w-[25px] h-[25px] rounded-full"
                                            />
                                        </div>
                                    </Show>

                                    <div class={`flex flex-col max-w-[75%] ${msg.role === 'assistant' ? 'items-start' : 'items-end'}`}>
                                        <div
                                            class={`rounded-[10px] leading-relaxed max-w-full min-h-[1.5em] px-[14px] py-[10px] transition-[height] duration-200 overflow-wrap-break-word group relative
                                                ${msg.role === 'assistant'
                                                    ? 'bg-[#333] border border-[#555] rounded-tl-[2px] color-white'
                                                    : 'bg-[var(--primary-5)] border border-[var(--primary-color)] rounded-tr-[2px] color-white'}
                                                ${props.typingIndex === index() ? 'after:content-["|"] after:ml-[2px] after:text-[var(--primary-color)] after:animate-[cursor-blink_0.8s_infinite]' : ''}`}
                                        >
                                            <Show when={msg.role === 'user' && msg.displayFiles && msg.displayFiles.length > 0}>
                                                <For each={msg.displayFiles}>
                                                    {(file: any) => (
                                                        <div class="flex items-center bg-[var(--primary-10)] border border-[var(--primary-5)] rounded-lg cursor-default mb-2 max-w-[300px] px-[14px] py-[10px] transition-all duration-200 hover:border-[var(--primary-color)] first:mt-3">
                                                            <div class="flex flex-shrink-0 items-center justify-center w-10 h-10 bg-[var(--primary-10)] rounded-md mr-3">
                                                                <svg class="w-6 h-6 text-[var(--primary-color)]" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                                                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
                                                                </svg>
                                                            </div>
                                                            <div class="flex-grow overflow-hidden">
                                                                <div class="text-white text-[0.9rem] font-medium overflow-hidden text-ellipsis white-space-nowrap">{file.name}</div>
                                                                <div class="text-[var(--primary-50)] text-[0.75rem] mt-[2px]">已解析</div>
                                                            </div>
                                                        </div>
                                                    )}
                                                </For>
                                            </Show>

                                            <div class="mt-1">
                                                <Markdown content={msg.role === 'user' && msg.displayText !== undefined ? msg.displayText : msg.content} />
                                            </div>
                                        </div>

                                        <Show when={msg.role === 'assistant' && (msg.modelId || selectedModel()?.model_id)}>
                                            <div class="text-[#888] font-mono text-[11px] ml-1 mt-1 opacity-70 pointer-events-none select-none text-left">
                                                {msg.modelId || selectedModel()?.model_id}
                                            </div>
                                        </Show>

                                        <div class={`flex mt-1 px-[10px] opacity-0 group-hover:opacity-100 transition-opacity duration-200 z-[5] ${msg.role === 'assistant' ? 'justify-start' : 'justify-end'}`}>
                                            <button
                                                class="flex items-center gap-1 relative bg-transparent border border-[var(--primary-20)] rounded-lg text-[var(--primary-color)] cursor-pointer text-[13px] px-3 py-1 transition-all duration-200 hover:bg-[var(--primary-10)] hover:border-[var(--primary-color)]"
                                                onClick={(e) => {
                                                    const currentBtn = e.currentTarget;
                                                    const text = msg.role === 'user' && msg.displayText !== undefined ? msg.displayText : msg.content;
                                                    if (!text) return;
                                                    navigator.clipboard.writeText(text).then(() => {
                                                        const label = currentBtn.querySelector('span');
                                                        if (label) {
                                                            const originalText = label.innerText;
                                                            currentBtn.style.color = '#4af908';
                                                            currentBtn.style.borderColor = '#4af908';
                                                            label.innerText = '已复制';
                                                            setTimeout(() => {
                                                                currentBtn.style.color = '';
                                                                currentBtn.style.borderColor = '';
                                                                label.innerText = originalText;
                                                            }, 2000);
                                                        }
                                                    });
                                                }}
                                            >
                                                <svg class="w-[14px] h-[14px]" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                                    <path d="M8 5H6a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2v-1M8 5a2 2 0 002 2h2a2 2 0 002-2M8 5a2 2 0 012-2h2a2 2 0 012 2m0 0h2a2 2 0 012 2v3m2 4H10m0 0l3-3m-3 3l3 3" />
                                                </svg>
                                                <span>复制</span>
                                            </button>
                                        </div>
                                    </div>

                                    <Show when={msg.role === 'user'}>
                                        <div class="flex flex-shrink-0 items-center justify-center w-9 h-9 rounded-full bg-[#333] border border-[#555] shadow-[0_2px_6px_rgba(0,0,0,0.15)] overflow-hidden">
                                            <img src={globalUserAvatar()} alt="User" class="w-full h-full object-cover" />
                                        </div>
                                    </Show>
                                </div>
                            </div>
                        )}
                    </For>

                    <Show when={props.isThinking}>
                        <div class="flex flex-col mb-3 items-start animate-[bubble-in_0.35s_ease-out_forwards]">
                            <div class="flex gap-3 w-full justify-start items-start">
                                <div class="flex flex-shrink-0 items-center justify-center w-9 h-9 rounded-full bg-[#dfdfdf] border border-[var(--primary-20)] overflow-hidden">
                                    <img src={getModelLogo(selectedModel()?.model_id || "")} class="w-[25px] h-[25px]" />
                                </div>
                                <div class="flex flex-col max-w-[75%] items-start">
                                    <div class="bg-[#333] border border-[#555] rounded-[10px] rounded-tl-[2px] px-[14px] py-[10px] text-white opacity-60">
                                        AI 正在思考中...
                                    </div>
                                </div>
                            </div>
                        </div>
                    </Show>
                </Show>
            </div>

            {/* 加载状态 */}
            <Show when={props.isProcessing}>
                <div class="absolute inset-0 flex items-center justify-center bg-[rgba(0,0,0,0.8)] text-[var(--primary-color)] text-sm z-[100]">
                    正在解析文件内容...
                </div>
            </Show>

            {/* 待上传文件标签 */}
            <div class="flex flex-wrap gap-[3px] bg-transparent pt-[3px] relative z-0">
                <For each={props.pendingFiles}>
                    {(file, i) => (
                        <div class="flex items-center bg-[var(--primary-10)] border border-[var(--primary-5)] rounded-[16px] text-[var(--primary-color)] text-[12px] px-[10px] py-1 transition-all duration-200 hover:bg-[var(--primary-20)] hover:border-[var(--primary-color)] animate-[tagFadeIn_0.3s_ease-out]">
                            <Show when={file.type === 'image'} fallback={<span class="mr-1">📄</span>}>
                                <img src={file.content} class="w-5 h-5 object-cover mr-1 rounded-[2px]" />
                            </Show>
                            {file.name}
                            <button
                                class="flex items-center bg-none border-none text-[rgba(255,255,255,0.5)] cursor-pointer text-lg leading-none ml-2 transition-colors duration-200 hover:text-[#ff4d4d]"
                                onClick={() => props.setPendingFiles(p => p.filter((_, idx) => idx !== i()))}
                            >
                                ×
                            </button>
                        </div>
                    )}
                </For>
            </div>

            {/* 输入框区域 */}
            <div class="bg-transparent flex flex-col relative w-full z-10">
                <div class="bg-[#151515] border border-[#333] rounded-xl box-border flex flex-col gap-[10px] mt-[3px] p-[10px] transition-all duration-200 focus-within:border-[var(--primary-color)] focus-within:shadow-[0_0_10px_var(--primary-10)] w-full">
                    <textarea
                        ref={textareaRef}
                        class="bg-transparent border-none text-white font-inherit text-base leading-relaxed min-h-[40px] max-h-[20vh] outline-none overflow-y-hidden px-[5px] pb-[5px] resize-none w-full focus:overflow-y-auto"
                        placeholder="输入消息... (Ctrl + Enter 换行)"
                        value={props.inputMessage}
                        onInput={(e) => {
                            props.setInputMessage(e.currentTarget.value);
                            e.currentTarget.style.height = 'auto';
                            const newHeight = e.currentTarget.scrollHeight;
                            e.currentTarget.style.height = `${newHeight}px`;
                            if (newHeight > 200) {
                                e.currentTarget.style.overflowY = 'auto';
                                e.currentTarget.style.height = '200px';
                            } else {
                                e.currentTarget.style.overflowY = 'hidden';
                            }
                        }}
                        onKeyDown={(e) => {
                            if (e.key === 'Enter' && !e.ctrlKey && !e.shiftKey) {
                                e.preventDefault();
                                props.handleSendMessage();
                                if (textareaRef) textareaRef.style.height = '40px';
                            }
                        }}
                    />

                    <div class="flex items-center justify-between border-top border-t border-[rgba(255,255,255,0.05)] pt-2">
                        <div class="flex items-center gap-2">
                            <button
                                class="flex items-center justify-center bg-transparent border-none rounded-md text-[#888] cursor-pointer p-1.5 transition-all duration-200 hover:bg-[rgba(255,255,255,0.1)] hover:text-[var(--primary-color)]"
                                title="上传文件"
                                onClick={async () => {
                                    const selected = await open({ multiple: true });
                                    if (!selected) return;
                                    const paths = Array.isArray(selected) ? selected : [selected];
                                    for (const path of paths) {
                                        await props.handleFileUpload(path, 'file');
                                    }
                                }}
                            >
                                <svg class="w-5 h-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                    <path d="M15.5 5.5L8.5 12.5C7.39543 13.6046 7.39543 15.3954 8.5 16.5C9.60457 17.6046 11.3954 17.6046 12.5 16.5L19.5 9.5C21.1569 7.84315 21.1569 5.15685 19.5 3.5C17.8431 1.84315 15.1569 1.84315 13.5 3.5L6.5 10.5C4.29086 12.7091 4.29086 16.2909 6.5 18.5C8.70914 20.7091 12.2909 20.7091 14.5 18.5L20.5 12.5" />
                                </svg>
                            </button>

                            <button
                                class="flex items-center justify-center bg-transparent border-none rounded-md text-[#888] cursor-pointer p-1.5 transition-all duration-200 hover:bg-[rgba(255,255,255,0.1)] hover:text-[var(--primary-color)]"
                                title="上传图片"
                                onClick={async () => {
                                    const selected = await open({
                                        multiple: true,
                                        filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp'] }]
                                    });
                                    if (!selected) return;
                                    const paths = Array.isArray(selected) ? selected : [selected];
                                    for (const path of paths) {
                                        await props.handleFileUpload(path, 'image');
                                    }
                                }}
                            >
                                <svg class="w-5 h-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                                    <rect x="3" y="3" width="18" height="18" rx="2" />
                                    <circle cx="8.5" cy="8.5" r="1.5" fill="currentColor" />
                                    <path d="M21 15L16 10L5 21" stroke-linecap="round" stroke-linejoin="round" />
                                </svg>
                            </button>
                        </div>

                        <div class="flex items-center gap-2">
                            <button
                                class={`flex items-center justify-center border-none rounded-lg cursor-pointer h-8 w-8 transition-all duration-100 hover:opacity-90 hover:scale-105 active:scale-95
                                    ${props.isThinking ? 'bg-[#ff4d4d] text-white' : 'bg-[var(--primary-color)] text-black'}`}
                                onClick={() => props.isThinking
                                    ? props.handleStopGeneration()
                                    : props.handleSendMessage()
                                }
                            >
                                <Show when={props.isThinking} fallback={
                                    <svg class="w-[18px] h-[18px]" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                        <path d="M22 2L11 13M22 2L15 22L11 13M11 13L2 9L22 2" />
                                    </svg>
                                }>
                                    <svg class="w-[18px] h-[18px]" viewBox="0 0 24 24" fill="currentColor">
                                        <rect x="6" y="6" width="12" height="12" rx="2" />
                                    </svg>
                                </Show>
                            </button>
                        </div>
                    </div>
                </div>
            </div>

            {/* 拖拽覆盖层 */}
            <Show when={props.isDragging}>
                <div class="absolute inset-0 flex items-center justify-center bg-[rgba(0,0,0,0.7)] backdrop-blur-[4px] pointer-events-none z-[9999] animate-[fadeIn_0.3s_ease-out]">
                    <div class="relative flex flex-col items-center justify-center w-[420px] h-[280px] bg-[#1a1a1a] border-2 border-[var(--primary-color)] rounded-xl shadow-[0_0_30px_var(--primary-20)] text-white text-center p-5">
                        <div class="flex items-end mb-[25px] mt-[-30px]">
                            <div class="flex items-center justify-center w-[60px] h-20 bg-[#2a2a2a] border border-[var(--primary-color)] rounded-md shadow-[0_4px_15px_rgba(0,0,0,0.5)] opacity-60 scale-[0.85] translate-y-[10px] -rotate-12 translate-x-[15px] z-[1]">
                                <svg width="24" height="24" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24">
                                    <path d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"></path>
                                </svg>
                            </div>
                            <div class="flex items-center justify-center w-[70px] h-[90px] bg-[#333] border border-[var(--primary-color)] rounded-md shadow-[0_0_20px_rgba(8,221,249,0.3)] text-[var(--primary-color)] z-[3]">
                                <svg width="32" height="32" fill="none" stroke="currentColor" stroke-width="2.5" viewBox="0 0 24 24">
                                    <path d="M12 16V8m0 0l-3 3m3-3l3 3m-9 8h12"></path>
                                </svg>
                            </div>
                            <div class="flex items-center justify-center w-[60px] h-20 bg-[#2a2a2a] border border-[rgba(8,221,249,0.5)] rounded-md shadow-[0_4px_15px_rgba(0,0,0,0.5)] opacity-60 scale-[0.85] translate-y-[10px] rotate-12 -translate-x-[15px] z-[1]">
                                <svg width="24" height="24" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24">
                                    <path d="M7 21h10a2 2 0 002-2V9.414a1 1 0 00-.293-.707l-5.414-5.414A1 1 0 0012.586 3H7a2 2 0 00-2 2v14a2 2 0 002 2z"></path>
                                </svg>
                            </div>
                        </div>
                        <h2 class="text-[var(--primary-color)] text-[22px] tracking-wider mb-2.5 z-[2]">上传文件</h2>
                        <p class="text-[rgba(255,255,255,0.7)] text-sm leading-relaxed max-w-[80%] z-[2]">支持 PDF、Docx、pptx 和图片解析</p>
                        <div class="absolute inset-3 border border-dashed border-[rgba(8,221,249,0.4)] rounded-lg pointer-events-none"></div>
                    </div>
                </div>
            </Show>
        </div>
    );
};

export default ChatInterface;
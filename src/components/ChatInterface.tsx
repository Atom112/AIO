import { Component, For, Show, Setter } from 'solid-js';
import Markdown from './Markdown';
import { Topic, globalUserAvatar, selectedModel, isStartingLocalModel, localModelStartProgress } from '../store/store';
import { open } from '@tauri-apps/plugin-dialog';
import Icon from './Icon';

interface ChatInterfaceProps {
    activeTopic: Topic | null; // 当前激活的话题对象
    isChangingTopic: boolean; // 话题切换中状态
    isThinking: boolean; // AI 是否正在生成回复
    isProcessing: boolean; // 是否正在处理文件解析
    isDragging: boolean; // 是否有文件被拖拽到聊天区域
    typingIndex: number | null; // 应用打字机效果的消息索引
    inputMessage: string; // 输入框当前文本值
    setInputMessage: Setter<string>; // 设置输入框文本
    pendingFiles: { name: string, content: string, type: 'text' | 'image' }[]; // 待发送文件列表
    setPendingFiles: Setter<{ name: string, content: string, type: 'text' | 'image' }[]>; // 设置待发送文件
    handleSendMessage: () => void; // 发送消息回调
    handleStopGeneration: () => void; // 停止 AI 生成回调
    handleFileUpload: (path: string, type: 'file' | 'image') => Promise<void>; // 文件上传处理回调
}

/**
 * 聊天界面组件
 * @param {ChatInterfaceProps} props - 组件属性
 * @returns {JSX.Element} 聊天界面 JSX 元素
 */
const ChatInterface: Component<ChatInterfaceProps> = (props) => {
    let textareaRef: HTMLTextAreaElement | undefined; // 文本域 DOM 引用

    /**
     * 根据模型名称获取对应的品牌 Logo 路径
     * @param {string} modelName - 模型名称或 ID
     * @returns {string} Logo 图片的 URL 路径
     */
    const getModelLogo = (modelName: string) => {
        const name = modelName.toLowerCase();
        if (name.includes('gpt')) return '/icons/model-logo/openai.svg';
        if (name.includes('claude')) return '/icons/model-logo/claude-color.svg';
        if (name.includes('grok')) return '/icons/model-logo/grok.svg';
        if (name.includes('gemini')) return '/icons/model-logo/gemini-color.svg';
        if (name.includes('deepseek')) return '/icons/model-logo/deepseek-color.svg';
        if (name.includes('qwen') || name.includes('qwq')) return '/icons/model-logo/qwen-color.svg';
        if (name.includes('kimi') || name.includes('moonshot')) return '/icons/model-logo/moonshot.svg';
        if (name.includes('doubao')) return '/icons/model-logo/doubao-color.svg';
        if (name.includes('glm')) return '/icons/model-logo/zhipu-color.svg';
        return '/icons/model-logo/ollama.svg';
    };

    return (
        <div class="flex flex-col flex-grow items-stretch glow-border rounded-lg box-border overflow-hidden p-[15px] pb-5 relative h-full">
            <div
                class={`flex-grow overflow-y-auto pb-[15px] transition-opacity duration-200 ease-out z-[1] ${props.isChangingTopic ? 'opacity-0' : 'opacity-100'}`}
            >
                <Show when={isStartingLocalModel()}>
                    <div class="w-full mb-4 p-4 bg-dark-300 rounded-lg border border-pri-20">
                        <div class="flex items-center gap-3 mb-2">
                            <Icon src="/icons/app-logo/loading.svg" class="w-5 h-5 animate-spin text-pri" />
                            <span class="text-white text-sm">正在启动本地 Llama 服务器...</span>
                        </div>
                        <div class="w-full bg-dark-500 rounded-full h-2">
                            <div 
                                class="bg-pri h-2 rounded-full transition-all duration-300" 
                                style={{ width: `${localModelStartProgress()}%` }}
                            ></div>
                        </div>
                        <div class="text-right text-xs text-pri-50 mt-1">
                            {Math.round(localModelStartProgress())}%
                        </div>
                    </div>
                </Show>

                <Show when={props.activeTopic}>
                    <For each={props.activeTopic?.history}>
                        {(msg: any, index) => (
                            <div
                                class={`flex flex-col mb-3 pointer-events-auto animate-message-in ${msg.role === 'assistant' ? 'items-start' : 'items-end'}`}
                            >
                                <div class={`flex gap-3 w-full ${msg.role === 'assistant' ? 'justify-start items-start' : 'justify-end items-start'}`}>
                                    <Show when={msg.role === 'assistant'}>
                                        <div class="flex flex-shrink-0 items-center justify-center w-9 h-9 rounded-full bg-[#dfdfdf] border border-pri-20 shadow-[0_2px_6px_rgba(0,0,0,0.15)] overflow-hidden">
                                            <img
                                                src={getModelLogo(msg.modelId || selectedModel()?.model_id || "")}
                                                alt="AI"
                                                class="w-[25px] h-[25px] rounded-full"
                                            />
                                        </div>
                                    </Show>

                                    <div class={`flex flex-col max-w-[75%] ${msg.role === 'assistant' ? 'items-start' : 'items-end'}`}>
                                        <div
                                            class={`rounded-[10px] leading-relaxed max-w-full min-h-[1.5em] px-[14px] py-[10px] transition-[height] duration-200 break-words group relative
                                                ${msg.role === 'assistant'
                                                    ? 'bg-dark-300 border border-dark-100 rounded-tl-[2px] text-white'
                                                    : 'bg-pri-5 border border-pri rounded-tr-[2px] text-white'}`}
                                        >
                                            <Show when={msg.role === 'user' && msg.displayFiles && msg.displayFiles.length > 0}>
                                                <For each={msg.displayFiles}>
                                                    {(file: any) => (
                                                        <div class="flex items-center bg-pri-10 border border-pri-5 rounded-lg cursor-default mb-2 max-w-[300px] px-[14px] py-[10px] transition-all duration-200 hover:border-pri first:mt-3">
                                                            <div class="flex flex-shrink-0 items-center justify-center w-10 h-10 bg-pri-10 rounded-md mr-3 text-pri">
                                                                <Icon src="/icons/app-logo/file-document.svg" class="w-6 h-6" />
                                                            </div>
                                                            <div class="flex-grow overflow-hidden">
                                                                <div class="text-white text-[0.9rem] font-medium overflow-hidden text-ellipsis whitespace-nowrap">{file.name}</div>
                                                                <div class="text-pri-50 text-[0.75rem] mt-[2px]">已解析</div>
                                                            </div>
                                                        </div>
                                                    )}
                                                </For>
                                            </Show>

                                            <div class="mt-1">
                                                <Show
                                                    when={msg.role === 'assistant' && !msg.content}
                                                    fallback={
                                                        <Markdown content={msg.role === 'user' && msg.displayText !== undefined ? msg.displayText : msg.content} />
                                                    }
                                                >
                                                    <div class="flex items-center gap-2 py-1 text-white/50 italic text-[14px] select-none">
                                                        <Icon src="/icons/app-logo/loading.svg" class="w-4 h-4 animate-spin opacity-50" />
                                                        <span class="animate-pulse">AI 正在思考中...</span>
                                                    </div>
                                                </Show>
                                            </div>
                                        </div>

                                        <Show when={msg.role === 'assistant' && (msg.modelId || selectedModel()?.model_id)}>
                                            <div class="text-[#888] font-mono text-[11px] ml-1 mt-1 opacity-70 pointer-events-none select-none text-left">
                                                {msg.modelId || selectedModel()?.model_id}
                                            </div>
                                        </Show>

                                        <div class={`flex mt-1 px-[10px] opacity-0 group-hover:opacity-100 transition-opacity duration-200 z-[5] ${msg.role === 'assistant' ? 'justify-start' : 'justify-end'}`}>
                                            <button
                                                class="flex items-center gap-1 relative bg-transparent border border-pri-20 rounded-lg text-pri cursor-pointer text-[13px] px-3 py-1 transition-all duration-200 hover:bg-pri-10 hover:border-pri"
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
                                                <Icon src="/icons/app-logo/clipboard-copy.svg" class="w-[14px] h-[14px]" />
                                                <span>复制</span>
                                            </button>
                                        </div>
                                    </div>

                                    <Show when={msg.role === 'user'}>
                                        <div class="flex flex-shrink-0 items-center justify-center w-9 h-9 rounded-full bg-dark-300 border border-dark-100 shadow-[0_2px_6px_rgba(0,0,0,0.15)] overflow-hidden">
                                            <img src={globalUserAvatar()} alt="User" class="w-full h-full object-cover" />
                                        </div>
                                    </Show>
                                </div>
                            </div>
                        )}
                    </For>
                </Show>
            </div>

            <Show when={props.isProcessing}>
                <div class="absolute inset-0 flex items-center justify-center bg-[rgba(0,0,0,0.8)] text-pri text-sm z-[100]">
                    正在解析文件内容...
                </div>
            </Show>

            <div class="flex flex-wrap gap-[3px] bg-transparent pt-[3px] relative z-0">
                <For each={props.pendingFiles}>
                    {(file, i) => (
                        <div class="flex items-center bg-pri-10 border border-pri-5 rounded-[16px] text-pri text-[12px] px-[10px] py-1 transition-all duration-200 hover:bg-pri-20 hover:border-pri">
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

            <div class="bg-transparent flex flex-col relative w-full z-10">
                <div class="bg-dark-900 border border-dark-300 rounded-xl box-border flex flex-col gap-[10px] mt-[3px] p-[10px] transition-all duration-200 focus-within:border-pri focus-within:shadow-[0_0_10px_var(--primary-10)] w-full">
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

                    <div class="flex items-center justify-between border-t border-[rgba(255,255,255,0.05)] pt-2">
                        <div class="flex items-center gap-2">
                            <button
                                class="flex items-center justify-center bg-transparent border-none rounded-md text-[#888] cursor-pointer p-1.5 transition-all duration-200 hover:bg-[rgba(255,255,255,0.1)] hover:text-pri"
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
                                <Icon src="/icons/app-logo/paperclip.svg" class="w-5 h-5" />
                            </button>

                            <button
                                class="flex items-center justify-center bg-transparent border-none rounded-md text-[#888] cursor-pointer p-1.5 transition-all duration-200 hover:bg-[rgba(255,255,255,0.1)] hover:text-pri"
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
                                <Icon src="/icons/app-logo/image-photo.svg" class="w-5 h-5" />
                            </button>
                        </div>

                        <div class="flex items-center gap-2">
                            <button
                                class={`flex items-center justify-center border-none rounded-lg cursor-pointer h-8 w-8 transition-all duration-100 hover:opacity-90 hover:scale-105 active:scale-95
                                    ${props.isThinking ? 'bg-[#ff4d4d] text-white' : 'bg-pri text-black'}`}
                                onClick={() => props.isThinking
                                    ? props.handleStopGeneration()
                                    : props.handleSendMessage()
                                }
                            >
                                <Show when={props.isThinking} fallback={
                                    <Icon src="/icons/app-logo/send.svg" class="w-[18px] h-[18px]" />
                                }>
                                    <Icon src="/icons/app-logo/stop-square.svg" class="w-[18px] h-[18px]" />
                                </Show>
                            </button>
                        </div>
                    </div>
                </div>
            </div>

            <Show when={props.isDragging}>
                <div class="absolute inset-0 flex items-center justify-center bg-[rgba(0,0,0,0.7)] backdrop-blur-[4px] pointer-events-none z-[9999]">
                    <div class="relative flex flex-col items-center justify-center w-[420px] h-[280px] bg-dark-850 border-2 border-pri rounded-xl shadow-[0_0_30px_var(--primary-20)] text-white text-center p-5">
                        <div class="flex items-end mb-[25px] mt-[-30px]">
                            <div class="flex items-center justify-center w-[60px] h-20 bg-dark-500 border border-pri rounded-md shadow-[0_4px_15px_rgba(0,0,0,0.5)] opacity-60 scale-[0.85] translate-y-[10px] -rotate-12 translate-x-[15px] z-[1]">
                                <Icon src="/icons/app-logo/file-document.svg" class="w-6 h-6" />
                            </div>
                            <div class="flex items-center justify-center w-[70px] h-[90px] bg-dark-300 border border-pri rounded-md shadow-[0_0_20px_rgba(8,221,249,0.3)] text-pri z-[3]">
                                <Icon src="/icons/app-logo/upload-arrow.svg" class="w-8 h-8" />
                            </div>
                            <div class="flex items-center justify-center w-[60px] h-20 bg-dark-500 border border-[rgba(8,221,249,0.5)] rounded-md shadow-[0_4px_15px_rgba(0,0,0,0.5)] opacity-60 scale-[0.85] translate-y-[10px] rotate-12 -translate-x-[15px] z-[1]">
                                <Icon src="/icons/app-logo/file-blank.svg" class="w-6 h-6" />
                            </div>
                        </div>
                        <h2 class="text-pri text-[22px] tracking-wider mb-2.5 z-[2]">上传文件</h2>
                        <p class="text-[rgba(255,255,255,0.7)] text-sm leading-relaxed max-w-[80%] z-[2]">支持 PDF、Docx、pptx 和图片解析</p>
                        <div class="absolute inset-3 border border-dashed border-[rgba(8,221,249,0.4)] rounded-lg pointer-events-none"></div>
                    </div>
                </div>
            </Show>
        </div>
    );
};

export default ChatInterface;
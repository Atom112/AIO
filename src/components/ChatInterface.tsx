import { Component, For, Show, Setter, createSignal, createEffect } from 'solid-js';
import Markdown from './Markdown';
import { Topic, globalUserAvatar, selectedModel, isStartingLocalModel, localModelStartProgress } from '../store/store';
import { open } from '@tauri-apps/plugin-dialog';
import { getLogo as getLogoByIds } from '../utils/modelLogo';
import Icon from './Icon';

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

const UserMessageAvatar: Component = () => {
    const [isLoaded, setIsLoaded] = createSignal(false);
    const [imgSrc, setImgSrc] = createSignal(globalUserAvatar());

    createEffect(() => {
        setImgSrc(globalUserAvatar());
        setIsLoaded(false);
    });

    return (
        <div class="relative flex flex-shrink-0 items-center justify-center w-9 h-9 rounded-full overflow-hidden"
             style="background: rgba(255,255,255,0.06); border: 1px solid rgba(255,255,255,0.04); box-shadow: 0 2px 6px rgba(0,0,0,0.15);">
            <img
                src={imgSrc()}
                alt="User"
                class="w-full h-full object-cover transition-opacity duration-300"
                classList={{ 'opacity-0': !isLoaded(), 'opacity-100': isLoaded() }}
                onLoad={() => setIsLoaded(true)}
                onError={() => setImgSrc('/icons/app-logo/user.svg')}
            />
            <div
                class="absolute inset-0 w-full h-full flex items-center justify-center transition-opacity duration-300 pointer-events-none"
                style="background: rgba(255,255,255,0.06);"
                classList={{ 'opacity-100': !isLoaded(), 'opacity-0': isLoaded() }}
            >
                <Icon src="/icons/app-logo/user.svg" class="w-5 h-5 opacity-50" />
            </div>
        </div>
    );
};

const ChatInterface: Component<ChatInterfaceProps> = (props) => {
    let textareaRef: HTMLTextAreaElement | undefined;

    const getModelLogo = (modelName: string) => {
        return getLogoByIds(null, modelName);
    };

    return (
        <div class="flex flex-col flex-grow items-stretch rounded-lg box-border overflow-hidden p-[15px] pb-5 relative h-full"
             style="background: rgba(18, 22, 35, 0.2); backdrop-filter: blur(20px); border: 1px solid rgba(255, 255, 255, 0.04);">
            <div
                class={`flex-grow overflow-y-auto pb-[15px] transition-opacity duration-200 ease-out z-[1] ${props.isChangingTopic ? 'opacity-0' : 'opacity-100'}`}
            >
                <Show when={isStartingLocalModel()}>
                    <div class="w-full mb-4 p-4 rounded-lg" style="background: rgba(255,255,255,0.04); border: 1px solid rgba(255,255,255,0.06);">
                        <div class="flex items-center gap-3 mb-2">
                            <span style="color: rgba(124,154,191,0.6);"><Icon src="/icons/app-logo/loading.svg" class="w-5 h-5 animate-spin" /></span>
                            <span style="color: rgba(255,255,255,0.85); font-size: 0.875rem;">正在启动本地推理引擎...</span>
                        </div>
                        <div class="w-full h-2 rounded-full" style="background: rgba(255,255,255,0.06);">
                            <div class="h-2 rounded-full transition-all duration-300" style={{ width: `${localModelStartProgress()}%`, background: 'rgba(124,154,191,0.4)' }}></div>
                        </div>
                        <div class="text-right text-xs mt-1" style="color: rgba(124,154,191,0.4);">
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
                                        <div class="flex flex-shrink-0 items-center justify-center w-9 h-9 rounded-full overflow-hidden"
                                             style="background: rgba(255,255,255,0.06); border: 1px solid rgba(124,154,191,0.1); box-shadow: 0 2px 6px rgba(0,0,0,0.15);">
                                            <img
                                                src={getModelLogo(msg.modelId || selectedModel()?.model_id || "")}
                                                alt="AI"
                                                class="w-[25px] h-[25px] rounded-full"
                                            />
                                        </div>
                                    </Show>

                                    <div class={`flex flex-col max-w-[75%] ${msg.role === 'assistant' ? 'items-start' : 'items-end'}`}>
                                        <div
                                            class={`rounded-[10px] leading-relaxed max-w-full min-h-[1.5em] px-[14px] py-[10px] transition-[height] duration-200 break-words group relative ${
                                                msg.role === 'assistant'
                                                    ? 'rounded-tl-[2px] text-white'
                                                    : 'rounded-tr-[2px] text-white'
                                            }`}
                                            style={`background: ${msg.role === 'assistant' ? 'rgba(255,255,255,0.04)' : 'rgba(124,154,191,0.08)'}; border: ${msg.role === 'assistant' ? '1px solid rgba(255,255,255,0.04)' : '1px solid rgba(124,154,191,0.06)'}; backdrop-filter: blur(8px);`}
                                        >
                                            <Show when={msg.role === 'user' && msg.displayFiles && msg.displayFiles.length > 0}>
                                                <For each={msg.displayFiles}>
                                                    {(file: any) => (
                                                        <div class="flex items-center rounded-lg cursor-default mb-2 max-w-[300px] px-[14px] py-[10px] transition-all duration-200 first:mt-3"
                                                             style="background: rgba(124,154,191,0.06); border: 1px solid rgba(124,154,191,0.04);">
                                                            <div class="flex flex-shrink-0 items-center justify-center w-10 h-10 rounded-md mr-3"
                                                                 style="background: rgba(124,154,191,0.08); color: rgba(124,154,191,0.6);">
                                                                <Icon src="/icons/app-logo/file-document.svg" class="w-6 h-6" />
                                                            </div>
                                                            <div class="flex-grow overflow-hidden">
                                                                <div class="text-white text-[0.9rem] font-medium overflow-hidden text-ellipsis whitespace-nowrap">{file.name}</div>
                                                                <div style="color: rgba(124,154,191,0.4); font-size: 0.75rem; margin-top: 2px;">已解析</div>
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
                                            <div style="color: rgba(255,255,255,0.3); font-family: monospace; font-size: 11px; margin-left: 4px; margin-top: 4px; opacity: 0.7; user-select: none; text-align: left;">
                                                {msg.modelId || selectedModel()?.model_id}
                                            </div>
                                        </Show>

                                        <div class={`flex mt-1 px-[10px] opacity-0 group-hover:opacity-100 transition-opacity duration-200 z-[5] ${msg.role === 'assistant' ? 'justify-start' : 'justify-end'}`}>
                                            <button
                                                class="flex items-center gap-1 relative bg-transparent rounded-lg cursor-pointer text-[13px] px-3 py-1 transition-all duration-200"
                                                style="border: 1px solid rgba(124,154,191,0.1); color: rgba(124,154,191,0.6);"
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
                                                onMouseEnter={(e) => { e.currentTarget.style.background = 'rgba(124,154,191,0.06)'; e.currentTarget.style.borderColor = 'rgba(124,154,191,0.2)'; }}
                                                onMouseLeave={(e) => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.borderColor = 'rgba(124,154,191,0.1)'; }}
                                            >
                                                <Icon src="/icons/app-logo/clipboard-copy.svg" class="w-[14px] h-[14px]" />
                                                <span>复制</span>
                                            </button>
                                        </div>
                                    </div>

                                    <Show when={msg.role === 'user'}>
                                        <UserMessageAvatar />
                                    </Show>
                                </div>
                            </div>
                        )}
                    </For>
                </Show>
            </div>

            <Show when={props.isProcessing}>
                <div class="absolute inset-0 flex items-center justify-center z-[100]"
                     style="background: rgba(0,0,0,0.6); color: rgba(124,154,191,0.6); font-size: 0.875rem; backdrop-filter: blur(4px);">
                    正在解析文件内容...
                </div>
            </Show>

            <div class="flex flex-wrap gap-[3px] bg-transparent pt-[3px] relative z-0">
                <For each={props.pendingFiles}>
                    {(file, i) => (
                        <div class="flex items-center rounded-[16px] text-[12px] px-[10px] py-1 transition-all duration-200"
                             style="background: rgba(124,154,191,0.08); border: 1px solid rgba(124,154,191,0.04); color: rgba(124,154,191,0.6);">
                            <Show when={file.type === 'image'} fallback={<span class="mr-1 inline-flex"><Icon name="file" size={12} class="opacity-60" /></span>}>
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
                <div class="rounded-xl box-border flex flex-col gap-[10px] mt-[3px] p-[10px] transition-all duration-200 w-full"
                     style="background: rgba(0,0,0,0.25); border: 1px solid rgba(255,255,255,0.06);">
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

                    <div class="flex items-center justify-between border-t pt-2" style="border-color: rgba(255,255,255,0.04);">
                        <div class="flex items-center gap-2">
                            <button
                                class="flex items-center justify-center bg-transparent border-none rounded-md cursor-pointer p-1.5 transition-all duration-200"
                                style="color: rgba(255,255,255,0.4);"
                                title="上传文件"
                                onClick={async () => {
                                    const selected = await open({ multiple: true });
                                    if (!selected) return;
                                    const paths = Array.isArray(selected) ? selected : [selected];
                                    for (const path of paths) {
                                        await props.handleFileUpload(path, 'file');
                                    }
                                }}
                                onMouseEnter={(e) => { e.currentTarget.style.background = 'rgba(255,255,255,0.06)'; e.currentTarget.style.color = 'rgba(124,154,191,0.6)'; }}
                                onMouseLeave={(e) => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = 'rgba(255,255,255,0.4)'; }}
                            >
                                <Icon src="/icons/app-logo/paperclip.svg" class="w-5 h-5" />
                            </button>

                            <button
                                class="flex items-center justify-center bg-transparent border-none rounded-md cursor-pointer p-1.5 transition-all duration-200"
                                style="color: rgba(255,255,255,0.4);"
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
                                onMouseEnter={(e) => { e.currentTarget.style.background = 'rgba(255,255,255,0.06)'; e.currentTarget.style.color = 'rgba(124,154,191,0.6)'; }}
                                onMouseLeave={(e) => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = 'rgba(255,255,255,0.4)'; }}
                            >
                                <Icon src="/icons/app-logo/image-photo.svg" class="w-5 h-5" />
                            </button>
                        </div>

                        <div class="flex items-center gap-2">
                            <button
                                class={`flex items-center justify-center border-none rounded-lg cursor-pointer h-8 w-8 transition-all duration-100 hover:opacity-90 hover:scale-105 active:scale-95 ${props.isThinking ? 'bg-[#ff4d4d] text-white' : 'text-white'}`}
                                style={!props.isThinking ? { background: 'rgba(124,154,191,0.3)' } : {}}
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
                <div class="absolute inset-0 flex items-center justify-center pointer-events-none z-[9999]"
                     style="background: rgba(0,0,0,0.5); backdrop-filter: blur(8px);">
                    <div class="relative flex flex-col items-center justify-center w-[420px] h-[280px] rounded-xl text-white text-center p-5"
                         style="background: rgba(18, 22, 35, 0.7); border: 1px solid rgba(255, 255, 255, 0.06);">
                        <div class="flex items-end mb-[25px] mt-[-30px]">
                            <div class="flex items-center justify-center w-[60px] h-20 rounded-md opacity-60 scale-[0.85] translate-y-[10px] -rotate-12 translate-x-[15px] z-[1]"
                                 style="background: rgba(124,154,191,0.06); border: 1px solid rgba(124,154,191,0.06);">
                                <span style="color: rgba(124,154,191,0.4);"><Icon src="/icons/app-logo/file-document.svg" class="w-6 h-6" /></span>
                            </div>
                            <div class="flex items-center justify-center w-[70px] h-[90px] rounded-md z-[3]"
                                 style="background: rgba(124,154,191,0.08); border: 1px solid rgba(124,154,191,0.1); color: rgba(124,154,191,0.5);">
                                <Icon src="/icons/app-logo/upload-arrow.svg" class="w-8 h-8" />
                            </div>
                            <div class="flex items-center justify-center w-[60px] h-20 rounded-md opacity-60 scale-[0.85] translate-y-[10px] rotate-12 -translate-x-[15px] z-[1]"
                                 style="background: rgba(124,154,191,0.06); border: 1px solid rgba(124,154,191,0.04);">
                                <span style="color: rgba(124,154,191,0.4);"><Icon src="/icons/app-logo/file-blank.svg" class="w-6 h-6" /></span>
                            </div>
                        </div>
                        <h2 style="color: rgba(124,154,191,0.6); font-size: 22px; letter-spacing: 0.1em; margin-bottom: 10px; z-index: 2;">上传文件</h2>
                        <p style="color: rgba(255,255,255,0.5); font-size: 0.875rem; max-width: 80%; z-index: 2;">支持 PDF、Docx、pptx 和图片解析</p>
                        <div class="absolute inset-3 rounded-lg pointer-events-none" style="border: 1px dashed rgba(255,255,255,0.1);"></div>
                    </div>
                </div>
            </Show>
        </div>
    );
};

export default ChatInterface;

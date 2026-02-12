/**
 * ============================================================================
 * 文件功能摘要
 * ============================================================================
 * 
 * 【核心功能】
 * AI 聊天界面组件，负责消息历史渲染、用户输入处理、文件上传和发送控制。
 * 支持 Markdown 渲染、代码高亮、文件拖拽上传、多模型头像识别等功能。
 * 
 * 【数据流流向】
 * ┌─────────────────────────────────────────────────────────────────────────┐
 * │  外部数据流入 (Props)                                                    │
 * │  ├── activeTopic: Topic | null ← 当前激活的话题（含消息历史）            │
 * │  ├── isChangingTopic: boolean ← 话题切换动画状态                         │
 * │  ├── isThinking: boolean ← AI 是否正在思考/生成中                        │
 * │  ├── isProcessing: boolean ← 是否正在解析文件                            │
 * │  ├── isDragging: boolean ← 是否正在拖拽文件到区域                        │
 * │  ├── typingIndex: number | null ← 当前正在打字机效果的消息索引           │
 * │  ├── inputMessage: string ← 输入框当前值（受控组件）                     │
 * │  ├── setInputMessage: Setter<string> ← 更新输入值的回调                  │
 * │  ├── pendingFiles: 待发送文件列表（文本或图片）                          │
 * │  ├── setPendingFiles: Setter<...> ← 更新待发送文件列表                   │
 * │  ├── handleSendMessage: () => void ← 发送消息回调                        │
 * │  ├── handleStopGeneration: () => void ← 停止生成回调                     │
 * │  └── handleFileUpload: (path, type) => Promise<void> ← 文件上传回调      │
 * │                                                                          │
 * │  全局状态                                                                │
 * │  ├── globalUserAvatar() ← 用户全局头像 URL                               │
 * │  └── selectedModel() ← 当前选中的 AI 模型信息                            │
 * │                                                                          │
 * │  本地文件                                                                │
 * │  ├── 导入 ChatInterface.css 样式文件                                     │
 * │  └── 导入 Markdown 子组件                                                │
 * │                                                                          │
 * │  系统 API                                                                │
 * │  └── open() ← Tauri dialog 插件，调用系统文件选择器                      │
 * └─────────────────────────────────────────────────────────────────────────┘
 * 
 * 【组件层级】
 * ChatInterface (本组件)
 * ├── 消息展示区域 (chat-messages-area)
 * │   ├── 历史消息列表 (For 循环)
 * │   │   ├── AI 消息：模型 Logo + Markdown 内容 + 复制按钮
 * │   │   └── 用户消息：文件卡片 + 文本内容 + 用户头像
 * │   └── 思考中提示 (Show 条件渲染)
 * ├── 文件解析状态遮罩 (isProcessing)
 * ├── 待发送文件标签列表 (pendingFiles)
 * ├── 输入区域 (chat-input-wrapper)
 * │   ├── 自适应高度文本域 (textarea)
 * │   └── 工具栏（文件上传、图片上传、发送/停止按钮）
 * └── 拖拽上传遮层 (isDragging)
 * ============================================================================
 */

// SolidJS 核心 API
import { Component, For, Show, Setter } from 'solid-js';
// Markdown 渲染子组件
import Markdown from './Markdown';
// 全局状态：话题类型、用户头像、选中模型
import { Topic, globalUserAvatar, selectedModel, setDatas } from '../store/store';
// Tauri 对话框插件：调用系统原生文件选择器
import { open } from '@tauri-apps/plugin-dialog';
// 本地样式文件
import './ChatInterface.css';
import { invoke } from '@tauri-apps/api/core';

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
     * 匹配规则（不区分大小写）：
     * - GPT → OpenAI Logo
     * - Claude → Anthropic Claude Logo
     * - Grok → xAI Grok Logo
     * - Gemini → Google Gemini Logo
     * - DeepSeek → DeepSeek Logo
     * - Qwen/QwQ → 阿里通义千问 Logo
     * - Kimi/Moonshot → 月之暗面 Logo
     * - Doubao → 字节豆包 Logo
     * - GLM → 智谱清言 Logo
     * - 默认 → Ollama Logo（本地模型）
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



    const handleManualSync = async (pushOnly: boolean) => {
        const token = localStorage.getItem('auth-token');
        if (!token) {
            alert("未登录，无法同步。请先登录账号。");
            return;
        }

        try {
            console.log(`[测试] 开始手动同步: ${pushOnly ? '仅推送' : '双向交换'}`);
            // 调用 Rust 命令
            const result = await invoke<string>("perform_sync", { token, pushOnly });
            console.log("[测试] 同步返回:", result);

            // 如果是全量同步，同步完后刷新本地 Store 数据
            if (!pushOnly) {
                const syncedData = await invoke<any>("load_assistants");
                if (syncedData) {
                    setDatas("assistants", syncedData);
                }
            }
            alert(pushOnly ? "本地数据已成功推送到后端" : "同步完成：已交换云端和本地数据");
        } catch (err) {
            console.error("[测试] 同步报错:", err);
            alert("同步失败: " + err);
        }
    };


    
    return (
        // 主容器：聊天界面根元素
        <div class="chat-input-container">

            {/* ==================== 消息展示区域 ==================== */}
            <div
                class="chat-messages-area"
                // 话题切换时添加 CSS 类触发过渡动画
                classList={{ 'topic-switching': props.isChangingTopic }}
            >
                {/* 条件渲染：有活跃话题时显示消息列表 */}
                <Show when={props.activeTopic}>
                    {/* 循环渲染消息历史 */}
                    <For each={props.activeTopic?.history}>
                        {(msg: any, index) => (
                            <div
                                class={`message ${msg.role}`}
                                // 动态样式：延迟动画（逐条出现效果）和打字机动画速度
                                style={{
                                    "animation-delay": `${Math.min(index() * 0.03, 0.4)}s`,
                                    "animation-duration": props.typingIndex === index() ? "0.1s" : "0.35s"
                                }}
                            >
                                <div class="message-wrapper">
                                    {/* AI 消息：显示模型 Logo 头像 */}
                                    <Show when={msg.role === 'assistant'}>
                                        <div class="chat-avatar-container ai">
                                            <img
                                                src={getModelLogo(msg.modelId || selectedModel()?.model_id || "")}
                                                alt="AI"
                                                class="chat-avatar-img"
                                            />
                                        </div>
                                    </Show>

                                    {/* 消息内容主体 */}
                                    <div class="message-body">
                                        <div
                                            class="message-content"
                                            // 打字机效果：当前正在生成的消息添加特殊样式
                                            classList={{ 'typing': props.typingIndex === index() }}
                                        >
                                            {/* 用户消息的文件附件卡片 */}
                                            <Show when={msg.role === 'user' && msg.displayFiles && msg.displayFiles.length > 0}>
                                                <For each={msg.displayFiles}>
                                                    {(file: any) => (
                                                        <div class="file-attachment-card">
                                                            <div class="file-icon-wrapper">
                                                                <svg fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                                                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
                                                                </svg>
                                                            </div>
                                                            <div class="file-info">
                                                                <div class="file-name">{file.name}</div>
                                                                <div class="file-meta">已解析</div>
                                                            </div>
                                                        </div>
                                                    )}
                                                </For>
                                            </Show>

                                            {/* 消息文本内容：Markdown 渲染 */}
                                            <div class="message-text-part">
                                                <Markdown content={msg.role === 'user' && msg.displayText !== undefined ? msg.displayText : msg.content} />
                                            </div>
                                        </div>

                                        {/* AI 消息底部：显示模型名称 */}
                                        <Show when={msg.role === 'assistant' && (msg.modelId || selectedModel()?.model_id)}>
                                            <div class="message-model-info">
                                                {msg.modelId || selectedModel()?.model_id}
                                            </div>
                                        </Show>

                                        {/* 消息操作按钮组 */}
                                        <div class="message-actions">
                                            {/* 复制按钮：点击复制消息文本到剪贴板 */}
                                            <button
                                                class="copy-bubble-button"
                                                onClick={(e) => {
                                                    // 1. 在异步动作开始前，立即锁定当前按钮的引用
                                                    const currentBtn = e.currentTarget;
                                                    const text = msg.role === 'user' && msg.displayText !== undefined
                                                        ? msg.displayText
                                                        : msg.content;

                                                    if (!text) return;

                                                    navigator.clipboard.writeText(text).then(() => {
                                                        // 2. 这里使用预先锁定的 currentBtn，而不是 e.currentTarget
                                                        const label = currentBtn.querySelector('span');
                                                        if (label) {
                                                            const originalText = label.innerText;
                                                            currentBtn.classList.add('copied');
                                                            label.innerText = '已复制';

                                                            setTimeout(() => {
                                                                currentBtn.classList.remove('copied');
                                                                label.innerText = originalText;
                                                            }, 2000);
                                                        }
                                                    });
                                                }}
                                            >
                                                <svg fill="none" viewBox="0 0 24 24" stroke="currentColor" style="width: 14px; height: 14px;">
                                                    <path d="M8 5H6a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2v-1M8 5a2 2 0 002 2h2a2 2 0 002-2M8 5a2 2 0 012-2h2a2 2 0 012 2m0 0h2a2 2 0 012 2v3m2 4H10m0 0l3-3m-3 3l3 3" />
                                                </svg>
                                                <span>复制</span>
                                            </button>
                                        </div>
                                    </div>

                                    {/* 用户消息：显示用户头像 */}
                                    <Show when={msg.role === 'user'}>
                                        <div class="chat-avatar-container user">
                                            <img src={globalUserAvatar()} alt="User" class="chat-avatar-img" />
                                        </div>
                                    </Show>
                                </div>
                            </div>
                        )}
                    </For>

                    {/* AI 思考中提示：显示在消息列表底部 */}
                    <Show when={props.isThinking}>
                        <div class="message assistant">
                            <div class="message-wrapper">
                                <div class="chat-avatar-container ai">
                                    <img
                                        src={getModelLogo(selectedModel()?.model_id || "")}
                                        class="chat-avatar-img"
                                    />
                                </div>
                                <div class="message-body">
                                    <div class="message-content" style="opacity: 0.6">
                                        AI 正在思考中...
                                    </div>
                                </div>
                            </div>
                        </div>
                    </Show>
                </Show>
            </div>

            {/* ==================== 文件解析状态指示 ==================== */}
            <Show when={props.isProcessing}>
                <div class="loading-overlay">正在解析文件内容...</div>
            </Show>

            {/* ==================== 待发送文件列表 ==================== */}
            <div class="file-tags-container">
                <For each={props.pendingFiles}>
                    {(file, i) => (
                        <div class="file-tag">
                            {/* 图片类型显示缩略图，其他类型显示文档图标 */}
                            <Show when={file.type === 'image'} fallback={<span class="file-icon">📄</span>}>
                                <img
                                    src={file.content}
                                    style="width: 20px; height: 20px; object-fit: cover; margin-right: 5px; border-radius: 2px;"
                                />
                            </Show>
                            {file.name}
                            {/* 删除按钮：从待发送列表移除 */}
                            <button onClick={() => props.setPendingFiles(p => p.filter((_, idx) => idx !== i()))}>
                                ×
                            </button>
                        </div>
                    )}
                </For>
            </div>

            {/* ==================== 输入区域 ==================== */}
            <div class="chat-input-wrapper">
                <div class="chat-input-unified">
                    {/* 自适应高度文本域 */}
                    <textarea
                        ref={textareaRef}
                        class="unified-textarea"
                        placeholder="输入消息... (Ctrl + Enter 换行)"
                        value={props.inputMessage}
                        onInput={(e) => {
                            // 更新输入值
                            props.setInputMessage(e.currentTarget.value);

                            // 自适应高度逻辑
                            e.currentTarget.style.height = 'auto';
                            const newHeight = e.currentTarget.scrollHeight;
                            e.currentTarget.style.height = `${newHeight}px`;

                            // 最大高度限制 200px，超出显示滚动条
                            if (newHeight > 200) {
                                e.currentTarget.style.overflowY = 'auto';
                                e.currentTarget.style.height = '200px';
                            } else {
                                e.currentTarget.style.overflowY = 'hidden';
                            }
                        }}
                        onKeyDown={(e) => {
                            // Enter 发送（无修饰键），Ctrl+Enter 换行
                            if (e.key === 'Enter' && !e.ctrlKey && !e.shiftKey) {
                                e.preventDefault();
                                props.handleSendMessage();
                                // 发送后重置高度
                                if (textareaRef) textareaRef.style.height = '40px';
                            }
                        }}
                    />

                    {/* 工具栏：上传按钮和发送按钮 */}
                    <div class="input-toolbar">
                        <div class="toolbar-left">


<button
        class="toolbar-icon-btn"
        title="测试：仅推送本地变更"
        style={{ color: '#3b82f6' }} // 蓝色区分
        onClick={() => handleManualSync(true)}
    >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" style="width: 18px;">
            <path d="M12 16V8m0 0l-3 3m3-3l3 3M4 17v1a2 2 0 002 2h12a2 2 0 002-2v-1" />
        </svg>
    </button>

    {/* 测试按钮 2: 双向同步 */}
    <button
        class="toolbar-icon-btn"
        title="测试：全量双向同步"
        style={{ color: '#10b981' }} // 绿色区分
        onClick={() => handleManualSync(false)}
    >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" style="width: 18px;">
            <path d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
        </svg>
    </button>

                            {/* 上传文件按钮：支持多选 */}
                            <button
                                class="toolbar-icon-btn"
                                title="上传文件"
                                onClick={async () => {
                                    const selected = await open({ multiple: true });
                                    if (!selected) return;

                                    // 统一转换为数组处理（单选时返回字符串，多选时返回数组）
                                    const paths = Array.isArray(selected) ? selected : [selected];

                                    // 逐个上传文件
                                    for (const path of paths) {
                                        await props.handleFileUpload(path, 'file');
                                    }
                                }}
                            >
                                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                    <path d="M15.5 5.5L8.5 12.5C7.39543 13.6046 7.39543 15.3954 8.5 16.5C9.60457 17.6046 11.3954 17.6046 12.5 16.5L19.5 9.5C21.1569 7.84315 21.1569 5.15685 19.5 3.5C17.8431 1.84315 15.1569 1.84315 13.5 3.5L6.5 10.5C4.29086 12.7091 4.29086 16.2909 6.5 18.5C8.70914 20.7091 12.2909 20.7091 14.5 18.5L20.5 12.5" />
                                </svg>
                            </button>

                            {/* 上传图片按钮：带格式过滤 */}
                            <button
                                class="toolbar-icon-btn"
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
                                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                                    <rect x="3" y="3" width="18" height="18" rx="2" />
                                    <circle cx="8.5" cy="8.5" r="1.5" fill="currentColor" />
                                    <path d="M21 15L16 10L5 21" stroke-linecap="round" stroke-linejoin="round" />
                                </svg>
                            </button>
                        </div>

                        <div class="toolbar-right">
                            {/* 发送/停止按钮：根据 isThinking 状态切换 */}
                            <button
                                classList={{
                                    'unified-send-btn': true,
                                    'stop-state': props.isThinking  // 停止状态样式
                                }}
                                onClick={() => props.isThinking
                                    ? props.handleStopGeneration()  // 思考中时点击停止
                                    : props.handleSendMessage()      // 空闲时点击发送
                                }
                            >
                                {/* 条件渲染：发送图标或停止图标 */}
                                <Show when={props.isThinking} fallback={
                                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                        <path d="M22 2L11 13M22 2L15 22L11 13M11 13L2 9L22 2" />
                                    </svg>
                                }>
                                    <svg viewBox="0 0 24 24" fill="currentColor">
                                        <rect x="6" y="6" width="12" height="12" rx="2" />
                                    </svg>
                                </Show>
                            </button>
                        </div>
                    </div>
                </div>
            </div>

            {/* ==================== 拖拽上传遮层 ==================== */}
            <Show when={props.isDragging}>
                <div class="drag-drop-overlay">
                    <div class="drag-drop-content">
                        <div class="drag-icons">
                            <div class="drag-icon-card side">
                                <svg width="24" height="24" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24">
                                    <path d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"></path>
                                </svg>
                            </div>
                            <div class="drag-icon-card center">
                                <svg width="32" height="32" fill="none" stroke="currentColor" stroke-width="2.5" viewBox="0 0 24 24">
                                    <path d="M12 16V8m0 0l-3 3m3-3l3 3m-9 8h12"></path>
                                </svg>
                            </div>
                            <div class="drag-icon-card side">
                                <svg width="24" height="24" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24">
                                    <path d="M7 21h10a2 2 0 002-2V9.414a1 1 0 00-.293-.707l-5.414-5.414A1 1 0 0012.586 3H7a2 2 0 00-2 2v14a2 2 0 002 2z"></path>
                                </svg>
                            </div>
                        </div>
                        <h2>上传文件</h2>
                        <p>支持 PDF、Docx、pptx 和图片解析</p>
                    </div>
                </div>
            </Show>
        </div>
    );
};

// 默认导出组件
export default ChatInterface;
import { Component, For, Show, createSignal, onMount, onCleanup } from 'solid-js';
import {
    Assistant,
    Topic,
    datas,
    setDatas,
    currentTopicId,
    setCurrentTopicId,
    saveSingleAssistantToBackend
} from '../store/store';

/**
 * 组件 Props 接口定义
 */
interface TopicSidebarProps {
    /** 侧边栏宽度百分比（0-100） */
    width: number;
    /** 拖拽调整宽度时的鼠标事件回调 */
    onResize: (e: MouseEvent) => void;
    /** 当前选中的助手对象，undefined 表示无选中助手 */
    currentAssistant: Assistant | undefined;
    /** 当前处于重命名编辑状态的话题 ID，null 表示无 */
    editingTopicId: string | null;
    /** 设置重命名状态的回调函数 */
    setEditingTopicId: (id: string | null) => void;
    /** 新建话题的回调函数 */
    addTopic: () => void;
    isCollapsed: boolean;
    onToggle: (e: MouseEvent) => void;
    isResizing: boolean;
}

const createTopic = (name?: string): Topic => ({
    id: Date.now().toString(), // 使用当前时间戳作为唯一标识符
    name: name || `新话题 ${new Date().toLocaleTimeString()}`,
    history: [],
    summary: ""
});

/**
 * 话题侧边栏组件
 * 
 * @component
 * @description 渲染助手的话题列表，支持话题管理操作和宽度调整。
 * 
 * @param {TopicSidebarProps} props - 组件属性
 * @returns {JSX.Element} 话题侧边栏 JSX 元素
 */
const TopicSidebar: Component<TopicSidebarProps> = (props) => {

    /** 
     * 控制菜单 DOM 是否渲染（布尔值）
     * 与 topicMenuState.isOpen 配合实现退出动画
     */
    const [showTopicMenuDiv, setShowTopicMenuDiv] = createSignal(false);

    /** 菜单是否正在执行退出动画，用于添加 CSS 退出动画类名 */
    const [isTopicMenuAnimatingOut, setIsTopicMenuAnimatingOut] = createSignal(false);

    /**
     * 菜单完整状态对象
     * @property isOpen - 是否展开（控制动画状态）
     * @property x, y - 菜单显示位置（视口坐标，相对于触发按钮）
     * @property targetTopicId - 当前菜单操作的目标话题 ID
     */
    const [topicMenuState, setTopicMenuState] = createSignal({
        isOpen: false,
        x: 0,
        y: 0,
        targetTopicId: null as string | null
    });

    /**
     * 组件挂载时：注册全局点击监听，实现点击外部关闭菜单
     * 
     * 清理函数：组件卸载时移除事件监听
     */
    onMount(() => {
        /**
         * 全局点击处理器：点击页面任意位置关闭话题菜单
         */
        const handleTopicClickOutside = () => {
            if (topicMenuState().isOpen) {
                closeTopicMenu();
            }
        };

        window.addEventListener('click', handleTopicClickOutside);
        onCleanup(() => window.removeEventListener('click', handleTopicClickOutside));
    });

    /**
     * 保存话题重命名结果
     * 
     * 数据流：
     * 1. 验证输入非空，否则取消编辑
     * 2. 乐观更新：先修改本地 Store 的话题名称（通过路径导航）
     *    路径：assistants → 匹配 assistant id → topics → 匹配 topic id → name
     * 3. 异步保存：调用 API 持久化整个助手数据到后端
     * 4. 清理状态：退出编辑模式
     * 
     * @param {string} asstId - 所属助手 ID
     * @param {string} topicId - 话题 ID
     * @param {string} newName - 新名称
     */
    const saveTopicRename = async (asstId: string, topicId: string, newName: string) => {
        // 输入验证：空值则取消编辑
        if (!newName.trim()) return props.setEditingTopicId(null);

        // 乐观更新：立即修改本地状态，UI 即时响应
        // SolidJS Store 路径导航语法：逐层定位到目标属性
        setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId, 'name', newName);

        // 异步持久化：保存助手数据到后端
        await saveSingleAssistantToBackend(asstId);

        // 退出编辑模式
        props.setEditingTopicId(null);
    };

    /**
     * 打开话题上下文菜单
     * 
     * 交互逻辑：
     * - 计算菜单位置：基于触发按钮的视口坐标，向左偏移 100px
     * - 显示菜单并设置目标话题 ID
     * 
     * @param {MouseEvent} e - 鼠标事件对象
     * @param {string} topicId - 目标话题 ID
     */
    const openTopicMenu = (e: MouseEvent, topicId: string) => {
        e.stopPropagation(); // 阻止冒泡，防止触发全局点击关闭

        setShowTopicMenuDiv(true);
        setIsTopicMenuAnimatingOut(false);

        // 获取触发按钮的位置信息
        const rect = (e.currentTarget as Element).getBoundingClientRect();

        // 设置菜单位置：按钮下方显示，向左偏移 100px
        setTopicMenuState({
            isOpen: true,
            x: rect.left - 100,
            y: rect.top + rect.height,
            targetTopicId: topicId
        });
    };

    /**
     * 关闭话题菜单（带退出动画）
     * 
     * 动画流程：
     * 1. 立即设置 isOpen=false，触发 CSS 退出动画
     * 2. 设置 isTopicMenuAnimatingOut=true，添加退出动画类
     * 3. 200ms 延迟后从 DOM 移除菜单
     */
    const closeTopicMenu = () => {
        setTopicMenuState(p => ({ ...p, isOpen: false }));
        setIsTopicMenuAnimatingOut(true);

        setTimeout(() => {
            setShowTopicMenuDiv(false);
            setIsTopicMenuAnimatingOut(false);
        }, 200);
    };

    /**
     * 删除话题
     * 
     * 数据流与状态处理：
     * 1. 防御性检查：确保助手至少保留一个话题
     * 2. 乐观更新：从本地话题列表过滤移除
     * 3. 如删除的是当前选中话题，自动切换到第一个话题
     * 4. 异步持久化：保存助手数据到后端
     * 5. 关闭菜单
     * 
     * @param {string} asstId - 所属助手 ID
     * @param {string} topicId - 要删除的话题 ID
     */
    const deleteTopic = async (asstId: string, topicId: string) => {
        const asst = datas.assistants.find(a => a.id === asstId);
        if (!asst) return;

        // 确定是否是删除最后一个话题
        if (asst.topics.length <= 1) {
            // 创建一个全新的“物理”话题，替换掉旧的
            const newT = createTopic('默认话题');

            // 先增加新的，再删除旧的（防止 Store 为空瞬间导致界面崩溃）
            setDatas('assistants', a => a.id === asstId, 'topics', [newT]);
            setCurrentTopicId(newT.id);
        } else {
            // 正常从界面和内存物理移除
            setDatas('assistants', a => a.id === asstId, 'topics', (topics: any[]) =>
                topics.filter((t: Topic) => t.id !== topicId)
            );
            // 如果删的是当前选中的，切到第一个
            if (currentTopicId() === topicId) {
                // 注意：这里要取过滤后的第一个
                const remainingTopics = asst.topics.filter((t:Topic) => t.id !== topicId);
                if (remainingTopics.length > 0) {
                    setCurrentTopicId(remainingTopics[0].id);
                }
            }
        }

        // 调用后端的 save_assistant，它会执行：已存在的更新，不存在的物理插入，
        // 注意：我们在 config.rs 中的 save_assistant 逻辑是先 DELETE 旧消息，这已经是物理操作了。
        await saveSingleAssistantToBackend(asstId);
        closeTopicMenu();
    };

return (
        <div 
            // 基础容器：对应 .dialog-container
            class="relative flex flex-col flex-shrink-0 transition-all duration-300 cubic-bezier[0.4,0,0.2,1] min-w-0"
            classList={{
                'is-collapsed': props.isCollapsed
            }}
            style={{
                width: props.isCollapsed ? '0%' : `${props.width}%`,
                padding: props.isCollapsed ? '0' : '15px',
                "border": props.isCollapsed ? 'none' : `1px solid var(--primary-color)`,
                "box-shadow": props.isCollapsed ? 'none' : `inset 0 0 20px 1px var(--primary-30)`,
                "border-radius": "8px",
                "transition": props.isResizing ? 'none' : 'all 0.3s cubic-bezier(0.4, 0, 0.2, 1)' // 调整宽度时取消过渡，避免卡顿
            }}
        >
            {/* 拖拽把手：对应 .resize-handle.right-handle */}
            <div 
                class="hover:bg-[var(--primary-20)] after:rounded-[2px] after:h-[calc(100%-30px)] after:transition-all after:duration-300 after:ease-in-out after:w-1 after:content-[''] after:bg-[var(--primary-10)] !bg-transparent absolute top-0 bottom-0 left-[-4px] w-1 flex items-center justify-center cursor-ew-resize z-[1000] group transition-colors duration-200"
                classList={{ 
                    'after:h-[calc(100%-20px)]': props.isResizing,
                    'after:bg-[var(--primary-color)]': props.isResizing,
                    'after:shadow-[0_0_10px_var(--primary-color)]': props.isResizing 
                }}
                onMouseDown={(e) => props.onResize(e as MouseEvent)}
            >
                {/* 把手内部的竖线：对应 .resize-handle::after */}
                <div class="absolute w-1 h-[calc(100%-30px)] bg-[var(--primary-10)] rounded-sm transition-all duration-300 group-hover:bg-[var(--primary-color)] group-hover:h-[calc(100%-20px)] group-hover:shadow-[0_0_10px_var(--primary-color)]"></div>

                {/* 折叠按钮：对应 .collapse-indicator */}
                <div 
                    class="collapse-indicator hover:scale-110 pointer-events-auto absolute z-[1001] w-[10px] h-12 bg-[var(--primary-color)] rounded-[20px] backdrop-blur-md cursor-pointer flex items-center justify-center text-black font-bold text-[10px] shadow-[0_0_10px_var(--primary-color)] opacity-0 transition-all duration-200 hover:scale-y-110 hover:opacity-100 group-hover:opacity-100"
                    classList={{ 'opacity-40 !opacity-100 scale-y-100 shadow-[0_0_15px_var(--primary-color)]': props.isCollapsed }}
                    title={props.isCollapsed ? "展开话题栏" : "折叠话题栏"} 
                    onClick={(e) => {
                        e.stopPropagation();
                        props.onToggle(e);
                    }}
                >
                    {props.isCollapsed ? '〈' : '〉'}
                </div>
            </div>

            {/* 内容遮罩层：对应 .collapsed-content-hide */}
            <div 
                class="h-full w-full overflow-hidden hover:overflow-y-auto transition-opacity duration-300"
                classList={{ "opacity-0 pointer-events-none overflow-hidden": props.isCollapsed }}
            >
                <Show when={props.currentAssistant}>
                    {(asst) => (
                        <div class="flex flex-col h-full">
                            <div class="info-header mb-4">
                                <h3 class="text-white text-lg font-medium">{asst().name} 的话题</h3>
                            </div>

                            <button 
                                class="add-topic-button w-full px-3 py-2 bg-transparent border border-[var(--primary-color)] rounded-lg shadow-[inset_0_0_20px_1px var(--primary-30)] text-white cursor-pointer transition-all duration-300 hover:bg-[var(--primary-10)] active:scale-[0.98]" 
                                onClick={props.addTopic}
                            >
                                + 新建话题
                            </button>

                            <div class="topics-list mt-[15px] space-y-1">
                                <For each={asst().topics}>
                                    {(topic) => (
                                        <div
                                            class="group flex items-center justify-between px-3 py-2 cursor-pointer rounded-lg border border-[var(--primary-color)] shadow-[inset_0_0_20px_1px var(--primary-30)] bg-transparent transition-all duration-200 hover:bg-[#2a2a2a]"
                                            classList={{
                                                'active bg-[var(--primary-20)] !border-[var(--primary-color)]': topic.id === currentTopicId()
                                            }}
                                            onClick={() => setCurrentTopicId(topic.id)}
                                        >
                                            <Show
                                                when={props.editingTopicId === topic.id}
                                                fallback={<span class="topic-name text-[#e0e0e0] flex-grow text-[0.9rem] truncate pr-2 select-none">{topic.name}</span>}
                                            >
                                                <input
                                                    class="bg-[#1a1a1a] border border-[var(--primary-color)] rounded px-2 py-0.5 text-white text-[0.85rem] h-5 outline-none w-[80%]"
                                                    value={topic.name}
                                                    ref={(el) => {
                                                        setTimeout(() => {
                                                            el.focus();
                                                            el.select();
                                                        }, 0);
                                                    }}
                                                    onBlur={(e) => saveTopicRename(asst().id, topic.id, e.currentTarget.value)}
                                                    onKeyDown={(e) => e.key === 'Enter' && saveTopicRename(asst().id, topic.id, e.currentTarget.value)}
                                                    onClick={(e) => e.stopPropagation()}
                                                />
                                            </Show>

                                            <button
                                                class="flex items-center justify-center w-[30px] h-[30px] bg-[#1e1e1e] border-none rounded cursor-pointer text-white transition-all duration-200 hover:bg-[var(--primary-5)] active:scale-90 active:bg-[var(--primary-10)] opacity-0 group-hover:opacity-100"
                                                onClick={(e) => openTopicMenu(e as MouseEvent, topic.id)}
                                            >
                                                <svg fill="#FFFFFF" viewBox="0 0 24 24" class="w-[18px]">
                                                    <path d="M10 12a2 2 0 1 0 4 0a2 2 0 0 0-4 0zm0-6a2 2 0 1 0 4 0a2 2 0 0 0-4 0zm0 12a2 2 0 1 0 4 0a2 2 0 0 0-4 0z" />
                                                </svg>
                                            </button>
                                        </div>
                                    )}
                                </For>
                            </div>
                        </div>
                    )}
                </Show>
            </div>

            {/* 右键菜单浮层：对应 .assistant-context-menu */}
            {showTopicMenuDiv() && (
                <div
                    class="fixed z-[100] min-w-[150px] bg-[#2e2e2e] border border-[var(--primary-color)] rounded-lg shadow-xl py-1 origin-top-left"
                    classList={{ 
                        'animate-[menuEnter_0.2s_ease-out_forwards]': !isTopicMenuAnimatingOut(),
                        'animate-[menuExit_0.2s_ease-in_forwards]': isTopicMenuAnimatingOut() 
                    }}
                    style={{
                        top: `${topicMenuState().y}px`,
                        left: `${topicMenuState().x}px`
                    }}
                >
                    <button
                        class="w-full text-left px-3 py-2 text-white bg-none border-none cursor-pointer rounded-lg transition-all duration-200 hover:bg-[var(--primary-10)]"
                        onClick={() => {
                            props.setEditingTopicId(topicMenuState().targetTopicId);
                            closeTopicMenu();
                        }}
                    >
                        重命名
                    </button>

                    <button
                        class="w-full text-left px-3 py-2 text-[#ff4d4d] bg-none border-none cursor-pointer rounded-lg transition-all duration-200 hover:bg-[var(--primary-10)]"
                        onClick={() => deleteTopic(props.currentAssistant!.id, topicMenuState().targetTopicId!)}
                    >
                        删除话题
                    </button>
                </div>
            )}
        </div>
    );
};

export default TopicSidebar;
/**
 * ============================================================================
 * 文件功能摘要
 * ============================================================================
 * 
 * 【核心功能】
 * 话题侧边栏组件，展示当前助手的话题列表，支持话题切换、新建、重命名、删除等操作。
 * 提供可拖拽调整宽度的布局能力，以及右键菜单交互。
 * 
 * 【数据流流向】
 * ┌─────────────────────────────────────────────────────────────────────────┐
 * │  外部数据流入 (Props)                                                    │
 * │  ├── width: number ← 父组件控制的侧边栏宽度百分比                        │
 * │  ├── onResize: (e: MouseEvent) => void ← 拖拽调整宽度的事件回调          │
 * │  ├── currentAssistant: Assistant | undefined ← 当前选中的助手对象      │
 * │  ├── editingTopicId: string | null ← 当前处于重命名编辑状态的话题 ID      │
 * │  ├── setEditingTopicId: (id: string | null) => void ← 设置编辑状态回调   │
 * │  └── addTopic: () => void ← 新建话题的回调函数                           │
 * │                                                                          │
 * │  全局状态流入                                                            │
 * │  ├── datas.assistants ← 助手列表（含话题数据）                           │
 * │  ├── currentTopicId ← 当前选中话题 ID                                    │
 * │  └── setCurrentTopicId ← 设置当前话题 ID                                 │
 * │                                                                          │
 * │  全局状态流出                                                            │
 * │  ├── setDatas() → 更新话题名称（乐观更新）                               │
 * │  ├── setCurrentTopicId() → 切换当前话题                                  │
 * │  └── saveSingleAssistantToBackend() → 持久化助手数据到后端             │
 * │                                                                          │
 * │  本地文件                                                                │
 * │  └── 导入 TopicSidebar.css 样式文件                                      │
 * └─────────────────────────────────────────────────────────────────────────┘
 * 
 * 【组件层级】
 * TopicSidebar (本组件)
 * ├── 拖拽调整手柄 (resize-handle)
 * ├── 对话内容容器
 * │   ├── 助手信息头部 (条件渲染)
 * │   ├── 新建话题按钮
 * │   └── 话题列表 (For 循环)
 * │       ├── 话题项 (点击切换、条件渲染编辑输入框或名称)
 * │       └── 菜单按钮 (打开上下文菜单)
 * └── 上下文菜单 (Portal 方式渲染，条件显示)
 * 
 * 【状态管理】
 * - 菜单状态：控制话题上下文菜单的显示/位置/动画
 * ============================================================================
 */

// SolidJS 核心 API
import { Component, For, Show, createSignal, onMount, onCleanup } from 'solid-js';
// 全局状态管理：助手数据、话题操作、后端持久化
import { 
    Assistant,      // 助手类型定义
    Topic,          // 话题类型定义
    datas,          // 全局数据对象
    setDatas,       // 修改全局数据的 Setter
    currentTopicId, // 当前选中话题 ID（Signal）
    setCurrentTopicId, // 设置当前话题 ID
    saveSingleAssistantToBackend // 保存助手到后端 API
} from '../store/store';
// 本地样式文件
import './TopicSidebar.css';

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
}

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
    // ==================== 本地状态定义 ====================

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

    // ==================== 生命周期钩子 ====================

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

    // ==================== 业务逻辑函数 ====================

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
        // 查找目标助手
        const asst = datas.assistants.find(a => a.id === asstId);
        
        // 防御性检查：至少保留一个话题
        if (!asst || asst.topics.length <= 1) { 
            alert("至少保留一个话题"); 
            return; 
        }
        
        // 乐观更新：从话题列表过滤移除
        setDatas('assistants', a => a.id === asstId, 'topics', topics => 
            topics.filter((t: Topic) => t.id !== topicId)
        );
        
        // 如果删除的是当前选中话题，切换到第一个话题
        if (currentTopicId() === topicId) {
            setCurrentTopicId(asst.topics[0].id);
        }
        
        // 异步持久化
        await saveSingleAssistantToBackend(asstId);
        
        // 关闭菜单
        closeTopicMenu();
    };

    // ==================== 渲染逻辑 ====================

    return (
        // 主容器：宽度由父组件 props 控制
        <div class="dialog-container" style={{ width: `${props.width}%` }}>
            {/* 拖拽调整宽度的手柄：位于右侧 */}
            <div 
                class="resize-handle right-handle" 
                onMouseDown={(e) => props.onResize(e as MouseEvent)}
            />
            
            {/* 内容区：话题列表和相关信息 */}
            <div class="dialog-content">
                {/* 条件渲染：有选中助手时显示话题列表 */}
                <Show when={props.currentAssistant}>
                    {/* SolidJS 的 Show 回调模式：asst 是 Signal，需调用 asst() 获取值 */}
                    {(asst) => (
                        <>
                            {/* 助手信息头部：显示助手名称 */}
                            <div class="info-header">
                                <h3>{asst().name} 的话题</h3>
                            </div>
                            
                            {/* 新建话题按钮 */}
                            <button class="add-topic-button" onClick={props.addTopic}>
                                + 新建话题
                            </button>
                            
                            {/* 话题列表容器 */}
                            <div class="topics-list">
                                {/* 循环渲染话题 */}
                                <For each={asst().topics}>
                                    {(topic) => (
                                        <div 
                                            // 动态类名：active 状态高亮当前选中话题
                                            classList={{ 
                                                'topic-item': true, 
                                                'active': topic.id === currentTopicId() 
                                            }}
                                            // 点击切换到该话题
                                            onClick={() => setCurrentTopicId(topic.id)}
                                        >
                                            {/* 条件渲染：重命名输入框或话题名称 */}
                                            <Show 
                                                when={props.editingTopicId === topic.id} 
                                                fallback={<span class="topic-name">{topic.name}</span>}
                                            >
                                                <input 
                                                    class="rename-input" 
                                                    value={topic.name}
                                                    // 自动聚焦并选中文字：使用 setTimeout 确保 DOM 已挂载
                                                    ref={(el) => { 
                                                        setTimeout(() => { 
                                                            el.focus(); 
                                                            el.select(); 
                                                        }, 0); 
                                                    }}
                                                    // 失焦保存
                                                    onBlur={(e) => saveTopicRename(asst().id, topic.id, e.currentTarget.value)}
                                                    // 回车键保存
                                                    onKeyDown={(e) => e.key === 'Enter' && saveTopicRename(asst().id, topic.id, e.currentTarget.value)}
                                                    // 阻止冒泡，防止触发话题项的点击切换
                                                    onClick={(e) => e.stopPropagation()}
                                                />
                                            </Show>
                                            
                                            {/* 菜单按钮：点击打开上下文菜单 */}
                                            <button 
                                                class="assistant-menu-button" 
                                                onClick={(e) => openTopicMenu(e as MouseEvent, topic.id)}
                                            >
                                                {/* 三点菜单图标 SVG */}
                                                <svg fill="#FFFFFF" viewBox="0 0 24 24" style="width: 18px;">
                                                    <path d="M10 12a2 2 0 1 0 4 0a2 2 0 0 0-4 0zm0-6a2 2 0 1 0 4 0a2 2 0 0 0-4 0zm0 12a2 2 0 1 0 4 0a2 2 0 0 0-4 0z" />
                                                </svg>
                                            </button>
                                        </div>
                                    )}
                                </For>
                            </div>
                        </>
                    )}
                </Show>
            </div>

            {/* 话题上下文菜单：条件渲染，使用 Portal 方式渲染在 body 层级 */}
            {showTopicMenuDiv() && (
                <div 
                    class="assistant-context-menu" 
                    // 动态添加退出动画类
                    classList={{ 'menu-exiting': isTopicMenuAnimatingOut() }}
                    // 绝对定位：基于触发按钮计算的坐标
                    style={{ 
                        top: `${topicMenuState().y}px`, 
                        left: `${topicMenuState().x}px` 
                    }}
                >
                    {/* 重命名按钮：设置编辑状态并关闭菜单 */}
                    <button 
                        class="context-menu-button" 
                        onClick={() => { 
                            props.setEditingTopicId(topicMenuState().targetTopicId); 
                            closeTopicMenu(); 
                        }}
                    >
                        重命名
                    </button>
                    
                    {/* 删除按钮：红色警示样式 */}
                    <button 
                        class="context-menu-button delete" 
                        onClick={() => deleteTopic(props.currentAssistant!.id, topicMenuState().targetTopicId!)}
                    >
                        删除话题
                    </button>
                </div>
            )}
        </div>
    );
};

// 默认导出组件
export default TopicSidebar;
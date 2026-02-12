/**
 * ============================================================================
 * 文件功能摘要
 * ============================================================================
 * 
 * 【核心功能】
 * AI 助手侧边栏组件，提供助手列表展示、切换、重命名、删除等功能。
 * 支持可拖拽调整宽度、右键菜单操作、重命名内联编辑等交互。
 * 
 * 【数据流流向】
 * ┌─────────────────────────────────────────────────────────────────────────┐
 * │  外部数据流入 (Props)                                                    │
 * │  ├── width: number ← 父组件控制的侧边栏宽度百分比                        │
 * │  ├── onResize: 拖拽调整宽度的事件回调                                    │
 * │  ├── editingAsstId: 当前正在重命名的助手 ID                              │
 * │  ├── setEditingAsstId: 设置重命名状态的回调                              │
 * │  └── addAssistant: 新增助手的回调函数                                    │
 * │                                                                          │
 * │  全局状态 (Store)                                                        │
 * │  ├── datas.assistants ← 助手列表数据（含话题信息）                       │
 * │  ├── currentAssistantId ← 当前选中助手 ID                                │
 * │  └── currentTopicId ← 当前选中话题 ID                                    │
 * │                                                                          │
 * │  用户交互输出                                                            │
 * │  ├── setCurrentAssistantId() → 切换当前助手                              │
 * │  ├── setCurrentTopicId() → 同步切换助手首个话题                          │
 * │  ├── setDatas() → 本地更新助手名称（乐观更新）                           │
 * │  ├── saveSingleAssistantToBackend() → 异步保存到后端                     │
 * │  └── deleteAssistantFile() → 异步删除后端文件                            │
 * │                                                                          │
 * │  本地文件                                                                │
 * │  └── 导入 AssistantSidebar.css 样式文件                                  │
 * └─────────────────────────────────────────────────────────────────────────┘
 * 
 * 【组件层级】
 * AssistantSidebar (本组件)
 * ├── 助手列表 (For 循环渲染)
 * │   ├── 助手项 (点击切换、菜单按钮)
 * │   └── 重命名输入框 (条件渲染)
 * ├── 新增助手按钮
 * ├── 右键上下文菜单 (Portal 方式渲染)
 * └── 拖拽调整手柄
 * 
 * 【状态管理】
 * - 菜单状态：控制右键菜单的显示/位置/动画
 * - 重命名状态：由父组件管理，本组件通过 props 接收
 * ============================================================================
 */

// SolidJS 核心 API
import { 
    Component,      // 组件类型定义
    For,            // 列表循环渲染
    Show,           // 条件渲染组件
    createSignal,   // 创建响应式状态
    onMount,        // 组件挂载生命周期
    onCleanup       // 组件卸载清理
} from 'solid-js';

// 全局状态管理：助手数据、当前选中状态、后端交互方法
import { 
    datas,                          // 全局数据对象（含 assistants 数组）
    setDatas,                       // 修改全局数据的 Setter
    currentAssistantId,             // 当前选中助手 ID（Signal）
    setCurrentAssistantId,          // 设置当前助手 ID
    saveSingleAssistantToBackend,   // 保存单个助手到后端 API
    deleteAssistantFile,            // 删除后端助手文件 API
    setCurrentTopicId               // 设置当前话题 ID
} from '../store/store';

// 本地样式文件
import './AssistantSidebar.css';

/**
 * 组件 Props 接口定义
 */
interface AssistantSidebarProps {
    /** 侧边栏宽度百分比（0-100） */
    width: number;
    /** 拖拽调整宽度时的鼠标事件回调 */
    onResize: (e: MouseEvent) => void;
    /** 当前处于重命名编辑状态的助手 ID，null 表示无 */
    editingAsstId: string | null;
    /** 设置重命名状态的回调函数 */
    setEditingAsstId: (id: string | null) => void;
    /** 新增助手的回调函数 */
    addAssistant: () => void;
}

/**
 * 助手侧边栏组件
 * 
 * @component
 * @description 渲染可交互的 AI 助手列表，支持切换、重命名、删除操作。
 *              包含右键菜单和拖拽调整宽度功能。
 * 
 * @param {AssistantSidebarProps} props - 组件属性
 * @returns {JSX.Element} 助手侧边栏 JSX 元素
 */
const AssistantSidebar: Component<AssistantSidebarProps> = (props) => {
    // ==================== 本地状态定义 ====================

    /** 
     * 控制菜单 DOM 是否渲染（布尔值）
     * 与 menuState.isOpen 配合实现退出动画：先 isOpen=false 触发动画，再 showMenuDiv=false 移除 DOM
     */
    const [showMenuDiv, setShowMenuDiv] = createSignal(false);
    
    /** 菜单是否正在执行退出动画，用于添加 CSS 退出动画类名 */
    const [isMenuAnimatingOut, setIsMenuAnimatingOut] = createSignal(false);
    
    /**
     * 菜单完整状态对象
     * @property isOpen - 是否展开（控制动画状态）
     * @property x, y - 菜单显示位置（视口坐标）
     * @property targetId - 当前菜单操作的助手 ID
     */
    const [menuState, setMenuState] = createSignal({ 
        isOpen: false, 
        x: 0, 
        y: 0, 
        targetId: null as string | null 
    });

    /** 菜单关闭延迟定时器 ID，用于清理和防止内存泄漏 */
    let menuCloseTimeoutId: any;

    // ==================== 生命周期钩子 ====================

    /**
     * 组件挂载时：注册全局点击监听，实现点击外部关闭菜单
     * 
     * 清理函数：组件卸载时移除事件监听
     */
    onMount(() => {
        /**
         * 全局点击处理器：点击页面任意位置关闭菜单
         */
        const handleClickOutside = () => {
            if (menuState().isOpen) {
                closeMenu();
            }
        };
        
        // 注册全局点击监听（捕获阶段确保优先执行）
        window.addEventListener('click', handleClickOutside);
        
        // 组件卸载时清理事件监听
        onCleanup(() => window.removeEventListener('click', handleClickOutside));
    });

    // ==================== 业务逻辑函数 ====================

    /**
     * 保存重命名结果
     * 
     * 数据流：
     * 1. 验证输入非空，否则取消编辑
     * 2. 乐观更新：先修改本地 Store 的助手名称
     * 3. 异步保存：调用 API 持久化到后端
     * 4. 清理状态：退出编辑模式
     * 
     * @param {string} id - 助手 ID
     * @param {string} newName - 新名称
     */
    const saveRename = async (id: string, newName: string) => {
        // 输入验证：空值则取消编辑
        if (!newName.trim()) return props.setEditingAsstId(null);
        
        // 乐观更新：立即修改本地状态，UI 即时响应
        setDatas('assistants', a => a.id === id, 'name', newName);
        
        // 异步持久化：保存到后端
        await saveSingleAssistantToBackend(id);
        
        // 退出编辑模式
        props.setEditingAsstId(null);
    };

    /**
     * 打开右键菜单
     * 
     * 交互逻辑：
     * - 如果点击的是已打开菜单的同一助手，则关闭菜单（切换行为）
     * - 否则在新位置打开菜单
     * 
     * @param {MouseEvent} e - 鼠标事件对象
     * @param {string} assistantId - 目标助手 ID
     */
    const openMenu = (e: MouseEvent, assistantId: string) => {
        e.stopPropagation(); // 阻止冒泡，防止触发全局点击关闭
        
        // 切换行为：点击同一助手按钮时关闭菜单
        if (menuState().isOpen && menuState().targetId === assistantId) {
            closeMenu();
            return;
        }
        
        // 显示菜单 DOM
        setShowMenuDiv(true);
        setIsMenuAnimatingOut(false);
        
        // 计算菜单位置：基于按钮的视口坐标
        const rect = (e.currentTarget as Element).getBoundingClientRect();
        setMenuState({ 
            isOpen: true, 
            x: rect.left, 
            y: rect.top + rect.height, // 按钮下方显示
            targetId: assistantId 
        });
    };

    /**
     * 关闭菜单（带退出动画）
     * 
     * 动画流程：
     * 1. 立即设置 isOpen=false，触发 CSS 退出动画
     * 2. 设置 isMenuAnimatingOut=true，添加退出动画类
     * 3. 200ms 延迟后从 DOM 移除菜单
     */
    const closeMenu = () => {
        setMenuState(p => ({ ...p, isOpen: false }));
        setIsMenuAnimatingOut(true);
        
        // 清理可能存在的旧定时器
        clearTimeout(menuCloseTimeoutId);
        
        // 延迟移除 DOM，等待动画完成
        menuCloseTimeoutId = setTimeout(() => {
            setShowMenuDiv(false);
            setIsMenuAnimatingOut(false);
        }, 200);
    };

    /**
     * 删除助手
     * 
     * 数据流与状态处理：
     * 1. 调用后端 API 删除文件
     * 2. 如果被删助手是当前选中，自动切换相邻助手
     * 3. 同步切换话题到目标助手的首个话题
     * 4. 从本地列表过滤移除
     * 
     * @param {string | null} id - 要删除的助手 ID
     */
    const removeAssistant = async (id: string | null) => {
        if (!id) return;
        
        // 异步删除后端文件
        await deleteAssistantFile(id);
        
        // 如果被删的是当前选中助手，需要切换选中状态
        if (currentAssistantId() === id) {
            const idx = datas.assistants.findIndex(a => a.id === id);
            
            // 查找相邻助手（优先上一个，否则下一个）
            const targetAsst = datas.assistants[idx - 1] || datas.assistants[idx + 1];

            if (targetAsst) {
                // 切换到相邻助手
                setCurrentAssistantId(targetAsst.id);
                
                // 同步切换话题：选中目标助手的第一个话题
                if (targetAsst.topics && targetAsst.topics.length > 0) {
                    setCurrentTopicId(targetAsst.topics[0].id);
                }
            } else {
                // 无其他助手，清空选中状态
                setCurrentAssistantId(null);
                setCurrentTopicId(null);
            }
        }
        
        // 从本地列表移除（乐观更新）
        setDatas('assistants', prev => prev.filter(a => a.id !== id));
        
        // 关闭菜单
        closeMenu();
    };

    // ==================== 渲染逻辑 ====================

    return (
        // 主容器：宽度由父组件 props 控制
        <div class="assistant-selector" style={{ width: `${props.width}%` }}>
            
            {/* 内容区：助手列表和新增按钮 */}
            <div class="assistant-content">
                {/* 循环渲染助手列表 */}
                <For each={datas.assistants}>
                    {(assistant) => (
                        <div
                            // 动态类名：active 状态高亮当前选中助手
                            classList={{ 
                                'assistant-item': true, 
                                'active': assistant.id === currentAssistantId() 
                            }}
                            // 点击切换当前助手，并同步切换其首个话题
                            onClick={() => {
                                setCurrentAssistantId(assistant.id);
                                if (assistant.topics && assistant.topics.length > 0) {
                                    setCurrentTopicId(assistant.topics[0].id);
                                }
                            }}
                        >
                            {/* 条件渲染：重命名输入框或助手名称 */}
                            <Show 
                                when={props.editingAsstId === assistant.id} 
                                fallback={<span class="assistant-name">{assistant.name}</span>}
                            >
                                <input
                                    class="rename-input"
                                    value={assistant.name}
                                    // 自动聚焦并选中文字：使用 setTimeout 确保 DOM 已挂载
                                    ref={(el) => { 
                                        setTimeout(() => { 
                                            el.focus(); 
                                            el.select(); 
                                        }, 0); 
                                    }}
                                    // 失焦保存
                                    onBlur={(e) => saveRename(assistant.id, e.currentTarget.value)}
                                    // 回车键保存
                                    onKeyDown={(e) => e.key === 'Enter' && saveRename(assistant.id, e.currentTarget.value)}
                                    // 阻止冒泡，防止触发助手项的点击切换
                                    onClick={(e) => e.stopPropagation()}
                                />
                            </Show>
                            
                            {/* 菜单按钮：点击打开上下文菜单 */}
                            <button 
                                class="assistant-menu-button" 
                                onClick={(e) => openMenu(e as MouseEvent, assistant.id)}
                            >
                                {/* 三点菜单图标 SVG */}
                                <svg 
                                    xmlns="http://www.w3.org/2000/svg" 
                                    fill="#FFFFFF" 
                                    viewBox="0 0 24 24" 
                                    stroke-width={1.5} 
                                    class="size-6"
                                >
                                    <path 
                                        stroke-linecap="round" 
                                        stroke-linejoin="round" 
                                        d="M10 12a2 2 0 1 0 4 0a2 2 0 0 0-4 0zm0-6a2 2 0 1 0 4 0a2 2 0 0 0-4 0zm0 12a2 2 0 1 0 4 0a2 2 0 0 0-4 0z" 
                                    />
                                </svg>
                            </button>
                        </div>
                    )}
                </For>
                
                {/* 新增助手按钮：调用父组件传入的回调 */}
                <button class="add-assistant-button" onClick={props.addAssistant}>
                    + 新增助手
                </button>
            </div>

            {/* 上下文菜单：条件渲染，使用 Portal 方式渲染在 body 层级 */}
            {showMenuDiv() && (
                <div 
                    class="assistant-context-menu" 
                    // 动态添加退出动画类
                    classList={{ 'menu-exiting': isMenuAnimatingOut() }}
                    // 绝对定位：基于触发按钮的坐标
                    style={{ 
                        top: `${menuState().y}px`, 
                        left: `${menuState().x}px` 
                    }}
                >
                    {/* 重命名按钮：设置编辑状态并关闭菜单 */}
                    <button 
                        class="context-menu-button" 
                        onClick={() => { 
                            props.setEditingAsstId(menuState().targetId); 
                            closeMenu(); 
                        }}
                    >
                        重命名
                    </button>
                    
                    {/* 删除按钮：红色警示样式 */}
                    <button 
                        class="context-menu-button delete" 
                        onClick={() => removeAssistant(menuState().targetId)}
                    >
                        删除助手
                    </button>
                </div>
            )}

            {/* 拖拽调整宽度的手柄：位于左侧 */}
            <div 
                class="resize-handle left-handle" 
                onMouseDown={(e) => props.onResize(e as MouseEvent)}
            />
        </div>
    );
};

// 默认导出组件
export default AssistantSidebar;
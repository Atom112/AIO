import { Component, For, Show, createSignal, onMount, onCleanup } from 'solid-js';
import { datas, setDatas, currentAssistantId, setCurrentAssistantId, saveSingleAssistantToBackend, deleteAssistantFile, setCurrentTopicId } from '../store/store';
import Icon from './Icon';

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
    isCollapsed: boolean;
    onToggle: (e: MouseEvent) => void;
    isResizing: boolean;
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
        if (!id || id === "default-assistant-id") {
            alert("默认助手无法删除");
            return;
        }

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

    return (
        <div
            // 基础容器：对应 .assistant-selector
            class="relative flex flex-col flex-shrink-0 transition-all duration-300 ease-[cubic-bezier(0.4,0,0.2,1)] min-w-0"
            style={{
                width: props.isCollapsed ? '0%' : `${props.width}%`,
                padding: props.isCollapsed ? '0' : '15px',
                "border": props.isCollapsed ? 'none' : `1px solid var(--primary-color)`,
                "box-shadow": props.isCollapsed ? 'none' : `inset 0 0 20px 1px var(--primary-30)`,
                "border-radius": "8px",
                "transition": props.isResizing ? 'none' : 'all 0.3s cubic-bezier(0.4, 0, 0.2, 1)' // 调整宽度时取消过渡，避免卡顿
            }}
        >
            {/* 内容遮罩层：对应 .collapsed-content-hide */}
            <div
                class="h-full w-full overflow-hidden hover:overflow-y-auto transition-opacity duration-300"
                classList={{ "opacity-0 pointer-events-none overflow-hidden": props.isCollapsed }}
            >
                <For each={datas.assistants}>
                    {(assistant) => (
                        <div
                            class="group sidebar-item my-1"
                            classList={{
                                'active bg-[var(--primary-20)] !border-[var(--primary-color)]': assistant.id === currentAssistantId()
                            }}
                            onClick={() => {
                                setCurrentAssistantId(assistant.id);
                                if (assistant.topics && assistant.topics.length > 0) {
                                    setCurrentTopicId(assistant.topics[0].id);
                                }
                            }}
                        >
                            <Show
                                when={props.editingAsstId === assistant.id && assistant.id !== "default-assistant-id"}
                                fallback={<span class="assistant-name flex-grow text-[0.95rem] overflow-hidden pr-[10px] text-ellipsis whitespace-nowrap text-white">{assistant.name}</span>}
                            >
                                <input
                                    class="rename-input bg-[#1e1e1e] border border-[var(--primary-color)] rounded px-1 py-0.5 text-white text-[0.95rem] outline-none w-[80%]"
                                    value={assistant.name}
                                    ref={(el) => {
                                        setTimeout(() => {
                                            el.focus();
                                            el.select();
                                        }, 0);
                                    }}
                                    onBlur={(e) => saveRename(assistant.id, e.currentTarget.value)}
                                    onKeyDown={(e) => e.key === 'Enter' && saveRename(assistant.id, e.currentTarget.value)}
                                    onClick={(e) => e.stopPropagation()}
                                />
                            </Show>

                            <button
                                class="dot-menu-btn"
                                onClick={(e) => openMenu(e as MouseEvent, assistant.id)}
                            >
                                <Icon src="/icons/app-logo/dot-menu.svg" class="w-[18px] h-[18px]" />
                            </button>
                        </div>
                    )}
                </For>

                <button
                    class="w-full mt-[10px] px-3 py-2 bg-none glow-border rounded-lg text-white cursor-pointer transition-all duration-300 hover:bg-[var(--primary-10)]"
                    onClick={props.addAssistant}
                >
                    + 新增助手
                </button>
            </div>

            {/* 右键菜单：对应 .assistant-context-menu 及其动画 */}
            {showMenuDiv() && (
                <div
                    class="context-menu"
                    style={{
                        top: `${menuState().y}px`,
                        left: `${menuState().x}px`
                    }}
                >
                    <button
                        class="context-menu-item text-white disabled:opacity-30"
                        disabled={menuState().targetId === "default-assistant-id"}
                        onClick={() => {
                            props.setEditingAsstId(menuState().targetId);
                            closeMenu();
                        }}
                    >
                        重命名
                    </button>

                    <button
                        class="context-menu-item text-[#ff4d4d] disabled:opacity-30"
                        disabled={menuState().targetId === "default-assistant-id"}
                        onClick={() => removeAssistant(menuState().targetId)}
                    >
                        删除助手
                    </button>
                </div>
            )}

            {/* 拖拽把手：对应 .resize-handle.left-handle */}
            <div
                class="hover:bg-[var(--primary-20)] after:rounded-[2px] after:h-[calc(100%-30px)] after:transition-all after:duration-300 after:ease-in-out after:w-1 after:content-[''] after:bg-[var(--primary-10)] !bg-transparent absolute top-0 bottom-0 right-[-4px] w-1 flex items-center justify-center cursor-ew-resize z-[1000] group transition-colors duration-200"
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
                    class="hover:scale-110 pointer-events-auto absolute z-[1001] w-[10px] h-12 bg-[var(--primary-color)] rounded-[20px] backdrop-blur-md cursor-pointer flex items-center justify-center text-black font-bold text-[10px] shadow-[0_0_10px_var(--primary-color)] opacity-0 transition-all duration-200 hover:scale-y-110 hover:opacity-100 group-hover:opacity-100"
                    classList={{ 'opacity-40 !opacity-100 scale-y-100 shadow-[0_0_15px_var(--primary-color)]': props.isCollapsed }}
                    title={props.isCollapsed ? "展开助手栏" : "折叠助手栏"}
                    onClick={(e) => {
                        e.stopPropagation();
                        props.onToggle(e);
                    }}
                >
                    {props.isCollapsed ? '〉' : '〈'}
                </div>
            </div>
        </div>
    );
};

export default AssistantSidebar;
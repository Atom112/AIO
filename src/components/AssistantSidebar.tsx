import { Component, For, Show, createSignal, onMount, onCleanup } from 'solid-js';
import { datas, setDatas, currentAssistantId, setCurrentAssistantId, saveSingleAssistantToBackend, deleteAssistantFile, setCurrentTopicId } from '../store/store';
import Icon from './Icon';

interface AssistantSidebarProps {
    width: number; // 侧边栏宽度百分比
    onResize: (e: MouseEvent) => void; // 拖拽调整宽度的事件处理
    editingAsstId: string | null; // 当前编辑中的助手 ID
    setEditingAsstId: (id: string | null) => void; // 设置编辑状态
    addAssistant: () => void; // 新增助手
    isCollapsed: boolean; // 侧边栏是否折叠
    onToggle: (e: MouseEvent) => void; // 切换折叠状态
    isResizing: boolean; // 是否正在调整宽度
}

/**
 * 助手侧边栏组件
 * @param {AssistantSidebarProps} props - 组件属性
 * @returns {JSX.Element} 助手侧边栏 JSX 元素
 */
const AssistantSidebar: Component<AssistantSidebarProps> = (props) => {

    const [showMenuDiv, setShowMenuDiv] = createSignal(false); // 菜单 DOM 是否渲染
    const [isMenuAnimatingOut, setIsMenuAnimatingOut] = createSignal(false); // 菜单是否执行退出动画
    const [menuState, setMenuState] = createSignal({
        isOpen: false,
        x: 0,
        y: 0,
        targetId: null as string | null
    }); // 菜单状态（位置、打开状态、目标助手ID）

    let menuCloseTimeoutId: any; // 菜单关闭延迟定时器 ID

    /**
     * 注册全局点击监听，实现点击外部关闭菜单
     */
    onMount(() => {
        const handleClickOutside = () => {
            if (menuState().isOpen) {
                closeMenu();
            }
        };

        window.addEventListener('click', handleClickOutside);
        onCleanup(() => window.removeEventListener('click', handleClickOutside));
    });

    /**
     * 保存重命名结果
     * @param {string} id - 助手 ID
     * @param {string} newName - 新名称
     */
    const saveRename = async (id: string, newName: string) => {
        if (!newName.trim()) return props.setEditingAsstId(null);

        setDatas('assistants', a => a.id === id, 'name', newName);
        await saveSingleAssistantToBackend(id);
        props.setEditingAsstId(null);
    };

    /**
     * 打开右键菜单
     * @param {MouseEvent} e - 鼠标事件对象
     * @param {string} assistantId - 目标助手 ID
     */
    const openMenu = (e: MouseEvent, assistantId: string) => {
        e.stopPropagation();

        if (menuState().isOpen && menuState().targetId === assistantId) {
            closeMenu();
            return;
        }

        setShowMenuDiv(true);
        setIsMenuAnimatingOut(false);

        const rect = (e.currentTarget as Element).getBoundingClientRect();
        setMenuState({
            isOpen: true,
            x: rect.left,
            y: rect.top + rect.height,
            targetId: assistantId
        });
    };

    /**
     * 关闭菜单并执行退出动画
     */
    const closeMenu = () => {
        setMenuState(p => ({ ...p, isOpen: false }));
        setIsMenuAnimatingOut(true);

        clearTimeout(menuCloseTimeoutId);
        menuCloseTimeoutId = setTimeout(() => {
            setShowMenuDiv(false);
            setIsMenuAnimatingOut(false);
        }, 200);
    };

    /**
     * 删除助手，若当前选中则自动切换至相邻助手
     * @param {string | null} id - 要删除的助手 ID
     */
    const removeAssistant = async (id: string | null) => {
        if (!id || id === "default-assistant-id") {
            alert("默认助手无法删除");
            return;
        }

        await deleteAssistantFile(id);

        if (currentAssistantId() === id) {
            const idx = datas.assistants.findIndex(a => a.id === id);
            const targetAsst = datas.assistants[idx - 1] || datas.assistants[idx + 1];

            if (targetAsst) {
                setCurrentAssistantId(targetAsst.id);
                if (targetAsst.topics && targetAsst.topics.length > 0) {
                    setCurrentTopicId(targetAsst.topics[0].id);
                }
            } else {
                setCurrentAssistantId(null);
                setCurrentTopicId(null);
            }
        }

        setDatas('assistants', prev => prev.filter(a => a.id !== id));
        closeMenu();
    };

    return (
        <div
            class="relative flex flex-col flex-shrink-0 transition-all duration-300 ease-[cubic-bezier(0.4,0,0.2,1)] min-w-0"
            style={{
                width: props.isCollapsed ? '0%' : `${props.width}%`,
                padding: props.isCollapsed ? '0' : '15px',
                "border": props.isCollapsed ? 'none' : `1px solid var(--primary-color)`,
                "box-shadow": props.isCollapsed ? 'none' : `inset 0 0 20px 1px var(--primary-30)`,
                "border-radius": "8px",
                "transition": props.isResizing ? 'none' : 'all 0.3s cubic-bezier(0.4, 0, 0.2, 1)'
            }}
        >
            <div
                class="h-full w-full overflow-hidden hover:overflow-y-auto transition-opacity duration-300"
                classList={{ "opacity-0 pointer-events-none overflow-hidden": props.isCollapsed }}
            >
                <For each={datas.assistants}>
                    {(assistant) => (
                        <div
                            class="group sidebar-item my-1"
                            classList={{
                                'active bg-pri-20 !border-pri': assistant.id === currentAssistantId()
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
                                    class="rename-input bg-dark border border-pri rounded px-1 py-0.5 text-white text-[0.95rem] outline-none w-[80%]"
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
                    class="w-full mt-[10px] px-3 py-2 bg-none glow-border rounded-lg text-white cursor-pointer transition-all duration-300 hover:bg-pri-10"
                    onClick={props.addAssistant}
                >
                    + 新增助手
                </button>
            </div>

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

            <div
                class="hover:bg-pri-20 after:rounded-[2px] after:h-[calc(100%-30px)] after:transition-all after:duration-300 after:ease-in-out after:w-1 after:content-[''] after:bg-pri-10 !bg-transparent absolute top-0 bottom-0 right-[-4px] w-1 flex items-center justify-center cursor-ew-resize z-[1000] group transition-colors duration-200"
                classList={{
                    'after:h-[calc(100%-20px)]': props.isResizing,
                    'after:bg-pri': props.isResizing,
                    'after:shadow-[0_0_10px_var(--primary-color)]': props.isResizing
                }}
                onMouseDown={(e) => props.onResize(e as MouseEvent)}
            >
                <div class="absolute w-1 h-[calc(100%-30px)] bg-pri-10 rounded-sm transition-all duration-300 group-hover:bg-pri group-hover:h-[calc(100%-20px)] group-hover:shadow-[0_0_10px_var(--primary-color)]"></div>

                <div
                    class="hover:scale-110 pointer-events-auto absolute z-[1001] w-[10px] h-12 bg-pri rounded-[20px] backdrop-blur-md cursor-pointer flex items-center justify-center text-black font-bold text-[10px] shadow-[0_0_10px_var(--primary-color)] opacity-0 transition-all duration-200 hover:scale-y-110 hover:opacity-100 group-hover:opacity-100"
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
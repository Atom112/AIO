import { Component, For, Show, createSignal, onMount, onCleanup } from 'solid-js';
import { Portal } from 'solid-js/web';
import { datas, setDatas, currentAssistantId, setCurrentAssistantId, saveSingleAssistantToBackend, deleteAssistantFile, setCurrentTopicId } from '../store/store';
import Icon from './Icon';

interface AssistantSidebarProps {
    width: number;
    onResize: (e: MouseEvent) => void;
    editingAsstId: string | null;
    setEditingAsstId: (id: string | null) => void;
    addAssistant: () => void;
    onOpenSettings: (id: string) => void;
    isCollapsed: boolean;
    onToggle: (e: MouseEvent) => void;
    isResizing: boolean;
}

const AssistantSidebar: Component<AssistantSidebarProps> = (props) => {

    const [showMenuDiv, setShowMenuDiv] = createSignal(false);
    const [isMenuAnimatingOut, setIsMenuAnimatingOut] = createSignal(false);
    const [menuState, setMenuState] = createSignal({
        isOpen: false,
        x: 0,
        y: 0,
        targetId: null as string | null
    });

    let menuCloseTimeoutId: any;

    onMount(() => {
        const handleClickOutside = () => {
            if (menuState().isOpen) closeMenu();
        };
        window.addEventListener('click', handleClickOutside);
        onCleanup(() => window.removeEventListener('click', handleClickOutside));
    });

    const saveRename = async (id: string, newName: string) => {
        if (!newName.trim()) return props.setEditingAsstId(null);
        setDatas('assistants', a => a.id === id, 'name', newName);
        await saveSingleAssistantToBackend(id);
        props.setEditingAsstId(null);
    };

    const openMenu = (e: MouseEvent, assistantId: string) => {
        e.stopPropagation();
        if (menuState().isOpen && menuState().targetId === assistantId) { closeMenu(); return; }
        setShowMenuDiv(true);
        setIsMenuAnimatingOut(false);
        const rect = (e.currentTarget as Element).getBoundingClientRect();
        setMenuState({ isOpen: true, x: rect.left, y: rect.top + rect.height, targetId: assistantId });
    };

    const closeMenu = () => {
        setMenuState(p => ({ ...p, isOpen: false }));
        setIsMenuAnimatingOut(true);
        clearTimeout(menuCloseTimeoutId);
        menuCloseTimeoutId = setTimeout(() => { setShowMenuDiv(false); setIsMenuAnimatingOut(false); }, 200);
    };

    const removeAssistant = async (id: string | null) => {
        if (!id || id === "default-assistant-id") { alert("默认助手无法删除"); return; }
        await deleteAssistantFile(id);
        if (currentAssistantId() === id) {
            const idx = datas.assistants.findIndex(a => a.id === id);
            const targetAsst = datas.assistants[idx - 1] || datas.assistants[idx + 1];
            if (targetAsst) {
                setCurrentAssistantId(targetAsst.id);
                if (targetAsst.topics?.length > 0) setCurrentTopicId(targetAsst.topics[0].id);
            } else { setCurrentAssistantId(null); setCurrentTopicId(null); }
        }
        setDatas('assistants', prev => prev.filter(a => a.id !== id));
        closeMenu();
    };

    return (
        <div
            class="relative flex flex-col flex-shrink-0 min-w-0"
            style={`width: ${props.isCollapsed ? '0%' : `${props.width}%`}; padding: ${props.isCollapsed ? '0' : '15px'}; background: ${props.isCollapsed ? 'none' : 'rgba(18, 22, 35, 0.25)'}; backdrop-filter: ${props.isCollapsed ? 'none' : 'blur(30px)'}; -webkit-backdrop-filter: ${props.isCollapsed ? 'none' : 'blur(30px)'}; border: ${props.isCollapsed ? 'none' : '1px solid rgba(255, 255, 255, 0.06)'}; border-radius: 12px; box-shadow: ${props.isCollapsed ? 'none' : '0 8px 32px rgba(0, 0, 0, 0.2)'}; transition: ${props.isResizing ? 'none' : 'all 0.3s cubic-bezier(0.4, 0, 0.2, 1)'};`}
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
                                '!bg-[rgba(124,154,191,0.15)] !border-[rgba(124,154,191,0.15)]': assistant.id === currentAssistantId()
                            }}
                            onClick={() => {
                                setCurrentAssistantId(assistant.id);
                                if (assistant.topics?.length > 0) setCurrentTopicId(assistant.topics[0].id);
                            }}
                        >
                            <Show
                                when={props.editingAsstId === assistant.id && assistant.id !== "default-assistant-id"}
                                fallback={<span class="flex-grow text-[0.95rem] overflow-hidden pr-[10px] text-ellipsis whitespace-nowrap" style="color: rgba(255,255,255,0.85);">{assistant.name}</span>}
                            >
                                <input
                                    class="rounded px-1 py-0.5 text-[0.95rem] outline-none w-[80%]"
                                    style="background: rgba(0,0,0,0.3); border: 1px solid rgba(255,255,255,0.1); color: rgba(255,255,255,0.85);"
                                    value={assistant.name}
                                    ref={(el) => { setTimeout(() => { el.focus(); el.select(); }, 0); }}
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
                    class="w-full mt-[10px] px-3 py-2 rounded-lg cursor-pointer transition-all duration-300"
                    style="background: rgba(255,255,255,0.04); border: 1px solid rgba(255,255,255,0.06); color: rgba(255,255,255,0.6);"
                    onClick={props.addAssistant}
                    onMouseEnter={(e) => e.currentTarget.style.background = 'rgba(124,154,191,0.12)'}
                    onMouseLeave={(e) => e.currentTarget.style.background = 'rgba(255,255,255,0.04)'}
                >
                    + 新增助手
                </button>
            </div>

            {showMenuDiv() && (
                <Portal>
                    <div
                        class="context-menu"
                        classList={{ closing: isMenuAnimatingOut() }}
                        style={`top: ${menuState().y}px; left: ${menuState().x}px;`}
                        onClick={(e) => e.stopPropagation()}
                    >
                        <button
                            class="context-menu-item"
                            onClick={() => { const id = menuState().targetId; if (id) props.onOpenSettings(id); closeMenu(); }}
                        >设置</button>
                        <button
                            class="context-menu-item disabled:opacity-30"
                            disabled={menuState().targetId === "default-assistant-id"}
                            onClick={() => { props.setEditingAsstId(menuState().targetId); closeMenu(); }}
                        >重命名</button>
                        <button
                            class="context-menu-item disabled:opacity-30"
                            style="color: rgba(255,77,77,0.8);"
                            disabled={menuState().targetId === "default-assistant-id"}
                            onClick={() => removeAssistant(menuState().targetId)}
                        >删除助手</button>
                    </div>
                </Portal>
            )}

            {/* 调整大小把手 */}
            <div
                class="absolute top-0 bottom-0 right-[-4px] w-1 flex items-center justify-center cursor-ew-resize z-[1000] group"
                onMouseDown={(e) => props.onResize(e as MouseEvent)}
            >
                <div
                    class="w-[3px] h-[calc(100%-40px)] rounded-full transition-all duration-300"
                    style={`background: ${props.isResizing ? 'rgba(124,154,191,0.4)' : 'rgba(255,255,255,0.08)'}; box-shadow: ${props.isResizing ? '0 0 8px rgba(124,154,191,0.3)' : 'none'};`}
                ></div>

                <div
                    class="absolute z-[1001] w-[10px] h-12 rounded-[20px] backdrop-blur-md cursor-pointer flex items-center justify-center text-xs font-bold transition-all duration-200 opacity-0 group-hover:opacity-100 hover:scale-110"
                    style="background: rgba(255,255,255,0.08); color: rgba(255,255,255,0.6); box-shadow: 0 2px 8px rgba(0,0,0,0.3);"
                    classList={{ '!opacity-100': props.isCollapsed }}
                    title={props.isCollapsed ? "展开助手栏" : "折叠助手栏"}
                    onClick={(e) => { e.stopPropagation(); props.onToggle(e); }}
                >
                    {props.isCollapsed ? '〉' : '〈'}
                </div>
            </div>
        </div>
    );
};

export default AssistantSidebar;

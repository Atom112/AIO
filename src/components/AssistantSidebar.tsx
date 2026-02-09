import { Component, For, Show, createSignal, onMount, onCleanup } from 'solid-js';
import { Assistant, datas, setDatas, currentAssistantId, setCurrentAssistantId, saveSingleAssistantToBackend, deleteAssistantFile } from '../store/store';
import './AssistantSidebar.css';

interface AssistantSidebarProps {
    width: number;
    onResize: (e: MouseEvent) => void;
    editingAsstId: string | null;
    setEditingAsstId: (id: string | null) => void;
    addAssistant: () => void;
}

const AssistantSidebar: Component<AssistantSidebarProps> = (props) => {
    const [showMenuDiv, setShowMenuDiv] = createSignal(false);
    const [isMenuAnimatingOut, setIsMenuAnimatingOut] = createSignal(false);
    const [menuState, setMenuState] = createSignal({ isOpen: false, x: 0, y: 0, targetId: null as string | null });

    let menuCloseTimeoutId: any;

    onMount(() => {
        const handleClickOutside = () => {
            if (menuState().isOpen) {
                closeMenu();
            }
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
        if (menuState().isOpen && menuState().targetId === assistantId) {
            closeMenu();
            return;
        }
        setShowMenuDiv(true);
        setIsMenuAnimatingOut(false);
        const rect = (e.currentTarget as Element).getBoundingClientRect();
        setMenuState({ isOpen: true, x: rect.left, y: rect.top + rect.height, targetId: assistantId });
    };

    const closeMenu = () => {
        setMenuState(p => ({ ...p, isOpen: false }));
        setIsMenuAnimatingOut(true);
        clearTimeout(menuCloseTimeoutId);
        menuCloseTimeoutId = setTimeout(() => {
            setShowMenuDiv(false);
            setIsMenuAnimatingOut(false);
        }, 200);
    };

    const removeAssistant = async (id: string | null) => {
        if (!id) return;
        await deleteAssistantFile(id);
        if (currentAssistantId() === id) {
            const idx = datas.assistants.findIndex(a => a.id === id);
            setCurrentAssistantId(datas.assistants[idx - 1]?.id || datas.assistants[idx + 1]?.id || null);
        }
        setDatas('assistants', prev => prev.filter(a => a.id !== id));
        closeMenu();
    };

    return (
        <div class="assistant-selector" style={{ width: `${props.width}%` }}>
            <div class="assistant-content">
                <For each={datas.assistants}>
                    {(assistant) => (
                        <div
                            classList={{ 'assistant-item': true, 'active': assistant.id === currentAssistantId() }}
                            onClick={() => setCurrentAssistantId(assistant.id)}
                        >
                            <Show when={props.editingAsstId === assistant.id} fallback={<span class="assistant-name">{assistant.name}</span>}>
                                <input
                                    class="rename-input"
                                    value={assistant.name}
                                    ref={(el) => { setTimeout(() => { el.focus(); el.select(); }, 0); }}
                                    onBlur={(e) => saveRename(assistant.id, e.currentTarget.value)}
                                    onKeyDown={(e) => e.key === 'Enter' && saveRename(assistant.id, e.currentTarget.value)}
                                    onClick={(e) => e.stopPropagation()}
                                />
                            </Show>
                            <button class="assistant-menu-button" onClick={(e) => openMenu(e as MouseEvent, assistant.id)}>
                                <svg xmlns="http://www.w3.org/2000/svg" fill="#FFFFFF" viewBox="0 0 24 24" stroke-width={1.5} class="size-6">
                                    <path stroke-linecap="round" stroke-linejoin="round" d="M10 12a2 2 0 1 0 4 0a2 2 0 0 0-4 0zm0-6a2 2 0 1 0 4 0a2 2 0 0 0-4 0zm0 12a2 2 0 1 0 4 0a2 2 0 0 0-4 0z" />
                                </svg>
                            </button>
                        </div>
                    )}
                </For>
                <button class="add-assistant-button" onClick={props.addAssistant}>+ 新增助手</button>
            </div>

            {showMenuDiv() && (
                <div class="assistant-context-menu" classList={{ 'menu-exiting': isMenuAnimatingOut() }} style={{ top: `${menuState().y}px`, left: `${menuState().x}px` }}>
                    <button class="context-menu-button" onClick={() => { props.setEditingAsstId(menuState().targetId); closeMenu(); }}>重命名</button>
                    <button class="context-menu-button delete" onClick={() => removeAssistant(menuState().targetId)}>删除助手</button>
                </div>
            )}

            <div class="resize-handle left-handle" onMouseDown={(e) => props.onResize(e as MouseEvent)}></div>
        </div>
    );
};

export default AssistantSidebar;

import { Component, For, Show, createSignal, onMount, onCleanup } from 'solid-js';
import {
    Assistant, Topic, datas, setDatas, currentTopicId, setCurrentTopicId, saveSingleAssistantToBackend
} from '../store/store';
import Icon from './Icon';

interface TopicSidebarProps {
    width: number;
    onResize: (e: MouseEvent) => void;
    currentAssistant: Assistant | undefined;
    editingTopicId: string | null;
    setEditingTopicId: (id: string | null) => void;
    addTopic: () => void;
    isCollapsed: boolean;
    onToggle: (e: MouseEvent) => void;
    isResizing: boolean;
}

const createTopic = (name?: string): Topic => ({
    id: Date.now().toString(),
    name: name || `新话题 ${new Date().toLocaleTimeString()}`,
    history: [],
    summary: ""
});

const TopicSidebar: Component<TopicSidebarProps> = (props) => {
    const [showTopicMenuDiv, setShowTopicMenuDiv] = createSignal(false);
    const [isTopicMenuAnimatingOut, setIsTopicMenuAnimatingOut] = createSignal(false);
    const [topicMenuState, setTopicMenuState] = createSignal({
        isOpen: false, x: 0, y: 0, targetTopicId: null as string | null
    });

    onMount(() => {
        const h = () => { if (topicMenuState().isOpen) closeTopicMenu(); };
        window.addEventListener('click', h);
        onCleanup(() => window.removeEventListener('click', h));
    });

    const saveTopicRename = async (asstId: string, topicId: string, newName: string) => {
        if (!newName.trim()) return props.setEditingTopicId(null);
        setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId, 'name', newName);
        await saveSingleAssistantToBackend(asstId);
        props.setEditingTopicId(null);
    };

    const openTopicMenu = (e: MouseEvent, topicId: string) => {
        e.stopPropagation();
        setShowTopicMenuDiv(true);
        setIsTopicMenuAnimatingOut(false);
        const rect = (e.currentTarget as Element).getBoundingClientRect();
        setTopicMenuState({ isOpen: true, x: rect.left - 100, y: rect.top + rect.height, targetTopicId: topicId });
    };

    const closeTopicMenu = () => {
        setTopicMenuState(p => ({ ...p, isOpen: false }));
        setIsTopicMenuAnimatingOut(true);
        setTimeout(() => { setShowTopicMenuDiv(false); setIsTopicMenuAnimatingOut(false); }, 200);
    };

    const deleteTopic = async (asstId: string, topicId: string) => {
        const asst = datas.assistants.find(a => a.id === asstId);
        if (!asst) return;
        if (asst.topics.length <= 1) {
            const newT = createTopic('默认话题');
            setDatas('assistants', a => a.id === asstId, 'topics', [newT]);
            setCurrentTopicId(newT.id);
        } else {
            setDatas('assistants', a => a.id === asstId, 'topics', (topics: any[]) =>
                topics.filter((t: Topic) => t.id !== topicId));
            if (currentTopicId() === topicId) {
                const remaining = asst.topics.filter((t: Topic) => t.id !== topicId);
                if (remaining.length > 0) setCurrentTopicId(remaining[0].id);
            }
        }
        await saveSingleAssistantToBackend(asstId);
        closeTopicMenu();
    };

    return (
        <div
            class="relative flex flex-col flex-shrink-0 min-w-0"
            style={`width: ${props.isCollapsed ? '0%' : `${props.width}%`}; padding: ${props.isCollapsed ? '0' : '15px'}; background: ${props.isCollapsed ? 'none' : 'rgba(18, 22, 35, 0.25)'}; backdrop-filter: ${props.isCollapsed ? 'none' : 'blur(30px)'}; -webkit-backdrop-filter: ${props.isCollapsed ? 'none' : 'blur(30px)'}; border: ${props.isCollapsed ? 'none' : '1px solid rgba(255, 255, 255, 0.06)'}; border-radius: 12px; box-shadow: ${props.isCollapsed ? 'none' : '0 8px 32px rgba(0, 0, 0, 0.2)'}; transition: ${props.isResizing ? 'none' : 'all 0.3s cubic-bezier(0.4, 0, 0.2, 1)'};`}
        >
            {/* 调整大小把手（左侧） */}
            <div
                class="absolute top-0 bottom-0 left-[-4px] w-1 flex items-center justify-center cursor-ew-resize z-[1000] group"
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
                    title={props.isCollapsed ? "展开话题栏" : "折叠话题栏"}
                    onClick={(e) => { e.stopPropagation(); props.onToggle(e); }}
                >
                    {props.isCollapsed ? '〈' : '〉'}
                </div>
            </div>

            <div
                class="h-full w-full overflow-hidden hover:overflow-y-auto transition-opacity duration-300"
                classList={{ "opacity-0 pointer-events-none overflow-hidden": props.isCollapsed }}
            >
                <Show when={props.currentAssistant}>
                    {(asst) => (
                        <div class="flex flex-col h-full">
                            <div class="mb-4">
                                <h3 style="color: rgba(255,255,255,0.85); font-size: 1rem; font-weight: 500;">
                                    {asst().name} 的话题
                                </h3>
                            </div>
                            <button
                                class="w-full px-3 py-2 rounded-lg cursor-pointer transition-all duration-300"
                                style="background: rgba(255,255,255,0.04); border: 1px solid rgba(255,255,255,0.06); color: rgba(255,255,255,0.6);"
                                onClick={props.addTopic}
                                onMouseEnter={(e) => e.currentTarget.style.background = 'rgba(124,154,191,0.12)'}
                                onMouseLeave={(e) => e.currentTarget.style.background = 'rgba(255,255,255,0.04)'}
                            >
                                + 新建话题
                            </button>
                            <div class="mt-[15px] space-y-1">
                                <For each={asst().topics}>
                                    {(topic) => (
                                        <div
                                            class="group sidebar-item"
                                            classList={{ '!bg-[rgba(124,154,191,0.15)] !border-[rgba(124,154,191,0.15)]': topic.id === currentTopicId() }}
                                            onClick={() => setCurrentTopicId(topic.id)}
                                        >
                                            <Show
                                                when={props.editingTopicId === topic.id}
                                                fallback={<span style="color: rgba(255,255,255,0.75); font-size: 0.9rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; padding-right: 8px; user-select: none;">{topic.name}</span>}
                                            >
                                                <input
                                                    class="rounded px-2 py-0.5 text-[0.85rem] h-5 outline-none w-[80%]"
                                                    style="background: rgba(0,0,0,0.3); border: 1px solid rgba(255,255,255,0.1); color: rgba(255,255,255,0.85);"
                                                    value={topic.name}
                                                    ref={(el) => { setTimeout(() => { el.focus(); el.select(); }, 0); }}
                                                    onBlur={(e) => saveTopicRename(asst().id, topic.id, e.currentTarget.value)}
                                                    onKeyDown={(e) => e.key === 'Enter' && saveTopicRename(asst().id, topic.id, e.currentTarget.value)}
                                                    onClick={(e) => e.stopPropagation()}
                                                />
                                            </Show>
                                            <button class="dot-menu-btn" onClick={(e) => openTopicMenu(e as MouseEvent, topic.id)}>
                                                <Icon src="/icons/app-logo/dot-menu.svg" class="w-[18px] h-[18px]" />
                                            </button>
                                        </div>
                                    )}
                                </For>
                            </div>
                        </div>
                    )}
                </Show>
            </div>

            {showTopicMenuDiv() && (
                <div class="context-menu" style={`top: ${topicMenuState().y}px; left: ${topicMenuState().x}px;`}>
                    <button class="context-menu-item" onClick={() => { props.setEditingTopicId(topicMenuState().targetTopicId); closeTopicMenu(); }}>重命名</button>
                    <button class="context-menu-item" style="color: rgba(255,77,77,0.8);" onClick={() => deleteTopic(props.currentAssistant!.id, topicMenuState().targetTopicId!)}>删除话题</button>
                </div>
            )}
        </div>
    );
};

export default TopicSidebar;

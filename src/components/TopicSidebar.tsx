import { Component, For, Show, createSignal, onMount, onCleanup } from 'solid-js';
import { Assistant, Topic, datas, setDatas, currentTopicId, setCurrentTopicId, saveSingleAssistantToBackend } from '../store/store';
import './TopicSidebar.css';

interface TopicSidebarProps {
    width: number;
    onResize: (e: MouseEvent) => void;
    currentAssistant: Assistant | undefined;
    editingTopicId: string | null;
    setEditingTopicId: (id: string | null) => void;
    addTopic: () => void;
}

const TopicSidebar: Component<TopicSidebarProps> = (props) => {
    const [showTopicMenuDiv, setShowTopicMenuDiv] = createSignal(false);
    const [isTopicMenuAnimatingOut, setIsTopicMenuAnimatingOut] = createSignal(false);
    const [topicMenuState, setTopicMenuState] = createSignal({ isOpen: false, x: 0, y: 0, targetTopicId: null as string | null });


    onMount(() => {
        const handleTopicClickOutside = () => {
            if (topicMenuState().isOpen) {
                closeTopicMenu();
            }
        };
        window.addEventListener('click', handleTopicClickOutside);
        onCleanup(() => window.removeEventListener('click', handleTopicClickOutside));
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
        if (!asst || asst.topics.length <= 1) { alert("至少保留一个话题"); return; }
        setDatas('assistants', a => a.id === asstId, 'topics', topics => topics.filter((t: Topic) => t.id !== topicId));
        if (currentTopicId() === topicId) setCurrentTopicId(asst.topics[0].id);
        await saveSingleAssistantToBackend(asstId);
        closeTopicMenu();
    };

    return (
        <div class="dialog-container" style={{ width: `${props.width}%` }}>
            <div class="resize-handle right-handle" onMouseDown={(e) => props.onResize(e as MouseEvent)}></div>
            <div class="dialog-content">
                <Show when={props.currentAssistant}>
                    {(asst) => (
                        <>
                            <div class="info-header"><h3>{asst().name} 的话题</h3></div>
                            <button class="add-topic-button" onClick={props.addTopic}>+ 新建话题</button>
                            <div class="topics-list">
                                <For each={asst().topics}>
                                    {(topic) => (
                                        <div classList={{ 'topic-item': true, 'active': topic.id === currentTopicId() }} onClick={() => setCurrentTopicId(topic.id)}>
                                            <Show when={props.editingTopicId === topic.id} fallback={<span class="topic-name">{topic.name}</span>}>
                                                <input class="rename-input" value={topic.name} ref={(el) => { setTimeout(() => { el.focus(); el.select(); }, 0); }} onBlur={(e) => saveTopicRename(asst().id, topic.id, e.currentTarget.value)} onKeyDown={(e) => e.key === 'Enter' && saveTopicRename(asst().id, topic.id, e.currentTarget.value)} onClick={(e) => e.stopPropagation()} />
                                            </Show>
                                            <button class="assistant-menu-button" onClick={(e) => openTopicMenu(e as MouseEvent, topic.id)}>
                                                <svg fill="#FFFFFF" viewBox="0 0 24 24" style="width: 18px;"><path d="M10 12a2 2 0 1 0 4 0a2 2 0 0 0-4 0zm0-6a2 2 0 1 0 4 0a2 2 0 0 0-4 0zm0 12a2 2 0 1 0 4 0a2 2 0 0 0-4 0z" /></svg>
                                            </button>
                                        </div>
                                    )}
                                </For>
                            </div>
                        </>
                    )}
                </Show>
            </div>

            {showTopicMenuDiv() && (
                <div class="assistant-context-menu" classList={{ 'menu-exiting': isTopicMenuAnimatingOut() }} style={{ top: `${topicMenuState().y}px`, left: `${topicMenuState().x}px` }}>
                    <button class="context-menu-button" onClick={() => { props.setEditingTopicId(topicMenuState().targetTopicId); closeTopicMenu(); }}>重命名</button>
                    <button class="context-menu-button delete" onClick={() => deleteTopic(props.currentAssistant!.id, topicMenuState().targetTopicId!)}>删除话题</button>
                </div>
            )}
        </div>
    );
};

export default TopicSidebar;

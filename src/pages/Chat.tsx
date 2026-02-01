import { Component, createSignal, onMount, onCleanup, createEffect } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { 
  datas, setDatas, currentAssistantId, setCurrentAssistantId, currentTopicId, setCurrentTopicId, 
  saveSingleAssistantToBackend, Assistant, Topic, Message, selectedModel, config, loadAvatarFromPath, 
  setGlobalUserAvatar, globalUserAvatar 
} from '../store/store';

import AssistantSidebar from '../components/AssistantSidebar';
import ChatInterface from '../components/ChatInterface';
import TopicSidebar from '../components/TopicSidebar';
import './Chat.css';

const createTopic = (name?: string): Topic => ({
  id: Date.now().toString(),
  name: name || `新话题 ${new Date().toLocaleTimeString()}`,
  history: []
});

const createAssistant = (name?: string, id?: string): Assistant => ({
  id: id ?? Date.now().toString(),
  name: name || 'New Assistant',
  prompt: 'You are a helpful assistant.',
  topics: [createTopic('默认话题')]
});

const ChatPage: Component = () => {
  // 布局与状态
  const [leftPanelWidth, setLeftPanelWidth] = createSignal(18);
  const [rightPanelWidth, setRightPanelWidth] = createSignal(18);
  const [inputMessage, setInputMessage] = createSignal("");
  const [pendingFiles, setPendingFiles] = createSignal<{name: string, content: string, type: 'text' | 'image'}[]>([]);
  const [isThinking, setIsThinking] = createSignal(false);
  const [isProcessing, setIsProcessing] = createSignal(false);
  const [isDragging, setIsDragging] = createSignal(false);
  const [isChangingTopic, setIsChangingTopic] = createSignal(false);
  const [typingIndex, setTypingIndex] = createSignal<number | null>(null);
  const [editingAsstId, setEditingAsstId] = createSignal<string | null>(null);
  const [editingTopicId, setEditingTopicId] = createSignal<string | null>(null);

  let chatPageRef: HTMLDivElement | undefined;

  // 基础数据获取
  const currentAssistant = () => datas.assistants.find(a => a.id === currentAssistantId());
  const activeTopic = () => {
    const asst = currentAssistant();
    return asst?.topics.find((t: Topic) => t.id === currentTopicId()) || asst?.topics[0] || null;
  };


  // 业务逻辑
  const handleFileUpload = async (filePath: string, fileType: 'file' | 'image') => {
    setIsProcessing(true);
    try {
      const fileName = filePath.split(/[\\/]/).pop() || '未知文件';
      const content = await invoke<string>('process_file_content', { path: filePath });
      const isImg = fileType === 'image' || ['png', 'jpg', 'jpeg'].includes(fileName.split('.').pop()?.toLowerCase() || '');
      setPendingFiles(prev => [...prev, { name: fileName, content, type: isImg ? 'image' : 'text' }]);
    } catch (err) { alert(err); } finally { setIsProcessing(false); }
  };

  const handleSendMessage = async () => {
    const currentMdl = selectedModel();
    if (!currentMdl || isThinking()) return;

    const userInput = inputMessage().trim();
    const files = pendingFiles();
    if (!userInput && files.length === 0) return;

    const documents = files.filter(f => f.type === 'text');
    const images = files.filter(f => f.type === 'image');
    
    let textContext = documents.length > 0 ? "参考文件：\n" + documents.map(d => `[${d.name}]\n${d.content}`).join('\n') + "\n---\n" : "";
    const finalPrompt = `${textContext}用户问题：${userInput}`;

    const apiContent = images.length > 0 ? [
      { type: "text", text: finalPrompt },
      ...images.map(img => ({ type: "image_url", image_url: { url: img.content } }))
    ] : finalPrompt;

    const asstId = currentAssistantId();
    const topicId = currentTopicId();
    if (!asstId || !topicId) return;

    const newUserMsg = { role: 'user' as const, content: apiContent, displayFiles: files.map(f => ({name: f.name})), displayText: userInput };
    
    setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId, 'history', h => [...h, newUserMsg, { role: 'assistant' as const, content: "", modelId: currentMdl.model_id }]);
    
    setInputMessage(""); setPendingFiles([]); setIsThinking(true);
    setTypingIndex(activeTopic()?.history.length! - 1);

    try {
      const asstObj = currentAssistant();
      const topicObj = activeTopic();
      const messagesForAI = [{ role: 'system', content: asstObj!.prompt }, ...topicObj!.history.map((m: any) => ({ role: m.role, content: m.content }))];
      await invoke('call_llm_stream', { apiUrl: currentMdl.api_url, apiKey: currentMdl.api_key, model: currentMdl.model_id, assistantId: asstId, topicId: topicId, messages: messagesForAI });
    } catch (err) { alert(err); setIsThinking(false); }
  };

  const handleStopGeneration = async () => {
    await invoke('stop_llm_stream', { assistantId: currentAssistantId(), topicId: currentTopicId() });
    setIsThinking(false); setTypingIndex(null);
  };

  // 辅助动作
  const addAssistant = async () => {
    const newAsst = createAssistant(`新助手 ${datas.assistants.length + 1}`);
    setDatas('assistants', prev => [...prev, newAsst]);
    setCurrentAssistantId(newAsst.id);
    await saveSingleAssistantToBackend(newAsst.id);
  };

  const addTopic = async () => {
    const asstId = currentAssistantId();
    if (!asstId) return;
    const newT = createTopic();
    setDatas('assistants', a => a.id === asstId, 'topics', prev => [...prev, newT]);
    setCurrentTopicId(newT.id);
    await saveSingleAssistantToBackend(asstId);
  };

  // 面板缩放逻辑
  const startResize = (e: MouseEvent, type: 'left' | 'right') => {
    e.preventDefault();
    const handleMove = (moveEvent: MouseEvent) => {
      const totalW = chatPageRef!.offsetWidth;
      if (type === 'left') setLeftPanelWidth(Math.min(Math.max((moveEvent.clientX / totalW) * 100, 15), 30));
      else setRightPanelWidth(Math.min(Math.max(((totalW - moveEvent.clientX) / totalW) * 100, 15), 30));
    };
    const stopResize = () => {
      document.removeEventListener('mousemove', handleMove);
      document.removeEventListener('mouseup', stopResize);
    };
    document.addEventListener('mousemove', handleMove);
    document.addEventListener('mouseup', stopResize);
  };

  // 初始化与监听
  onMount(() => {
    if (datas.assistants.length === 0) {
      invoke<Assistant[]>('load_assistants').then(loaded => {
        if (loaded.length > 0) { setDatas({ assistants: loaded }); setCurrentAssistantId(loaded[0].id); }
        else addAssistant();
      });
    }

    const unlistens = [
      listen('tauri://drag-enter', () => setIsDragging(true)),
      listen('tauri://drag-leave', () => setIsDragging(false)),
      listen<{paths: string[]}>('tauri://drag-drop', async (e) => {
        setIsDragging(false);
        for (const p of e.payload.paths) await handleFileUpload(p, 'file');
      }),
      listen<any>('llm-chunk', (e) => {
        const { assistant_id, topic_id, content, done } = e.payload;
        if (done) { setIsThinking(false); setTypingIndex(null); saveSingleAssistantToBackend(assistant_id); return; }
        const lastIdx = datas.assistants.find(a => a.id === assistant_id)?.topics.find((t: Topic) => t.id === topic_id)?.history.length! - 1;
        setDatas('assistants', a => a.id === assistant_id, 'topics', t => t.id === topic_id, 'history', lastIdx, 'content', (old: string) => old + content);
      })
    ];

    onCleanup(() => unlistens.forEach(u => u.then(fn => fn())));
  });

  createEffect(() => {
    const tId = currentTopicId();
    if (tId) { setIsChangingTopic(true); setTimeout(() => setIsChangingTopic(false), 50); }
  });

  return (
    <div class="chat-page" ref={chatPageRef}>
      <AssistantSidebar 
        width={leftPanelWidth()} 
        onResize={(e) => startResize(e, 'left')}
        editingAsstId={editingAsstId()}
        setEditingAsstId={setEditingAsstId}
        addAssistant={addAssistant}
      />
      
      <ChatInterface 
        activeTopic={activeTopic()}
        isChangingTopic={isChangingTopic()}
        isThinking={isThinking()}
        isProcessing={isProcessing()}
        isDragging={isDragging()}
        typingIndex={typingIndex()}
        inputMessage={inputMessage()}
        setInputMessage={setInputMessage}
        pendingFiles={pendingFiles()}
        setPendingFiles={setPendingFiles}
        handleSendMessage={handleSendMessage}
        handleStopGeneration={handleStopGeneration}
        handleFileUpload={handleFileUpload}
      />

      <TopicSidebar 
        width={rightPanelWidth()}
        onResize={(e) => startResize(e, 'right')}
        currentAssistant={currentAssistant()}
        editingTopicId={editingTopicId()}
        setEditingTopicId={setEditingTopicId}
        addTopic={addTopic}
      />
    </div>
  );
};

export default ChatPage;

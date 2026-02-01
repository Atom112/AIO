/**
 * @file NavBar.tsx
 * @description 应用程序顶部导航栏组件。
 * 包含功能：
 * 1. 路由导航（对话、设置）
 * 2. 模型切换与自动后端管理（特别是 Local-Llama.cpp 的启动与健康检查）
 * 3. 助手提示词（Prompt）管理弹窗
 * 4. 基于 Tauri API 的自定义窗口控制（最小化、最大化、关闭）
 * 5. 窗口拖拽区域实现
 */

import { createSignal, onMount, For, Component, Show } from 'solid-js';
import { Window } from '@tauri-apps/api/window';
import { A } from '@solidjs/router';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { readFile } from '@tauri-apps/plugin-fs';
import AvatarCropModal from './AvatarCropModel';
// 导入状态管理与组件
import {
  datas,
  setDatas,
  currentAssistantId,
  saveSingleAssistantToBackend,
  selectedModel,
  setSelectedModel,
  ActivatedModel, globalUserAvatar, setGlobalUserAvatar, loadAvatarFromPath
} from '../store/store';
import PromptModal from '../pages/PromptModal';
import './NavBar.css';

/** 
 * 初始化当前窗口实例
 * 标签 'main' 对应 tauri.conf.json 中的窗体配置 
 */
const appWindow = new Window('main');

interface NavBarProps { }

/**
 * 导航栏组件
 */
const NavBar: Component<NavBarProps> = () => {
  // --- 状态声明 ---

  /** 用于存储用户在弹窗中实时编辑的提示词内容 */
  const [modalPrompt, setModalPrompt] = createSignal('');
  /** 控制“设置提示词”弹窗的显示/隐藏 */
  const [isModalOpen, setIsModalOpen] = createSignal<boolean>(false);
  /** 控制模型选择下拉菜单的可见性 */
  const [isDropdownVisible, setDropdownVisible] = createSignal<boolean>(false);
  /** 跟踪窗口是否处于最大化状态，用于切换图标 */
  const [isMaximized, setIsMaximized] = createSignal<boolean>(false);
  const [userAvatar, setUserAvatar] = createSignal('/icons/user.svg');
  const [isUserMenuVisible, setUserMenuVisible] = createSignal(false);
  const [tempImage, setTempImage] = createSignal<string | null>(null);
  const onlineModels = () => datas.activatedModels.filter(m => m.owned_by !== "Local-Llama.cpp");
  const localModels = () => datas.activatedModels.filter(m => m.owned_by === "Local-Llama.cpp");
  // 根据后缀判断 MIME 类型（简单版）
  const getMimeType = (path: string) => {
    const ext = path.split('.').pop()?.toLowerCase();
    switch (ext) {
      case 'png': return 'image/png';
      case 'jpg':
      case 'jpeg': return 'image/jpeg';
      case 'webp': return 'image/webp';
      case 'svg': return 'image/svg+xml';
      default: return 'application/octet-stream';
    }
  };

  const handleEditAvatar = async () => {
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp'] }]
      });

      if (selected && typeof selected === 'string') {
        // 先读取文件并设为临时预览图，触发弹窗
        const contents = await readFile(selected);
        const blob = new Blob([contents], { type: 'image/png' });
        const blobUrl = URL.createObjectURL(blob);
        setTempImage(blobUrl); // 开启裁剪流程
      }
    } catch (err) {
      console.error("选择头像失败:", err);
    }
  };

  const onCropSave = async (croppedDataUrl: string) => {
    try {
      // 1. 调用 Rust 保存裁剪后的 Base64 数据
      const savedPath = await invoke<string>('upload_avatar', {
        dataUrl: croppedDataUrl
      });

      // 2. 刷新全局 UI
      const blobUrl = await loadAvatarFromPath(savedPath);
      setGlobalUserAvatar(blobUrl);

      // 3. 持久化并重置状态
      localStorage.setItem('user-avatar-path', savedPath);
      setTempImage(null);
      setUserMenuVisible(false);
    } catch (err) {
      alert("保存裁剪头像失败: " + err);
    }
  };
  // 根据模型名称返回对应的 SVG 路径 (建议与 Settings.tsx 保持一致)
  const getModelLogo = (modelName: string) => {
    const name = modelName.toLowerCase();
    if (name.includes('gpt')) return '/icons/openai.svg';
    if (name.includes('claude')) return '/icons/claude-color.svg';
    if (name.includes('grok')) return '/icons/grok.svg';
    if (name.includes('gemini')) return '/icons/gemini-color.svg';
    if (name.includes('deepseek')) return '/icons/deepseek-color.svg';
    if (name.includes('qwen')) return '/icons/qwen-color.svg';
    if (name.includes('kimi') || name.includes('moonshot')) return '/icons/moonshot.svg';
    if (name.includes('doubao')) return '/icons/doubao-color.svg';
    if (name.includes('glm')) return '/icons/zhipu-color.svg';

    // 默认或本地模型的图标
    return '/icons/ollama.svg';
  };
  /**
   * 静默启动本地模型服务
   * 用于初始化加载或后台静默拉起，不触发 UI 上的聊天记录反馈
   * @param model 需要启动的模型信息
   */
  const startLocalModel = async (model: ActivatedModel) => {
    if (model.owned_by === "Local-Llama.cpp" && model.local_path) {
      const isRunning = await invoke<boolean>('is_local_server_running');
      if (!isRunning) {
        try {
          await invoke('start_local_server', {
            modelPath: model.local_path,
            port: 8080,
            gpuLayers: 99
          });
          console.info("本地模型服务已静默拉起");
        } catch (e) {
          console.error("自动启动本地模型失败:", e);
        }
      }
    }
  };

  /**
   * 处理打开提示词设置弹窗
   */
  const handleOpenPromptModal = (e: MouseEvent) => {
    e.preventDefault();
    const activeId = currentAssistantId();
    if (!activeId) {
      alert("请先在聊天界面创建一个助手");
      return;
    }
    // 从 Store 中查找当前助手的 prompt 并同步到局部状态
    const assistant = datas.assistants.find(a => a.id === activeId);
    setModalPrompt(assistant?.prompt || '');
    setIsModalOpen(true);
  };

  /**
   * 处理 Prompt 的保存逻辑
   * @param newPrompt 用户输入的新提示词
   */
  const handleSavePrompt = (newPrompt: string) => {
    const activeId = currentAssistantId();
    if (activeId) {
      // 1. 更新全局内存状态
      setDatas('assistants', a => a.id === activeId, 'prompt', newPrompt);
      // 2. 触发后端持久化存储
      saveSingleAssistantToBackend(activeId);
      console.log("提示词已更新并同步到后端");
    }
  };

  /**
   * 检查本地服务器健康状况（心跳检测）
   * @param baseUrl 服务器基础地址
   * @returns 模型是否就绪
   */
  const checkServerHealth = async (baseUrl: string): Promise<boolean> => {
    try {
      const controller = new AbortController();
      // 设置 2 秒超时，防止探测挂起
      const timeoutId = setTimeout(() => controller.abort(), 2000);

      // llama.cpp 默认在根路径提供 /health
      const rootUrl = baseUrl.replace('/v1', '');
      const resp = await fetch(`${rootUrl}/health`, { signal: controller.signal });

      clearTimeout(timeoutId);
      return resp.ok; // 状态码 200 表示服务就绪
    } catch {
      return false;
    }
  };

  /**
   * 处理模型切换
   * 如果切换到本地模型，将包含：
   * 1. 启动后端进程
   * 2. 在 UI 聊天历史中注入“启动中”提示
   * 3. 轮询检测服务器健康，直到服务真正可用
   * @param model 用户选择的目标模型
   */
  const handleModelSelect = async (model: ActivatedModel) => {
    setSelectedModel(model);
    setDropdownVisible(false);

    // 1. 保存用户的模型偏好设置到配置文件
    try {
      const currentConfig = await invoke<any>('load_app_config');
      await invoke('save_app_config', {
        config: { ...currentConfig, defaultModel: model.model_id }
      });
    } catch (e) {
      console.error("保存模型偏好失败:", e);
    }

    // 2. 本地模型自动启动逻辑 (Llama.cpp 专有)
    if (model.owned_by === "Local-Llama.cpp" && model.local_path) {
      const isRunning = await invoke<boolean>('is_local_server_running');

      if (!isRunning) {
        // 确保助手列表已加载，以获取对话上下文
        if (datas.assistants.length === 0) {
          const loaded = await invoke<any[]>('load_assistants');
          if (loaded?.length > 0) setDatas('assistants', loaded);
        }

        let asstId = currentAssistantId() || datas.assistants[0]?.id;
        const assistant = datas.assistants.find(a => a.id === asstId);

        if (assistant) {
          const topicId = assistant.topics[0]?.id;
          const loadingText = "🚀 **正在启动本地 Llama 服务器...**";

          // 在 UI 注入启动反馈
          if (topicId) {
            setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId,
              'history', h => [...h, { role: 'assistant', content: loadingText }]
            );
          }

          try {
            // 启动后端命令
            await invoke('start_local_server', {
              modelPath: model.local_path,
              port: 8080,
              gpuLayers: 99
            });

            // 3. 轮询探测服务器直到就绪 (心跳检测)
            let attempts = 0;
            const maxAttempts = 60; // 最多等待约 90 秒 (60 * 1.5s)

            const poll = setInterval(async () => {
              attempts++;
              const isReady = await checkServerHealth("http://127.0.0.1:8080/v1");

              if (isReady) {
                clearInterval(poll);
                // 更新 UI 把“启动中”替换成“成功”状态
                setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId,
                  'history', h => h.map((msg: any) =>
                    msg.content === loadingText
                      ? { ...msg, content: "✅ **本地服务器启动成功，可以开始对话了！**" }
                      : msg
                  )
                );
              } else if (attempts >= maxAttempts) {
                clearInterval(poll);
                setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId,
                  'history', h => [...h, { role: 'assistant', content: "❌ **服务器启动超时，请检查显存空间或模型文件。**" }]
                );
              }
            }, 1500);

          } catch (err) {
            setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId,
              'history', h => [...h, { role: 'assistant', content: `❌ **启动失败: ${err}**` }]
            );
          }
        } else {
          // 无助手上下文时，仅静默拉起
          await invoke('start_local_server', { modelPath: model.local_path, port: 8080, gpuLayers: 99 });
        }
      }
    }
  };

  /** --- 窗口控制 API 封装 --- */
  const handleMinimize = async () => await appWindow.minimize();
  const handleToggleMaximize = async () => {
    await appWindow.toggleMaximize();
    setIsMaximized(await appWindow.isMaximized());
  };
  const handleClose = async () => await appWindow.close();

  /** --- 生命周期钩子 --- */
  onMount(async () => {
    // A. 处理头像恢复
    const savedPath = localStorage.getItem('user-avatar-path');
    if (savedPath) {
      const url = await loadAvatarFromPath(savedPath);
      setGlobalUserAvatar(url);
    }

    // B. 处理模型和窗口逻辑 (移出 if(savedPath) 分支，确保模型总能加载)
    try {
      const [models, config] = await Promise.all([
        invoke<ActivatedModel[]>('load_activated_models'),
        invoke<any>('load_app_config')
      ]);
      setDatas('activatedModels', models);

      if (models.length > 0) {
        const lastSelectedId = config.defaultModel;
        const found = models.find(m => m.model_id === lastSelectedId);
        const targetModel = found || models[0];
        setSelectedModel(targetModel);
        if (targetModel.owned_by === "Local-Llama.cpp") {
          startLocalModel(targetModel);
        }
      }
    } catch (e) {
      console.error("初始化数据失败:", e);
    }

    // C. 窗口控制逻辑
    setIsMaximized(await appWindow.isMaximized());
    const unlistenResized = await appWindow.onResized(async () => {
      setIsMaximized(await appWindow.isMaximized());
    });

    return () => {
      unlistenResized(); // 组件卸载清理
    };
  });

  return (
    <>
      {/* 窗口拖拽响应区 */}
      <div data-tauri-drag-region class="navbar-drag-region"></div>

      <nav class="navbar">
        {/* --- 左侧区域：Logo 与 主导航 --- */}
        <div class="logo-container">
          <img src="/icons/logo.png" alt="AIO" class="logo" />
        </div>

        <A href="/chat" class="nav-item" title="对话" activeClass="active">
          <svg /* 对话图标 */ xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width={1.5} stroke="currentColor" class="size-6">
            <path stroke-linecap="round" stroke-linejoin="round" d="M2.25 12.76c0 1.6 1.123 2.994 2.707 3.227 1.087.16 2.185.283 3.293.369V21l4.076-4.076a1.526 1.526 0 0 1 1.037-.443 48.282 48.282 0 0 0 5.68-.494c1.584-.233 2.707-1.626 2.707-3.228V6.741c0-1.602-1.123-2.995-2.707-3.228A48.394 48.394 0 0 0 12 3c-2.392 0-4.744.175-7.043.513C3.373 3.746 2.25 5.14 2.25 6.741v6.018Z" />
          </svg>
        </A>

        <A href="/settings" class="nav-item" title="设置" activeClass="active">
          <svg /* 设置图标 */ xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width={1.5} stroke="currentColor" class="size-6">
            <path stroke-linecap="round" stroke-linejoin="round" d="M9.594 3.94c.09-.542.56-.94 1.11-.94h2.593c.55 0 1.02.398 1.11.94l.213 1.281c.063.374.313.686.645.87.074.04.147.083.22.127.325.196.72.257 1.075.124l1.217-.456a1.125 1.125 0 0 1 1.37.49l1.296 2.247a1.125 1.125 0 0 1-.26 1.431l-1.003.827c-.293.241-.438.613-.43.992a7.723 7.723 0 0 1 0 .255c-.008.378.137.75.43.991l1.004.827c.424.35.534.955.26 1.43l-1.298 2.247a1.125 1.125 0 0 1-1.369.491l-1.217-.456c-.355-.133-.75-.072-1.076.124a6.47 6.47 0 0 1-.22.128c-.331.183-.581.495-.644.869l-.213 1.281c-.09.543-.56.94-1.11.94h-2.594c-.55 0-1.019-.398-1.11-.94l-.213-1.281c-.062-.374-.312-.686-.644-.87a6.52 6.52 0 0 1-.22-.127c-.325-.196-.72-.257-1.076-.124l-1.217.456a1.125 1.125 0 0 1-1.369-.49l-1.297-2.247a1.125 1.125 0 0 1 .26-1.431l1.004-.827c.292-.24.437-.613.43-.991a6.932 6.932 0 0 1 0-.255c.007-.38-.138-.751-.43-.992l-1.004-.827a1.125 1.125 0 0 1-.26-1.43l1.297-2.247a1.125 1.125 0 0 1 1.37-.491l1.216.456c.356.133.751.072 1.076-.124.072-.044.146-.086.22-.128.332-.183.582-.495.644-.869l.214-1.28Z" />
            <path stroke-linecap="round" stroke-linejoin="round" d="M15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z" />
          </svg>
        </A>

        {/* --- 中间区域：用户信息 --- */}

        <div
          class="user-avatar-wrapper"
          onMouseEnter={() => setUserMenuVisible(true)}
          onMouseLeave={() => setUserMenuVisible(false)}
        >
          <img
            src={globalUserAvatar()}
            alt="User Avatar"
            class="avatar"
            // 当图片加载失败（路径不存在）时触发
            onError={(e) => {
              e.currentTarget.src = "/icons/user.svg"; // 切回默认图标
            }}
          />
          {/* 用户下拉菜单 */}
          <div classList={{ 'user-dropdown-menu': true, 'active': isUserMenuVisible() }}>
            <div class="user-dropdown-item" onClick={handleEditAvatar}>
              <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" style="width:16px; height:16px;">
                <path stroke-linecap="round" stroke-linejoin="round" d="M6.827 6.175A2.31 2.31 0 0 1 5.186 7.23c-.38.054-.757.112-1.134.175C2.999 7.58 2.25 8.507 2.25 9.574V18a2.25 2.25 0 0 0 2.25 2.25h15A2.25 2.25 0 0 0 21.75 18V9.574c0-1.067-.75-1.994-1.802-2.169a47.865 47.865 0 0 0-1.134-.175 2.31 2.31 0 0 1-1.64-1.055l-.822-1.316a2.192 2.192 0 0 0-1.736-1.039 48.774 48.774 0 0 0-5.232 0 2.192 2.192 0 0 0-1.736 1.039l-.821 1.316Z" />
                <path stroke-linecap="round" stroke-linejoin="round" d="M16.5 12.75a4.5 4.5 0 1 1-9 0 4.5 4.5 0 0 1 9 0ZM18.75 10.5h.008v.008h-.008V10.5Z" />
              </svg>
              更换头像
            </div>
            <div class="user-dropdown-item login-disabled" title="暂未开放">
              <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" style="width:16px; height:16px;">
                <path stroke-linecap="round" stroke-linejoin="round" d="M15.75 6a3.75 3.75 0 1 1-7.5 0 3.75 3.75 0 0 1 7.5 0ZM4.501 20.118a7.5 7.5 0 0 1 14.998 0A17.933 17.933 0 0 1 12 21.75c-2.676 0-5.216-.584-7.499-1.632Z" />
              </svg>
              登录账号
            </div>
          </div>
        </div>

        {/* --- 右侧区域：工具与控制 --- */}
        <div
          class="model-selector-wrapper"
          onMouseEnter={() => setDropdownVisible(true)}
          onMouseLeave={() => setDropdownVisible(false)}
        >
          <div class="nav-item model-selector" title="选择模型">
            <svg /* 模型选择图标 */ xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width={1.5} stroke="currentColor" class="size-6">
              <path stroke-linecap="round" stroke-linejoin="round" d="M13.5 16.875h3.375m0 0h3.375m-3.375 0V13.5m0 3.375v3.375M6 10.5h2.25a2.25 2.25 0 0 0 2.25-2.25V6a2.25 2.25 0 0 0-2.25-2.25H6A2.25 2.25 0 0 0 3.75 6v2.25A2.25 2.25 0 0 0 6 10.5Zm0 9.75h2.25A2.25 2.25 0 0 0 10.5 18v-2.25a2.25 2.25 0 0 0-2.25-2.25H6a2.25 2.25 0 0 0-2.25 2.25V18A2.25 2.25 0 0 0 6 20.25Zm9.75-9.75H18a2.25 2.25 0 0 0 2.25-2.25V6A2.25 2.25 0 0 0 18 3.75h-2.25A2.25 2.25 0 0 0 13.5 6v2.25a2.25 2.25 0 0 0 2.25 2.25Z" />
            </svg>
          </div>

          {/* 下拉模型列表内容 */}
          <div classList={{ 'dropdown-menu': true, 'active': isDropdownVisible() }}>
            <div class="dropdown-columns-container">

              {/* 左列：线上模型 */}
              <div class="dropdown-column">
                <div class="column-header">线上模型</div>
                <div class="column-content">
                  <For each={onlineModels()}>
                    {(model) => (
                      <div
                        class="dropdown-item"
                        classList={{ 'selected': selectedModel()?.model_id === model.model_id }}
                        onClick={() => handleModelSelect(model)}
                      >
                        <div class="nav-model-logo-container">
                          <img src={getModelLogo(model.model_id)} alt="logo" class="nav-model-logo" />
                        </div>
                        <div class="model-text-group">
                          <div class="model-id-text">{model.model_id}</div>
                          <div class="model-provider-text">{model.owned_by}</div>
                        </div>
                      </div>
                    )}
                  </For>
                  {onlineModels().length === 0 && <div class="no-model-tip">无线上模型</div>}
                </div>
              </div>

              {/* 中间分割线 */}
              <div class="column-divider"></div>

              {/* 右列：本地模型 */}
              <div class="dropdown-column">
                <div class="column-header">本地模型</div>
                <div class="column-content">
                  <For each={localModels()}>
                    {(model) => (
                      <div
                        class="dropdown-item"
                        classList={{ 'selected': selectedModel()?.model_id === model.model_id }}
                        onClick={() => handleModelSelect(model)}
                      >
                        <div class="nav-model-logo-container">
                          <img src={getModelLogo(model.model_id)} alt="logo" class="nav-model-logo" />
                        </div>
                        <div class="model-text-group">
                          <div class="model-id-text">{model.model_id}</div>
                          <div class="model-provider-text">Local</div>
                        </div>
                      </div>
                    )}
                  </For>
                  {localModels().length === 0 && <div class="no-model-tip">无本地模型</div>}
                </div>
              </div>
            </div>
          </div>
        </div>

        <a href="#" title="设置提示词" class="nav-item" onClick={handleOpenPromptModal}>
          <svg /* 提示词图标 */ xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width={1.5} stroke="currentColor" class="size-6">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 18v-5.25m0 0a6.01 6.01 0 0 0 1.5-.189m-1.5.189a6.01 6.01 0 0 1-1.5-.189m3.75 7.478a12.06 12.06 0 0 1-4.5 0m3.75 2.383a14.406 14.406 0 0 1-3 0M14.25 18v-.192c0-.983.658-1.823 1.508-2.316a7.5 7.5 0 1 0-7.517 0c.85.493 1.509 1.333 1.509 2.316V18" />
          </svg>
        </a>

        {/* --- 窗口最小化/大化/关闭控制块 --- */}
        <div class="window-controls">
          <button class="control-button minimize" onClick={handleMinimize} title="最小化">
            <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width={1.5} stroke="currentColor" class="size-6">
              <path stroke-linecap="round" stroke-linejoin="round" d="M5 12h14" />
            </svg>
          </button>

          <button class="control-button maximize" onClick={handleToggleMaximize} title={isMaximized() ? "还原" : "最大化"}>
            {isMaximized() ? (
              <svg /* 还原图标 */ xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width={1.5} stroke="currentColor" class="size-6">
                <path stroke-linecap="round" stroke-linejoin="round" d="M16.5 8.25V6a2.25 2.25 0 0 0-2.25-2.25H6A2.25 2.25 0 0 0 3.75 6v8.25A2.25 2.25 0 0 0 6 16.5h2.25m8.25-8.25H18a2.25 2.25 0 0 1 2.25 2.25V18A2.25 2.25 0 0 1 18 20.25h-7.5A2.25 2.25 0 0 1 8.25 18v-1.5m8.25-8.25h-6a2.25 2.25 0 0 0-2.25 2.25v6" />
              </svg>
            ) : (
              <svg /* 最大化图标 */ xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width={1.5} stroke="currentColor" class="size-6">
                <path stroke-linecap="round" stroke-linejoin="round" d="M5.25 7.5A2.25 2.25 0 0 1 7.5 5.25h9a2.25 2.25 0 0 1 2.25 2.25v9a2.25 2.25 0 0 1-2.25 2.25h-9a2.25 2.25 0 0 1-2.25-2.25v-9Z" />
              </svg>
            )}
          </button>

          <button class="control-button close" onClick={handleClose} title="关闭">
            <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width={1.5} stroke="currentColor" class="size-6">
              <path stroke-linecap="round" stroke-linejoin="round" d="M6 18 18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
      </nav>
      <Show when={tempImage()}>
        <AvatarCropModal
          imageSrc={tempImage()!}
          onCancel={() => setTempImage(null)}
          onSave={onCropSave}
        />
      </Show>
      {/* 提示词设置模态框 */}
      <PromptModal
        show={isModalOpen()}
        onClose={() => setIsModalOpen(false)}
        onSave={handleSavePrompt}
        initialPrompt={modalPrompt()}
      />
    </>
  );
}

export default NavBar;
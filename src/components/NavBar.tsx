import { createSignal, onMount, For, Component, Show } from 'solid-js';
import { Window } from '@tauri-apps/api/window';
import { A } from '@solidjs/router';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { readFile } from '@tauri-apps/plugin-fs';
import AvatarCropModal from './AvatarCropModel';
import PromptModal from './PromptModal';
import LoginModal from './LoginModal';
import Icon from './Icon';
import {
  datas,
  setDatas,
  currentAssistantId,
  saveSingleAssistantToBackend,
  selectedModel,
  setSelectedModel,
  ActivatedModel,
  globalUserAvatar,
  setGlobalUserAvatar,
  loadAvatarFromPath,
  logout
} from '../store/store';

/**
 * 初始化当前窗口实例
 * 'main' 对应 tauri.conf.json 中配置的窗体标签
 */
const appWindow = new Window('main');

/** NavBar 组件 Props（当前无外部传入，使用全局状态） */
interface NavBarProps { }

/**
 * 导航栏组件
 * 
 * @component
 * @description 应用顶部导航栏，集成所有全局控制功能
 * 
 * @returns {JSX.Element} 导航栏 JSX 元素
 */
const NavBar: Component<NavBarProps> = () => {

  /** 提示词弹窗中临时编辑的提示词内容 */
  const [modalPrompt, setModalPrompt] = createSignal('');
  /** 控制提示词设置弹窗的显示/隐藏 */
  const [isModalOpen, setIsModalOpen] = createSignal<boolean>(false);
  /** 控制模型选择下拉菜单的可见性 */
  const [isDropdownVisible, setDropdownVisible] = createSignal<boolean>(false);
  /** 窗口是否处于最大化状态，用于切换图标 */
  const [isMaximized, setIsMaximized] = createSignal<boolean>(false);
  /** 控制用户下拉菜单的显示/隐藏 */
  const [isUserMenuVisible, setUserMenuVisible] = createSignal(false);
  /** 临时图片 DataURL，用于头像裁剪流程 */
  const [tempImage, setTempImage] = createSignal<string | null>(null);
  /** 控制登录弹窗的显示/隐藏 */
  const [isLoginModalOpen, setIsLoginModalOpen] = createSignal(false);

  /** 线上模型列表：过滤出 owned_by 不为 Local-Llama.cpp 的模型 */
  const onlineModels = () => datas.activatedModels.filter(m => m.owned_by !== "Local-Llama.cpp");
  /** 本地模型列表：过滤出 owned_by 为 Local-Llama.cpp 的模型 */
  const localModels = () => datas.activatedModels.filter(m => m.owned_by === "Local-Llama.cpp");

  /**
   * 登录成功回调处理
   * 
   * 数据流：
   * 1. 更新全局 Store 的用户信息和登录状态
   * 2. 保存 Token 到 localStorage
   * 3. 如用户有云端头像，设置头像并清理本地缓存
   * 
   * @param {any} user - 后端返回的用户信息对象
   */
  const handleLoginSuccess = async (user: any) => {
    console.log("登录成功:", user);
    setDatas('user', user);
    setDatas('isLoggedIn', true);
    
    if (user.token) {
      localStorage.setItem('auth-token', user.token);
    }
    
    if (user.avatar) {
      setGlobalUserAvatar(user.avatar);
    }
  };

  /**
   * 退出登录处理
   * 
   * 调用全局 logout 清理状态，关闭用户菜单
   */
  const handleLogout = async () => {
    logout();
    setUserMenuVisible(false);
    const localSavedPath = localStorage.getItem('user-avatar-path');
    if (localSavedPath) {
        try {
            // 调用 store 中定义的加载函数将本地存储的路径转为 Base64/ObjectURL
            const url = await loadAvatarFromPath(localSavedPath);
            setGlobalUserAvatar(url);
            console.log("退出成功，已恢复本地头像");
        } catch (err) {
            console.error("恢复本地头像失败:", err);
            setGlobalUserAvatar('/icons/app-logo/user.svg'); // 失败则回退默认
        }
    }
  };

  /**
   * 处理编辑头像：打开文件选择器并触发裁剪流程
   * 
   * 数据流：
   * 1. 调用 Tauri open() 打开系统图片选择器（支持 png/jpg/jpeg/webp）
   * 2. 使用 readFile() 读取文件内容为 Uint8Array
   * 3. 转换为 Blob 并生成 ObjectURL
   * 4. 设置 tempImage 触发 AvatarCropModal 显示
   */
  const handleEditAvatar = async () => {
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp'] }]
      });

      if (selected && typeof selected === 'string') {
        const contents = await readFile(selected);
        const blob = new Blob([contents], { type: 'image/png' });
        const blobUrl = URL.createObjectURL(blob);
        setTempImage(blobUrl); // 触发裁剪弹窗
      }
    } catch (err) {
      console.error("选择头像失败:", err);
    }
  };

  /**
   * 头像裁剪完成回调
   * 
   * 数据流分支：
   * 已登录：调用 sync_avatar_to_backend 同步到云端，更新全局头像状态，释放本地 Blob URL
   * 未登录：调用 upload_avatar 保存到本地，记录路径到 localStorage
   * 
   * @param {string} croppedDataUrl - 裁剪后的 Base64 DataURL
   */
  const onCropSave = async (croppedDataUrl: string) => {
    try {
      if (datas.isLoggedIn && datas.user?.token) {
        // 云端同步分支
        await invoke('sync_avatar_to_backend', {
          token: datas.user.token,
          avatarData: croppedDataUrl
        });
        setGlobalUserAvatar(croppedDataUrl);
        console.log("头像已存入云端，本地文件已释放空间");
      } else {
        // 本地保存分支
        const savedPath = await invoke<string>('upload_avatar', {
          dataUrl: croppedDataUrl
        });
        setGlobalUserAvatar(croppedDataUrl);
        localStorage.setItem('user-avatar-path', savedPath);
      }

      setTempImage(null);
      setUserMenuVisible(false);
    } catch (err) {
      alert("头像同步失败: " + err);
    }
  };

  /**
   * 根据模型名称获取对应的品牌 Logo 路径
   * 
   * @param {string} modelName - 模型名称或 ID
   * @returns {string} Logo 图片的 URL 路径
   */
  const getModelLogo = (modelName: string) => {
    const name = modelName.toLowerCase();
    if (name.includes('gpt')) return '/icons/model-logo/openai.svg';
    if (name.includes('claude')) return '/icons/model-logo/claude-color.svg';
    if (name.includes('grok')) return '/icons/model-logo/grok.svg';
    if (name.includes('gemini')) return '/icons/model-logo/gemini-color.svg';
    if (name.includes('deepseek')) return '/icons/model-logo/deepseek-color.svg';
    if (name.includes('qwen')) return '/icons/model-logo/qwen-color.svg';
    if (name.includes('kimi') || name.includes('moonshot')) return '/icons/model-logo/moonshot.svg';
    if (name.includes('doubao')) return '/icons/model-logo/doubao-color.svg';
    if (name.includes('glm')) return '/icons/model-logo/zhipu-color.svg';
    return '/icons/model-logo/ollama.svg';
  };

  /**
   * 静默启动本地模型服务
   * 
   * 用于初始化加载或后台静默拉起，不触发 UI 聊天记录反馈
   * 
   * @param {ActivatedModel} model - 需要启动的本地模型信息
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
   * 
   * 前置检查：必须有选中的助手，否则提示用户创建
   * 
   * @param {MouseEvent} e - 点击事件
   */
  const handleOpenPromptModal = (e: MouseEvent) => {
    e.preventDefault();
    const activeId = currentAssistantId();
    if (!activeId) {
      alert("请先在聊天界面创建一个助手");
      return;
    }
    const assistant = datas.assistants.find(a => a.id === activeId);
    setModalPrompt(assistant?.prompt || '');
    setIsModalOpen(true);
  };

  /**
   * 处理 Prompt 保存
   * 
   * 数据流：
   * 1. 更新全局 Store 中对应助手的 prompt 字段
   * 2. 调用 saveSingleAssistantToBackend 持久化到后端
   * 
   * @param {string} newPrompt - 用户输入的新提示词
   */
  const handleSavePrompt = (newPrompt: string) => {
    const activeId = currentAssistantId();
    if (activeId) {
      setDatas('assistants', a => a.id === activeId, 'prompt', newPrompt);
      saveSingleAssistantToBackend(activeId);
      console.log("提示词已更新并同步到后端");
    }
  };

  /**
   * 检查本地服务器健康状况（心跳检测）
   * 
   * @param {string} baseUrl - 服务器基础地址
   * @returns {Promise<boolean>} 模型是否就绪
   */
  const checkServerHealth = async (baseUrl: string): Promise<boolean> => {
    try {
      const controller = new AbortController();
      const timeoutId = setTimeout(() => controller.abort(), 2000);

      const rootUrl = baseUrl.replace('/v1', '');
      const resp = await fetch(`${rootUrl}/health`, { signal: controller.signal });

      clearTimeout(timeoutId);
      return resp.ok;
    } catch {
      return false;
    }
  };

  /**
   * 处理模型切换
   * 
   * 核心逻辑：
   * 1. 保存模型偏好到配置文件
   * 2. 如切换到本地模型且未运行，自动启动并轮询健康检查
   * 3. 在聊天历史注入启动状态反馈
   * 
   * @param {ActivatedModel} model - 用户选择的目标模型
   */
  const handleModelSelect = async (model: ActivatedModel) => {
    setSelectedModel(model);
    setDropdownVisible(false);

    // 保存模型偏好
    try {
      const currentConfig = await invoke<any>('load_app_config');
      await invoke('save_app_config', {
        config: { ...currentConfig, defaultModel: model.model_id }
      });
    } catch (e) {
      console.error("保存模型偏好失败:", e);
    }

    // 本地模型自动启动逻辑
    if (model.owned_by === "Local-Llama.cpp" && model.local_path) {
      const isRunning = await invoke<boolean>('is_local_server_running');

      if (!isRunning) {
        // 确保助手列表已加载
        if (datas.assistants.length === 0) {
          const loaded = await invoke<any[]>('load_assistants');
          if (loaded?.length > 0) setDatas('assistants', loaded);
        }

        let asstId = currentAssistantId() || datas.assistants[0]?.id;
        const assistant = datas.assistants.find(a => a.id === asstId);

        if (assistant) {
          const topicId = assistant.topics[0]?.id;
          const loadingText = "**正在启动本地 Llama 服务器...**";

          // UI 注入启动反馈
          if (topicId) {
            setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId,
              'history', h => [...h, { role: 'assistant', content: loadingText }]
            );
          }

          try {
            await invoke('start_local_server', {
              modelPath: model.local_path,
              port: 8080,
              gpuLayers: 99
            });

            // 轮询探测服务器直到就绪
            let attempts = 0;
            const maxAttempts = 60;

            const poll = setInterval(async () => {
              attempts++;
              const isReady = await checkServerHealth("http://127.0.0.1:8080/v1");

              if (isReady) {
                clearInterval(poll);
                setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId,
                  'history', h => h.map((msg: any) =>
                    msg.content === loadingText
                      ? { ...msg, content: "**本地服务器启动成功，可以开始对话了！**" }
                      : msg
                  )
                );
              } else if (attempts >= maxAttempts) {
                clearInterval(poll);
                setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId,
                  'history', h => [...h, { role: 'assistant', content: "**服务器启动超时，请检查显存空间或模型文件。**" }]
                );
              }
            }, 1500);

          } catch (err) {
            setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId,
              'history', h => [...h, { role: 'assistant', content: `**启动失败: ${err}**` }]
            );
          }
        } else {
          // 无助手上下文时静默拉起
          await invoke('start_local_server', { modelPath: model.local_path, port: 8080, gpuLayers: 99 });
        }
      }
    }
  };

  /** 最小化窗口 */
  const handleMinimize = async () => await appWindow.minimize();
  
  /** 切换最大化/还原窗口 */
  const handleToggleMaximize = async () => {
    await appWindow.toggleMaximize();
    setIsMaximized(await appWindow.isMaximized());
  };
  
  /** 关闭窗口 */
  const handleClose = async () => await appWindow.close();

  /**
   * 组件挂载时初始化：
   * 1. Token 验证与自动登录
   * 2. 头像加载（优先云端，兜底本地）
   * 3. 模型列表加载与默认模型恢复
   * 4. 本地模型自动启动
   * 5. 窗口状态监听
   */
  onMount(async () => {
    const savedToken = localStorage.getItem('auth-token');

    if (savedToken) {
      try {
        const userData = await invoke<any>('validate_token', { token: savedToken });
        setDatas('user', userData);
        setDatas('isLoggedIn', true);

        if (userData.avatar) {
          setGlobalUserAvatar(userData.avatar);
        }
      } catch (err) {
        console.warn("身份过期或云端获取失败:", err);
      }
    }

    // 本地头像兜底
    const localSavedPath = localStorage.getItem('user-avatar-path');
    if (localSavedPath && globalUserAvatar() === '/icons/app-logo/user.svg') {
      const url = await loadAvatarFromPath(localSavedPath);
      setGlobalUserAvatar(url);
    }

    // 加载模型和配置
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

    // 窗口控制
    setIsMaximized(await appWindow.isMaximized());
    const unlistenResized = await appWindow.onResized(async () => {
      setIsMaximized(await appWindow.isMaximized());
    });

    return () => {
      unlistenResized();
    };
  });

return (
    <>
      {/* 顶部拖拽背景区域，确保在边缘也能触发拖拽 */}
      <div 
        data-tauri-drag-region 
        class="absolute top-0 left-0 right-0 h-[60px] z-[1] [app-region:drag]"
      ></div>

      <nav 
        data-tauri-drag-region
        class="navbar relative flex justify-center items-center gap-6 px-5 h-[60px] bg-dark glow-border rounded-lg m-0 mr-[1px] z-[1000] [app-region:drag] select-none"
      >
        {/* Logo 容器 - 绝对定位在左侧 */}
        <div class="absolute left-[10px] top-1/2 -translate-y-1/2 flex items-center justify-center z-[1001] pointer-events-none">
          <img src="/icons/app-logo/logo.svg" alt="AIO" class="w-10 h-10 object-contain block [app-region:no-drag]" />
        </div>

        {/* 聊天导航 */}
        <A 
          href="/chat" 
          title="对话" 
          activeClass="!text-pri font-bold" 
          class="nav-icon-link [app-region:no-drag]"
        >
          <Icon src="/icons/app-logo/chat.svg" class="w-6 h-6" />
        </A>

        {/* 设置导航 */}
        <A 
          href="/settings" 
          title="设置" 
          activeClass="!text-pri font-bold" 
          class="nav-icon-link [app-region:no-drag]"
        >
          <Icon src="/icons/app-logo/settings-gear.svg" class="w-6 h-6" />
        </A>

        {/* 用户头像及其下拉菜单 */}
        <div
          class="relative flex items-center cursor-pointer [app-region:no-drag]"
          onMouseEnter={() => setUserMenuVisible(true)}
          onMouseLeave={() => setUserMenuVisible(false)}
        >
          <img
            src={globalUserAvatar()}
            alt="User Avatar"
            class="w-10 h-10 rounded-full border-2 border-dark-300 transition-all duration-200 object-cover hover:border-pri"
            onError={(e) => {
              e.currentTarget.src = "/icons/app-logo/user.svg";
            }}
          />
          
          <div 
            class="absolute top-full left-1/2 -translate-x-1/2 mt-3 bg-dark min-w-[140px] border border-pri rounded-lg shadow-[0_4px_15px_rgba(0,0,0,0.4)] z-[1000] transition-all duration-200 p-1.5 before:content-[''] before:absolute before:-top-1.5 before:left-1/2 before:-translate-x-1/2 before:rotate-45 before:w-2.5 before:h-2.5 before:bg-dark before:border-l before:border-t before:border-pri"
            classList={{ 'invisible opacity-0': !isUserMenuVisible(), 'visible opacity-100': isUserMenuVisible() }}
          >
            <div class="user-menu-item" onClick={handleEditAvatar}>
              <Icon src="/icons/app-logo/camera.svg" class="w-4 h-4" />
              更换头像
            </div>

            <Show
              when={datas.isLoggedIn}
              fallback={
                <div class="user-menu-item"
                  onClick={() => {
                    setIsLoginModalOpen(true);
                    setUserMenuVisible(false);
                  }}>
                  <Icon src="/icons/app-logo/user-profile.svg" class="w-4 h-4" />
                  登录账号
                </div>
              }
            >
              <div class="h-[1px] bg-dark-300 my-1.5 mx-2 opacity-60"></div>
              <div class="user-menu-item font-medium">
                <Icon src="/icons/app-logo/info-circle.svg" class="w-4 h-4" />
                账号信息
              </div>
              <div class="user-menu-item"
                onClick={() => {
                  setIsLoginModalOpen(true);
                  setUserMenuVisible(false);
                }}>
                <Icon src="/icons/app-logo/switch-arrows.svg" class="w-4 h-4" />
                切换账号
              </div>
              <div class="flex items-center gap-2.5 p-2.5 text-[#E08090] opacity-90 text-[13px] rounded-md transition-all hover:bg-[rgba(255,77,79,0.15)] hover:text-[#E08090]" onClick={handleLogout}>
                <Icon src="/icons/app-logo/logout.svg" class="w-4 h-4" />
                退出登录
              </div>
            </Show>
          </div>
        </div>

        {/* 模型选择器 */}
        <div
          class="relative flex items-center [app-region:no-drag]"
          onMouseEnter={() => setDropdownVisible(true)}
          onMouseLeave={() => setDropdownVisible(false)}
        >
          <div class="nav-icon-link" title="选择模型">
            <Icon src="/icons/app-logo/model-selector.svg" class="w-6 h-6" />
          </div>

          <div 
            class="absolute top-full left-1/2 -translate-x-1/2 mt-2 bg-dark min-w-[480px] border border-dark-300 rounded-xl shadow-[0_10px_25px_rgba(0,0,0,0.5)] z-[1000] transition-all duration-200 overflow-hidden"
            classList={{ 
              'invisible opacity-0 translate-y-2': !isDropdownVisible(), 
              'visible opacity-100 translate-y-0 border-pri': isDropdownVisible() 
            }}
          >
            <div class="flex flex-row h-[400px]">
              {/* 左列：线上模型 */}
              <div class="flex-1 flex flex-col min-w-[240px]">
                <div class="px-4 py-3 text-[12px] font-bold text-[#888] uppercase tracking-widest bg-dark-600 border-b border-dark-300">线上模型</div>
                <div class="flex-1 overflow-y-auto p-2 scrollbar-thin scrollbar-thumb-[#444]">
                  <For each={onlineModels()}>
                    {(model) => (
                      <div
                        class="flex flex-row items-center gap-2.5 p-2 text-[#a0a0a0] text-sm rounded-lg cursor-pointer select-none transition-all hover:bg-dark-200 hover:text-white"
                        classList={{ 'bg-pri-20 border-l-3 border-pri': selectedModel()?.model_id === model.model_id }}
                        onClick={() => handleModelSelect(model)}
                      >
                        <div class="w-7 h-7 bg-white rounded-full flex items-center justify-center shrink-0 shadow-sm"
                             classList={{ 'border border-pri': selectedModel()?.model_id === model.model_id }}>
                          <img src={getModelLogo(model.model_id)} alt="logo" class="w-[18px] h-[18px] object-contain" />
                        </div>
                        <div class="flex-1 flex flex-col items-start justify-center overflow-hidden text-left">
                          <div class="max-w-[160px] text-[13px] text-white font-medium truncate">{model.model_id}</div>
                          <div class="text-[10px] text-pri opacity-70">{model.owned_by}</div>
                        </div>
                      </div>
                    )}
                  </For>
                  {onlineModels().length === 0 && <div class="p-5 text-center text-[#555] text-[13px]">无线上模型</div>}
                </div>
              </div>

              <div class="w-[1px] bg-dark-300 self-stretch"></div>

              {/* 右列：本地模型 */}
              <div class="flex-1 flex flex-col min-w-[240px]">
                <div class="px-4 py-3 text-[12px] font-bold text-[#888] uppercase tracking-widest bg-dark-600 border-b border-dark-300">本地模型</div>
                <div class="flex-1 overflow-y-auto p-2 scrollbar-thin scrollbar-thumb-[#444]">
                  <For each={localModels()}>
                    {(model) => (
                      <div
                        class="flex flex-row items-center gap-2.5 p-2 text-[#a0a0a0] text-sm rounded-lg cursor-pointer select-none transition-all hover:bg-dark-200 hover:text-white"
                        classList={{ 'bg-pri-20 border-l-3 border-pri': selectedModel()?.model_id === model.model_id }}
                        onClick={() => handleModelSelect(model)}
                      >
                        <div class="w-7 h-7 bg-white rounded-full flex items-center justify-center shrink-0 shadow-sm"
                             classList={{ 'border border-pri': selectedModel()?.model_id === model.model_id }}>
                          <img src={getModelLogo(model.model_id)} alt="logo" class="w-[18px] h-[18px] object-contain" />
                        </div>
                        <div class="flex-1 flex flex-col items-start justify-center overflow-hidden text-left">
                          <div class="max-w-[160px] text-[13px] text-white font-medium truncate">{model.model_id}</div>
                          <div class="text-[10px] text-pri opacity-70">Local</div>
                        </div>
                      </div>
                    )}
                  </For>
                  {localModels().length === 0 && <div class="p-5 text-center text-[#555] text-[13px]">无本地模型</div>}
                </div>
              </div>
            </div>
          </div>
        </div>

        {/* 提示词设置按钮 */}
        <a href="#" title="设置提示词" class="nav-icon-link [app-region:no-drag]" onClick={handleOpenPromptModal}>
          <Icon src="/icons/app-logo/prompt.svg" class="w-6 h-6" />
        </a>

        {/* 窗口控制按钮 */}
        <div class="absolute right-5 flex items-center [app-region:no-drag]">
          <button class="win-ctrl-btn hover:bg-dark-300" onClick={handleMinimize} title="最小化">
            <Icon src="/icons/app-logo/minimize.svg" class="w-6 h-6" />
          </button>

          <button class="win-ctrl-btn hover:bg-dark-300" onClick={handleToggleMaximize} title={isMaximized() ? "还原" : "最大化"}>
            {isMaximized() ? (
              <Icon src="/icons/app-logo/restore.svg" class="w-6 h-6" />
            ) : (
              <Icon src="/icons/app-logo/maximize.svg" class="w-6 h-6" />
            )}
          </button>

          <button class="win-ctrl-btn hover:bg-danger" onClick={handleClose} title="关闭">
            <Icon src="/icons/app-logo/close-x.svg" class="w-6 h-6" />
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
      
      <PromptModal
        show={isModalOpen()}
        onClose={() => setIsModalOpen(false)}
        onSave={handleSavePrompt}
        initialPrompt={modalPrompt()}
      />
      
      <LoginModal
        show={isLoginModalOpen()}
        onClose={() => setIsLoginModalOpen(false)}
        onSuccess={handleLoginSuccess}
      />
    </>
  );
};

export default NavBar;
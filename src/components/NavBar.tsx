/**
 * ============================================================================
 * 文件功能摘要
 * ============================================================================
 * 
 * @file NavBar.tsx
 * @description 应用程序顶部导航栏组件，集成路由导航、模型管理、用户系统、窗口控制于一体。
 * 
 * 【核心功能】
 * 1. 路由导航：对话页面与设置页面的切换
 * 2. 模型选择器：线上/本地模型分类展示，支持 Local-Llama.cpp 自动启动与健康检查
 * 3. 用户系统：头像上传（支持裁剪）、登录/登出、账号信息展示
 * 4. 助手提示词管理：快速编辑当前助手的 System Prompt
 * 5. 窗口控制：基于 Tauri API 的自定义标题栏（最小化、最大化、关闭）
 * 6. 拖拽区域：实现无边框窗口的拖拽移动
 * 
 * 【数据流流向】
 * ┌─────────────────────────────────────────────────────────────────────────┐
 * │  全局状态流入                                                            │
 * │  ├── datas.activatedModels ← 已激活的模型列表                            │
 * │  ├── datas.assistants ← 助手列表                                         │
 * │  ├── datas.user ← 当前登录用户信息                                       │
 * │  ├── datas.isLoggedIn ← 登录状态                                         │
 * │  ├── currentAssistantId ← 当前选中助手 ID                                │
 * │  ├── selectedModel ← 当前选中模型                                        │
 * │  └── globalUserAvatar ← 用户头像 URL                                     │
 * │                                                                          │
 * │  全局状态流出                                                            │
 * │  ├── setSelectedModel() → 切换当前模型                                   │
 * │  ├── setDatas() → 更新助手提示词、模型列表、用户信息                     │
 * │  ├── setGlobalUserAvatar() → 更新用户头像                                │
 * │  └── saveSingleAssistantToBackend() → 持久化助手数据                     │
 * │                                                                          │
 * │  Tauri 后端命令调用                                                      │
 * │  ├── invoke('load_activated_models') → 加载已激活模型                    │
 * │  ├── invoke('load_app_config') → 加载应用配置                            │
 * │  ├── invoke('save_app_config') → 保存模型偏好                            │
 * │  ├── invoke('start_local_server') → 启动 Llama.cpp 本地服务              │
 * │  ├── invoke('is_local_server_running') → 检查本地服务状态                │
 * │  ├── invoke('validate_token') → 验证登录 Token                           │
 * │  ├── invoke('sync_avatar_to_backend') → 同步头像到云端                   │
 * │  ├── invoke('upload_avatar') → 保存头像到本地                            │
 * │  ├── invoke('clear_local_avatar_cache') → 清理本地头像缓存               │
 * │  ├── appWindow.minimize/maximize/close → 窗口控制                        │
 * │  └── open/readFile (plugin) → 文件选择器与读取                           │
 * │                                                                          │
 * │  网络请求                                                                │
 * │  └── fetch(/health) → 本地 Llama 服务健康检查                            │
 * │                                                                          │
 * │  本地存储                                                                │
 * │  ├── localStorage.getItem('auth-token') → 读取登录凭证                   │
 * │  ├── localStorage.setItem('auth-token') → 保存登录凭证                   │
 * │  ├── localStorage.getItem('user-avatar-path') → 读取本地头像路径         │
 * │  └── localStorage.removeItem('user-avatar-path') → 清理本地头像路径      │
 * └─────────────────────────────────────────────────────────────────────────┘
 * 
 * 【组件层级】
 * NavBar (本组件)
 * ├── 拖拽区域 (data-tauri-drag-region)
 * ├── 导航栏主体
 * │   ├── 左侧：Logo + 路由链接（对话/设置）
 * │   ├── 中间：用户头像 + 下拉菜单（登录/头像/登出）
 * │   ├── 右侧：模型选择器 + 提示词按钮 + 窗口控制
 * │   └── 子组件
 * │       ├── AvatarCropModal (头像裁剪弹窗)
 * │       ├── PromptModal (提示词编辑弹窗)
 * │       └── LoginModal (登录弹窗)
 * ============================================================================
 */

// SolidJS 核心 API
import { createSignal, onMount, For, Component, Show } from 'solid-js';
// Tauri 窗口 API：自定义标题栏控制
import { Window } from '@tauri-apps/api/window';
// SolidJS 路由组件
import { A } from '@solidjs/router';
// Tauri 核心 API：调用 Rust 命令
import { invoke } from '@tauri-apps/api/core';
// Tauri 对话框插件：系统文件选择器
import { open } from '@tauri-apps/plugin-dialog';
// Tauri 文件系统插件：读取文件
import { readFile } from '@tauri-apps/plugin-fs';
// 子组件导入
import AvatarCropModal from './AvatarCropModel';
import PromptModal from './PromptModal';
import LoginModal from './LoginModal';
// 全局状态管理
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
// 本地样式
import './NavBar.css';

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
  // ==================== 状态声明 ====================

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

  // ==================== 派生状态 ====================

  /** 线上模型列表：过滤出 owned_by 不为 Local-Llama.cpp 的模型 */
  const onlineModels = () => datas.activatedModels.filter(m => m.owned_by !== "Local-Llama.cpp");
  /** 本地模型列表：过滤出 owned_by 为 Local-Llama.cpp 的模型 */
  const localModels = () => datas.activatedModels.filter(m => m.owned_by === "Local-Llama.cpp");

  // ==================== 用户认证处理 ====================

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
      await invoke('clear_local_avatar_cache');
      localStorage.removeItem('user-avatar-path');
    }
  };

  /**
   * 退出登录处理
   * 
   * 调用全局 logout 清理状态，关闭用户菜单
   */
  const handleLogout = () => {
    logout();
    setUserMenuVisible(false);
  };

  // ==================== 头像管理 ====================

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
   * - 已登录：调用 sync_avatar_to_backend 同步到云端，清理本地文件
   * - 未登录：调用 upload_avatar 保存到本地，记录路径到 localStorage
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
        await invoke('clear_local_avatar_cache');
        localStorage.removeItem('user-avatar-path');
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

  // ==================== 模型管理 ====================

  /**
   * 根据模型名称获取对应的品牌 Logo 路径
   * 
   * @param {string} modelName - 模型名称或 ID
   * @returns {string} Logo 图片的 URL 路径
   */
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
    return '/icons/ollama.svg';
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
          const loadingText = "🚀 **正在启动本地 Llama 服务器...**";

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
          // 无助手上下文时静默拉起
          await invoke('start_local_server', { modelPath: model.local_path, port: 8080, gpuLayers: 99 });
        }
      }
    }
  };

  // ==================== 窗口控制 ====================

  /** 最小化窗口 */
  const handleMinimize = async () => await appWindow.minimize();
  
  /** 切换最大化/还原窗口 */
  const handleToggleMaximize = async () => {
    await appWindow.toggleMaximize();
    setIsMaximized(await appWindow.isMaximized());
  };
  
  /** 关闭窗口 */
  const handleClose = async () => await appWindow.close();

  // ==================== 生命周期钩子 ====================

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
          await invoke('clear_local_avatar_cache');
          localStorage.removeItem('user-avatar-path');
          console.log("检测到云端头像，已清理本地陈旧缓存空间");
        }
      } catch (err) {
        console.warn("身份过期或云端获取失败:", err);
      }
    }

    // 本地头像兜底
    const localSavedPath = localStorage.getItem('user-avatar-path');
    if (localSavedPath && globalUserAvatar() === '/icons/user.svg') {
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

  // ==================== 渲染逻辑 ====================

  return (
    <>
      {/* 窗口拖拽响应区：实现无边框窗口拖拽 */}
      <div data-tauri-drag-region class="navbar-drag-region"></div>

      <nav class="navbar">
        {/* --- 左侧区域：Logo 与主导航 --- */}
        <div class="logo-container">
          <img src="/icons/logo.png" alt="AIO" class="logo" />
        </div>

        {/* 对话页面链接 */}
        <A href="/chat" class="nav-item" title="对话" activeClass="active">
          <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width={1.5} stroke="currentColor" class="size-6">
            <path stroke-linecap="round" stroke-linejoin="round" d="M2.25 12.76c0 1.6 1.123 2.994 2.707 3.227 1.087.16 2.185.283 3.293.369V21l4.076-4.076a1.526 1.526 0 0 1 1.037-.443 48.282 48.282 0 0 0 5.68-.494c1.584-.233 2.707-1.626 2.707-3.228V6.741c0-1.602-1.123-2.995-2.707-3.228A48.394 48.394 0 0 0 12 3c-2.392 0-4.744.175-7.043.513C3.373 3.746 2.25 5.14 2.25 6.741v6.018Z" />
          </svg>
        </A>

        {/* 设置页面链接 */}
        <A href="/settings" class="nav-item" title="设置" activeClass="active">
          <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width={1.5} stroke="currentColor" class="size-6">
            <path stroke-linecap="round" stroke-linejoin="round" d="M9.594 3.94c.09-.542.56-.94 1.11-.94h2.593c.55 0 1.02.398 1.11.94l.213 1.281c.063.374.313.686.645.87.074.04.147.083.22.127.325.196.72.257 1.075.124l1.217-.456a1.125 1.125 0 0 1 1.37.49l1.296 2.247a1.125 1.125 0 0 1-.26 1.431l-1.003.827c-.293.241-.438.613-.43.992a7.723 7.723 0 0 1 0 .255c-.008.378.137.75.43.991l1.004.827c.424.35.534.955.26 1.43l-1.298 2.247a1.125 1.125 0 0 1-1.369.491l-1.217-.456c-.355-.133-.75-.072-1.076.124a6.47 6.47 0 0 1-.22.128c-.331.183-.581.495-.644.869l-.213 1.281c-.09.543-.56.94-1.11.94h-2.594c-.55 0-1.019-.398-1.11-.94l-.213-1.281c-.062-.374-.312-.686-.644-.87a6.52 6.52 0 0 1-.22-.127c-.325-.196-.72-.257-1.076-.124l-1.217.456a1.125 1.125 0 0 1-1.369-.49l-1.297-2.247a1.125 1.125 0 0 1 .26-1.431l1.004-.827c.292-.24.437-.613.43-.991a6.932 6.932 0 0 1 0-.255c.007-.38-.138-.751-.43-.992l-1.004-.827a1.125 1.125 0 0 1-.26-1.43l1.297-2.247a1.125 1.125 0 0 1 1.37-.491l1.216.456c.356.133.751.072 1.076-.124.072-.044.146-.086.22-.128.332-.183.582-.495.644-.869l.214-1.28Z" />
            <path stroke-linecap="round" stroke-linejoin="round" d="M15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z" />
          </svg>
        </A>

        {/* --- 中间区域：用户头像与下拉菜单 --- */}
        <div
          class="user-avatar-wrapper"
          onMouseEnter={() => setUserMenuVisible(true)}
          onMouseLeave={() => setUserMenuVisible(false)}
        >
          <img
            src={globalUserAvatar()}
            alt="User Avatar"
            class="avatar"
            onError={(e) => {
              e.currentTarget.src = "/icons/user.svg"; // 加载失败回退默认图标
            }}
          />
          
          {/* 用户下拉菜单 */}
          <div classList={{ 'user-dropdown-menu': true, 'active': isUserMenuVisible() }}>
            {/* 更换头像选项 */}
            <div class="user-dropdown-item" onClick={handleEditAvatar}>
              <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" style="width:16px; height:16px;">
                <path stroke-linecap="round" stroke-linejoin="round" d="M6.827 6.175A2.31 2.31 0 0 1 5.186 7.23c-.38.054-.757.112-1.134.175C2.999 7.58 2.25 8.507 2.25 9.574V18a2.25 2.25 0 0 0 2.25 2.25h15A2.25 2.25 0 0 0 21.75 18V9.574c0-1.067-.75-1.994-1.802-2.169a47.865 47.865 0 0 0-1.134-.175 2.31 2.31 0 0 1-1.64-1.055l-.822-1.316a2.192 2.192 0 0 0-1.736-1.039 48.774 48.774 0 0 0-5.232 0 2.192 2.192 0 0 0-1.736 1.039l-.821 1.316Z" />
                <path stroke-linecap="round" stroke-linejoin="round" d="M16.5 12.75a4.5 4.5 0 1 1-9 0 4.5 4.5 0 0 1 9 0ZM18.75 10.5h.008v.008h-.008V10.5Z" />
              </svg>
              更换头像
            </div>

            {/* 条件渲染：登录状态决定菜单内容 */}
            <Show
              when={datas.isLoggedIn}
              fallback={
                // 未登录：显示登录选项
                <div class="user-dropdown-item"
                  onClick={() => {
                    setIsLoginModalOpen(true);
                    setUserMenuVisible(false);
                  }}>
                  <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" style="width:16px; height:16px;">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M15.75 6a3.75 3.75 0 1 1-7.5 0 3.75 3.75 0 0 1 7.5 0ZM4.501 20.118a7.5 7.5 0 0 1 14.998 0A17.933 17.933 0 0 1 12 21.75c-2.676 0-5.216-.584-7.499-1.632Z" />
                  </svg>
                  登录账号
                </div>
              }
            >
              {/* 已登录：显示账号信息、切换账号、退出登录 */}
              <div class="user-dropdown-divider"></div>
              <div class="user-dropdown-item" style="font-weight: 500;">
                <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" style="width:16px; height:16px;">
                  <path stroke-linecap="round" stroke-linejoin="round" d="M11.25 11.25l.041-.02a.75.75 0 011.063.852l-.708 2.836a.75.75 0 001.063.853l.041-.021M21 12a9 9 0 11-18 0 9 9 0 0118 0zm-9-3.75h.008v.008H12V8.25z" />
                </svg>
                账号信息
              </div>
              <div class="user-dropdown-item"
                onClick={() => {
                  setIsLoginModalOpen(true);
                  setUserMenuVisible(false);
                }}>
                <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" style="width:16px; height:16px;">
                  <path stroke-linecap="round" stroke-linejoin="round" d="M16.023 9.348h4.992v-.001M2.985 19.644v-4.992m0 0h4.992m-4.993 0l3.181 3.183a8.25 8.25 0 0013.803-3.7M4.031 9.865a8.25 8.25 0 0113.803-3.7l3.181 3.182m0-4.991v4.99" />
                </svg>
                切换账号
              </div>
              <div class="user-dropdown-item logout-item" onClick={handleLogout}>
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="16" height="16">
                  <path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4M16 17l5-5-5-5M21 12H9" />
                </svg>
                退出登录
              </div>
            </Show>
          </div>
        </div>

        {/* --- 右侧区域：模型选择器 --- */}
        <div
          class="model-selector-wrapper"
          onMouseEnter={() => setDropdownVisible(true)}
          onMouseLeave={() => setDropdownVisible(false)}
        >
          <div class="nav-item model-selector" title="选择模型">
            <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width={1.5} stroke="currentColor" class="size-6">
              <path stroke-linecap="round" stroke-linejoin="round" d="M13.5 16.875h3.375m0 0h3.375m-3.375 0V13.5m0 3.375v3.375M6 10.5h2.25a2.25 2.25 0 0 0 2.25-2.25V6a2.25 2.25 0 0 0-2.25-2.25H6A2.25 2.25 0 0 0 3.75 6v2.25A2.25 2.25 0 0 0 6 10.5Zm0 9.75h2.25A2.25 2.25 0 0 0 10.5 18v-2.25a2.25 2.25 0 0 0-2.25-2.25H6a2.25 2.25 0 0 0-2.25 2.25V18A2.25 2.25 0 0 0 6 20.25Zm9.75-9.75H18a2.25 2.25 0 0 0 2.25-2.25V6A2.25 2.25 0 0 0 18 3.75h-2.25A2.25 2.25 0 0 0 13.5 6v2.25a2.25 2.25 0 0 0 2.25 2.25Z" />
            </svg>
          </div>

          {/* 模型下拉菜单：双列布局（线上/本地） */}
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

        {/* 提示词设置按钮 */}
        <a href="#" title="设置提示词" class="nav-item" onClick={handleOpenPromptModal}>
          <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width={1.5} stroke="currentColor" class="size-6">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 18v-5.25m0 0a6.01 6.01 0 0 0 1.5-.189m-1.5.189a6.01 6.01 0 0 1-1.5-.189m3.75 7.478a12.06 12.06 0 0 1-4.5 0m3.75 2.383a14.406 14.406 0 0 1-3 0M14.25 18v-.192c0-.983.658-1.823 1.508-2.316a7.5 7.5 0 1 0-7.517 0c.85.493 1.509 1.333 1.509 2.316V18" />
          </svg>
        </a>

        {/* --- 窗口控制按钮组 --- */}
        <div class="window-controls">
          <button class="control-button minimize" onClick={handleMinimize} title="最小化">
            <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width={1.5} stroke="currentColor" class="size-6">
              <path stroke-linecap="round" stroke-linejoin="round" d="M5 12h14" />
            </svg>
          </button>

          <button class="control-button maximize" onClick={handleToggleMaximize} title={isMaximized() ? "还原" : "最大化"}>
            {isMaximized() ? (
              <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width={1.5} stroke="currentColor" class="size-6">
                <path stroke-linecap="round" stroke-linejoin="round" d="M16.5 8.25V6a2.25 2.25 0 0 0-2.25-2.25H6A2.25 2.25 0 0 0 3.75 6v8.25A2.25 2.25 0 0 0 6 16.5h2.25m8.25-8.25H18a2.25 2.25 0 0 1 2.25 2.25V18A2.25 2.25 0 0 1 18 20.25h-7.5A2.25 2.25 0 0 1 8.25 18v-1.5m8.25-8.25h-6a2.25 2.25 0 0 0-2.25 2.25v6" />
              </svg>
            ) : (
              <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width={1.5} stroke="currentColor" class="size-6">
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

      {/* 子组件渲染 */}
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
}

export default NavBar;
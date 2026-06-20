import { createSignal, onMount, Component, Show } from 'solid-js';
import { Window } from '@tauri-apps/api/window';
import { A } from '@solidjs/router';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import AvatarCropModal from './AvatarCropModel';
import PromptModal from './PromptModal';
import LoginModal from './LoginModal';
import UserDropdown from './UserDropdown';
import ModelDropdown from './ModelDropdown';
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
  logout,
  setIsStartingLocalModel,
  setLocalModelStartProgress,
  activeProviderModels,
  providerConfigs,
  isLocalAutoStartConfirmed,
  setLocalAutoStartConfirmed,
} from '../store/store';
import { getLogo as getLogoByIds } from '../utils/modelLogo';

/**
 * 初始化窗口实例
 */
const appWindow = new Window('main');

interface NavBarProps { }

/**
 * 导航栏组件
 * @returns {JSX.Element} 导航栏 JSX 元素
 */
const NavBar: Component<NavBarProps> = () => {

  const [modalPrompt, setModalPrompt] = createSignal(''); // 提示词弹窗的临时编辑内容
  const [isModalOpen, setIsModalOpen] = createSignal<boolean>(false); // 提示词弹窗显示状态
  const [isDropdownVisible, setDropdownVisible] = createSignal<boolean>(false); // 模型选择下拉菜单可见性
  const [isMaximized, setIsMaximized] = createSignal<boolean>(false); // 窗口最大化状态
  const [isUserMenuVisible, setUserMenuVisible] = createSignal(false); // 用户下拉菜单显示状态
  const [tempImage, setTempImage] = createSignal<string | null>(null); // 头像裁剪用的临时图片 DataURL
  const [isLoginModalOpen, setIsLoginModalOpen] = createSignal(false); // 登录弹窗显示状态

  const isLocalModel = (m: ActivatedModel) => !!(m.local_path || m.engine_type);
  const localModels = () => datas.activatedModels.filter(m => isLocalModel(m));
  // 云端模型：来自 providerConfigs (lobehub 形态)
  const cloudModels = () => activeProviderModels().map(m => ({
      model_id: m.modelId,
      owned_by: m.providerName,
      api_url: m.apiUrl,
      api_key: m.apiKey,
      provider_id: m.provider,
  } as ActivatedModel & { provider_id: string }));

  /**
   * 登录成功回调处理
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
      const avatarUrl = await loadAvatarFromPath(user.avatar);
      setGlobalUserAvatar(avatarUrl);
    }
  };

  /**
   * 退出登录处理
   */
  const handleLogout = async () => {
    await logout();
    setUserMenuVisible(false);
    const localSavedPath = localStorage.getItem('user-avatar-path');
    if (localSavedPath) {
        try {
            const url = await loadAvatarFromPath(localSavedPath);
            setGlobalUserAvatar(url);
            console.log("退出成功，已恢复本地头像");
        } catch (err) {
            console.error("恢复本地头像失败:", err);
            setGlobalUserAvatar('/icons/app-logo/user.svg');
        }
    }
  };

  /**
   * 处理编辑头像：打开文件选择器并触发裁剪流程
   * H4 适配：不再直接 readFile 任意路径，改用 Rust read_avatar_source
   */
  const handleEditAvatar = async () => {
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp', 'bmp', 'gif'] }]
      });

      if (selected && typeof selected === 'string') {
        // 由 Rust 读取并 base64 编码（无需 fs:allow-read-file ** scope）
        const dataUrl = await invoke<string>('read_avatar_source', { path: selected });
        setTempImage(dataUrl);
      }
    } catch (err) {
      console.error("选择头像失败:", err);
      alert('选择头像失败: ' + err);
    }
  };

  /**
   * 头像裁剪完成回调
   * @param {string} croppedDataUrl - 裁剪后的 Base64 DataURL
   */
  const onCropSave = async (croppedDataUrl: string) => {
    try {
      if (datas.isLoggedIn && datas.user?.token) {
        // 云端同步
        await invoke('sync_avatar_to_backend', {
          token: datas.user.token,
          avatarData: croppedDataUrl
        });
        setGlobalUserAvatar(croppedDataUrl);
        console.log("头像已存入云端，本地文件已释放空间");
      } else {
        // 本地保存
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
   * 获取模型 Logo 路径
   * @param {string} modelName - 模型名称或 ID
   * @returns {string} Logo 图片路径
   */
  const getModelLogo = (modelName: string) => {
    // 根据 model_name 找 logo
    // 注: 传入的 modelName 实际可能是 model_id (来自 activatedModels / onlineModels)
    // 这里走文本匹配兜底
    return getLogoByIds(null, modelName);
  };

  /**
   * 静默启动本地模型服务
   * M13: 首次启动需用户确认；确认后写入 localStorage 永久放行
   * @param {ActivatedModel} model - 需要启动的本地模型信息
   */
  const startLocalModel = async (model: ActivatedModel) => {
    if (!model.local_path) return;
    // M13 防护：未确认时不静默启动
    if (!isLocalAutoStartConfirmed()) {
      const ok = confirm(
        `检测到本地模型 "${model.model_id}"\n` +
        `路径: ${model.local_path}\n\n` +
        `是否允许 AIO 在应用启动时自动拉起该本地推理引擎？\n` +
        `（点击"取消"后，可随时在设置页手动启动）`
      );
      if (!ok) return;
      setLocalAutoStartConfirmed();
    }
    const isRunning = await invoke<boolean>('is_local_server_running');
    if (!isRunning) {
      try {
        await invoke('start_local_server', {
          modelPath: model.local_path,
          port: 8080,
          gpuLayers: 99,
          engineType: model.engine_type || 'llama_cpp'
        });
        console.info("本地模型服务已启动");
      } catch (e) {
        console.error("自动启动本地模型失败:", e);
      }
    }
  };

  /**
   * 处理打开提示词设置弹窗
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
   * 检查本地服务器健康状况
   * @param {string} baseUrl - 服务器基础地址
   * @returns {Promise<boolean>} 服务器是否就绪
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
   * @param {ActivatedModel} model - 用户选择的目标模型
   */
  const handleModelSelect = async (model: ActivatedModel) => {
    setSelectedModel(model);
    setDropdownVisible(false);

    try {
      const currentConfig = await invoke<any>('load_app_config');
      await invoke('save_app_config', {
        config: { ...currentConfig, defaultModel: model.model_id }
      });
    } catch (e) {
      console.error("保存模型偏好失败:", e);
    }

    if (isLocalModel(model) && model.local_path) {
      const isRunning = await invoke<boolean>('is_local_server_running');

      if (!isRunning) {
        if (datas.assistants.length === 0) {
          const loaded = await invoke<any[]>('load_assistants');
          if (loaded?.length > 0) setDatas('assistants', loaded);
        }

        let asstId = currentAssistantId() || datas.assistants[0]?.id;
        const assistant = datas.assistants.find(a => a.id === asstId);

        if (assistant) {
          const topicId = assistant.topics[0]?.id;
          const loadingText = "**正在启动本地推理引擎...**";

          if (topicId) {
            setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId,
              'history', h => [...h, { role: 'assistant', content: loadingText }]
            );
          }

          try {
            setIsStartingLocalModel(true);
            setLocalModelStartProgress(0);
            await invoke('start_local_server', {
              modelPath: model.local_path,
              port: 8080,
              gpuLayers: 99,
              engineType: model.engine_type || 'llama_cpp'
            });

            let attempts = 0;
            const maxAttempts = 60;

            const poll = setInterval(async () => {
              attempts++;
              const isReady = await checkServerHealth("http://127.0.0.1:8080/v1");

              if (isReady) {
                clearInterval(poll);
                setLocalModelStartProgress(100);
                setTimeout(() => {
                  setIsStartingLocalModel(false);
                  setLocalModelStartProgress(0);
                }, 1000);
                setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId,
                  'history', h => h.map((msg: any) =>
                    msg.content === loadingText
                      ? { ...msg, content: "**本地服务器启动成功，可以开始对话了！**" }
                      : msg
                  )
                );
              } else if (attempts >= maxAttempts) {
                clearInterval(poll);
                setLocalModelStartProgress(100);
                setTimeout(() => {
                  setIsStartingLocalModel(false);
                  setLocalModelStartProgress(0);
                }, 1000);
                setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId,
                  'history', h => [...h, { role: 'assistant', content: "**服务器启动超时，请检查显存空间或模型文件。**" }]
                );
              }
            }, 500);

          } catch (err) {
            setLocalModelStartProgress(100);
            setTimeout(() => {
              setIsStartingLocalModel(false);
              setLocalModelStartProgress(0);
            }, 1000);
            setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId,
              'history', h => [...h, { role: 'assistant', content: `**启动失败: ${err}**` }]
            );
          }
        } else {
          await invoke('start_local_server', { modelPath: model.local_path, port: 8080, gpuLayers: 99, engineType: model.engine_type || 'llama_cpp' });
        }
      }
    }
  };

  const handleMinimize = async () => await appWindow.minimize(); // 最小化窗口
  
  const handleToggleMaximize = async () => { // 切换最大化/还原窗口
    await appWindow.toggleMaximize();
    setIsMaximized(await appWindow.isMaximized());
  };
  
  const handleClose = async () => await appWindow.close(); // 关闭窗口

  /**
   * 组件挂载时初始化：Token 验证、头像加载、模型加载、窗口监听
   */
  onMount(async () => {
    // 监听llama启动进度事件
    const unlistenProgress = await listen('llama-progress', (event) => {
      setLocalModelStartProgress((event.payload as number) * 100);
    });
    const unlistenEngineProgress = await listen('engine-progress', (event) => {
      setLocalModelStartProgress((event.payload as number) * 100);
    });

    // H5 适配：从 Rust keyring 读取 token（HTTPS 校验）
    let savedToken: string | null = null;
    try {
      savedToken = await invoke<string | null>('read_auth_token');
    } catch (err) {
      console.warn('读 keyring 失败:', err);
    }

    if (savedToken) {
      try {
        const userData = await invoke<any>('validate_token', { token: savedToken });
        setDatas('user', userData);
        setDatas('isLoggedIn', true);

        if (userData.avatar) {
          const avatarUrl = await loadAvatarFromPath(userData.avatar);
          setGlobalUserAvatar(avatarUrl);
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
        if (isLocalModel(targetModel)) {
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
      unlistenProgress();
      unlistenEngineProgress();
    };
  });

return (
    <>
      <div 
        data-tauri-drag-region 
        class="absolute top-0 left-0 right-0 h-[60px] z-[1] [app-region:drag]"
      ></div>

      <nav
        data-tauri-drag-region
        class="navbar relative flex justify-center items-center gap-6 px-5 h-[60px] m-0 mr-[1px] z-[100] [app-region:drag] select-none"
      >
        <div class="absolute left-[10px] top-1/2 -translate-y-1/2 flex items-center justify-center z-[1001] pointer-events-none">
          <img src="/icons/app-logo/logo.svg" alt="AIO" class="w-10 h-10 object-contain block [app-region:no-drag]" />
        </div>
        <A 
          href="/chat" 
          title="对话" 
          activeClass="!text-pri font-bold" 
          class="nav-icon-link [app-region:no-drag]"
        >
          <Icon src="/icons/app-logo/chat.svg" class="w-6 h-6" />
        </A>

        <A 
          href="/settings" 
          title="设置" 
          activeClass="!text-pri font-bold" 
          class="nav-icon-link [app-region:no-drag]"
        >
          <Icon src="/icons/app-logo/settings-gear.svg" class="w-6 h-6" />
        </A>

        <UserDropdown 
          avatar={globalUserAvatar()}
          isLoggedIn={datas.isLoggedIn}
          onEditAvatar={handleEditAvatar}
          onLoginClick={() => setIsLoginModalOpen(true)}
          onLogout={handleLogout}
        />

        <ModelDropdown
          selectedModel={selectedModel()}
          onlineModels={cloudModels()}
          localModels={localModels()}
          onSelect={handleModelSelect}
          getModelLogo={getModelLogo}
        />

        <a href="#" title="设置提示词" class="nav-icon-link [app-region:no-drag]" onClick={handleOpenPromptModal}>
          <Icon src="/icons/app-logo/prompt.svg" class="w-6 h-6" />
        </a>

        <div class="absolute right-5 flex items-center [app-region:no-drag]">
          <button class="win-ctrl-btn hover:bg-white/10" onClick={handleMinimize} title="最小化">
            <Icon src="/icons/app-logo/minimize.svg" class="w-6 h-6" />
          </button>

          <button class="win-ctrl-btn hover:bg-white/10" onClick={handleToggleMaximize} title={isMaximized() ? "还原" : "最大化"}>
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
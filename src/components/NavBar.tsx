import { createSignal, onMount, Component, Show } from 'solid-js';
import { Window } from '@tauri-apps/api/window';
import { A } from '@solidjs/router';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import AvatarCropModal from './AvatarCropModel';
import LoginModal from './LoginModal';
import UserDropdown from './UserDropdown';
import Icon from './Icon';
import {
  datas,
  setDatas,
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
  isLocalModel,
  startLocalEngineForAssistant,
  currentAssistantId,
} from '../store/store';

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

  const [isMaximized, setIsMaximized] = createSignal<boolean>(false); // 窗口最大化状态
  const [isUserMenuVisible, setUserMenuVisible] = createSignal(false); // 用户下拉菜单显示状态
  const [tempImage, setTempImage] = createSignal<string | null>(null); // 头像裁剪用的临时图片 DataURL
  const [isLoginModalOpen, setIsLoginModalOpen] = createSignal(false); // 登录弹窗显示状态

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
   * 启动时拉起本地推理引擎（若默认/当前模型为本地模型）。
   * 委托给 store 的 startLocalEngineForAssistant，由其负责确认、轮询与进度反馈。
   * @param {ActivatedModel} model - 启动时解析出的本地模型
   */
  const startLocalModel = async (model: ActivatedModel) => {
    if (!model.local_path) return;
    // 启动时尚无确定的当前助手，用首个助手（若有）作为 loading 落点
    let asstId = currentAssistantId() || datas.assistants[0]?.id;
    if (!asstId) {
      // 助手尚未加载：退化为直接拉起，不写 loading 消息
      const isRunning = await invoke<boolean>('is_local_server_running');
      if (!isRunning) {
        try {
          await invoke('start_local_server', {
            modelPath: model.local_path, port: 8080, gpuLayers: 99,
            engineType: model.engine_type || 'llama_cpp'
          });
        } catch (e) { console.error("自动启动本地模型失败:", e); }
      }
      return;
    }
    await startLocalEngineForAssistant(model, asstId);
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

        <UserDropdown
          avatar={globalUserAvatar()}
          isLoggedIn={datas.isLoggedIn}
          onEditAvatar={handleEditAvatar}
          onLoginClick={() => setIsLoginModalOpen(true)}
          onLogout={handleLogout}
        />

        <A 
          href="/settings" 
          title="设置" 
          activeClass="!text-pri font-bold" 
          class="nav-icon-link [app-region:no-drag]"
        >
          <Icon src="/icons/app-logo/settings-gear.svg" class="w-6 h-6" />
        </A>

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

      <LoginModal
        show={isLoginModalOpen()}
        onClose={() => setIsLoginModalOpen(false)}
        onSuccess={handleLoginSuccess}
      />
    </>
  );
};

export default NavBar;

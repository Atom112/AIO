import { createSignal, onMount, Component, Show } from 'solid-js';
import { Window } from '@tauri-apps/api/window';
import { A } from '@solidjs/router';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import AvatarCropModal from './AvatarCropModel';
import UserDropdown from './UserDropdown';
import Icon from './Icon';
import {
  datas,
  setDatas,
  selectedModel,
  setSelectedModel,
  ActivatedModel,
  globalUserAvatar,
  setGlobalUserAvatar,
  loadAvatarFromPath,
  setIsStartingLocalModel,
  setLocalModelStartProgress,
  isLocalModel,
  startLocalEngineForAssistant,
  currentAssistantId,
} from '../../core/store/store';
import { reportError, t } from '../../core/i18n';

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
  const [isMaximized, setIsMaximized] = createSignal<boolean>(false);
  const [tempImage, setTempImage] = createSignal<string | null>(null);

  /**
   * 处理编辑头像：打开文件选择器并触发裁剪流程
   */
  const handleEditAvatar = async () => {
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp', 'bmp', 'gif'] }]
      });

      if (selected && typeof selected === 'string') {
        const dataUrl = await invoke<string>('read_avatar_source', { path: selected });
        setTempImage(dataUrl);
      }
    } catch (err) {
      console.error("选择头像失败:", err);
      alert(reportError('error.load', err));
    }
  };

  /**
   * 头像裁剪完成回调 — 本地保存
   */
  const onCropSave = async (croppedDataUrl: string) => {
    try {
      const savedPath = await invoke<string>('upload_avatar', {
        dataUrl: croppedDataUrl
      });
      setGlobalUserAvatar(croppedDataUrl);
      localStorage.setItem('user-avatar-path', savedPath);
      setTempImage(null);
    } catch (err) {
      alert(reportError('error.save', err));
    }
  };

  /**
   * 启动时拉起本地推理引擎（若默认/当前模型为本地模型）。
   */
  const startLocalModel = async (model: ActivatedModel) => {
    if (!model.local_path) return;
    let asstId = currentAssistantId() || datas.assistants[0]?.id;
    if (!asstId) {
      const isRunning = await invoke<boolean>('is_local_server_running');
      if (!isRunning) {
        try {
          await invoke('start_local_server', {
            modelPath: model.local_path, port: 8080, gpuLayers: 99,
            engineType: model.engine_type || 'llama_cpp',
            trustRemoteCode: false,
          });
        } catch (e) { console.error("自动启动本地模型失败:", e); }
      }
      return;
    }
    await startLocalEngineForAssistant(model, asstId);
  };

  const handleMinimize = async () => await appWindow.minimize();
  const handleToggleMaximize = async () => {
    await appWindow.toggleMaximize();
    setIsMaximized(await appWindow.isMaximized());
  };
  const handleClose = async () => await appWindow.close();

  /**
   * 组件挂载时初始化：头像加载、模型加载、窗口监听
   */
  onMount(async () => {
    const unlistenProgress = await listen('llama-progress', (event) => {
      setLocalModelStartProgress((event.payload as number) * 100);
    });
    const unlistenEngineProgress = await listen('engine-progress', (event) => {
      setLocalModelStartProgress((event.payload as number) * 100);
    });

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
          title={t('nav.chat')}
          activeClass="!text-pri font-bold"
          class="flex items-center gap-2 px-3 py-2 rounded-md transition-all duration-200 cursor-pointer text-white/50 hover:text-white/85 hover:bg-white/[0.06] [app-region:no-drag]"
        >
          <Icon src="/icons/app-logo/chat.svg" class="w-6 h-6" />
        </A>

        <UserDropdown
          avatar={globalUserAvatar()}
          onEditAvatar={handleEditAvatar}
        />

        <A
          href="/settings"
          title={t('nav.settings')}
          activeClass="!text-pri font-bold"
          class="flex items-center gap-2 px-3 py-2 rounded-md transition-all duration-200 cursor-pointer text-white/50 hover:text-white/85 hover:bg-white/[0.06] [app-region:no-drag]"
        >
          <Icon src="/icons/app-logo/settings-gear.svg" class="w-6 h-6" />
        </A>

        <div class="absolute right-5 flex items-center [app-region:no-drag]">
          <button class="w-[30px] h-[30px] flex justify-center items-center bg-transparent border-none text-lg cursor-pointer rounded-md transition-all ml-1 text-white/40 hover:text-white hover:bg-white/10" onClick={handleMinimize} title={t('nav.minimize')}>
            <Icon src="/icons/app-logo/minimize.svg" class="w-6 h-6" />
          </button>

          <button class="w-[30px] h-[30px] flex justify-center items-center bg-transparent border-none text-lg cursor-pointer rounded-md transition-all ml-1 text-white/40 hover:text-white hover:bg-white/10" onClick={handleToggleMaximize} title={isMaximized() ? t('nav.restore') : t('nav.maximize')}>
            {isMaximized() ? (
              <Icon src="/icons/app-logo/restore.svg" class="w-6 h-6" />
            ) : (
              <Icon src="/icons/app-logo/maximize.svg" class="w-6 h-6" />
            )}
          </button>

          <button class="w-[30px] h-[30px] flex justify-center items-center bg-transparent border-none text-lg cursor-pointer rounded-md transition-all ml-1 text-white/40 hover:text-white hover:bg-danger" onClick={handleClose} title={t('nav.close')}>
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
    </>
  );
};

export default NavBar;

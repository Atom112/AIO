// src/components/NavBar.tsx

//---------------------- imports --------------------------------

import { createSignal, onMount, For, createEffect } from 'solid-js';
import { datas, setDatas, currentAssistantId, saveSingleAssistantToBackend, selectedModel, setSelectedModel, ActivatedModel } from '../store/store'
import { Window } from '@tauri-apps/api/window';
import { A } from '@solidjs/router';
import type { JSX } from 'solid-js';
import PromptModal from '../pages/PromptModal';
import { invoke } from '@tauri-apps/api/core';
import './NavBar.css';


//---------------------------------------------------------------

//创建窗口实例（标签为main，在src-tauri/tauri.conf.json中的windows.title字段决定。默认为main）
const appWindow = new Window('main');

interface NavBarProps { }  // 目前没有传入属性，可以根据需要添加

function NavBar(props: NavBarProps): JSX.Element {

  //用于存储用户在“设置提示词”弹窗中输入的提示词
  const [modalPrompt, setModalPrompt] = createSignal('');
  //创建响应式状态："设置提示词"弹窗是否被打开（默认为否）
  const [isModalOpen, setIsModalOpen] = createSignal<boolean>(false);


  //可选的模型列表，在后期开发中需要从外部导入真实的模型列表
  const allModels: string[] = ['GPT-4', 'Claude 3', 'Gemini Pro', 'Llama 3'];

  // 使用从 ../store/store 导入的 selectedModel 与 setSelectedModel（避免与全局 store 重名）
  // selectedModel / setSelectedModel 已在文件顶部导入

  //模型选择下拉菜单是否可见（默认为否）
  const [isDropdownVisible, setDropdownVisible] = createSignal<boolean>(false);

  // 用于跟踪窗口是否最大化，以便更新全屏按钮的图标（例如，最大化/还原。默认为未最大化）
  const [isMaximized, setIsMaximized] = createSignal<boolean>(false);

  // 用于存储 setTimeout 的 ID，用来处理下拉菜单的可见性问题
  let hideTimeoutId: ReturnType<typeof setTimeout> | undefined;

const startLocalModel = async (model: ActivatedModel) => {
    if (model.owned_by === "Local-Llama.cpp" && model.local_path) {
      const isRunning = await invoke<boolean>('is_local_server_running');
      if (!isRunning) {
        // 这里可以使用你原本 handleModelSelect 中的启动逻辑
        // 或者简单地静默启动（不发聊天消息），取决于你的需求
        try {
            await invoke('start_local_server', {
              modelPath: model.local_path,
              port: 8080,
              gpuLayers: 99
            });
            console.log("本地模型自动启动成功");
        } catch(e) {
            console.error("自动启动本地模型失败", e);
        }
      }
    }
};

  const handleOpenPromptModal = (e: MouseEvent) => {
    e.preventDefault();
    const activeId = currentAssistantId();
    if (!activeId) {
      alert("请先在聊天界面创建一个助手");
      return;
    }
    // 从 Store 中查找当前助手的 prompt
    const assistant = datas.assistants.find(a => a.id === activeId);
    setModalPrompt(assistant?.prompt || '');
    setIsModalOpen(true);
  };

  // 保存 Prompt 的逻辑
  const handleSavePrompt = (newPrompt: string) => {
    const activeId = currentAssistantId();
    if (activeId) {
      // 更新 Store
      setDatas('assistants', a => a.id === activeId, 'prompt', newPrompt);
      // 触发后端保存
      saveSingleAssistantToBackend(activeId);
      console.log("提示词已更新并保存");
    }
    // setIsModalOpen(false) 在 PromptModal 组件内部或由 onClose 触发，这里通常不需要手动调，除非 Modal 设计如此
  };

  // 监听窗口最大化/还原事件，以便即时更新按钮状态
  onMount(async () => {

    try {
      // 1. 并行加载：模型列表 和 应用配置
      const [models, config] = await Promise.all([
        invoke<ActivatedModel[]>('load_activated_models'),
        invoke<any>('load_app_config') // 这里类型设为 any 方便读取字段，或定义 AppConfig 接口
      ]);

      setDatas('activatedModels', models);

      if (models.length > 0) {
        // 2. 寻找匹配上次保存的模型 ID
        const lastSelectedId = config.defaultModel; // 注意这是 Rust 序列化回来的驼峰名
        const lastUsedModel = models.find(m => m.model_id === lastSelectedId);

        if (lastUsedModel) {
          setSelectedModel(lastUsedModel);
          if (lastUsedModel.owned_by === "Local-Llama.cpp") {
             // 注意：这里建议做一个简单的启动，不要像点击那样在这个阶段给聊天框发“正在启动”的消息，
             // 因为这时候聊天界面可能还未完全准备好数据。
            startLocalModel(lastUsedModel); 
          }
        } else {
          // 如果没找到（比如模型被删了），默认选第一个
          setSelectedModel(models[0]);
        }
      }
    } catch (e) {
      console.error("初始化加载失败", e);
    }

    // 在组件挂载时检查窗口的初始最大化状态
    setIsMaximized(await appWindow.isMaximized());
    // onResized 也会在最大化/还原时触发
    const unlistenMaximized = await appWindow.onResized(async () => {
      setIsMaximized(await appWindow.isMaximized());
    });

    return () => {
      // 组件卸载时取消监听
      unlistenMaximized();
    };
  });


  // // 处理鼠标进入模型选择栏，显示下拉框
  // const handleMouseEnter = (): void => {
  //   clearTimeout(hideTimeoutId);
  //   setDropdownVisible(true);
  // };

  // // 鼠标离开下拉菜单栏 0.2 秒后隐藏下拉菜单栏
  // const handleMouseLeave = (): void => {
  //   hideTimeoutId = setTimeout(() => {
  //     setDropdownVisible(false);
  //   }, 200); // 延迟 0.2 秒隐藏
  // };

  // 修改 NavBar.tsx 中的 checkServerHealth
  const checkServerHealth = async (baseUrl: string): Promise<boolean> => {
    try {
      const controller = new AbortController();
      // 适当放宽超时时间，因为服务器在高载入模型时响应可能慢
      const id = setTimeout(() => controller.abort(), 2000);

      // 使用原生的 health 接口（注意：llama-server 默认在根路径提供 /health）
      // 如果你的 baseUrl 是 http://127.0.0.1:8080/v1，需要处理一下
      const rootUrl = baseUrl.replace('/v1', '');
      const resp = await fetch(`${rootUrl}/health`, { signal: controller.signal });

      clearTimeout(id);
      // llama-server 就绪时通常返回 {"status": "ok"}
      return resp.ok;
    } catch {
      return false;
    }
  };

  // 下拉菜单栏点击选择模型后立即隐藏
  const handleModelSelect = async (model: ActivatedModel) => {
    setSelectedModel(model);
    setDropdownVisible(false);

    // 保存选择偏好
    const currentConfig = await invoke<any>('load_app_config');
    await invoke('save_app_config', {
      config: { ...currentConfig, defaultModel: model.model_id }
    });

    // --- 本地模型自动启动逻辑 ---
    if (model.owned_by === "Local-Llama.cpp" && model.local_path) {
      const isRunning = await invoke<boolean>('is_local_server_running');

      if (!isRunning) {
        if (datas.assistants.length === 0) {
          try {
            const loaded = await invoke<any[]>('load_assistants');
            if (loaded && loaded.length > 0) {
              setDatas('assistants', loaded);
            }
          } catch (e) {
            console.error("加载助手失败:", e);
          }
        }

        let asstId = currentAssistantId();

        // 如果仍然没有 ID（说明是在新装设备或首次启动），尝试获取第一个
        if (!asstId && datas.assistants.length > 0) {
          asstId = datas.assistants[0].id;
        }

        const assistant = datas.assistants.find(a => a.id === asstId);

        if (assistant) {
          const topicId = assistant.topics[0]?.id; // 简单起见取当前第一个话题

          // 1. 在聊天框显示“启动中”
          const loadingText = "🚀 **正在启动本地 Llama 服务器...**";
          if (topicId) {
            setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId,
              'history', h => [...h, { role: 'assistant', content: loadingText }]
            );
          }

          try {
            // 2. 调用后端启动
            await invoke('start_local_server', {
              modelPath: model.local_path,
              port: 8080,
              gpuLayers: 99
            });

            // 3. 循环探测服务器直到就绪 (心跳检测)
            let attempts = 0;
            const maxAttempts = 60; // 最多等 60 秒

            const poll = setInterval(async () => {
              attempts++;
              const ready = await checkServerHealth("http://127.0.0.1:8080/v1");

              if (ready) {
                clearInterval(poll);
                setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId,
                  'history', h => {
                    return h.map((msg: any) =>
                      msg.content === loadingText
                        ? { ...msg, content: "✅ **本地服务器启动成功，可以开始对话了！**" }
                        : msg
                    );
                  }
                );
              } else if (attempts >= maxAttempts) {
                clearInterval(poll);
                setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId,
                  'history', h => [...h, { role: 'assistant', content: "❌ **服务器启动超时，请检查显存空间或模型文件。**" }]
                );
              }
            }, 1500); // 每秒探测一次

          } catch (err) {
            setDatas('assistants', a => a.id === asstId, 'topics', t => t.id === topicId,
              'history', h => [...h, { role: 'assistant', content: `❌ **启动失败: ${err}**` }]
            );
          }
        } else {

          try {
            await invoke('start_local_server', {
              modelPath: model.local_path,
              port: 8080,
              gpuLayers: 99
            });
            console.log("无助手环境下，本地服务已后台启动");
          } catch (err) {
            alert("本地服务启动失败: " + err);
          }

        }
      }
    }
  };

  // --------------- Tauri 窗口控制功能 ----------------------
  // 最小化窗口
  const handleMinimize = async (): Promise<void> => {
    await appWindow.minimize();
  };

  // 切换窗口最大化/还原
  const handleToggleMaximize = async (): Promise<void> => {
    await appWindow.toggleMaximize(); // Tauri 提供 `toggleMaximize` 方法
    setIsMaximized(await appWindow.isMaximized()); // 更新最大化状态
  };

  // 关闭窗口
  const handleClose = async (): Promise<void> => {
    await appWindow.close();
  };

  return (
    <>
      {/* 添加一个可以拖拽的区域，通常用于无边框窗口 */}
      <div data-tauri-drag-region class="navbar-drag-region"></div>
      {/* 阻止navbar自身接收拖拽事件 */}
      <nav class="navbar">

        {/* ------------------------------ 左侧项目 ---------------------------------- */}
        {/* logo图标 */}
        <div class="logo-container" id='1'>
          <img src="/icons/logo.png" alt="AIO" class="logo" />
        </div>

        {/* 对话按钮 */}

        <A href="/chat" class="nav-item" title="对话" activeClass="active">
          <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-Width={1.5} stroke="currentColor" class="size-6">
            <path stroke-Linecap="round" stroke-Linejoin="round" d="M2.25 12.76c0 1.6 1.123 2.994 2.707 3.227 1.087.16 2.185.283 3.293.369V21l4.076-4.076a1.526 1.526 0 0 1 1.037-.443 48.282 48.282 0 0 0 5.68-.494c1.584-.233 2.707-1.626 2.707-3.228V6.741c0-1.602-1.123-2.995-2.707-3.228A48.394 48.394 0 0 0 12 3c-2.392 0-4.744.175-7.043.513C3.373 3.746 2.25 5.14 2.25 6.741v6.018Z" />
          </svg>
        </A>


        {/* 设置按钮 */}
        <A href="/settings" class="nav-item" title="设置" activeClass="active">
          <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-Width={1.5} stroke="currentColor" class="size-6">
            <path stroke-Linecap="round" stroke-Linejoin="round" d="M9.594 3.94c.09-.542.56-.94 1.11-.94h2.593c.55 0 1.02.398 1.11.94l.213 1.281c.063.374.313.686.645.87.074.04.147.083.22.127.325.196.72.257 1.075.124l1.217-.456a1.125 1.125 0 0 1 1.37.49l1.296 2.247a1.125 1.125 0 0 1-.26 1.431l-1.003.827c-.293.241-.438.613-.43.992a7.723 7.723 0 0 1 0 .255c-.008.378.137.75.43.991l1.004.827c.424.35.534.955.26 1.43l-1.298 2.247a1.125 1.125 0 0 1-1.369.491l-1.217-.456c-.355-.133-.75-.072-1.076.124a6.47 6.47 0 0 1-.22.128c-.331.183-.581.495-.644.869l-.213 1.281c-.09.543-.56.94-1.11.94h-2.594c-.55 0-1.019-.398-1.11-.94l-.213-1.281c-.062-.374-.312-.686-.644-.87a6.52 6.52 0 0 1-.22-.127c-.325-.196-.72-.257-1.076-.124l-1.217.456a1.125 1.125 0 0 1-1.369-.49l-1.297-2.247a1.125 1.125 0 0 1 .26-1.431l1.004-.827c.292-.24.437-.613.43-.991a6.932 6.932 0 0 1 0-.255c.007-.38-.138-.751-.43-.992l-1.004-.827a1.125 1.125 0 0 1-.26-1.43l1.297-2.247a1.125 1.125 0 0 1 1.37-.491l1.216.456c.356.133.751.072 1.076-.124.072-.044.146-.086.22-.128.332-.183.582-.495.644-.869l.214-1.28Z" />
            <path stroke-Linecap="round" stroke-Linejoin="round" d="M15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z" />
          </svg>
        </A>

        {/* --------------------------------中心头像---------------------------------------- */}

        <img
          src="/icons/user.png"
          alt="User Avatar" class="avatar"
        />

        {/* --------------------------------右侧项目---------------------------------------- */}
        {/* 模型选择下拉菜单 */}
        <div
          class="model-selector-wrapper"
          onMouseEnter={() => setDropdownVisible(true)}
          onMouseLeave={() => setDropdownVisible(false)}
        >
          {/* 选择模型按钮 */}
          <div class="nav-item model-selector" title="选择模型">
            <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-Width={1.5} stroke="currentColor" class="size-6">
              <path stroke-Linecap="round" stroke-Linejoin="round" d="M13.5 16.875h3.375m0 0h3.375m-3.375 0V13.5m0 3.375v3.375M6 10.5h2.25a2.25 2.25 0 0 0 2.25-2.25V6a2.25 2.25 0 0 0-2.25-2.25H6A2.25 2.25 0 0 0 3.75 6v2.25A2.25 2.25 0 0 0 6 10.5Zm0 9.75h2.25A2.25 2.25 0 0 0 10.5 18v-2.25a2.25 2.25 0 0 0-2.25-2.25H6a2.25 2.25 0 0 0-2.25 2.25V18A2.25 2.25 0 0 0 6 20.25Zm9.75-9.75H18a2.25 2.25 0 0 0 2.25-2.25V6A2.25 2.25 0 0 0 18 3.75h-2.25A2.25 2.25 0 0 0 13.5 6v2.25a2.25 2.25 0 0 0 2.25 2.25Z" />
            </svg>
          </div>

          {/* 控制下拉菜单 */}
          <div classList={{ 'dropdown-menu': true, 'active': isDropdownVisible() }}>
            <For each={datas.activatedModels}>
              {(model) => (
                <div class="dropdown-item"
                  classList={{ 'selected': selectedModel()?.model_id === model.model_id }}
                  onClick={() => handleModelSelect(model)}>
                  <div class="model-id-text">{model.model_id}</div>
                  <div class="model-provider-text">{model.owned_by}</div>
                </div>
              )}
            </For>
            {datas.activatedModels.length === 0 && <div class="dropdown-item">无激活模型</div>}
          </div>
        </div>

        {/* 设置提示词按钮 */}
        <a
          href="#"
          title="设置提示词"
          class="nav-item"
          onClick={handleOpenPromptModal}
        >
          <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-Width={1.5} stroke="currentColor" class="size-6">
            <path stroke-Linecap="round" stroke-Linejoin="round" d="M12 18v-5.25m0 0a6.01 6.01 0 0 0 1.5-.189m-1.5.189a6.01 6.01 0 0 1-1.5-.189m3.75 7.478a12.06 12.06 0 0 1-4.5 0m3.75 2.383a14.406 14.406 0 0 1-3 0M14.25 18v-.192c0-.983.658-1.823 1.508-2.316a7.5 7.5 0 1 0-7.517 0c.85.493 1.509 1.333 1.509 2.316V18" />
          </svg>
        </a>

        {/* --- 窗口控制按钮 --- */}
        <div class="window-controls">
          <button class="control-button minimize" onClick={handleMinimize} title="最小化">
            <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-Width={1.5} stroke="currentColor" class="size-6">
              <path stroke-Linecap="round" stroke-Linejoin="round" d="M5 12h14" />
            </svg>
          </button>

          <button class="control-button maximize" onClick={handleToggleMaximize} title={isMaximized() ? "还原" : "最大化"}>
            {/* 根据窗口状态切换图标 */}
            {isMaximized() ?
              <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-Width={1.5} stroke="currentColor" class="size-6">
                <path stroke-Linecap="round" stroke-Linejoin="round" d="M16.5 8.25V6a2.25 2.25 0 0 0-2.25-2.25H6A2.25 2.25 0 0 0 3.75 6v8.25A2.25 2.25 0 0 0 6 16.5h2.25m8.25-8.25H18a2.25 2.25 0 0 1 2.25 2.25V18A2.25 2.25 0 0 1 18 20.25h-7.5A2.25 2.25 0 0 1 8.25 18v-1.5m8.25-8.25h-6a2.25 2.25 0 0 0-2.25 2.25v6" />
              </svg>
              :
              <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-Width={1.5} stroke="currentColor" class="size-6">
                <path stroke-Linecap="round" stroke-Linejoin="round" d="M5.25 7.5A2.25 2.25 0 0 1 7.5 5.25h9a2.25 2.25 0 0 1 2.25 2.25v9a2.25 2.25 0 0 1-2.25 2.25h-9a2.25 2.25 0 0 1-2.25-2.25v-9Z" />
              </svg>
            }
          </button>

          <button class="control-button close" onClick={handleClose} title="关闭">
            <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-Width={1.5} stroke="currentColor" class="size-6">
              <path stroke-Linecap="round" stroke-Linejoin="round" d="M6 18 18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
      </nav>

      {/* 提示词设置浮窗组件 */}
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
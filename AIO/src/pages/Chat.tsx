import { Component, For, Show, createSignal, createEffect, onMount, onCleanup, createResource } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import './Chat.css';

interface Assistant {
  id: string;
  name: string;
  // 可以在这里添加更多属性，例如配置等
}

interface MenuState {
  isOpen: boolean;
  x: number;
  y: number;
  targetAssistantId: string | null;
}

const createAssistant = (name = 'New Assistant', id?: string): Assistant => ({
  id: id ?? Date.now().toString(),
  name,
  // 可以在这里添加更多属性，例如配置等
});

const Chat: Component = () => {
  // 使用 createSignal 定义左右面板的当前宽度（百分比）
  const [leftPanelWidth, setLeftPanelWidth] = createSignal<number>(15); // 初始左侧面板宽度
  const [rightPanelWidth, setRightPanelWidth] = createSignal<number>(15); // 初始右侧面板宽度

  //---------------------------------------------------------------

  const [assistants, setAssistants] = createSignal<Assistant[]>([]);

  const fetchAssistants = async (): Promise<Assistant[]> => {
    try {
      // 调用 Tauri 命令来读取助手数据
      const data = await invoke<any>('load_assistants');
      console.log('Loaded assistants:', data);
      // 确保返回的是数组格式
      if (Array.isArray(data)) {
        // 如果数据是从文件加载的，确保 ID 字段存在
        return data.map((item: any) => createAssistant(item.name, item.id));
      } else {
        console.warn('Loaded data is not an array, returning empty array.');
        return [];
      }
    } catch (err) {
      console.error('Failed to load assistants:', err);
      // 如果加载失败（比如第一次运行，文件不存在），返回默认助手
      return [createAssistant('Default Assistant')];
    }
  };

  // 使用 createResource 在组件挂载时自动加载数据
  const [assistantsResource] = createResource<Assistant[]>(fetchAssistants);

  // 初始化助手状态
  createEffect(() => {
    const data = assistantsResource();
    if (data && !assistantsResource.loading) {
      setAssistants(data);
    }
  });

  // 保存助手的函数
  const saveAssistants = async (assistantsToSave: Assistant[]) => {
    try {
      // 调用 Tauri 命令来保存助手数据
      await invoke('save_assistants', { assistants: assistantsToSave });
      console.log('Assistants saved successfully.');
    } catch (err) {
      console.error('Failed to save assistants:', err);
      // 可以在这里添加用户提示，比如 toast 消息
    }
  };

  // 创建一个包装 setAssistants 的函数，以便在数据改变时自动保存
  const updateAssistants = (updater: Assistant[] | ((prev: Assistant[]) => Assistant[])) => {
    setAssistants((prev) => {
      const updated = typeof updater === 'function' ? (updater as (p: Assistant[]) => Assistant[])(prev) : (updater as Assistant[]);
      // 在状态更新后，触发保存
      saveAssistants(updated);
      return updated;
    });
  };

  // 添加助手函数 (使用 updateAssistants)
  const addAssistant = () => {
    const newAssistant = createAssistant();
    updateAssistants((prev) => [...prev, newAssistant]);
  };

  // 删除助手函数 (使用 updateAssistants)
  const removeAssistant = (idToRemove: string | null) => {
    if (!idToRemove) return;
    updateAssistants((prev) => prev.filter((assistant) => assistant.id !== idToRemove));
    closeMenu(); // 关闭菜单
  };

  // 打开助手设置函数（示例）- 不需要修改
  const openAssistantSettings = (assistantId: string | null) => {
    console.log(`Opening settings for assistant ID: ${assistantId}`);
    alert(`Settings for assistant ID: ${assistantId}`);
    closeMenu();
  };

  const [showMenuDiv, setShowMenuDiv] = createSignal(false); // 控制菜单 Div 是否在 DOM 中渲染
  const [isMenuAnimatingOut, setIsMenuAnimatingOut] = createSignal(false); // 控制是否应用退出动画类

  const [menuState, setMenuState] = createSignal<MenuState>({
    isOpen: false,
    x: 0,
    y: 0,
    targetAssistantId: null, // 记录是哪个助手触发的菜单
  });

  const ANIMATION_DURATION = 200;

  // 打开菜单的函数
  const openMenu = (event: MouseEvent, assistantId: string) => {
    event.stopPropagation();

    // 如果点击的是当前已打开菜单的按钮，则关闭菜单（实现 Toggle 效果）
    if (menuState().isOpen && menuState().targetAssistantId === assistantId) {
      closeMenu();
      return;
    }

    // 如果菜单正在退出动画中，但用户点击了另一个按钮，立即清除退出动画并打开新菜单
    if (isMenuAnimatingOut()) {
      clearTimeout(menuCloseTimeoutId); // 清除可能存在的延时关闭
      setIsMenuAnimatingOut(false);
    }

    setShowMenuDiv(true); // 确保菜单 Div 已经渲染在 DOM 中

    const button = event.currentTarget as Element;
    const buttonRect = button.getBoundingClientRect();

    const x = buttonRect.left; // 视口坐标
    const y = buttonRect.top + buttonRect.height; // 在按钮下方

    setMenuState({
      isOpen: true, // 逻辑状态为打开
      x,
      y,
      targetAssistantId: assistantId,
    });
  };

  let menuCloseTimeoutId: ReturnType<typeof setTimeout> | undefined; // 用于存储 setTimeout 的 ID

  // 修改后的 closeMenu 函数
  const closeMenu = () => {
    // 只有当菜单是逻辑打开状态，或者正在退出动画中时才执行关闭逻辑
    if (!menuState().isOpen && !showMenuDiv()) return;

    setMenuState((prev) => ({ ...prev, isOpen: false })); // 立即将逻辑状态设为关闭
    setIsMenuAnimatingOut(true); // 立即应用退出动画类

    // 在动画结束后，将菜单元素从 DOM 中移除
    // 使用 setTimeout 保证动画播放完毕
    menuCloseTimeoutId = setTimeout(() => {
      setShowMenuDiv(false); // 从 DOM 中移除菜单 Div
      setIsMenuAnimatingOut(false); // 重置退出动画状态
      setMenuState((prev) => ({ ...prev, targetAssistantId: null })); // 清除目标助手ID
    }, ANIMATION_DURATION);
  };

  // 修改后的 handleClickOutside 函数
  const handleClickOutside = (event: MouseEvent) => {
    const target = event.target as HTMLElement;

    // 1. 如果点击的是菜单内部（无论动画进出），不关闭。
    //    这里的条件需要确保即使在退出动画期间点击菜单内容，也不会立即关闭，而是让退出动画完成。
    if (target.closest('.assistant-context-menu')) {
      return;
    }

    // 2. 如果点击的是触发按钮（.assistant-menu-button），忽略该事件。
    //    因为 openMenu 函数已经处理了点击按钮的逻辑（包括 toggle）。
    if (target.closest('.assistant-menu-button')) {
      return;
    }

    // 只有当菜单是逻辑打开状态，并且点击的是真正的“外部”区域时才关闭
    if (menuState().isOpen) {
      closeMenu();
    }
  };

  // 监听 document 上的点击事件，用于关闭菜单
  onMount(() => {
    document.addEventListener('click', handleClickOutside);
  });

  onCleanup(() => {
    document.removeEventListener('click', handleClickOutside);
  });

  //---------------------------------------------------------------

  let chatPageRef: HTMLDivElement | undefined; // 用于获取最外层容器（.chat-page）的引用

  // 拖拽状态变量
  let isResizingLeft = false;
  let isResizingRight = false;
  let initialMouseX = 0; // 鼠标按下时的X坐标
  let initialLeftPanelPxWidth = 0; // 鼠标按下时左侧面板的像素宽度
  let initialRightPanelPxWidth = 0; // 鼠标按下时右侧面板的像素宽度

  const MIN_PERCENT = 10; // 最小宽度百分比
  const MAX_PERCENT = 25; // 最大宽度百分比

  /**
   * 开始拖拽事件
   * @param {MouseEvent} e - 鼠标事件对象
   * @param {'left'|'right'} panelType - 正在拖拽的面板类型
   */
  const startResize = (e: MouseEvent, panelType: 'left' | 'right') => {
    e.preventDefault(); // 阻止默认的文本选择行为

    isResizingLeft = panelType === 'left';
    isResizingRight = panelType === 'right';
    initialMouseX = e.clientX;

    // 获取当前被拖拽面板的像素宽度
    if (panelType === 'left') {
      initialLeftPanelPxWidth = chatPageRef?.querySelector('.assistant-selector')?.clientWidth ?? 0;
    } else {
      // panelType === 'right'
      initialRightPanelPxWidth = chatPageRef?.querySelector('.dialog-container')?.clientWidth ?? 0;
    }

    // 在文档上添加 mousemove 和 mouseup 事件监听器，以便在鼠标移出拖拽区域时也能捕获事件
    document.addEventListener('mousemove', handleMouseMove as any);
    document.addEventListener('mouseup', stopResize as any);
    document.body.style.userSelect = 'none'; // 拖拽时禁用文本选择
    document.body.style.cursor = 'ew-resize'; // 改变鼠标图标为左右拖拽样式
  };

  /**
   * 处理鼠标移动事件（拖拽过程中）
   * @param {MouseEvent} e - 鼠标事件对象
   */
  const handleMouseMove = (e: MouseEvent) => {
    if (!isResizingLeft && !isResizingRight) return;

    const deltaX = e.clientX - initialMouseX; // 鼠标水平移动的距离
    const totalPageWidth = chatPageRef?.offsetWidth ?? 1; // 获取父容器的总宽度

    let newPercent: number;

    if (isResizingLeft) {
      const newPixelWidth = initialLeftPanelPxWidth + deltaX;
      newPercent = (newPixelWidth / totalPageWidth) * 100;
      // 限制新宽度在最小和最大百分比之间
      newPercent = Math.min(Math.max(newPercent, MIN_PERCENT), MAX_PERCENT);
      setLeftPanelWidth(newPercent);
    } else if (isResizingRight) {
      // 对于右侧面板，鼠标向左移动（deltaX为负）会增加其宽度
      // 鼠标向右移动（deltaX为正）会减少其宽度
      const newPixelWidth = initialRightPanelPxWidth - deltaX;
      newPercent = (newPixelWidth / totalPageWidth) * 100;
      // 限制新宽度在最小和最大百分比之间
      newPercent = Math.min(Math.max(newPercent, MIN_PERCENT), MAX_PERCENT);
      setRightPanelWidth(newPercent);
    }
  };

  /**
   * 停止拖拽事件
   */
  const stopResize = () => {
    isResizingLeft = false;
    isResizingRight = false;

    // 移除文档上的事件监听器
    document.removeEventListener('mousemove', handleMouseMove as any);
    document.removeEventListener('mouseup', stopResize as any);
    document.body.style.userSelect = ''; // 恢复文本选择
    document.body.style.cursor = ''; // 恢复默认鼠标图标
  };

  // 在组件卸载时清理事件监听器，防止内存泄漏
  onCleanup(() => {
    document.removeEventListener('mousemove', handleMouseMove as any);
    document.removeEventListener('mouseup', stopResize as any);
  });

  return (
    <div class="chat-page" ref={(el) => (chatPageRef = el as HTMLDivElement)}>
      {/* 左侧选择助手区域 */}
      <div
        class="assistant-selector"
        style={{ width: `${leftPanelWidth()}%` }} // 动态绑定宽度
      >
        <div class="assistant-content">
          {/* 渲染助手列表 - 添加加载状态判断 */}
          {assistantsResource.loading ? (
            <p>Loading assistants...</p> // 或者显示一个加载指示器
          ) : (
            <>
              <For each={assistants()}>
                {(assistant) => (
                  <div class="assistant-item">
                    <span>{assistant.name}</span>
                    <button
                      class="assistant-menu-button"
                      onClick={(e) => openMenu(e as unknown as MouseEvent, assistant.id)}
                      aria-haspopup="true"
                      aria-expanded={menuState().isOpen && menuState().targetAssistantId === assistant.id}
                    >
                      <svg xmlns="http://www.w3.org/2000/svg" fill="#FFFFFF" viewBox="0 0 24 24" stroke-Width={1.5} class="size-6">
                        <path stroke-Linecap="round" stroke-Linejoin="round" d="M10 12a2 2 0 1 0 4 0a2 2 0 0 0-4 0zm0-6a2 2 0 1 0 4 0a2 2 0 0 0-4 0zm0 12a2 2 0 1 0 4 0a2 2 0 0 0-4 0z" />
                      </svg>

                    </button>
                  </div>
                )}
              </For>
            </>
          )}

          {/* 添加助手按钮 - 即使在加载状态下也显示 */}
          <button
            class="add-assistant-button"
            onClick={addAssistant}
            disabled={assistantsResource.loading} // 加载时禁用
          >
            + Add Assistant
          </button>

          {/* 下拉菜单 - (保持不变) */}
          {showMenuDiv() && (
            <div
              class="assistant-context-menu"
              // 根据 isMenuAnimatingOut 状态动态添加/移除 'menu-exiting' 类
              classList={{ 'menu-exiting': isMenuAnimatingOut() }}
              style={{
                top: `${menuState().y}px`,
                left: `${menuState().x}px`,
                // 位置使用内联绑定（动态 top/left），视觉和动画样式已移入 CSS
              }}
            >
              <button
                class="context-menu-button"
                onClick={() => openAssistantSettings(menuState().targetAssistantId)}
              >
                助手设置
              </button>
              <button
                class="context-menu-button delete"
                onClick={() => removeAssistant(menuState().targetAssistantId)}
              >
                删除助手
              </button>
            </div>
          )}
        </div>
        <div class="resize-handle left-handle" onMouseDown={(e) => startResize(e as unknown as MouseEvent, 'left')}></div>
      </div>

      {/* 中间聊天区域 */}
      <div class="chat-input-container">
        <div class="chat-messages-area">
          {/* 聊天消息内容将在这里显示 */}
          Chat Messages content goes here...
          <p>Hello! How can I help you today?</p>
          <p>I'm fine, thanks for asking!</p>
          {/* 可以添加更多的聊天消息来模拟滚动 */}
        </div>
        <input type="text" class="chat-input" placeholder="Type your message..." />
      </div>

      {/* 右侧对话或相关信息区域 */}
      <div
        class="dialog-container"
        style={{ width: `${rightPanelWidth()}%` }} // 动态绑定宽度
      >
        <div class="dialog-content">
          Dialog
          {/* 你可以在这里添加对话历史、用户信息或其他相关信息 */}
        </div>
        <div class="resize-handle right-handle" onMouseDown={(e) => startResize(e as unknown as MouseEvent, 'right')}></div>
      </div>
    </div>
  );
};

export default Chat;
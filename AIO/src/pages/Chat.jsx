import { createSignal, onCleanup,createEffect, onMount, createResource } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import './Chat.css';

function Chat() {
    // 使用 createSignal 定义左右面板的当前宽度（百分比）
    const [leftPanelWidth, setLeftPanelWidth] = createSignal(15); // 初始左侧面板宽度
    const [rightPanelWidth, setRightPanelWidth] = createSignal(15); // 初始右侧面板宽度
    
//---------------------------------------------------------------

    const createAssistant = (name = "New Assistant", id = null) => ({
        id: id || Date.now().toString(), // 如果提供了 ID 就用提供的，否则生成新的
        name: name,
        // 可以在这里添加更多属性，例如配置等
    });
    const [assistants, setAssistants] = createSignal([]);
    

    const fetchAssistants = async () => {
    try {
      // 调用 Tauri 命令来读取助手数据
      const data = await invoke('load_assistants');
      console.log('Loaded assistants:', data);
      // 确保返回的是数组格式
      if (Array.isArray(data)) {
        // 如果数据是从文件加载的，确保 ID 字段存在
        return data.map(item => createAssistant(item.name, item.id));
      } else {
        console.warn('Loaded data is not an array, returning empty array.');
        return [];
      }
    } catch (err) {
      console.error('Failed to load assistants:', err);
      // 如果加载失败（比如第一次运行，文件不存在），返回默认助手
      return [createAssistant("Default Assistant")];
    }
    };

    // 使用 createResource 在组件挂载时自动加载数据
    const [assistantsResource] = createResource(fetchAssistants);

  // 初始化助手状态
    createEffect(() => {
        const data = assistantsResource();
        if (data && !assistantsResource.loading) {
        setAssistants(data);
        }
    });

  // 保存助手的函数
    const saveAssistants = async (assistantsToSave) => {
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
    const updateAssistants = (updater) => {
        setAssistants(prev => {
        const updated = typeof updater === 'function' ? updater(prev) : updater;
        // 在状态更新后，触发保存
        saveAssistants(updated);
        return updated;
        });
    };

  // 添加助手函数 (使用 updateAssistants)
    const addAssistant = () => {
        const newAssistant = createAssistant();
        updateAssistants(prev => [...prev, newAssistant]);
    };

  // 删除助手函数 (使用 updateAssistants)
    const removeAssistant = (idToRemove) => {
        updateAssistants(prev => prev.filter(assistant => assistant.id !== idToRemove));
        closeMenu(); // 关闭菜单
    };

  // 打开助手设置函数（示例）- 不需要修改
    const openAssistantSettings = (assistantId) => {
        console.log(`Opening settings for assistant ID: ${assistantId}`);
        alert(`Settings for assistant ID: ${assistantId}`);
        closeMenu();
    };

  const [menuState, setMenuState] = createSignal({
    isOpen: false,
    x: 0,
    y: 0,
    targetAssistantId: null, // 记录是哪个助手触发的菜单
  });

  // 打开菜单的函数
  const openMenu = (event, assistantId) => {
    event.stopPropagation(); // 阻止事件冒泡到 document，避免立即触发关闭

    // 获取触发按钮的位置
    const buttonRect = event.currentTarget.getBoundingClientRect();
    const pageRect = chatPageRef.getBoundingClientRect();

    // 计算相对于 .chat-page 的位置
    const x = buttonRect.left - pageRect.left;
    const y = buttonRect.top - pageRect.top + buttonRect.height; // 显示在按钮下方

    setMenuState({
      isOpen: true,
      x: x,
      y: y,
      targetAssistantId: assistantId,
    });
  };

  // 关闭菜单的函数
  const closeMenu = () => {
    setMenuState(prev => ({ ...prev, isOpen: false }));
  };

  // 点击外部区域关闭菜单的处理器
  const handleClickOutside = (event) => {
    // 检查点击的目标是否是菜单本身或者菜单的触发按钮
    // 注意：由于事件委托和 React 渲染机制，直接检查 event.target 可能不够准确。
    // 更稳妥的方式是利用 menuState.isOpen 状态结合全局点击事件。
    if (menuState().isOpen) {
        // 如果菜单是打开的，则关闭它
        // 因为我们已经在菜单项点击处理函数中调用了 closeMenu，
        // 这里的逻辑主要是为了点击空白处关闭。
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

    let chatPageRef; // 用于获取最外层容器（.chat-page）的引用
    
    // 拖拽状态变量
    let isResizingLeft = false;
    let isResizingRight = false;
    let initialMouseX; // 鼠标按下时的X坐标
    let initialLeftPanelPxWidth; // 鼠标按下时左侧面板的像素宽度
    let initialRightPanelPxWidth; // 鼠标按下时右侧面板的像素宽度

    const MIN_PERCENT = 10; // 最小宽度百分比
    const MAX_PERCENT = 25; // 最大宽度百分比

    /**
     * 开始拖拽事件
     * @param {MouseEvent} e - 鼠标事件对象
     * @param {'left'|'right'} panelType - 正在拖拽的面板类型
     */
    const startResize = (e, panelType) => {
        e.preventDefault(); // 阻止默认的文本选择行为

        isResizingLeft = (panelType === 'left');
        isResizingRight = (panelType === 'right');
        initialMouseX = e.clientX;

        // 获取当前被拖拽面板的像素宽度
        if (panelType === 'left') {
            initialLeftPanelPxWidth = chatPageRef.querySelector('.assistant-selector').offsetWidth;
        } else { // panelType === 'right'
            initialRightPanelPxWidth = chatPageRef.querySelector('.dialog-container').offsetWidth;
        }

        // 在文档上添加 mousemove 和 mouseup 事件监听器，以便在鼠标移出拖拽区域时也能捕获事件
        document.addEventListener('mousemove', handleMouseMove);
        document.addEventListener('mouseup', stopResize);
        document.body.style.userSelect = 'none'; // 拖拽时禁用文本选择
        document.body.style.cursor = 'ew-resize'; // 改变鼠标图标为左右拖拽样式
    };

    /**
     * 处理鼠标移动事件（拖拽过程中）
     * @param {MouseEvent} e - 鼠标事件对象
     */
    const handleMouseMove = (e) => {
        if (!isResizingLeft && !isResizingRight) return;

        const deltaX = e.clientX - initialMouseX; // 鼠标水平移动的距离
        const totalPageWidth = chatPageRef.offsetWidth; // 获取父容器的总宽度

        let newPercent;

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
        document.removeEventListener('mousemove', handleMouseMove);
        document.removeEventListener('mouseup', stopResize);
        document.body.style.userSelect = ''; // 恢复文本选择
        document.body.style.cursor = ''; // 恢复默认鼠标图标
    };

    // 在组件卸载时清理事件监听器，防止内存泄漏
    onCleanup(() => {
        document.removeEventListener('mousemove', handleMouseMove);
        document.removeEventListener('mouseup', stopResize);
    });

    return (
        <div class="chat-page" ref={chatPageRef}>
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
                       onClick={(e) => openMenu(e, assistant.id)}
                       aria-haspopup="true"
                       aria-expanded={menuState().isOpen && menuState().targetAssistantId === assistant.id}
                     >
                       ...
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
          {menuState().isOpen && (
            <div
              class="assistant-context-menu"
              style={{
                position: 'absolute',
                top: `${menuState().y}px`,
                left: `${menuState().x}px`,
                'background-color': '#2e2e2e',
                border: '1px solid #08ddf9',
                'border-radius': '4px',
                'box-shadow': '0 2px 5px rgba(0,0,0,0.2)',
                'z-index': 100,
                'min-width': '150px',
              }}
            >
              <button
                onClick={() => openAssistantSettings(menuState().targetAssistantId)}
                style={{
                  display: 'block',
                  width: '100%',
                  padding: '8px 12px',
                  background: 'none',
                  border: 'none',
                  color: '#FFFFFF',
                  'text-align': 'left',
                  cursor: 'pointer',
                }}
                onMouseEnter={(e) => e.target.style['background-color'] = '#3a3a3a'}
                onMouseLeave={(e) => e.target.style['background-color'] = 'transparent'}
              >
                助手设置
              </button>
              <button
                onClick={() => removeAssistant(menuState().targetAssistantId)}
                style={{
                  display: 'block',
                  width: '100%',
                  padding: '8px 12px',
                  background: 'none',
                  border: 'none',
                  color: '#ff4d4d',
                  'text-align': 'left',
                  cursor: 'pointer',
                }}
                onMouseEnter={(e) => e.target.style['background-color'] = '#3a3a3a'}
                onMouseLeave={(e) => e.target.style['background-color'] = 'transparent'}
              >
                删除助手
              </button>
            </div>
          )}


                </div>
                <div
                    class="resize-handle left-handle"
                    onMouseDown={(e) => startResize(e, 'left')} // 绑定拖拽开始事件
                ></div>
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
                <div
                    class="resize-handle right-handle"
                    onMouseDown={(e) => startResize(e, 'right')} // 绑定拖拽开始事件
                ></div>
            </div>
        </div>
    );
}

export default Chat;
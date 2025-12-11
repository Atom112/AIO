import { createSignal, onCleanup } from 'solid-js';
import './Chat.css';

function Chat() {
    // 使用 createSignal 定义左右面板的当前宽度（百分比）
    const [leftPanelWidth, setLeftPanelWidth] = createSignal(15); // 初始左侧面板宽度
    const [rightPanelWidth, setRightPanelWidth] = createSignal(15); // 初始右侧面板宽度

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
                    Choose Assistant
                    {/* 你可以在这里添加助手列表或选择器 */}
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
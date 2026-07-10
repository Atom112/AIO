/**
 * 联网搜索开关按钮
 *
 * 位于输入框工具栏中 ReasoningButton 右侧，点击切换 web_fetch / web_search 工具。
 * 开启后即使在对话模式下也能使用内置 Web 工具获取最新网络信息。
 */
import { Component } from 'solid-js';
import { webSearchEnabled, persistWebSearch } from '../../../core/store/store';
import Icon from '../../../shared/components/Icon';

const WebSearchButton: Component = () => {
    const isActive = () => webSearchEnabled();

    const toggle = () => {
        persistWebSearch(!webSearchEnabled());
    };

    const color = () => isActive() ? 'var(--primary-color)' : 'rgba(255,255,255,0.4)';

    return (
        <button
            type="button"
            class="reasoning-trigger"
            title={isActive() ? '关闭联网搜索' : '开启联网搜索'}
            onClick={toggle}
            style={{ color: color() }}
        >
            <Icon name="globe" size={15} class="reasoning-trigger-icon" />
            <span class="reasoning-trigger-label">联网</span>
        </button>
    );
};

export default WebSearchButton;

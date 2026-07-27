/**
 * 联网搜索开关按钮
 *
 * 位于输入框工具栏中 ReasoningButton 右侧，点击切换 web_fetch / web_search 工具。
 * 开启后即使在对话模式下也能使用内置 Web 工具获取最新网络信息。
 */
import { Component } from 'solid-js';
import { webSearchEnabled, persistWebSearch } from '../../../core/store/store';
import Icon from '../../../shared/components/Icon';
import { t } from '../../../core/i18n';

const WebSearchButton: Component = () => {
  const isActive = () => webSearchEnabled();

  const toggle = () => {
    persistWebSearch(!webSearchEnabled());
  };

  const color = () => (isActive() ? 'var(--primary-color)' : 'rgba(var(--text-base-rgb),0.4)');

  return (
    <button
      type="button"
      class="flex items-center gap-1.5 px-2.5 h-8 rounded-md border-none cursor-pointer transition-all duration-200 select-none bg-transparent text-xs font-medium hover:bg-white/[0.06] hover:text-[#7c9abf]/60"
      title={isActive() ? t('chat.webSearchOff') : t('chat.webSearchOn')}
      onClick={toggle}
      style={{ color: color() }}
    >
      <Icon name="globe" size={15} class="flex items-center justify-center shrink-0" />
      <span class="leading-none">{t('chat.web')}</span>
    </button>
  );
};

export default WebSearchButton;

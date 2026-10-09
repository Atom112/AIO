/**
 * Web search toggle button
 *
 * Sits right of ReasoningButton in the input toolbar; clicking toggles the web_fetch / web_search tools.
 * When enabled, the built-in web tools can fetch fresh online info even in plain chat mode.
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
      class="flex items-center gap-1.5 px-2.5 h-8 rounded-md border-none cursor-pointer transition-all duration-200 select-none bg-transparent text-xs font-medium hover:bg-white/[0.06] hover:text-[var(--color-pri)]/60"
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

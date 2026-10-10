import { Component, Show } from 'solid-js';
import Markdown from '../../../shared/components/Markdown';
import Icon from '../../../shared/components/Icon';
import { btnIcon } from '../../../shared/components/buttonStyles';
import { t } from '../../../core/i18n';

export interface BtwOverlayItem {
  id: string;
  question: string;
  answer: string | null;
  loading: boolean;
}

interface BtwResponseBoxProps {
  item: BtwOverlayItem;
  onDismiss: (id: string) => void;
}

const BtwResponseBox: Component<BtwResponseBoxProps> = (props) => {
  return (
    <div
      class="rounded-lg overflow-hidden w-[85%] self-center my-2"
      style={{
        border: '1px solid var(--border-dim)',
        background: 'rgba(200,180,255,0.03)',
      }}
    >
      <div
        class="flex items-center gap-2 px-3 py-2"
        style={{
          'border-bottom': '1px solid var(--border-dim)',
          color: 'rgba(var(--text-base-rgb),0.5)',
          'font-size': '13px',
        }}
      >
        <span
          class="font-medium px-1.5 py-px rounded text-[12px]"
          style={{
            background: 'rgba(var(--primary-rgb),0.12)',
            color: 'rgba(var(--primary-rgb),0.8)',
            border: '1px solid rgba(var(--primary-rgb),0.15)',
          }}
        >
          {t('chat.btw.label')}
        </span>
        <span class="flex-1" style={{ color: 'rgba(var(--text-base-rgb),0.6)' }}>
          {props.item.question || t('chat.btw.empty')}
        </span>
        <button
          onClick={() => props.onDismiss(props.item.id)}
          class={'w-5 h-5 text-white/40 hover:text-white hover:bg-white/10 ' + btnIcon}
          title={t('chat.btw.close')}
        >
          <Icon name="x" class="w-3 h-3" />
        </button>
      </div>

      <div class="px-3 py-2 text-[14px] leading-relaxed text-white/85">
        <Show
          when={props.item.answer}
          fallback={
            <Show
              when={props.item.loading}
              fallback={
                <span style={{ color: 'rgba(var(--text-base-rgb),0.3)', 'font-style': 'italic' }}>
                  {t('chat.btw.awaitingAnswer')}
                </span>
              }
            >
              <span style={{ color: 'rgba(var(--text-base-rgb),0.4)' }} class="animate-pulse">
                {t('chat.btw.thinking')}
              </span>
            </Show>
          }
        >
          <Markdown content={props.item.answer!} />
        </Show>
      </div>
    </div>
  );
};

export default BtwResponseBox;

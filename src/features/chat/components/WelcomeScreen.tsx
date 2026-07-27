import { Component, For } from 'solid-js';
import Icon, { type IconName } from '../../../shared/components/Icon';
import { t, type TranslationKey } from '../../../core/i18n';

interface WelcomeScreenProps {
  onSuggestionClick: (text: string) => void;
}

interface Suggestion {
  icon: IconName;
  textKey: TranslationKey;
  promptKey: TranslationKey;
}

const suggestions: Suggestion[] = [
  {
    icon: 'lightbulb',
    textKey: 'chat.welcome.code',
    promptKey: 'chat.welcome.codePrompt',
  },
  {
    icon: 'globe',
    textKey: 'chat.welcome.translate',
    promptKey: 'chat.welcome.translatePrompt',
  },
  {
    icon: 'book',
    textKey: 'chat.welcome.explain',
    promptKey: 'chat.welcome.explainPrompt',
  },
  {
    icon: 'sparkles',
    textKey: 'chat.welcome.brainstorm',
    promptKey: 'chat.welcome.brainstormPrompt',
  },
];

const WelcomeScreen: Component<WelcomeScreenProps> = (props) => {
  return (
    <div class="flex flex-col items-center justify-center w-full pointer-events-auto select-none">
      {/* Logo with floating animation */}
      <div class="animate-welcome-logo-float mb-6">
        <img
          src="/icons/app-logo/logo.svg"
          alt="AIO"
          class="w-20 h-20 drop-shadow-lg"
          style={{
            filter:
              'drop-shadow(0 0 24px rgba(0,179,255,0.25)) drop-shadow(0 0 8px rgba(0,255,255,0.15))',
          }}
        />
      </div>

      {/* Greeting text */}
      <div class="animate-welcome-text-fade-in text-center mb-8">
        <h1
          class="text-2xl font-bold mb-2 tracking-wide"
          style={{ color: 'rgba(var(--text-base-rgb),0.9)' }}
        >
          {t('chat.welcome.title')}
        </h1>
        <p class="text-sm" style={{ color: 'rgba(var(--text-base-rgb),0.45)' }}>
          {t('chat.welcome.subtitle')}
        </p>
      </div>

      {/* Suggestion cards */}
      <div class="grid grid-cols-2 gap-3 w-full max-w-[420px] px-4">
        <For each={suggestions}>
          {(item, index) => (
            <button
              class="animate-welcome-card-in flex items-center gap-2.5 px-4 py-3 rounded-xl cursor-pointer border-none text-left transition-all duration-200"
              style={{
                'animation-delay': `${0.15 + index() * 0.08}s`,
                background: 'rgba(var(--text-base-rgb),0.03)',
                border: '1px solid var(--border-dim)',
              }}
              onMouseEnter={(e) => {
                e.currentTarget.style.background = 'rgba(var(--text-base-rgb),0.06)';
                e.currentTarget.style.borderColor = 'rgba(var(--primary-rgb),0.25)';
                e.currentTarget.style.transform = 'translateY(-1px)';
              }}
              onMouseLeave={(e) => {
                e.currentTarget.style.background = 'rgba(var(--text-base-rgb),0.03)';
                e.currentTarget.style.borderColor = 'var(--border-dim)';
                e.currentTarget.style.transform = 'translateY(0)';
              }}
              onClick={() => props.onSuggestionClick(t(item.promptKey))}
            >
              <Icon name={item.icon} size={14} />
              <span
                class="text-[13px] font-medium whitespace-nowrap overflow-hidden text-ellipsis"
                style={{ color: 'rgba(var(--text-base-rgb),0.75)' }}
              >
                {t(item.textKey)}
              </span>
            </button>
          )}
        </For>
      </div>
    </div>
  );
};

export default WelcomeScreen;

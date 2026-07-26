import { Component, For } from 'solid-js';
import Icon, { type IconName } from '../../../shared/components/Icon';

interface WelcomeScreenProps {
    onSuggestionClick: (text: string) => void;
}

interface Suggestion {
    icon: IconName;
    text: string;
    prompt: string;
}

const suggestions: Suggestion[] = [
    {
        icon: 'lightbulb',
        text: '帮我写一段代码',
        prompt: '帮我写一段代码实现',
    },
    {
        icon: 'globe',
        text: '翻译文本内容',
        prompt: '请帮我把以下文本翻译成英文：',
    },
    {
        icon: 'book',
        text: '解释一个概念',
        prompt: '请帮我解释一下什么是',
    },
    {
        icon: 'sparkles',
        text: '生成创意想法',
        prompt: '请帮我头脑风暴一些关于',
    },
];

const WelcomeScreen: Component<WelcomeScreenProps> = (props) => {
    return (
        <div
            class="flex flex-col items-center justify-center w-full pointer-events-auto select-none"
        >
            {/* Logo with floating animation */}
            <div class="animate-welcome-logo-float mb-6">
                <img
                    src="/icons/app-logo/logo.svg"
                    alt="AIO"
                    class="w-20 h-20 drop-shadow-lg"
                    style="filter: drop-shadow(0 0 24px rgba(0,179,255,0.25)) drop-shadow(0 0 8px rgba(0,255,255,0.15));"
                />
            </div>

            {/* Greeting text */}
            <div class="animate-welcome-text-fade-in text-center mb-8">
                <h1
                    class="text-2xl font-bold mb-2 tracking-wide"
                    style="color: rgba(255,255,255,0.9);"
                >
                    我是 AIO
                </h1>
                <p
                    class="text-sm"
                    style="color: rgba(255,255,255,0.45);"
                >
                    有什么可以帮你的？
                </p>
            </div>

            {/* Suggestion cards */}
            <div class="grid grid-cols-2 gap-3 w-full max-w-[420px] px-4">
                <For each={suggestions}>
                    {(item, index) => (
                        <button
                            class="animate-welcome-card-in flex items-center gap-2.5 px-4 py-3 rounded-xl cursor-pointer border-none text-left transition-all duration-200"
                            style={`animation-delay: ${0.15 + index() * 0.08}s; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,255,255,0.05);`}
                            onMouseEnter={(e) => {
                                e.currentTarget.style.background = 'rgba(255,255,255,0.06)';
                                e.currentTarget.style.borderColor = 'rgba(124,154,191,0.25)';
                                e.currentTarget.style.transform = 'translateY(-1px)';
                            }}
                            onMouseLeave={(e) => {
                                e.currentTarget.style.background = 'rgba(255,255,255,0.03)';
                                e.currentTarget.style.borderColor = 'rgba(255,255,255,0.05)';
                                e.currentTarget.style.transform = 'translateY(0)';
                            }}
                            onClick={() => props.onSuggestionClick(item.prompt)}
                        >
                            <Icon name={item.icon} size={14} />
                            <span
                                class="text-[13px] font-medium whitespace-nowrap overflow-hidden text-ellipsis"
                                style="color: rgba(255,255,255,0.75);"
                            >
                                {item.text}
                            </span>
                        </button>
                    )}
                </For>
            </div>
        </div>
    );
};

export default WelcomeScreen;

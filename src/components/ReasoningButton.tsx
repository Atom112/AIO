/**
 * 推理强度按钮 (LobeHub 风格)
 *
 * 位于聊天输入框上传文件按钮左侧, 点击展开弹窗选择推理等级.
 * 弹窗向上弹出 (避免被聊天区遮挡), 包含 4 个等级选项 (关/低/中/高).
 */
import { Component, For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import { reasoningLevel, persistReasoningLevel, type ReasoningLevel } from '../store/store';
import Icon from './Icon';

interface LevelOption {
    value: ReasoningLevel;
    label: string;
    desc: string;
    iconName: 'bolt' | 'sparkles' | 'brain' | 'lightbulb';
}

const LEVELS: LevelOption[] = [
    { value: 'off', label: '关闭', desc: '不使用推理', iconName: 'bolt' },
    { value: 'low', label: '轻度', desc: '简单问题快速回答', iconName: 'sparkles' },
    { value: 'medium', label: '中度', desc: '平衡思考深度', iconName: 'lightbulb' },
    { value: 'high', label: '深度', desc: '深度分析复杂问题', iconName: 'brain' },
];

const ReasoningButton: Component = () => {
    const [open, setOpen] = createSignal(false);
    let containerRef: HTMLDivElement | undefined;
    const current = () => LEVELS.find(l => l.value === reasoningLevel()) || LEVELS[0];

    /** 当前等级不是 off 时, 按钮高亮显示 */
    const isActive = () => reasoningLevel() !== 'off';

    /** 点击外部关闭 */
    const onDocClick = (e: MouseEvent) => {
        if (!containerRef) return;
        if (!containerRef.contains(e.target as Node)) {
            setOpen(false);
        }
    };

    onMount(() => document.addEventListener('mousedown', onDocClick));
    onCleanup(() => document.removeEventListener('mousedown', onDocClick));

    const choose = (lvl: ReasoningLevel) => {
        persistReasoningLevel(lvl);
        setOpen(false);
    };

    return (
        <div ref={containerRef} class="relative inline-block">
            <button
                type="button"
                class="reasoning-trigger"
                classList={{ 'is-active': isActive() }}
                title="推理强度"
                onClick={(e) => { e.stopPropagation(); setOpen(!open()); }}
            >
                <Icon name={current().iconName} size={15} class="reasoning-trigger-icon" />
                <span class="reasoning-trigger-label">{current().label}</span>
            </button>

            <Show when={open()}>
                <div class="reasoning-popup" onClick={(e) => e.stopPropagation()}>
                    <div class="reasoning-popup-header">
                        <div class="reasoning-popup-title">
                            <Icon name="brain" size={14} />
                            <span>推理强度</span>
                        </div>
                        <div class="reasoning-popup-sub">控制模型是否以及如何深入思考后再回答</div>
                    </div>
                    <div class="reasoning-options">
                        <For each={LEVELS}>
                            {(opt) => (
                                <button
                                    type="button"
                                    class="reasoning-option"
                                    classList={{ 'is-selected': reasoningLevel() === opt.value }}
                                    onClick={() => choose(opt.value)}
                                >
                                    <span class="reasoning-option-icon">
                                        <Icon name={opt.iconName} size={14} />
                                    </span>
                                    <span class="reasoning-option-text">
                                        <span class="reasoning-option-label">{opt.label}</span>
                                        <span class="reasoning-option-desc">{opt.desc}</span>
                                    </span>
                                    <Show when={reasoningLevel() === opt.value}>
                                        <Icon name="check" size={13} class="reasoning-option-check" />
                                    </Show>
                                </button>
                            )}
                        </For>
                    </div>
                </div>
            </Show>
        </div>
    );
};

export default ReasoningButton;

/**
 * 思考过程块 (reasoning / chain-of-thought)
 *
 * 渲染模型输出的 <think>...</think> 思考内容。
 * - 流式生成中: 自动展开，header 显示"正在思考" + 动画点
 * - 生成完成: 默认折叠，header 显示"已思考 X.Xs"
 * - 亚克力主题: 微妙透明背景 + 斜体文本 + 软边框
 */
import { Component, createSignal, Show, onCleanup, createEffect } from 'solid-js';
import Icon from './Icon';

interface ThinkBlockProps {
    /** 思考过程纯文本 (不含 <think> 标签) */
    content: string;
    /** 是否仍在流式生成 */
    isStreaming?: boolean;
}

const ThinkBlock: Component<ThinkBlockProps> = (props) => {
    const [userExpanded, setUserExpanded] = createSignal<boolean | null>(null);

    /** 流式时强制展开，完成后允许用户控制 */
    const isExpanded = () => {
        if (props.isStreaming) return true;
        return userExpanded() ?? false;
    };

    const [elapsedMs, setElapsedMs] = createSignal(0);
    let startTs = 0;
    let rafId: number | null = null;

    createEffect(() => {
        if (props.isStreaming) {
            startTs = performance.now();
            const tick = () => {
                if (!props.isStreaming) return;
                setElapsedMs(performance.now() - startTs);
                rafId = requestAnimationFrame(tick);
            };
            rafId = requestAnimationFrame(tick);
        } else {
            if (rafId) cancelAnimationFrame(rafId);
            if (startTs) setElapsedMs(performance.now() - startTs);
        }
    });

    onCleanup(() => { if (rafId) cancelAnimationFrame(rafId); });

    const formatDuration = (ms: number) => {
        if (ms < 50) return '0.1s';
        return `${(ms / 1000).toFixed(1)}s`;
    };

    const toggle = () => {
        if (props.isStreaming) return;
        setUserExpanded(p => !(p ?? false));
    };

    return (
        <div
            class="think-block"
            classList={{ 'is-open': isExpanded(), 'is-streaming': props.isStreaming }}
        >
            <button
                type="button"
                class="think-header"
                onClick={toggle}
                aria-expanded={isExpanded()}
            >
                <span class="think-icon-wrap">
                    <Icon name={props.isStreaming ? 'sparkles' : 'brain'} size={13} class="think-icon" />
                </span>
                <span class="think-title">
                    {props.isStreaming ? '正在思考' : '已思考'}
                </span>
                <span class="think-duration">{formatDuration(elapsedMs())}</span>
                <span class="think-chevron" aria-hidden="true">
                    <Icon name="arrow-left" size={11} class="think-chevron-icon" />
                </span>
            </button>
            <div class="think-body">
                <div class="think-content">{props.content}</div>
            </div>
        </div>
    );
};

export default ThinkBlock;

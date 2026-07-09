/**
 * Agent 工作过程折叠块
 *
 * 将单个 Agent 轮次内的 reasoning + toolCalls 统一包裹。
 * - 工作中: 自动展开，header 显示"正在工作..." + 实时计时
 * - 完成后: 默认折叠，header 显示"已工作 X.Xs"
 * - 用户可随时点击 header 展开/折叠查看完整过程
 */
import { Component, createSignal, Show, For, onCleanup, createEffect } from 'solid-js';
import type { ToolCallDisplay } from '../store/store';
import Icon from './Icon';
import ToolCallBubble from './ToolCallBubble';
import Markdown from './Markdown';

interface AgentProcessBlockProps {
    /** 模型原生思维链文本 */
    reasoning?: string;
    /** 该轮次产生的工具调用列表 */
    toolCalls?: ToolCallDisplay[];
    /** 中间轮次的阶段性总结文本累积 */
    interimContent?: string;
    /** 是否仍在工作中（当前轮次未结束） */
    isActive: boolean;
    /** Agent 轮次开始时间戳（ms） */
    startTime?: number;
}

const AgentProcessBlock: Component<AgentProcessBlockProps> = (props) => {
    const [userExpanded, setUserExpanded] = createSignal<boolean | null>(null);

    /** 工作中强制展开，完成后允许用户控制 */
    const isExpanded = () => {
        if (props.isActive) return true;
        return userExpanded() ?? false;
    };

    const [elapsedMs, setElapsedMs] = createSignal(0);
    let rafId: number | null = null;

    createEffect(() => {
        if (props.isActive && props.startTime) {
            const tick = () => {
                if (!props.isActive) return;
                setElapsedMs(Date.now() - props.startTime!);
                rafId = requestAnimationFrame(tick);
            };
            rafId = requestAnimationFrame(tick);
        } else if (!props.isActive && props.startTime) {
            // 完成后定格最终耗时
            if (rafId) cancelAnimationFrame(rafId);
            setElapsedMs(Date.now() - props.startTime);
        } else {
            if (rafId) cancelAnimationFrame(rafId);
        }
    });

    onCleanup(() => { if (rafId) cancelAnimationFrame(rafId); });

    const formatDuration = (ms: number) => {
        if (ms < 50) return '0.1s';
        const seconds = ms / 1000;
        if (seconds < 60) return `${seconds.toFixed(1)}s`;
        const mins = Math.floor(seconds / 60);
        const secs = Math.floor(seconds % 60);
        return `${mins}分 ${secs}秒`;
    };

    const toggle = () => {
        if (props.isActive) return;
        setUserExpanded(p => !(p ?? false));
    };

    const hasContent = () => !!(props.reasoning?.trim()) || !!(props.interimContent?.trim()) || !!(props.toolCalls && props.toolCalls.length > 0);

    return (
        <Show when={hasContent()}>
            <div
                class="agent-process-block"
                classList={{ 'is-open': isExpanded(), 'is-active': props.isActive }}
            >
                <button
                    type="button"
                    class="agent-process-header"
                    onClick={toggle}
                    aria-expanded={isExpanded()}
                >
                    <span class="agent-process-icon-wrap">
                        <Icon
                            name={props.isActive ? 'sparkles' : 'brain'}
                            size={13}
                            class="agent-process-icon"
                        />
                    </span>
                    <span class="agent-process-title">
                        {props.isActive ? '正在工作' : '已工作'}
                    </span>
                    <span class="agent-process-duration">
                        {props.startTime ? formatDuration(elapsedMs()) : ''}
                    </span>
                    <span class="agent-process-chevron" aria-hidden="true">
                        <Icon name="arrow-left" size={11} class="agent-process-chevron-icon" />
                    </span>
                </button>

                <div class="agent-process-body">
                    {/* reasoning 直接渲染，不再嵌套 ThinkBlock 避免双重 header */}
                    <Show when={props.reasoning && props.reasoning.trim()}>
                        <div class="agent-process-reasoning">
                            <div class="agent-process-reasoning-label">
                                <Icon name="brain" size={11} class="mr-1.5 opacity-60" />
                                思考过程
                            </div>
                            <div class="agent-process-reasoning-content">
                                {props.reasoning}
                            </div>
                        </div>
                    </Show>

                    {/* 中间轮次的阶段性总结文本 */}
                    <Show when={props.interimContent && props.interimContent.trim()}>
                        <div class="agent-process-interim">
                            <div class="agent-process-interim-label">
                                <Icon name="message-dots" size={11} class="mr-1.5 opacity-60" />
                                工作过程
                            </div>
                            <div class="agent-process-interim-content">
                                <Markdown content={props.interimContent} />
                            </div>
                        </div>
                    </Show>

                    {/* toolCalls 复用现有 ToolCallBubble */}
                    <Show when={props.toolCalls && props.toolCalls.length > 0}>
                        <div class="agent-process-toolcalls">
                            <For each={props.toolCalls}>
                                {(tc) => (
                                    <ToolCallBubble
                                        toolCall={tc}
                                        state={tc.state ?? 'calling'}
                                        result={tc.result}
                                        error={tc.error}
                                    />
                                )}
                            </For>
                        </div>
                    </Show>
                </div>
            </div>
        </Show>
    );
};

export default AgentProcessBlock;

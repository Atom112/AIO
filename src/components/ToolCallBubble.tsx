import { Component, createSignal, Show, For } from 'solid-js';
import type { ToolCall, ToolResultContent } from '../types/mcp';
import Icon from './Icon';

interface Props {
    toolCall: ToolCall;
    /** 当前状态：'calling' = 工具调用中；'success' = 已返回结果；'error' = 失败 */
    state: 'calling' | 'success' | 'error';
    /** 工具执行结果（success 时） */
    result?: ToolResultContent[];
    /** 错误信息（error 时） */
    error?: string;
    /** 工具名（用于显示） */
    serverName?: string;
}

const ToolCallBubble: Component<Props> = (props) => {
    const [expanded, setExpanded] = createSignal(false);
    const tc = () => props.toolCall;
    const argsObj = () => {
        try { return JSON.parse(tc().function.arguments || '{}'); }
        catch { return tc().function.arguments; }
    };

    return (
        <div
            class="my-2 rounded-lg overflow-hidden"
            style="background: rgba(255,255,255,0.04); border: 1px solid rgba(255,255,255,0.08);"
        >
            <div
                class="flex items-center gap-2 px-3 py-2 cursor-pointer select-none"
                onClick={() => setExpanded(!expanded())}
            >
                <Icon src="/icons/app-logo/wrench.svg" class="w-4 h-4" />
                <span class="text-sm font-medium flex-1 min-w-0 truncate" style="color: rgba(255,255,255,0.85);">
                    {tc().function.name}
                </span>
                <Show when={props.serverName}>
                    <span class="text-xs px-1.5 py-0.5 rounded" style="background: rgba(124,154,191,0.15); color: rgba(124,154,191,0.9);">
                        {props.serverName}
                    </span>
                </Show>
                <span
                    class="text-[10px] uppercase tracking-wide px-1.5 py-0.5 rounded"
                    style={
                        props.state === 'calling'
                            ? "background: rgba(224,192,96,0.15); color: #e0c060;"
                            : props.state === 'error'
                            ? "background: rgba(255,107,107,0.15); color: #ff8a8a;"
                            : "background: rgba(124,217,160,0.15); color: #7cd9a0;"
                    }
                >
                    {props.state === 'calling' ? '执行中…' : props.state === 'error' ? '失败' : '完成'}
                </span>
                <span
                    class="text-xs transition-transform"
                    style={`color: rgba(255,255,255,0.5); transform: ${expanded() ? 'rotate(90deg)' : 'rotate(0deg)'};`}
                >▶</span>
            </div>
            <Show when={expanded()}>
                <div class="px-3 py-2 text-xs flex flex-col gap-2" style="background: rgba(0,0,0,0.2); border-top: 1px solid rgba(255,255,255,0.05);">
                    <div>
                        <div class="font-semibold mb-1" style="color: rgba(255,255,255,0.6);">参数：</div>
                        <pre
                            class="px-2 py-1.5 rounded overflow-x-auto whitespace-pre-wrap break-all"
                            style="background: rgba(0,0,0,0.3); color: rgba(255,255,255,0.85); font-size: 11px;"
                        >{JSON.stringify(argsObj(), null, 2)}</pre>
                    </div>
                    <Show when={props.state === 'success' && props.result && props.result.length > 0}>
                        <div>
                            <div class="font-semibold mb-1" style="color: rgba(255,255,255,0.6);">结果：</div>
                            <div class="flex flex-col gap-1">
                                <For each={props.result}>
                                    {(c) => (
                                        <Show when={c.type === 'text'}>
                                            <pre
                                                class="px-2 py-1.5 rounded overflow-x-auto whitespace-pre-wrap break-all"
                                                style="background: rgba(0,0,0,0.3); color: rgba(255,255,255,0.85); font-size: 11px;"
                                            >{typeof (c as any).text === 'string' ? (c as any).text : JSON.stringify(c, null, 2)}</pre>
                                        </Show>
                                    )}
                                </For>
                                <For each={props.result}>
                                    {(c) => (
                                        <Show when={c.type === 'image'}>
                                            <span style="color: rgba(255,255,255,0.5);">[图像]</span>
                                        </Show>
                                    )}
                                </For>
                                <For each={props.result}>
                                    {(c) => (
                                        <Show when={c.type === 'resource'}>
                                            <span style="color: rgba(255,255,255,0.5);">[资源]</span>
                                        </Show>
                                    )}
                                </For>
                            </div>
                        </div>
                    </Show>
                    <Show when={props.state === 'error'}>
                        <div>
                            <div class="font-semibold mb-1" style="color: rgba(255,107,107,0.9);">错误：</div>
                            <pre
                                class="px-2 py-1.5 rounded overflow-x-auto whitespace-pre-wrap break-all"
                                style="background: rgba(255,77,77,0.1); color: #ff8a8a; font-size: 11px;"
                            >{props.error}</pre>
                        </div>
                    </Show>
                </div>
            </Show>
        </div>
    );
};

export default ToolCallBubble;

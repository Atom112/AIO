import { Component, createSignal, Show, For } from 'solid-js';
import type { ToolCall, ToolResultContent } from '../../../core/types/mcp';
import Icon from '../../../shared/components/Icon';

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
            class="mb-1.5 rounded-md overflow-hidden text-xs animate-expand-width"
            style="border: 1px solid rgba(255,255,255,0.06);"
        >
            {/* 更紧凑的头部：无衬底背景，仅 hover 时交互 */}
            <div
                class="flex items-center gap-1.5 px-2 py-1 cursor-pointer select-none rounded-md transition-colors duration-150"
                style="background: rgba(255,255,255,0.03);"
                classList={{ 'rounded-b-none': expanded() }}
                onClick={() => setExpanded(!expanded())}
            >
                <span
                    class="flex items-center justify-center w-4 h-4 rounded-full flex-shrink-0"
                    style={
                        props.state === 'calling'
                            ? "background: rgba(224,192,96,0.15);"
                            : props.state === 'error'
                            ? "background: rgba(255,107,107,0.15);"
                            : "background: rgba(124,217,160,0.15);"
                    }
                >
                    <Show when={props.state === 'calling'} fallback={
                        <Show when={props.state === 'error'} fallback={
                            <span style="color: #7cd9a0; font-size: 10px;">✓</span>
                        }>
                            <Icon src="/icons/app-logo/close-x.svg" class="w-2.5 h-2.5" style="color: #ff8a8a;" />
                        </Show>
                    }>
                        <Icon src="/icons/app-logo/loading.svg" class="w-2.5 h-2.5 animate-spin" style="color: #e0c060;" />
                    </Show>
                </span>
                <span class="font-medium truncate min-w-0" style="color: rgba(255,255,255,0.65); font-size: 11px;">
                    {tc().function.name}
                </span>
                <Show when={props.serverName}>
                    <span class="text-[10px] px-1 py-[1px] rounded flex-shrink-0" style="background: rgba(124,154,191,0.1); color: rgba(124,154,191,0.7);">
                        {props.serverName}
                    </span>
                </Show>
                <span class="ml-auto flex-shrink-0" style="color: rgba(255,255,255,0.25); font-size: 9px; letter-spacing: 0.5px;">
                    {props.state === 'calling' ? '…' : props.state === 'error' ? '✗' : '✓'}
                </span>
            </div>

            {/* 展开详情 */}
            <Show when={expanded()}>
                <div class="px-2 py-1.5 flex flex-col gap-1.5 text-[11px]" style="background: rgba(0,0,0,0.15);">
                    <Show when={tc().function.arguments && tc().function.arguments !== '{}'}>
                        <div>
                            <div class="mb-0.5" style="color: rgba(255,255,255,0.35);">参数</div>
                            <pre
                                class="px-1.5 py-1 rounded overflow-x-auto whitespace-pre-wrap break-all"
                                style="background: rgba(0,0,0,0.2); color: rgba(255,255,255,0.55); font-size: 10px; line-height: 1.4;"
                            >{JSON.stringify(argsObj(), null, 2)}</pre>
                        </div>
                    </Show>
                    <Show when={props.state === 'success' && props.result && props.result.length > 0}>
                        <div>
                            <div class="mb-0.5" style="color: rgba(255,255,255,0.35);">结果</div>
                            <div class="flex flex-col gap-0.5">
                                <For each={props.result}>
                                    {(c) => (
                                        <Show when={c.type === 'text'}>
                                            <pre
                                                class="px-1.5 py-1 rounded overflow-x-auto whitespace-pre-wrap break-all"
                                                style="background: rgba(0,0,0,0.2); color: rgba(255,255,255,0.55); font-size: 10px; line-height: 1.4; max-height: 120px; overflow-y: auto;"
                                            >{typeof (c as any).text === 'string' ? (c as any).text : JSON.stringify(c, null, 2)}</pre>
                                        </Show>
                                    )}
                                </For>
                                <For each={props.result}>
                                    {(c) => (
                                        <Show when={c.type === 'image'}>
                                            <span style="color: rgba(255,255,255,0.4); font-size: 10px;">[图像]</span>
                                        </Show>
                                    )}
                                </For>
                                <For each={props.result}>
                                    {(c) => (
                                        <Show when={c.type === 'resource'}>
                                            <span style="color: rgba(255,255,255,0.4); font-size: 10px;">[资源]</span>
                                        </Show>
                                    )}
                                </For>
                            </div>
                        </div>
                    </Show>
                    <Show when={props.state === 'error'}>
                        <div>
                            <div class="mb-0.5" style="color: rgba(255,107,107,0.7);">错误</div>
                            <pre
                                class="px-1.5 py-1 rounded overflow-x-auto whitespace-pre-wrap break-all"
                                style="background: rgba(255,77,77,0.08); color: rgba(255,138,138,0.7); font-size: 10px;"
                            >{props.error}</pre>
                        </div>
                    </Show>
                </div>
            </Show>
        </div>
    );
};

export default ToolCallBubble;

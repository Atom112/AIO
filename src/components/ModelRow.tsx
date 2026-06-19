/**
 * 仿 LobeHub v2 风格的单行模型展示
 * 数据源: catalog `ModelMeta` (含 displayName, releaseDate, contextWindow, capabilities, pricing, status)
 * 启用状态: 由调用方通过 `enabled` prop 传入
 */
import { Component, Show } from 'solid-js';
import type { ModelMeta } from '@aio/models-data';
import { formatContextWindow, formatReleaseDate } from '../utils/models';

const ModelRow: Component<{
    meta: ModelMeta;
    enabled: boolean;
    onToggle: () => void;
    /** 隐藏价格（详情页默认显示，列表/孤儿行可隐藏） */
    showPricing?: boolean;
}> = (props) => {
    const m = () => props.meta;
    const caps = () => m().capabilities ?? {};
    return (
        <div class="flex items-center gap-3 px-3 py-2.5 rounded border border-dark-300 hover:border-pri-30 transition-colors bg-dark-900/40">
            <div class="grow min-w-0">
                <div class="text-sm text-white truncate">
                    {m().displayName || m().id}
                </div>
                <div class="flex items-center gap-2 text-[10px] text-[#888] font-mono mt-1 flex-wrap">
                    <span class="px-1.5 py-0.5 rounded bg-dark-850 border border-dark-300">
                        {m().id}
                    </span>
                    <Show when={m().releaseDate}>
                        <span>发布于 {formatReleaseDate(m().releaseDate)}</span>
                    </Show>
                    <Show when={m().contextWindow > 0}>
                        <span>📏 {formatContextWindow(m().contextWindow)}</span>
                    </Show>
                    <Show when={caps().vision}>
                        <span title="支持视觉">👁</span>
                    </Show>
                    <Show when={caps().tools}>
                        <span title="支持工具调用">🛠</span>
                    </Show>
                    <Show when={caps().reasoning}>
                        <span title="支持推理">🧠</span>
                    </Show>
                    <Show when={caps().streaming}>
                        <span title="支持流式">⚡</span>
                    </Show>
                    <Show when={caps().json_mode}>
                        <span title="支持 JSON 模式">{}</span>
                    </Show>
                    <Show when={m().status && m().status !== 'active'}>
                        <span
                            class="px-1.5 py-0.5 rounded text-[9px]"
                            classList={{
                                'bg-yellow-500/20 text-yellow-300': m().status === 'deprecated',
                                'bg-blue-500/20 text-blue-300': m().status === 'preview' || m().status === 'beta',
                                'bg-purple-500/20 text-purple-300': m().status === 'experimental' || m().status === 'alpha',
                            }}
                        >
                            {m().status}
                        </span>
                    </Show>
                </div>
                <Show when={props.showPricing !== false && m().pricing}>
                    <div class="text-[10px] text-[#aaa] mt-1">
                        输入 ${m().pricing!.input}/M · 输出 ${m().pricing!.output}/M
                        <Show when={m().pricing!.cacheRead != null}>
                            <span class="ml-2 text-[#888]">缓存读 ${m().pricing!.cacheRead}/M</span>
                        </Show>
                    </div>
                </Show>
            </div>
            <button
                type="button"
                class="relative inline-flex h-5 w-9 items-center rounded-full transition-colors shrink-0 focus:outline-none focus:ring-2 focus:ring-pri/50"
                classList={{
                    'bg-pri': props.enabled,
                    'bg-dark-300': !props.enabled,
                }}
                onClick={(e) => { e.stopPropagation(); props.onToggle(); }}
                title={props.enabled ? '点击停用' : '点击启用'}
            >
                <span
                    class="inline-block h-3 w-3 transform rounded-full bg-white transition-transform"
                    classList={{
                        'translate-x-5': props.enabled,
                        'translate-x-1': !props.enabled,
                    }}
                />
            </button>
        </div>
    );
};

export default ModelRow;

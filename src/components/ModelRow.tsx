/**
 * 单行模型展示 (仿 LobeHub v2 风格, 亚克力主题)
 * 数据源: catalog `ModelMeta` (含 displayName, releaseDate, contextWindow, capabilities, pricing, status)
 * 启用状态: 由调用方通过 `enabled` prop 传入
 */
import { Component, Show } from 'solid-js';
import type { ModelMeta } from '@aio/models-data';
import { formatContextWindow, formatReleaseDate } from '../utils/models';
import Icon from './Icon';

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
        <div class="model-row list-row flex items-center gap-3 px-3 py-2.5">
            <div class="grow min-w-0">
                <div class="text-sm text-white truncate font-medium">
                    {m().displayName || m().id}
                </div>
                <div class="flex items-center gap-1.5 text-[10px] text-[#888] font-mono mt-1 flex-wrap">
                    <span class="px-1.5 py-0.5 rounded bg-white/5 border border-white/8 text-[#aaa]">
                        {m().id}
                    </span>
                    <Show when={m().releaseDate}>
                        <span>发布于 {formatReleaseDate(m().releaseDate)}</span>
                    </Show>
                    <Show when={m().contextWindow > 0}>
                        <span class="inline-flex items-center gap-0.5">
                            <Icon name="bolt" size={10} class="text-pri" /> {formatContextWindow(m().contextWindow)}
                        </span>
                    </Show>
                    <Show when={caps().vision}>
                        <span title="支持视觉" class="opacity-70 hover:opacity-100 transition-opacity inline-flex"><Icon name="eye" size={12} /></span>
                    </Show>
                    <Show when={caps().tools}>
                        <span title="支持工具调用" class="opacity-70 hover:opacity-100 transition-opacity inline-flex"><Icon name="wrench" size={12} /></span>
                    </Show>
                    <Show when={caps().reasoning}>
                        <span title="支持推理" class="opacity-70 hover:opacity-100 transition-opacity inline-flex"><Icon name="brain" size={12} /></span>
                    </Show>
                    <Show when={caps().streaming}>
                        <span title="支持流式" class="opacity-70 hover:opacity-100 transition-opacity inline-flex"><Icon name="bolt" size={12} /></span>
                    </Show>
                    <Show when={caps().json_mode}>
                        <span title="支持 JSON 模式" class="opacity-70 hover:opacity-100 transition-opacity inline-flex"><Icon name="code" size={12} /></span>
                    </Show>
                    <Show when={m().status && m().status !== 'active'}>
                        <span
                            class="chip"
                            classList={{
                                'chip-warn': m().status === 'deprecated',
                                'chip-info': m().status === 'preview' || m().status === 'beta',
                                'chip': m().status === 'experimental' || m().status === 'alpha',
                            }}
                            style={m().status === 'experimental' || m().status === 'alpha'
                                ? 'background: rgba(168, 85, 247, 0.15); color: #d8b4fe; border: 1px solid rgba(168, 85, 247, 0.25);'
                                : undefined}
                        >
                            {m().status}
                        </span>
                    </Show>
                </div>
                <Show when={props.showPricing !== false && m().pricing}>
                    <div class="text-[10px] text-[#aaa] mt-1.5 font-mono">
                        <span class="text-[#888]">输入</span> ${m().pricing!.input}/M
                        <span class="text-[#666] mx-1.5">·</span>
                        <span class="text-[#888]">输出</span> ${m().pricing!.output}/M
                        <Show when={m().pricing!.cacheRead != null}>
                            <span class="text-[#666] mx-1.5">·</span>
                            <span class="text-[#888]">缓存读</span> ${m().pricing!.cacheRead}/M
                        </Show>
                    </div>
                </Show>
            </div>
            <button
                type="button"
                class="toggle-glass"
                classList={{ 'on': props.enabled }}
                onClick={(e) => { e.stopPropagation(); props.onToggle(); }}
                title={props.enabled ? '点击停用' : '点击启用'}
            >
                <span class="toggle-knob" />
            </button>
        </div>
    );
};

export default ModelRow;

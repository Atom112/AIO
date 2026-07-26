/**
 * 单行模型展示 (仿 LobeHub v2 风格, 亚克力主题)
 * 数据源: catalog `ModelMeta` (含 displayName, releaseDate, contextWindow, capabilities, pricing, status)
 * 启用状态: 由调用方通过 `enabled` prop 传入
 */
import { Component, Show } from 'solid-js';
import type { ModelMeta } from '@aio/models-data';
import { formatContextWindow, formatReleaseDate } from '../../core/utils/models';
import Icon from './Icon';
import { t } from '../../core/i18n';

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
        <div class="transition-all duration-200 hover:bg-pri-5 hover:border-pri relative bg-white/[0.025] border border-white/[0.05] rounded-[10px] transition-all duration-[250ms] hover:bg-pri-5 hover:border-pri hover:translate-x-0.5 hover:shadow-[0_4px_16px_rgba(0,0,0,0.25)] active:translate-x-0.5 active:scale-[0.995] flex items-center gap-3 px-3 py-2.5">
            <div class="grow min-w-0">
                <div class="text-sm text-white truncate font-medium">
                    {m().displayName || m().id}
                </div>
                <div class="flex items-center gap-1.5 text-[10px] text-[#888] font-mono mt-1 flex-wrap">
                    <Show when={m().releaseDate}>
                        <span>{t('model.released', { date: formatReleaseDate(m().releaseDate) })}</span>
                    </Show>
                    <Show when={m().contextWindow > 0}>
                        <span class="inline-flex items-center gap-0.5">
                            <Icon name="bolt" size={10} class="text-pri" /> {formatContextWindow(m().contextWindow)}
                        </span>
                    </Show>
                    <Show when={caps().vision}>
                        <span title={t('model.capability.vision')} class="opacity-70 hover:opacity-100 transition-opacity inline-flex"><Icon name="eye" size={12} /></span>
                    </Show>
                    <Show when={caps().tools}>
                        <span title={t('model.capability.tools')} class="opacity-70 hover:opacity-100 transition-opacity inline-flex"><Icon name="wrench" size={12} /></span>
                    </Show>
                    <Show when={caps().reasoning}>
                        <span title={t('model.capability.reasoning')} class="opacity-70 hover:opacity-100 transition-opacity inline-flex"><Icon name="brain" size={12} /></span>
                    </Show>
                    <Show when={caps().streaming}>
                        <span title={t('model.capability.streaming')} class="opacity-70 hover:opacity-100 transition-opacity inline-flex"><Icon name="bolt" size={12} /></span>
                    </Show>
                    <Show when={caps().json_mode}>
                        <span title={t('model.capability.json')} class="opacity-70 hover:opacity-100 transition-opacity inline-flex"><Icon name="code" size={12} /></span>
                    </Show>
                    <Show when={m().status && m().status !== 'active'}>
                        <span
                            class="chip rounded p-0.5"
                            classList={{
                                'chip-warn': m().status === 'deprecated',
                                'chip': m().status === 'experimental' || m().status === 'alpha',
                            }}
                            style={(() => {
                                if (m().status === 'preview' || m().status === 'beta') return { background: 'rgba(var(--primary-rgb), 0.15)', color: 'rgba(var(--primary-rgb), 1)', border: '1px solid rgba(var(--primary-rgb), 0.25)', borderRadius: '4px' };
                                if (m().status === 'experimental' || m().status === 'alpha') return { background: 'rgba(168, 85, 247, 0.15)', color: '#d8b4fe', border: '1px solid rgba(168, 85, 247, 0.25)', borderRadius: '4px' };
                                return undefined;
                            })()}
                        >
                            {m().status}
                        </span>
                    </Show>
                </div>
                <Show when={props.showPricing !== false && m().pricing}>
                    <div class="text-[10px] text-[#aaa] mt-1.5 font-mono">
                        <span class="text-[#888]">{t('model.price.input')}</span> ${m().pricing!.input}/M
                        <span class="text-[#666] mx-1.5">·</span>
                        <span class="text-[#888]">{t('model.price.output')}</span> ${m().pricing!.output}/M
                        <Show when={m().pricing!.cacheRead != null}>
                            <span class="text-[#666] mx-1.5">·</span>
                            <span class="text-[#888]">{t('model.price.cacheRead')}</span> ${m().pricing!.cacheRead}/M
                        </Show>
                    </div>
                </Show>
            </div>
            <button
                type="button"
                class="relative inline-flex items-center h-[22px] w-[40px] rounded-full bg-white/[0.08] border border-white/[0.08] cursor-pointer shrink-0 focus:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--primary-rgb),0.25)]"
                style={{
                    transition: 'background 0.3s cubic-bezier(0.4, 0, 0.2, 1), border-color 0.3s ease, box-shadow 0.3s ease',
                    ...(props.enabled ? { background: 'rgba(var(--primary-rgb), 0.7)', 'border-color': 'rgba(var(--primary-rgb), 0.5)', 'box-shadow': '0 0 12px rgba(var(--primary-rgb), 0.35)' } : {})
                }}
                onClick={(e) => { e.stopPropagation(); props.onToggle(); }}
                title={props.enabled ? t('common.disable') : t('common.enable')}
            >
                <span
                    class="inline-block h-4 w-4 rounded-full bg-white transition-transform duration-300 shadow-[0_2px_6px_rgba(0,0,0,0.4)]"
                    style={{ transform: props.enabled ? 'translateX(21px)' : 'translateX(3px)' }}
                />
            </button>
        </div>
    );
};

export default ModelRow;

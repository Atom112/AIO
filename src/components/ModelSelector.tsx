/**
 * 输入框模型快速选择器（LobeHub 风格）
 * @description 在输入框工具栏左侧展示当前模型，点击弹出向上展开的下拉，
 * 列出全部可用模型（线上 / 本地）供快速切换。选中后绑定到当前助手并持久化。
 */
import { Component, createSignal, Show, For } from 'solid-js';
import Icon from './Icon';
import {
    datas, currentAssistantId, allAvailableModels, isLocalModel,
    resolveAssistantModel, modelKey, setAssistantModel, ActivatedModel,
} from '../store/store';
import { getLogo as getLogoByIds } from '../utils/modelLogo';

const getModelLogo = (name: string) => getLogoByIds(null, name);

const ModelSelector: Component = () => {
    const [open, setOpen] = createSignal(false);

    /** 当前助手对象（用于解析其绑定的生效模型） */
    const currentAssistant = () =>
        datas.assistants.find(a => a.id === currentAssistantId()) as any;

    /** 当前生效模型（优先按助手绑定的 modelId 解析，未绑定则回退全局 selectedModel） */
    const activeModel = () => resolveAssistantModel(currentAssistant());

    const cloudModels = () => allAvailableModels().filter(m => !isLocalModel(m));
    const localModels = () => allAvailableModels().filter(m => isLocalModel(m));

    /** 判断某模型是否为当前选中（统一按复合键精确匹配） */
    const isCurrent = (m: ActivatedModel): boolean => {
        const cur = activeModel();
        return cur ? modelKey(m) === modelKey(cur) : false;
    };

    /** 选中某模型：绑定到当前助手 + 同步全局 + 持久化（本地模型顺带拉起引擎） */
    const handlePick = (m: ActivatedModel) => {
        const id = currentAssistantId();
        if (id) void setAssistantModel(id, m);
        setOpen(false);
    };

    return (
        <div class="relative">
            {/* 触发按钮：仅显示当前模型图标（hover title 显示完整名称） */}
            <button
                class="flex items-center justify-center bg-transparent border-none rounded-md cursor-pointer p-1 transition-all duration-200"
                title={activeModel() ? activeModel()!.model_id : '切换模型'}
                onClick={(e) => { e.stopPropagation(); setOpen(o => !o); }}
                onMouseEnter={(e) => { e.currentTarget.style.background = 'rgba(255,255,255,0.06)'; }}
                onMouseLeave={(e) => { if (!open()) { e.currentTarget.style.background = 'transparent'; } }}
            >
                <Show
                    when={activeModel()}
                    fallback={<Icon name="model" size={18} class="opacity-50" />}
                >
                    <div class="w-[22px] h-[22px] bg-white rounded-full flex items-center justify-center shrink-0 shadow-sm">
                        <Show
                            when={getModelLogo(activeModel()!.model_id)}
                            fallback={<span class="text-[11px] font-bold" style="color: #1a1e2c;">{activeModel()!.model_id.charAt(0).toUpperCase()}</span>}
                        >
                            <img src={getModelLogo(activeModel()!.model_id)!} alt="logo" class="w-[15px] h-[15px] object-contain" />
                        </Show>
                    </div>
                </Show>
            </button>

            {/* 透明遮罩：仅展开时拦截点击空白处收起 */}
            <Show when={open()}>
                <div class="fixed inset-0 z-[40]" onClick={() => setOpen(false)} />
            </Show>
            {/* 下拉面板：始终渲染，通过 class 切换实现展开/收起过渡动画（向上展开） */}
            <div
                class="absolute bottom-full left-0 mb-2 z-[41] w-[300px] rounded-xl overflow-hidden flex flex-col transition-all duration-200 ease-out origin-bottom"
                classList={{
                    'opacity-0 scale-95 translate-y-1 pointer-events-none': !open(),
                    'opacity-100 scale-100 translate-y-0 pointer-events-auto': open(),
                }}
                style="background: rgba(18,22,35,0.96); border: 1px solid rgba(255,255,255,0.08); backdrop-filter: blur(12px); box-shadow: 0 -8px 30px rgba(0,0,0,0.4); max-height: 340px;"
            >
                    <div class="flex flex-col overflow-y-auto scrollbar-thin">
                        {/* 线上模型 */}
                        <div class="px-3 py-2 text-[11px] font-bold uppercase tracking-widest sticky top-0"
                            style="color: rgba(255,255,255,0.35); background: rgba(255,255,255,0.04); border-bottom: 1px solid rgba(255,255,255,0.04);">
                            线上模型
                        </div>
                        <div class="p-1.5">
                            <For each={cloudModels()}>
                                {(model) => (
                                    <div
                                        class="flex items-center gap-2 p-2 rounded-lg cursor-pointer select-none transition-all"
                                        style="color: rgba(255,255,255,0.6);"
                                        classList={{ '!bg-[rgba(124,154,191,0.12)]': isCurrent(model) }}
                                        onClick={() => handlePick(model)}
                                        onMouseEnter={(e) => { if (!isCurrent(model)) { e.currentTarget.style.background = 'rgba(255,255,255,0.06)'; e.currentTarget.style.color = 'white'; } }}
                                        onMouseLeave={(e) => { if (!isCurrent(model)) { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = 'rgba(255,255,255,0.6)'; } }}
                                    >
                                        <div class="w-6 h-6 bg-white rounded-full flex items-center justify-center shrink-0">
                                            <img src={getModelLogo(model.model_id)} alt="logo" class="w-[14px] h-[14px] object-contain" />
                                        </div>
                                        <div class="flex-1 min-w-0">
                                            <div class="text-[13px] text-white font-medium truncate">{model.model_id}</div>
                                            <div class="text-[10px] truncate" style="color: rgba(124,154,191,0.5);">{model.owned_by}</div>
                                        </div>
                                        <Show when={isCurrent(model)}>
                                            <Icon name="check" size={14} class="shrink-0" style="color: rgba(124,154,191,0.9);" />
                                        </Show>
                                    </div>
                                )}
                            </For>
                            <Show when={cloudModels().length === 0}>
                                <div class="p-3 text-center text-[12px]" style="color: rgba(255,255,255,0.25);">无线上模型</div>
                            </Show>
                        </div>

                        {/* 本地模型 */}
                        <div class="px-3 py-2 text-[11px] font-bold uppercase tracking-widest"
                            style="color: rgba(255,255,255,0.35); background: rgba(255,255,255,0.04); border-top: 1px solid rgba(255,255,255,0.04); border-bottom: 1px solid rgba(255,255,255,0.04);">
                            本地模型
                        </div>
                        <div class="p-1.5">
                            <For each={localModels()}>
                                {(model) => (
                                    <div
                                        class="flex items-center gap-2 p-2 rounded-lg cursor-pointer select-none transition-all"
                                        style="color: rgba(255,255,255,0.6);"
                                        classList={{ '!bg-[rgba(124,154,191,0.12)]': isCurrent(model) }}
                                        onClick={() => handlePick(model)}
                                        onMouseEnter={(e) => { if (!isCurrent(model)) { e.currentTarget.style.background = 'rgba(255,255,255,0.06)'; e.currentTarget.style.color = 'white'; } }}
                                        onMouseLeave={(e) => { if (!isCurrent(model)) { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = 'rgba(255,255,255,0.6)'; } }}
                                    >
                                        <div class="w-6 h-6 bg-white rounded-full flex items-center justify-center shrink-0">
                                            <img src={getModelLogo(model.model_id)} alt="logo" class="w-[14px] h-[14px] object-contain" />
                                        </div>
                                        <div class="flex-1 min-w-0">
                                            <div class="text-[13px] text-white font-medium truncate">{model.model_id}</div>
                                            <div class="text-[10px] truncate" style="color: rgba(124,154,191,0.5);">{model.owned_by}</div>
                                        </div>
                                        <Show when={isCurrent(model)}>
                                            <Icon name="check" size={14} class="shrink-0" style="color: rgba(124,154,191,0.9);" />
                                        </Show>
                                    </div>
                                )}
                            </For>
                            <Show when={localModels().length === 0}>
                                <div class="p-3 text-center text-[12px]" style="color: rgba(255,255,255,0.25);">无本地模型</div>
                            </Show>
                        </div>
                    </div>
                </div>
        </div>
    );
};

export default ModelSelector;

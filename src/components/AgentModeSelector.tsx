/**
 * Agent 模式快速选择器
 *
 * 位于聊天输入框模型选择器与推理按钮之间，点击弹出上拉菜单选择工作模式。
 * 四种模式：对话（关闭工具调用）、普通（写入确认）、自动（自主执行）、Plan（先列计划）。
 * 切换到 Agent 模式时自动恢复上次的工作目录，若无则显示"选择目录"按钮供用户自行选择或新建。
 */
import { Component, For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import {
    datas, setDatas, currentAssistantId, saveSingleAssistantToBackend,
    currentProjectId, setCurrentProjectId,
    projects, initProjects, initSkills, initMcpServers,
    getLastAgentProjectId, saveLastAgentProjectId,
    type AgentMode,
} from '../store/store';
import Icon, { type IconName } from './Icon';

interface ModeOption {
    value: AgentMode;
    label: string;
    desc: string;
    icon: IconName;
}

const MODES: ModeOption[] = [
    { value: 'off',    label: '对话',   desc: '纯对话，禁用工具调用',          icon: 'chat' },
    { value: 'normal', label: '普通',   desc: '修改文件前向用户确认',            icon: 'check-circle' },
    { value: 'auto',   label: '自动',   desc: '自主迭代直至目标完成',            icon: 'refresh' },
    { value: 'plan',   label: 'Plan',   desc: '先做调研，后列出实现方案',        icon: 'document' },
];

const MODE_COLORS: Record<string, string> = {
    off:    'rgba(124,154,191,0.4)',
    normal: 'rgba(124,217,160,0.7)',
    auto:   'rgba(224,192,96,0.7)',
    plan:   'rgba(160,124,217,0.7)',
};

const AgentModeSelector: Component = () => {
    const [open, setOpen] = createSignal(false);
    let containerRef: HTMLDivElement | undefined;

    const asst = () => datas.assistants.find(a => a.id === currentAssistantId()) as any;
    const currentMode = (): AgentMode => asst()?.agentMode || 'off';

    const activeOption = () => MODES.find(m => m.value === currentMode()) || MODES[0];

    /** 点击外部关闭 */
    const onDocClick = (e: MouseEvent) => {
        if (!containerRef) return;
        if (!containerRef.contains(e.target as Node)) {
            setOpen(false);
        }
    };

    onMount(() => document.addEventListener('mousedown', onDocClick));
    onCleanup(() => document.removeEventListener('mousedown', onDocClick));

    const choose = async (mode: AgentMode) => {
        const id = currentAssistantId();
        if (!id) return;
        setDatas('assistants', a => a.id === id, 'agentMode', mode);
        await saveSingleAssistantToBackend(id);
        setOpen(false);

        // 切换到 Agent 模式时自动恢复上次的工作目录
        if (mode !== 'off' && !currentProjectId()) {
            await initProjects();
            const lastId = getLastAgentProjectId();
            if (lastId && projects().find(p => p.id === lastId)) {
                // 有上次的工作目录且仍有效 → 直接恢复
                setCurrentProjectId(lastId);
                saveLastAgentProjectId(lastId);
                await Promise.all([initSkills(lastId), initMcpServers(lastId)]);
            }
            // 没有上次的工作目录→ 什么也不做，ProjectSelector 会显示"选择目录..."供用户自行选择或新建
        } else if (mode !== 'off' && currentProjectId()) {
            // 已有工作目录，刷新保存为 lastAgentProjectId
            saveLastAgentProjectId(currentProjectId());
        }

        // 切换到对话模式时清除 lastAgentProjectId（下次切回 Agent 时不自动恢复）
        if (mode === 'off') {
            saveLastAgentProjectId(null);
        }
    };

    const isDisabled = () => !currentAssistantId();

    return (
        <div ref={containerRef} class="relative inline-block">
            <button
                type="button"
                class="reasoning-trigger"
                classList={{ 'is-active': currentMode() !== 'off' }}
                style={{
                    color: currentMode() !== 'off' ? (MODE_COLORS[currentMode()] || MODE_COLORS.off) : undefined,
                    opacity: isDisabled() ? 0.4 : 1,
                }}
                title={isDisabled() ? '请先选择助手' : activeOption().desc}
                onClick={(e) => {
                    if (isDisabled()) return;
                    e.stopPropagation();
                    setOpen(!open());
                }}
            >
                <Icon name={activeOption().icon} size={15} class="reasoning-trigger-icon" />
                <span class="reasoning-trigger-label">{activeOption().label}</span>
            </button>

            {/* 下拉面板 */}
            <Show when={open()}>
                <div
                    class="absolute bottom-full left-0 mb-2 z-[41] w-[260px] rounded-xl overflow-hidden"
                    style="background: rgba(18,22,35,0.96); border: 1px solid rgba(255,255,255,0.08); backdrop-filter: blur(12px); box-shadow: 0 -8px 30px rgba(0,0,0,0.4);"
                    onClick={(e) => e.stopPropagation()}
                >
                    <div class="px-3 py-2 text-[11px] font-bold uppercase tracking-widest"
                        style="color: rgba(255,255,255,0.35); background: rgba(255,255,255,0.04); border-bottom: 1px solid rgba(255,255,255,0.04);">
                        Agent 工作模式
                    </div>
                    <div class="py-1">
                        <For each={MODES}>
                            {(opt) => (
                                <button
                                    type="button"
                                    class="w-full flex items-start gap-2.5 px-3 py-2.5 text-left transition-colors cursor-pointer border-none"
                                    style="color: rgba(255,255,255,0.75);"
                                    classList={{ '!bg-[rgba(124,154,191,0.12)]': currentMode() === opt.value }}
                                    onClick={() => choose(opt.value)}
                                    onMouseEnter={(e) => {
                                        if (currentMode() !== opt.value) e.currentTarget.style.background = 'rgba(255,255,255,0.05)';
                                    }}
                                    onMouseLeave={(e) => {
                                        if (currentMode() !== opt.value) e.currentTarget.style.background = 'transparent';
                                    }}
                                >
                                    <Icon name={opt.icon} size={14} class="mt-0.5 shrink-0" style={`color: ${MODE_COLORS[opt.value]}`} />
                                    <div class="flex-1 min-w-0">
                                        <div class="text-sm font-medium">{opt.label}</div>
                                        <div class="text-[11px] mt-0.5" style="color: rgba(255,255,255,0.35);">{opt.desc}</div>
                                    </div>
                                    <Show when={currentMode() === opt.value}>
                                        <Icon name="check" size={13} class="shrink-0" style="color: rgba(124,154,191,0.8);" />
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

export default AgentModeSelector;

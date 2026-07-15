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
} from '../../../core/store/store';
import Icon, { type IconName } from '../../../shared/components/Icon';

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
    { value: 'workflow', label: '工作流', desc: '强制拆解任务为工作流并自动执行', icon: 'layers' },
];

const MODE_COLORS: Record<string, string> = {
    off:    'rgba(124,154,191,0.4)',
    normal: 'rgba(124,217,160,0.7)',
    auto:   'rgba(224,192,96,0.7)',
    plan:   'rgba(160,124,217,0.7)',
    workflow: 'rgba(255,183,77,0.7)',
};

const AgentModeSelector: Component = () => {
    const [open, setOpen] = createSignal(false);
    const [showAutoWarning, setShowAutoWarning] = createSignal(false);
    let containerRef: HTMLDivElement | undefined;
    let autoModeConfirmed = false; // 会话内记住，确认一次后不再弹窗
    let pendingMode: AgentMode = 'auto'; // 当前正在确认的模式

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
        // 切换到自动模式或工作流模式时，先展示风险提醒弹窗
        if ((mode === 'auto' || mode === 'workflow') && !autoModeConfirmed) {
            pendingMode = mode;
            setOpen(false);
            setShowAutoWarning(true);
            return;
        }

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
                class="flex items-center gap-1.5 px-2.5 h-8 rounded-md border-none cursor-pointer transition-all duration-200 select-none bg-transparent text-white/40 text-xs font-medium hover:bg-white/[0.06] hover:text-[#7c9abf]/60"
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
                <Icon name={activeOption().icon} size={15} class="flex items-center justify-center shrink-0" />
                <span class="leading-none">{activeOption().label}</span>
            </button>

            {/* 下拉面板 */}
            <div
                class="absolute bottom-full left-0 mb-2 z-[41] w-[260px] rounded-xl overflow-hidden transition-all duration-150 ease-out origin-bottom"
                style="background: rgba(18,22,35,0.96); border: 1px solid rgba(255,255,255,0.08); backdrop-filter: blur(12px); box-shadow: 0 -8px 30px rgba(0,0,0,0.4);"
                classList={{
                    'invisible opacity-0 scale-95 -translate-y-1 pointer-events-none': !open(),
                    'visible opacity-100 scale-100 translate-y-0 pointer-events-auto': open(),
                }}
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

            {/* 自动模式风险提醒弹窗 */}
            <Show when={showAutoWarning()}>
                <div
                    class="modal-overlay"
                    style="z-index: 2100;"
                    onClick={() => setShowAutoWarning(false)}
                >
                    <div
                        class="modal-panel bg-dark-500 p-6 rounded-lg max-w-[420px] w-full"
                        style="background: rgba(18,22,35,0.96); border: 1px solid rgba(255,255,255,0.08); backdrop-filter: blur(20px); box-shadow: 0 8px 32px rgba(0,0,0,0.5);"
                        onClick={(e) => e.stopPropagation()}
                    >
                        <div class="flex items-center gap-3 mb-4">
                            <Icon name="alert-triangle" size={24} />
                            <h2 style="color: rgba(224,192,96,0.9); font-size: 1.1rem; font-weight: 600; margin: 0;">
                                自动模式风险提醒
                            </h2>
                        </div>

                        <div style="color: rgba(255,255,255,0.7); font-size: 0.875rem; line-height: 1.7; margin-bottom: 1.5rem;">
                            <p style="margin: 0 0 0.75rem 0;">
                                自动模式下，AI 将<strong style="color: rgba(224,192,96,0.9);">自主执行命令和文件操作</strong>，无需逐条您的确认。
                            </p>
                            <p style="margin: 0 0 0.75rem 0;">
                                包括但不限于：执行系统命令、读取/修改/删除文件、调用 MCP 工具等。
                            </p>
                            <p style="margin: 0;">
                                请确保你<strong style="color: rgba(255,255,255,0.85);">信任当前的工作目录内容</strong>，并了解 AI 可能产生的副作用。
                            </p>
                        </div>

                        <div class="flex justify-end gap-3">
                            <button
                                type="button"
                                class="px-4 py-2 rounded-lg text-sm font-medium transition-all duration-200 cursor-pointer border-none"
                                style="background: rgba(255,255,255,0.06); color: rgba(255,255,255,0.5);"
                                onClick={() => setShowAutoWarning(false)}
                            >
                                取消
                            </button>
                            <button
                                type="button"
                                class="px-4 py-2 rounded-lg text-sm font-medium transition-all duration-200 cursor-pointer border-none"
                                onClick={async () => {
                                    autoModeConfirmed = true;
                                    setShowAutoWarning(false);
                                    // 执行实际的模式切换
                                    await choose(pendingMode);
                                }}
                            >
                                我已知晓，进入{ (pendingMode as AgentMode) === 'workflow' ? '工作流' : '自动' }模式
                            </button>
                        </div>
                    </div>
                </div>
            </Show>
        </div>
    );
};

export default AgentModeSelector;

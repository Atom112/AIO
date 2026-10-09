/**
 * 子智能体独立对话视图。
 *
 * 从状态条右侧上拉菜单（或主代理时间线的紧凑卡片）进入，替换整个对话栏。
 * 消息流完全复用主话题的界面样式：
 * - 主代理下发的任务 → 右侧用户气泡（primary 底色，与主话题用户消息一致）
 * - 子代理每轮叙述 + 该轮工具执行 → 左侧助手气泡（头像 + text 底色，与主话题助手消息一致）
 * - 完成后的最终报告 → 最后一条左侧助手气泡（完整 Markdown）
 * - Header：左上角返回按钮 + 子智能体身份与状态 + 停止按钮
 *
 * 数据来源：父话题 assistant 消息 agentSteps 中的 type='subagent' 步骤
 * （subagentSteps / subagentResult 随消息持久化，重启后仍可回看）。
 */
import {
  Component,
  createSignal,
  createMemo,
  createEffect,
  Show,
  For,
  onCleanup,
  type JSX,
} from 'solid-js';
import Icon, { type IconName } from '../../../shared/components/Icon';
import Markdown from '../../../shared/components/Markdown';
import { t } from '../../../core/i18n';
import {
  activeSubagentView,
  setActiveSubagentView,
  findSubagentStep,
  type SubagentStep,
  type Topic,
} from '../../../core/store/store';
import { stopSubagent } from '../../../core/api/chat';
import { btnGhost, btnIcon } from '../../../shared/components/buttonStyles';

// ---- Profile 配置（自原 SubagentBlock 迁移；供紧凑卡片/菜单复用） ----
export interface SubagentProfileStyle {
  icon: IconName;
  color: string;
  bgColor: string;
  borderColor: string;
}

const PROFILE_STYLES: Record<string, SubagentProfileStyle> = {
  explorer: {
    icon: 'search',
    color: 'rgba(100,149,237,0.85)',
    bgColor: 'rgba(100,149,237,0.08)',
    borderColor: 'rgba(100,149,237,0.25)',
  },
  coder: {
    icon: 'code',
    color: 'rgba(76,175,144,0.85)',
    bgColor: 'rgba(76,175,144,0.08)',
    borderColor: 'rgba(76,175,144,0.25)',
  },
  general: {
    icon: 'sparkles',
    color: 'rgba(186,104,200,0.85)',
    bgColor: 'rgba(186,104,200,0.08)',
    borderColor: 'rgba(186,104,200,0.25)',
  },
  requirements: {
    icon: 'lightbulb',
    color: 'rgba(255,183,77,0.85)',
    bgColor: 'rgba(255,183,77,0.08)',
    borderColor: 'rgba(255,183,77,0.25)',
  },
};

const DEFAULT_PROFILE: SubagentProfileStyle = {
  icon: 'sparkles',
  color: 'rgba(156,163,175,0.75)',
  bgColor: 'rgba(156,163,175,0.06)',
  borderColor: 'rgba(156,163,175,0.2)',
};

/** profile id → 展示名（i18n） */
export function subagentProfileLabel(profileId?: string): string {
  switch (profileId) {
    case 'explorer':
      return t('agent.profile.explorer');
    case 'coder':
      return t('agent.profile.coder');
    case 'general':
      return t('agent.profile.general');
    case 'requirements':
      return t('agent.profile.requirements');
    default:
      return t('agent.profile.subagent');
  }
}

/** profile id → 颜色/图标样式 */
export function getSubagentProfileStyle(profileId?: string): SubagentProfileStyle {
  return PROFILE_STYLES[profileId || ''] || DEFAULT_PROFILE;
}

/** 毫秒 → 耗时文案 */
export function formatSubagentDuration(ms: number): string {
  if (ms < 50) return '0.1s';
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds.toFixed(1)}s`;
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  return t('agent.step.duration', { mins, secs });
}

// ---- 气泡内工具执行行（观感对齐主话题 AgentProcessBlock 的工具卡片） ----
const ToolRow: Component<{ step: SubagentStep }> = (props) => (
  <div
    class="flex items-start gap-1.5 px-2 py-1 rounded-md text-[11px]"
    style={{
      background: 'rgba(var(--text-base-rgb),0.03)',
      'border-left': '2px solid rgba(240,160,64,0.55)',
    }}
  >
    <Icon name="wrench" size={11} class="mt-[2px] shrink-0 opacity-70" />
    <div class="flex-1 min-w-0">
      <span class="font-medium text-white/60 break-all">{props.step.toolName}</span>
      <Show when={props.step.summary}>
        <div class="font-mono text-[10px] text-white/40 leading-relaxed break-words mt-0.5">
          {props.step.summary}
        </div>
      </Show>
    </div>
  </div>
);

/** 一个对话气泡：子代理某轮的叙述文本 + 该轮内执行的工具 */
interface SubagentTurn {
  text: string;
  tools: SubagentStep[];
}

interface SubagentConversationViewProps {
  /** 主代理当前话题（子代理步骤挂在其 assistant 消息的 agentSteps 上） */
  topic: Topic | null;
}

const SubagentConversationView: Component<SubagentConversationViewProps> = (props) => {
  const step = createMemo(() => findSubagentStep(props.topic, activeSubagentView()));

  // 兜底：消息被删除 / 话题数据消失时自动返回主对话
  createEffect(() => {
    if (activeSubagentView() && !step()) setActiveSubagentView(null);
  });

  const profileStyle = () => getSubagentProfileStyle(step()?.subagentProfile);
  const displayName = () => step()?.subagentName || subagentProfileLabel(step()?.subagentProfile);
  const taskText = () => step()?.subagentTask || '';
  const resultText = () => step()?.subagentResult || '';
  const isRunning = () => step()?.status === 'running';
  const isError = () => step()?.status === 'error';
  const isComplete = () => step()?.status === 'complete';

  // 实时计时器（工作中计时，完成后冻结为 duration）
  const [elapsedMs, setElapsedMs] = createSignal(0);
  createEffect(() => {
    const s = step();
    if (s && s.status === 'running' && s.timestamp) {
      const intervalId = setInterval(() => {
        if (step()?.status !== 'running') {
          clearInterval(intervalId);
          return;
        }
        setElapsedMs(Date.now() - s.timestamp);
      }, 100);
      onCleanup(() => clearInterval(intervalId));
    } else if (s && s.timestamp) {
      setElapsedMs((s.duration ?? 0) || Date.now() - s.timestamp);
    }
  });

  // 消息流分组：无 toolName 的 tool_call 步骤 = 子代理该轮的叙述文本（开启新气泡）；
  // tool_result 步骤挂到当前气泡内；content 步骤与最终结果重复（由结果气泡展示），
  // streaming 心跳与 thinking 步骤不渲染。
  // 后端对空文本轮次发 "[工具调用]" 占位，按无文本处理（该轮气泡仅含工具行）。
  const turns = createMemo<SubagentTurn[]>(() => {
    const steps = step()?.subagentSteps || [];
    const out: SubagentTurn[] = [];
    let cur: SubagentTurn | null = null;
    const ensure = (): SubagentTurn => {
      if (!cur) {
        cur = { text: '', tools: [] };
        out.push(cur);
      }
      return cur;
    };
    for (const s of steps) {
      if (s.type === 'content' || s.type === 'streaming' || s.type === 'thinking') continue;
      if (!s.toolName) {
        cur = { text: s.summary === '[工具调用]' ? '' : s.summary || '', tools: [] };
        out.push(cur);
      } else {
        ensure().tools.push(s);
      }
    }
    return out;
  });

  // 单独停止后台子智能体（仅 fire-and-forget 启动的在注册表中可查；
  // 等待型子代理随整轮停止，此处点击为幂等 no-op）
  const [stopping, setStopping] = createSignal(false);
  const handleStop = async () => {
    const id = step()?.subagentId;
    if (!id || stopping()) return;
    setStopping(true);
    try {
      await stopSubagent(id);
    } finally {
      setStopping(false);
    }
  };

  // ---- 自动滚动（复用主对话模式） ----
  let bodyRef: HTMLDivElement | undefined;
  // 内容包装层：流式跟底 ResizeObserver 的观察对象（bodyRef 是 flex-1 固定高度
  // 滚动盒，观察它时内容增长不触发 RO；包装层高度随气泡内容真实增长）。
  let contentRef: HTMLDivElement | undefined;
  const [autoScroll, setAutoScroll] = createSignal(true);
  let userScrolledUp = false;
  let suppressScroll = false;

  const isAtBottom = () => {
    if (!bodyRef) return true;
    const el = bodyRef;
    return el.scrollHeight - el.scrollTop - el.clientHeight < 50;
  };

  const snapToBottom = () => {
    if (!bodyRef) return;
    const el = bodyRef;
    const target = Math.max(0, el.scrollHeight - el.clientHeight);
    // 仅在实际改变位置时压制回显事件：已在底部时（流式跟底的常见情形）
    // 无事件产生，若仍置位 suppressScroll 会吃掉用户的下一个滚动事件
    if (el.scrollTop < target) {
      suppressScroll = true;
      el.scrollTop = target;
    }
  };

  const handleWheel = (e: WheelEvent) => {
    if (e.deltaY < 0) {
      userScrolledUp = true;
      suppressScroll = false;
      setAutoScroll(false);
    }
  };

  const handleScroll = () => {
    if (suppressScroll) {
      suppressScroll = false;
      return;
    }
    const atBottom = isAtBottom();
    setAutoScroll(atBottom);
    if (atBottom) userScrolledUp = false;
  };

  createEffect(() => {
    if (contentRef) {
      const observer = new ResizeObserver(() => {
        if (!userScrolledUp && autoScroll()) snapToBottom();
      });
      observer.observe(contentRef);
      if (bodyRef) observer.observe(bodyRef);
      onCleanup(() => observer.disconnect());
    }
  });

  // 进入视图时贴底
  createEffect(() => {
    if (step() && bodyRef) {
      queueMicrotask(() => {
        if (!userScrolledUp) snapToBottom();
      });
    }
  });

  // ---- 气泡渲染辅助（复用主话题 assistant 消息的样式） ----
  const avatar = () => {
    const ps = profileStyle();
    return (
      <div
        class="flex flex-shrink-0 items-center justify-center w-9 h-9 rounded-full"
        style={{
          background: ps.bgColor,
          border: `1px solid ${ps.borderColor}`,
          'box-shadow': '0 2px 6px rgba(0,0,0,0.25)',
        }}
      >
        <Icon name={ps.icon} size={15} style={{ color: ps.color }} />
      </div>
    );
  };

  /** 左侧助手气泡行（头像 + text 底色气泡，与主话题 assistant 消息同款） */
  const agentRow = (children: JSX.Element) => (
    <div class="flex items-start gap-2 mb-3">
      {avatar()}
      <div class="flex flex-col items-start max-w-[75%]">
        <div
          class="rounded-[10px] rounded-tl-[2px] leading-relaxed max-w-full px-[14px] py-[10px] break-words text-white"
          style={{
            background: 'rgba(var(--text-base-rgb),0.04)',
            border: '1px solid rgba(var(--text-base-rgb),0.13)',
            'backdrop-filter': 'blur(6px)',
          }}
        >
          {children}
        </div>
      </div>
    </div>
  );

  return (
    <div class="flex flex-col flex-grow items-stretch rounded-[12px] box-border overflow-hidden p-[15px] pb-5 relative h-full">
      {/* Header：左上角返回按钮 + 子智能体身份 + 停止 */}
      <div
        class="flex items-center gap-2 px-3 h-10 shrink-0 border-b"
        style={{ 'border-color': 'var(--border-dim)' }}
      >
        <button
          type="button"
          class={btnGhost + ' px-2 py-1 rounded-md text-xs shrink-0'}
          onClick={() => setActiveSubagentView(null)}
        >
          <Icon name="arrow-left" size={13} />
          <span class="ml-1">{t('agent.subagent.back')}</span>
        </button>

        <span class="w-px h-4 shrink-0" style={{ background: 'rgba(var(--text-base-rgb),0.12)' }} />

        <div class="flex items-center gap-2 min-w-0 flex-1">
          <span
            class="flex items-center justify-center w-5 h-5 rounded shrink-0"
            style={{
              background: profileStyle().bgColor,
              border: `1px solid ${profileStyle().borderColor}`,
              color: profileStyle().color,
            }}
          >
            <Icon
              name={profileStyle().icon}
              size={12}
              classList={{ 'animate-spin': isRunning() }}
            />
          </span>
          <span class="text-[13px] font-semibold text-white/85 truncate shrink-0">
            {displayName()}
          </span>
          <span
            class="text-[10px] px-1.5 py-0.5 rounded font-medium shrink-0"
            style={{
              color: profileStyle().color,
              background: 'rgba(var(--text-base-rgb),0.05)',
              border: `1px solid ${profileStyle().borderColor}`,
            }}
          >
            {subagentProfileLabel(step()?.subagentProfile)}
          </span>
          <Show when={step()}>
            <span class="font-mono text-[10px] text-white/25 shrink-0">
              {formatSubagentDuration(elapsedMs())}
            </span>
          </Show>
          <Show when={isRunning()}>
            <span
              class="w-1.5 h-1.5 rounded-full animate-pulse shrink-0"
              style={{ background: profileStyle().color }}
            />
          </Show>
          <Show when={isError()}>
            <Icon name="x" size={11} class="text-red-400/70 shrink-0" />
          </Show>
        </div>

        <Show when={isRunning() && step()?.subagentId}>
          <button
            type="button"
            class={btnIcon + ' w-6 h-6 shrink-0 text-white/40 hover:text-red-400/80'}
            title={t('agent.subagent.stop')}
            aria-label={t('agent.subagent.stop')}
            onClick={() => void handleStop()}
          >
            <Icon name="stop" size={10} classList={{ 'animate-pulse': stopping() }} />
          </button>
        </Show>
      </div>

      {/* 消息流：与主话题同款（右用户气泡 + 左助手气泡） */}
      <div
        ref={bodyRef}
        class="flex-1 overflow-y-auto min-h-0"
        onScroll={handleScroll}
        onWheel={handleWheel}
      >
        <div ref={contentRef} class="pt-3 pb-[15px]">
          <Show when={step()}>
            {/* 主代理下发的任务：右侧用户气泡 */}
            <Show when={taskText()}>
              <div class="flex items-start gap-2 mb-3 justify-end pr-[10px]">
                <div class="flex flex-col items-end max-w-[75%]">
                  <div
                    class="rounded-[10px] rounded-tr-[2px] leading-relaxed max-w-full px-[14px] py-[10px] break-words text-white"
                    style={{
                      background: 'rgba(var(--primary-rgb),0.08)',
                      border: '1px solid rgba(var(--primary-rgb),0.16)',
                      'backdrop-filter': 'blur(6px)',
                    }}
                  >
                    <div class="mt-1">
                      <Markdown content={taskText()} />
                    </div>
                  </div>
                </div>
              </div>
            </Show>

            {/* 子代理各轮叙述 + 工具执行：左侧助手气泡 */}
            <For each={turns()}>
              {(turn) =>
                agentRow(
                  <>
                    <Show
                      when={turn.text || turn.tools.length > 0}
                      fallback={
                        <div class="flex items-center gap-2 py-1 text-white/50 italic text-[14px] select-none">
                          <Icon name="spinner" size={14} class="animate-spin opacity-50" />
                          <span class="animate-pulse">{t('agent.sub.working')}</span>
                        </div>
                      }
                    >
                      <Show when={turn.text}>
                        <div class="mt-1 text-[13px]">
                          <Markdown content={turn.text} />
                        </div>
                      </Show>
                      <Show when={turn.tools.length > 0}>
                        <div class="flex flex-col gap-1" classList={{ 'mt-2': !!turn.text }}>
                          <For each={turn.tools}>{(tool) => <ToolRow step={tool} />}</For>
                        </div>
                      </Show>
                    </Show>
                  </>,
                )
              }
            </For>

            {/* 工作中：尾随气泡（正在执行工具 / 等待下一轮输出） */}
            <Show when={isRunning()}>
              {agentRow(
                <div class="flex items-center gap-2 py-1 text-white/50 italic text-[14px] select-none">
                  <Icon name="spinner" size={14} class="animate-spin opacity-50" />
                  <span class="animate-pulse">{t('agent.sub.working')}</span>
                </div>,
              )}
            </Show>

            {/* 出错：红色文本助手气泡 */}
            <Show when={isError()}>
              {agentRow(
                <div
                  class="text-[13px] leading-relaxed break-words"
                  style={{ color: 'rgba(224,85,85,0.75)' }}
                >
                  {resultText() || t('agent.sub.error')}
                </div>,
              )}
            </Show>

            {/* 完成后的最终报告：最后一条助手气泡（完整 Markdown） */}
            <Show when={isComplete() && resultText()}>
              {agentRow(
                <div class="mt-1 text-[13px] text-white/85">
                  <Markdown content={resultText()} />
                </div>,
              )}
            </Show>
          </Show>
        </div>
      </div>
    </div>
  );
};

export default SubagentConversationView;

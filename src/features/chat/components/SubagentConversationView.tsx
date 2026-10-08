/**
 * 子智能体独立对话视图。
 *
 * 从状态条右侧上拉菜单（或主代理时间线的紧凑卡片）进入，替换整个对话栏：
 * - Header：左上角返回按钮（回到主代理对话）+ 子智能体身份与状态 + 停止按钮
 * - Body：任务卡 + 按轮次分组的步骤时间线 + 完整工作结果（Markdown）
 *
 * 数据来源：父话题 assistant 消息 agentSteps 中的 type='subagent' 步骤
 * （subagentSteps / subagentResult 随消息持久化，重启后仍可回看）。
 */
import { Component, createSignal, createMemo, createEffect, Show, For, onCleanup } from 'solid-js';
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

// ---- 步骤行渲染 ----
const StepRow: Component<{ step: SubagentStep }> = (props) => {
  // 无 toolName 的 tool_call 行 = 子代理本轮的叙述文本；带 toolName = 工具执行结果
  const isToolResult = () => !!props.step.toolName;
  const icon = (): IconName => {
    if (props.step.type === 'thinking') return 'brain';
    if (isToolResult()) return 'wrench';
    return 'document';
  };
  const label = () =>
    props.step.type === 'thinking'
      ? t('agent.sub.step.thinking')
      : isToolResult()
        ? t('agent.sub.step.toolCall')
        : t('agent.sub.step.content');

  return (
    <div
      class="flex items-start gap-2 px-2 py-1.5 rounded-md"
      style={{ background: 'rgba(var(--text-base-rgb),0.03)' }}
    >
      <span class="flex items-center justify-center w-4 h-4 mt-0.5 shrink-0 text-white/40">
        <Icon name={icon()} size={12} />
      </span>
      <div class="flex-1 min-w-0">
        <div class="flex items-center gap-1.5 text-[11px] font-medium text-white/60">
          <span class="shrink-0">{label()}</span>
          <Show when={props.step.toolName}>
            <span class="truncate text-white/40">· {props.step.toolName}</span>
          </Show>
        </div>
        <Show when={props.step.summary}>
          <div
            classList={{
              'text-[12px] leading-relaxed break-words mt-0.5 text-white/55': !isToolResult(),
              'text-[11px] leading-relaxed break-words mt-0.5 font-mono text-white/40':
                isToolResult(),
            }}
          >
            {props.step.summary}
          </div>
        </Show>
      </div>
    </div>
  );
};

// 渲染条目：轮次分隔行 / 步骤行
type RenderItem = { kind: 'round'; round: number } | { kind: 'step'; step: SubagentStep };

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

  // 步骤时间线渲染条目：
  // - content 行与结果区重复（最终文本），跳过；
  // - streaming 心跳行仅用于轮次分隔（旧数据无 round 时不产生分隔）；
  // - 其余行按序渲染。
  const renderItems = createMemo<RenderItem[]>(() => {
    const steps = step()?.subagentSteps || [];
    const items: RenderItem[] = [];
    let lastRound: number | undefined;
    for (const s of steps) {
      if (s.type === 'content') continue;
      if (s.round !== undefined && s.round !== lastRound) {
        items.push({ kind: 'round', round: s.round });
        lastRound = s.round;
      }
      if (s.type === 'streaming') continue;
      items.push({ kind: 'step', step: s });
    }
    return items;
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

  // ---- 自动滚动（复用 AgentProcessBlock 模式） ----
  let bodyRef: HTMLDivElement | undefined;
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
    suppressScroll = true;
    const el = bodyRef;
    el.scrollTop = Math.max(0, el.scrollHeight - el.clientHeight);
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
    if (bodyRef) {
      const observer = new ResizeObserver(() => {
        if (!userScrolledUp && autoScroll()) snapToBottom();
      });
      observer.observe(bodyRef);
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

  return (
    <div class="flex flex-col flex-grow items-stretch rounded-[12px] box-border overflow-hidden p-[15px] pb-5 relative h-full">
      {/* Header：左上角返回按钮 + 子智能体身份 + 停止 */}
      <div
        class="flex items-center gap-2 px-3 h-10 shrink-0 border-b"
        style={{
          'border-color': 'var(--border-dim)',
          background: 'rgba(var(--text-base-rgb),0.02)',
        }}
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

      {/* Body：任务 + 步骤时间线 + 结果 */}
      <div
        ref={bodyRef}
        class="flex-1 overflow-y-auto min-h-0 rounded-xl"
        style={{ background: 'rgba(0,0,0,0.25)', border: '1px solid var(--border-dim)' }}
        onScroll={handleScroll}
        onWheel={handleWheel}
      >
        <Show when={step()}>
          {/* 任务卡 */}
          <Show when={taskText()}>
            <div
              class="mx-4 mt-3 rounded-lg border px-3 py-2"
              style={{
                background: 'rgba(var(--text-base-rgb),0.03)',
                'border-color': 'rgba(var(--text-base-rgb),0.08)',
              }}
            >
              <div class="flex items-center gap-1.5 text-[11px] font-medium text-white/40 mb-1">
                <Icon name="clipboard" size={11} />
                {t('agent.subagent.task')}
              </div>
              <div class="text-[13px] leading-relaxed break-words text-white/65">{taskText()}</div>
            </div>
          </Show>

          {/* 步骤时间线（按轮次分组） */}
          <div class="flex flex-col gap-1 px-4 mt-3">
            <For each={renderItems()}>
              {(item) =>
                item.kind === 'round' ? (
                  <div class="flex items-center gap-2 py-1.5 select-none">
                    <span
                      class="flex-1 h-px"
                      style={{ background: 'rgba(var(--text-base-rgb),0.08)' }}
                    />
                    <span class="font-mono text-[10px] text-white/25">
                      {t('agent.subagent.round', { round: item.round })}
                    </span>
                    <span
                      class="flex-1 h-px"
                      style={{ background: 'rgba(var(--text-base-rgb),0.08)' }}
                    />
                  </div>
                ) : (
                  <StepRow step={item.step} />
                )
              }
            </For>
          </div>

          {/* 工作中指示 */}
          <Show when={isRunning()}>
            <div class="flex items-center gap-2 px-6 py-3 text-[12px] text-white/35 italic">
              <Icon name="spinner" size={12} class="animate-spin" />
              {t('agent.sub.working')}
            </div>
          </Show>

          {/* 错误状态 */}
          <Show when={isError()}>
            <div
              class="mx-4 mt-3 mb-3 px-3 py-2 rounded-lg text-[12px] leading-relaxed break-words"
              style={{
                background: 'rgba(224,85,85,0.08)',
                border: '1px solid rgba(224,85,85,0.2)',
                color: 'rgba(224,85,85,0.75)',
              }}
            >
              {resultText() || t('agent.sub.error')}
            </div>
          </Show>

          {/* 最终结果 */}
          <Show when={isComplete() && resultText()}>
            <div
              class="mx-4 mt-3 mb-4 rounded-lg px-3 py-2.5"
              style={{
                background: 'rgba(var(--text-base-rgb),0.04)',
                border: '1px solid rgba(var(--text-base-rgb),0.13)',
                'backdrop-filter': 'blur(6px)',
              }}
            >
              <div class="flex items-center gap-1.5 text-[11px] font-medium text-white/40 mb-1.5">
                <Icon name="check-circle" size={11} />
                {t('agent.subagent.result')}
              </div>
              <div class="text-[13px] leading-relaxed break-words text-white/75">
                <Markdown content={resultText()} />
              </div>
            </div>
          </Show>

          {/* 无任何步骤时的空态 */}
          <Show when={renderItems().length === 0 && !isComplete()}>
            <div class="flex items-center justify-center py-8 text-[12px] text-white/25 italic">
              {isError() ? t('agent.sub.error') : t('agent.sub.working')}
            </div>
          </Show>
        </Show>
      </div>
    </div>
  );
};

export default SubagentConversationView;

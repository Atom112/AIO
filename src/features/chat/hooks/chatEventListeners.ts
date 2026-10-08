// ChatPage 的事件监听器注册集合（18 个 listen）—— 从 ChatPage.onMount 抽出。
//
// 抽取策略（低风险）：函数顶部把依赖从 deps 解构，监听器主体与原代码**逐字一致**；
// 行为不变，仅把 ~740 行的注册块移出组件，便于独立测试与审阅。
// deps 里的每一项都来自 ChatPage 组件作用域（Solid 的 setter/signal 是稳定引用，可安全传递）。

import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { EVENTS } from '../../../core/api/events';
import { t } from '../../../core/i18n';
import type { Setter } from 'solid-js';
import { reasoningLevel } from '../../../core/store/store';
import {
  setDatas as storeSetDatas,
  type ActivatedModel,
  type AgentStep,
  type DatasState,
  type Message,
  type Topic,
  type WorkflowState,
} from '../../../core/store/store';
import type { PendingApproval } from '../components/ToolApprovalBubble';
import { TOOL_INTERRUPTED } from '../constants';

/** ChatPage 组件提供给监听器集合的依赖包（真实类型：保证 setDatas 回调的参数也被类型检查）。 */
export interface ChatEventDeps {
  setDatas: typeof storeSetDatas;
  datas: DatasState;
  setIsDragging: Setter<boolean>;
  setIsThinking: Setter<boolean>;
  setPendingApprovals: Setter<PendingApproval[]>;
  setTypingIndex: Setter<number | null>;
  setWorkflowState: Setter<WorkflowState | null>;
  handleFileUpload: (path: string, kind: 'file' | 'image') => Promise<void>;
  selectedModel: () => ActivatedModel | null;
  saveSingleAssistantToBackend: (id: string) => Promise<void>;
  checkAndRename: (eventAsstId: string, eventTopicId: string) => Promise<void>;
  checkAndSummarize: (keepBudgetOverride?: number) => Promise<void>;
}

/**
 * 注册 ChatPage 的全部事件监听器，返回 unlisten Promise 数组（onCleanup 时逐个调用）。
 * 注意：listener 的注册先于 invoke 调用，避免丢失首个 chunk 的竞态。
 */
export function registerChatEventListeners(deps: ChatEventDeps): Promise<UnlistenFn>[] {
  // 解构依赖：与监听器主体使用的名字一一对应
  const {
    setDatas,
    setIsDragging,
    setIsThinking,
    setPendingApprovals,
    setTypingIndex,
    setWorkflowState,
    handleFileUpload,
    datas,
    selectedModel,
    saveSingleAssistantToBackend,
    checkAndRename,
    checkAndSummarize,
  } = deps;

  // 流式内容批量合并状态（50ms 窗口）—— 从组件作用域随迁（仅本模块监听器使用）。
  let streamBatch: {
    assistant_id: string;
    topic_id: string;
    content: string;
    agentStepsContent: string;
    lastIdx: number;
  } | null = null;
  let streamBatchRAF: number | undefined;

  return [
    listen('tauri://drag-enter', () => setIsDragging(true)),
    listen('tauri://drag-leave', () => setIsDragging(false)),
    listen<{ paths: string[] }>('tauri://drag-drop', async (e) => {
      setIsDragging(false);
      for (const p of e.payload.paths) await handleFileUpload(p, 'file');
    }),
    // 新一轮 LLM 调用开始：多轮 Agent 工作时复用同一条 assistant 消息
    listen<any>(EVENTS.llmRoundStart, (e) => {
      const { assistant_id, topic_id, round } = e.payload;

      // 后续轮次：关闭上一轮的步骤，准备新轮次（不再整体压扁到 interimContent）
      if (round > 1) {
        const asst = datas.assistants.find((a) => a.id === assistant_id);
        const topic = asst?.topics.find((t: Topic) => t.id === topic_id);
        if (topic) {
          const lastIdx = topic.history.length - 1;
          const lastMsg = topic.history[lastIdx];
          if (lastMsg?.role === 'assistant') {
            // 关闭当前运行的步骤
            const now = Date.now();
            setDatas(
              'assistants',
              (a) => a.id === assistant_id,
              'topics',
              (t) => t.id === topic_id,
              'history',
              lastIdx,
              'agentSteps',
              (steps: any[] = []) => {
                if (steps.length === 0) return steps;
                const last = steps[steps.length - 1];
                if (last.status !== 'running') return steps;
                return [
                  ...steps.slice(0, -1),
                  { ...last, status: 'complete', duration: now - last.timestamp },
                ];
              },
            );
            // 重置 content / reasoning，准备接收新轮次输出
            setDatas(
              'assistants',
              (a) => a.id === assistant_id,
              'topics',
              (t) => t.id === topic_id,
              'history',
              lastIdx,
              {
                content: '',
                reasoning: '',
              },
            );
            // 打字机索引保持不变（仍指向同一条消息）
            return;
          }
        }
      }

      // 第一轮：创建新 assistant 占位消息
      const currentMdl = selectedModel();
      setDatas(
        'assistants',
        (a) => a.id === assistant_id,
        'topics',
        (t) => t.id === topic_id,
        'history',
        (h) => [
          ...h,
          {
            id: crypto.randomUUID(),
            role: 'assistant' as const,
            content: '',
            modelId: currentMdl?.model_id,
            reasoning: '',
            agentStartTime: Date.now(),
            agentSteps: [],
          },
        ],
      );
      // 设置打字机索引为新 assistant 消息位置
      const asst = datas.assistants.find((a) => a.id === assistant_id);
      const topic = asst?.topics.find((t: Topic) => t.id === topic_id);
      if (topic) setTypingIndex(topic.history.length - 1);
    }),
    listen<any>(EVENTS.llmChunk, async (e) => {
      const {
        assistant_id,
        topic_id,
        content,
        done,
        error,
        input_tokens,
        output_tokens,
        context_tokens,
        cached_input_tokens,
        images,
      } = e.payload;
      if (done) {
        // 刷新可能残余的 rAF 批量内容
        if (streamBatchRAF !== undefined) {
          clearTimeout(streamBatchRAF);
          streamBatchRAF = undefined;
        }
        if (streamBatch) {
          const batch = streamBatch;
          streamBatch = null;
          setDatas(
            'assistants',
            (a) => a.id === batch.assistant_id,
            'topics',
            (t) => t.id === batch.topic_id,
            'history',
            batch.lastIdx,
            'content',
            (old) => (typeof old === 'string' ? old : '') + batch.content,
          );
        }
        // 整轮真正结束（后端 run_agent_turn epilogue 唯一发出 done）
        if (error) {
          // 错误：追加错误文本到最后一条 assistant 消息
          const asst = datas.assistants.find((a) => a.id === assistant_id);
          const topic = asst?.topics.find((t: Topic) => t.id === topic_id);
          if (topic) {
            const lastIdx = topic.history.length - 1;
            if (lastIdx >= 0) {
              setDatas(
                'assistants',
                (a) => a.id === assistant_id,
                'topics',
                (t) => t.id === topic_id,
                'history',
                lastIdx,
                'content',
                (old) => (typeof old === 'string' ? old : '') + content,
              );
            }
          }
        }
        // 关闭未完结状态：最后一个 running 步骤、以及中断产生的孤儿 toolCalls。
        // 用户点停止时后端 cancel 直接 break，不会为在途工具发 llm-tool-result，
        // 导致 msg.toolCalls 中残留 state==='calling' 的条目。下一次请求重建 API 载荷时，
        // 这些条目会被 filter 跳过，assistant.tool_calls 失去配对的 role:tool 消息，
        // 触发 API 400（missing field tool_call_id）。这里统一标记为 error 并写明中断原因，
        // 重建逻辑会自动为它们生成配对的 tool 消息，模型也能感知“上次操作被中断”。
        const finishTime = Date.now();
        setDatas(
          'assistants',
          (a: any) => a.id === assistant_id,
          'topics',
          (t: any) => t.id === topic_id,
          'history',
          (h: any[]) => {
            const lastIdx = h.length - 1;
            if (lastIdx < 0 || h[lastIdx]?.role !== 'assistant') return h;
            const msg = h[lastIdx];
            const steps: any[] = msg.agentSteps || [];
            const toolCalls: any[] = msg.toolCalls || [];
            const updatedMsg: any = { ...msg };
            // 保存 token 用量
            if (input_tokens != null) updatedMsg.inputTokens = input_tokens;
            if (output_tokens != null) updatedMsg.outputTokens = output_tokens;
            if (context_tokens != null) updatedMsg.contextTokens = context_tokens;
            if (cached_input_tokens != null) updatedMsg.cachedInputTokens = cached_input_tokens;
            // 模型生成的图像元数据（下载按钮）+ 后端重写（去 token 标记）后的最终文本
            if (images) updatedMsg.images = images;
            if (!error && content) updatedMsg.content = content;
            // 孤儿 toolCalls：仍为 calling 的条目标记为中断错误
            if (toolCalls.some((tc: any) => tc.state === 'calling')) {
              updatedMsg.toolCalls = toolCalls.map((tc: any) =>
                tc.state === 'calling' ? { ...tc, state: 'error', error: TOOL_INTERRUPTED } : tc,
              );
            }
            if (steps.length === 0) {
              return [...h.slice(0, lastIdx), updatedMsg];
            }
            return [
              ...h.slice(0, lastIdx),
              {
                ...updatedMsg,
                agentSteps: steps.map((s: any, i: number) => {
                  if (s.status !== 'running') return s;
                  // 中断的 tool_call 步骤与其 toolCall 状态保持一致（标记 error）；
                  // 其余 running 步骤（含最后一个）按完成关闭
                  if (s.type === 'tool_call' && s.toolCall?.state === 'calling') {
                    return {
                      ...s,
                      status: 'error',
                      duration: finishTime - s.timestamp,
                      toolCall: { ...s.toolCall, state: 'error', error: TOOL_INTERRUPTED },
                    };
                  }
                  return i === steps.length - 1
                    ? { ...s, status: 'complete', duration: finishTime - s.timestamp }
                    : s;
                }),
              },
            ];
          },
        );
        setIsThinking(false);
        saveSingleAssistantToBackend(assistant_id);
        setTypingIndex(null);
        // 串行执行：先尝试自动重命名（非默认话题的首次对话），再触发历史压缩总结
        setTimeout(async () => {
          await checkAndRename(assistant_id, topic_id);
          await checkAndSummarize();
        }, 500);
        return;
      }

      // 流式数据追加：rAF 批量合并，避免逐 token 的打字机效果
      const asst = datas.assistants.find((a) => a.id === assistant_id);
      const topic = asst?.topics.find((t: Topic) => t.id === topic_id);
      if (topic) {
        const lastIdx = topic.history.length - 1;
        if (lastIdx >= 0) {
          if (!streamBatch) {
            streamBatch = {
              assistant_id,
              topic_id,
              content,
              agentStepsContent: content,
              lastIdx,
            };
            streamBatchRAF = window.setTimeout(() => {
              const batch = streamBatch!;
              streamBatch = null;
              streamBatchRAF = undefined;
              // 一次性应用批量内容（50ms 窗口内累积的所有 token）
              setDatas(
                'assistants',
                (a) => a.id === batch.assistant_id,
                'topics',
                (t) => t.id === batch.topic_id,
                'history',
                batch.lastIdx,
                'content',
                (old) => (typeof old === 'string' ? old : '') + batch.content,
              );
              if (batch.agentStepsContent) {
                const batchStepsContent = batch.agentStepsContent;
                setDatas(
                  'assistants',
                  (a) => a.id === batch.assistant_id,
                  'topics',
                  (t) => t.id === batch.topic_id,
                  'history',
                  batch.lastIdx,
                  'agentSteps',
                  (steps: any[] = []) => {
                    const lastStep = steps[steps.length - 1];
                    if (lastStep && lastStep.type === 'content' && lastStep.status === 'running') {
                      return [
                        ...steps.slice(0, -1),
                        {
                          ...lastStep,
                          contentText: (lastStep.contentText || '') + batchStepsContent,
                        },
                      ];
                    }
                    const now = Date.now();
                    const closed =
                      lastStep && lastStep.status === 'running'
                        ? [
                            ...steps.slice(0, -1),
                            {
                              ...lastStep,
                              status: 'complete',
                              duration: now - lastStep.timestamp,
                            },
                          ]
                        : steps;
                    return [
                      ...closed,
                      {
                        id: crypto.randomUUID(),
                        type: 'content',
                        timestamp: now,
                        status: 'running',
                        contentText: batchStepsContent,
                      },
                    ];
                  },
                );
              }
            }, 80) as unknown as number;
          } else {
            streamBatch.content += content;
            streamBatch.agentStepsContent += content;
          }
        }
      }
    }),
    // 思维链片段追加：原生 reasoning_content 流式累积到对应消息的 reasoning 字段
    // 同时构建 agentSteps 时间线中的 thinking 步骤
    listen<any>(EVENTS.llmReasoning, (e) => {
      if (reasoningLevel() === 'off') return;
      const { assistant_id, topic_id, content } = e.payload;
      const asst = datas.assistants.find((a) => a.id === assistant_id);
      const topic = asst?.topics.find((t: Topic) => t.id === topic_id);
      if (topic) {
        const lastIdx = topic.history.length - 1;
        if (lastIdx >= 0) {
          // 追加到 reasoning 字段（保持向后兼容）
          setDatas(
            'assistants',
            (a) => a.id === assistant_id,
            'topics',
            (t) => t.id === topic_id,
            'history',
            lastIdx,
            'reasoning',
            (old) => (typeof old === 'string' ? old : '') + content,
          );
          // 构建 agentSteps 时间线：若最后一步不是 thinking 则新建，否则追加
          setDatas(
            'assistants',
            (a) => a.id === assistant_id,
            'topics',
            (t) => t.id === topic_id,
            'history',
            lastIdx,
            'agentSteps',
            (steps: any[] = []) => {
              const lastStep = steps[steps.length - 1];
              if (lastStep && lastStep.type === 'thinking' && lastStep.status === 'running') {
                // 追加到当前 thinking 步骤
                return [
                  ...steps.slice(0, -1),
                  {
                    ...lastStep,
                    thinkingText: (lastStep.thinkingText || '') + content,
                  },
                ];
              }
              // 关闭上一步（如果仍在运行），新建 thinking 步骤
              const now = Date.now();
              const closed =
                lastStep && lastStep.status === 'running'
                  ? [
                      ...steps.slice(0, -1),
                      { ...lastStep, status: 'complete', duration: now - lastStep.timestamp },
                    ]
                  : steps;
              return [
                ...closed,
                {
                  id: crypto.randomUUID(),
                  type: 'thinking',
                  timestamp: now,
                  status: 'running',
                  thinkingText: content,
                },
              ];
            },
          );
        }
      }
    }),
    // LLM 工具调用事件：展示"调用中"气泡并构建 agentSteps 时间线
    listen<any>(EVENTS.llmToolCall, (e) => {
      const { assistant_id, topic_id, tool_call_id, name, arguments: argsJson } = e.payload;
      const newTc = {
        id: tool_call_id,
        type: 'function',
        function: { name, arguments: argsJson },
        state: 'calling',
      };
      setDatas(
        'assistants',
        (a: any) => a.id === assistant_id,
        'topics',
        (t: Topic) => t.id === topic_id,
        'history',
        (h: any[]) => {
          const lastIdx = h.length - 1;
          if (lastIdx >= 0 && h[lastIdx]?.role === 'assistant') {
            const msg = h[lastIdx];
            // 保持 toolCalls 数组（消息重建用）
            const existing = msg.toolCalls || [];
            if (existing.find((tc: any) => tc.id === tool_call_id)) return h;
            const newToolCalls = [...existing, newTc];
            // 构建 agentSteps：关闭上一步，新建 tool_call 步骤
            const now = Date.now();
            const steps: any[] = msg.agentSteps || [];
            const lastStep = steps[steps.length - 1];
            const closed =
              lastStep && lastStep.status === 'running'
                ? [
                    ...steps.slice(0, -1),
                    { ...lastStep, status: 'complete', duration: now - lastStep.timestamp },
                  ]
                : steps;
            const newSteps = [
              ...closed,
              {
                id: crypto.randomUUID(),
                type: 'tool_call',
                timestamp: now,
                status: 'running',
                toolCall: newTc,
              },
            ];
            return [
              ...h.slice(0, lastIdx),
              { ...msg, toolCalls: newToolCalls, agentSteps: newSteps },
            ];
          }
          return h;
        },
      );
    }),
    // 工具执行结果：更新 assistant 消息中对应 toolCall 的状态与 agentSteps 时间线
    listen<any>(EVENTS.llmToolResult, (e) => {
      const {
        assistant_id,
        topic_id,
        tool_call_id,
        content,
        result,
        is_error,
        file_changes,
        full_content,
      } = e.payload;
      setDatas(
        'assistants',
        (a: any) => a.id === assistant_id,
        'topics',
        (t: Topic) => t.id === topic_id,
        'history',
        (h: any[]) =>
          h.map((m: any) => {
            if (m.role !== 'assistant' || !m.toolCalls) return m;
            const newToolCalls = m.toolCalls.map((tc: any) =>
              tc.id === tool_call_id
                ? {
                    ...tc,
                    state: is_error ? 'error' : 'success',
                    result,
                    content,
                    fullContent: full_content ?? content,
                    error: is_error ? content : undefined,
                    fileChanges: file_changes,
                  }
                : tc,
            );
            // 同步更新 agentSteps 中对应 tool_call 步骤
            const now = Date.now();
            const newSteps = (m.agentSteps || []).map((s: any) => {
              if (s.type === 'tool_call' && s.toolCall?.id === tool_call_id) {
                return {
                  ...s,
                  status: is_error ? 'error' : 'complete',
                  fullResult: full_content ?? s.fullResult,
                  duration: now - s.timestamp,
                  toolCall: {
                    ...s.toolCall,
                    state: is_error ? 'error' : 'success',
                    result,
                    content,
                    fullContent: full_content ?? content,
                    error: is_error ? content : undefined,
                    fileChanges: file_changes,
                  },
                };
              }
              return s;
            });
            return { ...m, toolCalls: newToolCalls, agentSteps: newSteps };
          }),
      );
    }),
    // 运行内上下文压缩事件：仅主 Agent 压缩会更新会话记忆——
    // 覆盖式更新 topic.summary 并按 token 预算裁剪历史，使后续轮次不再全量重发已压缩部分（P1-5）
    listen<any>(EVENTS.llmCompression, (e) => {
      const { assistant_id, topic_id, source, summary } = e.payload;
      if (!assistant_id || !topic_id) return;
      if (source === 'subagent') return; // 子智能体上下文压缩与主会话无关
      if (!summary || typeof summary !== 'string' || summary.trim() === '') return;
      const asst = datas.assistants.find((a: any) => a.id === assistant_id);
      const topic = asst?.topics.find((t: Topic) => t.id === topic_id);
      if (!topic) return;
      // 与后端 keep_recent_tokens 对齐：从尾部累计约 2 万 token，至少保留最近 4 条（一个完整回合）
      const KEEP_TOKEN_BUDGET = 20_000;
      const estTokens = (m: any): number => {
        const text = typeof m.content === 'string' ? m.content : (m.displayText ?? '');
        return Math.ceil(text.length / 4) + 8;
      };
      let keepCount = 0;
      let acc = 0;
      const hist = topic.history;
      for (let i = hist.length - 1; i >= 0; i--) {
        acc += estTokens(hist[i]);
        keepCount = hist.length - i;
        if (acc >= KEEP_TOKEN_BUDGET) break;
      }
      keepCount = Math.max(keepCount, 4);
      const trimmed = keepCount < hist.length ? hist.slice(hist.length - keepCount) : hist;
      if (trimmed.length === hist.length && topic.summary === summary) return;
      setDatas(
        'assistants',
        (a: any) => a.id === assistant_id,
        'topics',
        (t: Topic) => t.id === topic_id,
        { history: trimmed, summary },
      );
      saveSingleAssistantToBackend(assistant_id);
    }),
    // 未完成任务已生成交接文档：在最后一条 assistant 消息的时间线中记录可追溯的交接步骤
    // （后端同时会在消息文本末尾追加说明；此监听提供结构化记录与文档路径）
    listen<{ assistant_id: string; topic_id: string; path: string; reason: string }>(
      EVENTS.llmHandoff,
      (e) => {
        const { assistant_id, topic_id, path } = e.payload;
        if (!assistant_id || !topic_id || !path) return;
        setDatas(
          'assistants',
          (a: DatasState['assistants'][number]) => a.id === assistant_id,
          'topics',
          (t: Topic) => t.id === topic_id,
          'history',
          (h: Message[]) => {
            const lastIdx = h.length - 1;
            if (lastIdx < 0 || h[lastIdx]?.role !== 'assistant') return h;
            const msg = h[lastIdx];
            const steps: AgentStep[] = msg.agentSteps ?? [];
            const newStep: AgentStep = {
              id: 'handoff-' + Date.now(),
              type: 'content',
              timestamp: Date.now(),
              status: 'complete',
              contentText: t('agent.handoff.note', { path }),
            };
            return [...h.slice(0, lastIdx), { ...msg, agentSteps: [...steps, newStep] }];
          },
        );
      },
    ),
    // 工具调用审批请求事件：后端需要用户确认才能执行工具（字段 snake_case 与后端对齐）
    listen<any>(EVENTS.toolApprovalRequested, (e) => {
      const {
        approval_id,
        server_id,
        tool_name,
        arguments: args,
        reason,
        preview_diff,
        file_path,
      } = e.payload;
      setPendingApprovals((prev) => [
        ...prev,
        {
          approvalId: approval_id,
          serverId: server_id,
          toolName: tool_name,
          arguments: args,
          reason,
          previewDiff: preview_diff,
          filePath: file_path,
        },
      ]);
    }),

    // ===== 子智能体事件监听 =====

    // 子智能体启动：在当前 assistant 消息的 agentSteps 中推入一个 subagent 步骤
    listen<any>(EVENTS.subagentStart, (e) => {
      const {
        parent_assistant_id,
        parent_topic_id,
        subagent_id,
        profile_id,
        profile_name,
        task_summary,
      } = e.payload;
      setDatas(
        'assistants',
        (a: any) => a.id === parent_assistant_id,
        'topics',
        (t: any) => t.id === parent_topic_id,
        'history',
        (h: any[]) => {
          const lastIdx = h.length - 1;
          if (lastIdx < 0 || h[lastIdx]?.role !== 'assistant') return h;
          const msg = h[lastIdx];
          const steps: any[] = msg.agentSteps || [];
          const newStep = {
            id: `subagent-${subagent_id}`,
            type: 'subagent',
            timestamp: Date.now(),
            status: 'running',
            subagentId: subagent_id,
            subagentProfile: profile_id,
            subagentName: profile_name,
            subagentTask: task_summary,
            subagentSteps: [],
            subagentResult: '',
          };
          return [...h.slice(0, lastIdx), { ...msg, agentSteps: [...steps, newStep] }];
        },
      );
    }),

    // 子智能体步骤更新：追加子 Agent 内部步骤
    listen<any>(EVENTS.subagentStep, (e) => {
      const { parent_assistant_id, parent_topic_id, subagent_id, round, step_type, summary } =
        e.payload;
      setDatas(
        'assistants',
        (a: any) => a.id === parent_assistant_id,
        'topics',
        (t: any) => t.id === parent_topic_id,
        'history',
        (h: any[]) => {
          const lastIdx = h.length - 1;
          if (lastIdx < 0 || h[lastIdx]?.role !== 'assistant') return h;
          const msg = h[lastIdx];
          const steps: any[] = msg.agentSteps || [];
          const subIdx = steps.findIndex((s: any) => s.subagentId === subagent_id);
          if (subIdx < 0) return h;
          const subStep = steps[subIdx];
          const newSubStep = {
            id: `substep-${round}-${Date.now()}`,
            type: (step_type || '').startsWith('tool_result')
              ? ('tool_call' as const)
              : (step_type as any),
            timestamp: Date.now(),
            status: 'complete' as const,
            summary: summary || '',
            toolName: (step_type || '').startsWith('tool_result')
              ? (step_type as string).replace('tool_result:', '')
              : undefined,
          };
          const updatedSub = {
            ...subStep,
            subagentSteps: [...(subStep.subagentSteps || []), newSubStep],
          };
          return [
            ...h.slice(0, lastIdx),
            {
              ...msg,
              agentSteps: steps.map((s: any, i: number) => (i === subIdx ? updatedSub : s)),
            },
          ];
        },
      );
    }),

    // 子智能体完成：标记 subagent 步骤为完成，记录结果
    listen<any>(EVENTS.subagentDone, (e) => {
      const { parent_assistant_id, parent_topic_id, subagent_id, result } = e.payload;
      const finishTime = Date.now();
      setDatas(
        'assistants',
        (a: any) => a.id === parent_assistant_id,
        'topics',
        (t: any) => t.id === parent_topic_id,
        'history',
        (h: any[]) => {
          const lastIdx = h.length - 1;
          if (lastIdx < 0 || h[lastIdx]?.role !== 'assistant') return h;
          const msg = h[lastIdx];
          const steps: any[] = msg.agentSteps || [];
          const subIdx = steps.findIndex((s: any) => s.subagentId === subagent_id);
          if (subIdx < 0) return h;
          const subStep = steps[subIdx];
          const updatedSub = {
            ...subStep,
            status: 'complete',
            duration: finishTime - subStep.timestamp,
            subagentResult: result || '',
          };
          return [
            ...h.slice(0, lastIdx),
            {
              ...msg,
              agentSteps: steps.map((s: any, i: number) => (i === subIdx ? updatedSub : s)),
            },
          ];
        },
      );
    }),

    // 子智能体出错
    listen<any>(EVENTS.subagentError, (e) => {
      const { parent_assistant_id, parent_topic_id, subagent_id, error } = e.payload;
      setDatas(
        'assistants',
        (a: any) => a.id === parent_assistant_id,
        'topics',
        (t: any) => t.id === parent_topic_id,
        'history',
        (h: any[]) => {
          const lastIdx = h.length - 1;
          if (lastIdx < 0 || h[lastIdx]?.role !== 'assistant') return h;
          const msg = h[lastIdx];
          const steps: any[] = msg.agentSteps || [];
          const subIdx = steps.findIndex((s: any) => s.subagentId === subagent_id);
          if (subIdx < 0) return h;
          const subStep = steps[subIdx];
          const updatedSub = {
            ...subStep,
            status: 'error',
            duration: Date.now() - subStep.timestamp,
            subagentResult: error || t('agent.sub.error'),
          };
          return [
            ...h.slice(0, lastIdx),
            {
              ...msg,
              agentSteps: steps.map((s: any, i: number) => (i === subIdx ? updatedSub : s)),
            },
          ];
        },
      );
    }),
    // ---- 工作流事件 ----
    listen<WorkflowState>(EVENTS.workflowStart, (e) => {
      setWorkflowState({
        workflowId: e.payload.workflowId,
        title: e.payload.title,
        steps: e.payload.steps.map((s) => ({ ...s, status: 'pending' as const })),
        active: true,
      });
    }),
    listen<{ stepId: string }>(EVENTS.workflowStepStart, (e) => {
      setWorkflowState((prev) =>
        prev
          ? {
              ...prev,
              steps: prev.steps.map((s) =>
                s.stepId === e.payload.stepId
                  ? { ...s, status: 'running' as const, startedAt: Date.now() }
                  : s,
              ),
            }
          : null,
      );
    }),
    listen<{ stepId: string; status: 'completed' | 'failed'; duration?: number }>(
      'workflow-step-complete',
      (e) => {
        setWorkflowState((prev) =>
          prev
            ? {
                ...prev,
                steps: prev.steps.map((s) =>
                  s.stepId === e.payload.stepId
                    ? { ...s, status: e.payload.status, duration: e.payload.duration }
                    : s,
                ),
              }
            : null,
        );
      },
    ),
    listen<{ workflowId: string }>(EVENTS.workflowComplete, () => {
      setWorkflowState((prev) => (prev ? { ...prev, active: false } : null));
      setTimeout(() => setWorkflowState(null), 5000);
    }),
  ];
}

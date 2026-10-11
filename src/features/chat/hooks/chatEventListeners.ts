// Event listener registration set for the chat UI.
//
// CHAT-12: the STREAM listeners (llm-chunk / reasoning / tool events / subagents /
// workflow / approvals …) register from the root Layout for the whole app lifetime —
// they used to live in ChatPage, so leaving the page mid-generation destroyed them
// while the backend kept streaming: chunks never reached the store, the `done`
// handler (which persists the assistant) never ran, and the turn was lost.
// Only the drag-and-drop UI trio stays page-scoped (registerChatUiListeners).
//
// Every state piece these listeners touch is a module-level store singleton
// (core/store/store + core/store/stream), so no page component needs to outlive them.

import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { EVENTS } from '../../../core/api/events';
import type {
  LlmRoundStartPayload,
  LlmChunkPayload,
  LlmReasoningPayload,
  LlmToolCallPayload,
  LlmToolResultPayload,
  LlmCompressionPayload,
  ToolApprovalRequestedPayload,
  SubagentStartPayload,
  SubagentStepPayload,
  SubagentDonePayload,
  SubagentErrorPayload,
} from '../../../core/api/streamPayloads';
import { t } from '../../../core/i18n';
import type { Setter } from 'solid-js';
import { reasoningLevel } from '../../../core/store/store';
import {
  datas,
  saveSingleAssistantToBackend,
  selectedModel,
  setAgentTaskBoard,
  setDatas,
  setPendingApprovals,
  setPendingQuestions,
  setIsThinking,
  setTypingIndex,
  setWorkflowState,
  type AgentStep,
  type AgentTaskState,
  type DatasState,
  type Message,
  type SubagentStep,
  type Topic,
  type ToolCallDisplay,
  type WorkflowState,
} from '../../../core/store/store';
import { checkAndRename, checkAndSummarize } from './chatTurnEffects';
import { TOOL_INTERRUPTED } from '../constants';

/**
 * Stream listeners: registered once from the root Layout, alive for the whole app.
 * Returns the unlisten promises (Layout never unmounts; kept for symmetry).
 */
export function registerChatStreamListeners(): Promise<UnlistenFn>[] {
  // ===== Stream content batching (CHAT-7 / CHAT-9) =====
  // Keyed per stream (assistant::topic): interleaved streams previously shared ONE
  // batch slot, so a second stream's tokens appended into the first stream's message.
  // The target message is captured by ID and re-resolved at flush time — a snapshot
  // array index could silently redirect the append after a compression trim.
  // Reasoning chunks (CHAT-9) share the same batch: one flush applies content +
  // reasoning + the agentSteps timeline instead of per-chunk setDatas storms.
  interface StreamBatch {
    assistantId: string;
    topicId: string;
    messageId: string;
    content: string;
    agentStepsContent: string;
    reasoning: string;
    timer: number;
  }
  const streamBatches = new Map<string, StreamBatch>();
  const FLUSH_MS = 80;
  const streamKey = (assistantId: string, topicId: string) => `${assistantId}::${topicId}`;

  // CHAT-9: shared timeline appender (thinking/content step runs)
  const appendTimelineStep = (
    steps: AgentStep[],
    type: 'thinking' | 'content',
    textField: 'thinkingText' | 'contentText',
    text: string,
  ): AgentStep[] => {
    const lastStep = steps[steps.length - 1];
    if (lastStep && lastStep.type === type && lastStep.status === 'running') {
      return [
        ...steps.slice(0, -1),
        { ...lastStep, [textField]: (lastStep[textField] || '') + text },
      ];
    }
    const now = Date.now();
    const closed: AgentStep[] =
      lastStep && lastStep.status === 'running'
        ? [
            ...steps.slice(0, -1),
            { ...lastStep, status: 'complete', duration: now - lastStep.timestamp },
          ]
        : steps;
    return [
      ...closed,
      { id: crypto.randomUUID(), type, timestamp: now, status: 'running', [textField]: text },
    ];
  };

  const flushStreamBatch = (key: string) => {
    const batch = streamBatches.get(key);
    if (!batch) return;
    streamBatches.delete(key);
    window.clearTimeout(batch.timer);
    // CHAT-7: resolve the target message FRESH by id at flush time (the history may
    // have been trimmed/shifted since the chunk arrived)
    const asst = datas.assistants.find((a) => a.id === batch.assistantId);
    const topic = asst?.topics.find((tp) => tp.id === batch.topicId);
    const idx = topic?.history.findIndex((m) => m.id === batch.messageId) ?? -1;
    if (!topic || idx < 0) return; // message gone (mid-window compaction) — drop
    if (batch.content) {
      setDatas(
        'assistants',
        (a) => a.id === batch.assistantId,
        'topics',
        (tp) => tp.id === batch.topicId,
        'history',
        idx,
        'content',
        (old) => (typeof old === 'string' ? old : '') + batch.content,
      );
    }
    if (batch.reasoning) {
      setDatas(
        'assistants',
        (a) => a.id === batch.assistantId,
        'topics',
        (tp) => tp.id === batch.topicId,
        'history',
        idx,
        'reasoning',
        (old) => (typeof old === 'string' ? old : '') + batch.reasoning,
      );
    }
    if (batch.agentStepsContent || batch.reasoning) {
      setDatas(
        'assistants',
        (a) => a.id === batch.assistantId,
        'topics',
        (tp) => tp.id === batch.topicId,
        'history',
        idx,
        'agentSteps',
        (steps: AgentStep[] = []) => {
          let out = steps;
          if (batch.reasoning)
            out = appendTimelineStep(out, 'thinking', 'thinkingText', batch.reasoning);
          if (batch.agentStepsContent)
            out = appendTimelineStep(out, 'content', 'contentText', batch.agentStepsContent);
          return out;
        },
      );
    }
  };

  // Append one chunk (content or reasoning) to the stream's batch, creating the batch
  // (and its flush timer) when absent. If the target message changed mid-window (new
  // round/turn reset the message), the stale batch flushes first.
  const pushToBatch = (
    assistantId: string,
    topicId: string,
    piece: 'content' | 'reasoning',
    text: string,
  ) => {
    const asst = datas.assistants.find((a) => a.id === assistantId);
    const topic = asst?.topics.find((tp) => tp.id === topicId);
    if (!topic) return;
    const lastIdx = topic.history.length - 1;
    if (lastIdx < 0) return;
    const messageId = topic.history[lastIdx]?.id;
    if (!messageId) return;
    const key = streamKey(assistantId, topicId);
    let batch = streamBatches.get(key);
    if (batch && batch.messageId !== messageId) {
      flushStreamBatch(key);
      batch = undefined;
    }
    if (batch) {
      if (piece === 'content') {
        batch.content += text;
        batch.agentStepsContent += text;
      } else {
        batch.reasoning += text;
      }
      return;
    }
    const fresh: StreamBatch = {
      assistantId,
      topicId,
      messageId,
      content: piece === 'content' ? text : '',
      agentStepsContent: piece === 'content' ? text : '',
      reasoning: piece === 'reasoning' ? text : '',
      timer: 0,
    };
    fresh.timer = window.setTimeout(() => flushStreamBatch(key), FLUSH_MS) as unknown as number;
    streamBatches.set(key, fresh);
  };

  return [
    // A new LLM round starts: multi-round agent work reuses the same assistant message
    listen<LlmRoundStartPayload>(EVENTS.llmRoundStart, (e) => {
      const { assistant_id, topic_id, round } = e.payload;

      // Later rounds: close the previous round's steps and set up the new round (no longer flattened wholesale into interimContent)
      if (round > 1) {
        const asst = datas.assistants.find((a) => a.id === assistant_id);
        const topic = asst?.topics.find((t: Topic) => t.id === topic_id);
        if (topic) {
          const lastIdx = topic.history.length - 1;
          const lastMsg = topic.history[lastIdx];
          if (lastMsg?.role === 'assistant') {
            // Flush anything still batched for this stream before resetting the message
            flushStreamBatch(streamKey(assistant_id, topic_id));
            // Close the currently running step
            const now = Date.now();
            setDatas(
              'assistants',
              (a) => a.id === assistant_id,
              'topics',
              (t) => t.id === topic_id,
              'history',
              lastIdx,
              'agentSteps',
              (steps: AgentStep[] = []): AgentStep[] => {
                if (steps.length === 0) return steps;
                const last = steps[steps.length - 1];
                if (last.status !== 'running') return steps;
                return [
                  ...steps.slice(0, -1),
                  { ...last, status: 'complete', duration: now - last.timestamp },
                ];
              },
            );
            // Reset content / reasoning to receive the new round's output
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
            // Typewriter index stays unchanged (still pointing at the same message)
            return;
          }
        }
      }

      // First round: create a new assistant placeholder message
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
      // Point the typewriter index at the new assistant message
      const asst = datas.assistants.find((a) => a.id === assistant_id);
      const topic = asst?.topics.find((t: Topic) => t.id === topic_id);
      if (topic) setTypingIndex(topic.history.length - 1);
    }),
    listen<LlmChunkPayload>(EVENTS.llmChunk, async (e) => {
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
        // Flush this stream's residual batched content (CHAT-7: per-stream keying)
        flushStreamBatch(streamKey(assistant_id, topic_id));
        // The whole round is truly over (only the backend run_agent_turn epilogue emits done)
        if (error) {
          // Error: append the error text to the last assistant message
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
        // Close unfinished state: the last running step plus orphaned toolCalls left by an interrupt.
        // When the user hits stop, the backend cancel breaks without emitting llm-tool-result for
        // in-flight tools, leaving state==='calling' entries in msg.toolCalls. On the next request those
        // are filtered out, so assistant.tool_calls loses its paired role:tool messages and the API
        // rejects with 400 (missing field tool_call_id). Here they are uniformly marked as error with the
        // interrupt reason; the rebuild logic generates paired tool messages so the model sees the interruption.
        const finishTime = Date.now();
        setDatas(
          'assistants',
          (a) => a.id === assistant_id,
          'topics',
          (t) => t.id === topic_id,
          'history',
          (h) => {
            const lastIdx = h.length - 1;
            if (lastIdx < 0 || h[lastIdx]?.role !== 'assistant') return h;
            const msg = h[lastIdx];
            const steps: AgentStep[] = msg.agentSteps || [];
            const toolCalls: ToolCallDisplay[] = msg.toolCalls || [];
            const updatedMsg: Message = { ...msg };
            // Save token usage
            if (input_tokens != null) updatedMsg.inputTokens = input_tokens;
            if (output_tokens != null) updatedMsg.outputTokens = output_tokens;
            if (context_tokens != null) updatedMsg.contextTokens = context_tokens;
            if (cached_input_tokens != null) updatedMsg.cachedInputTokens = cached_input_tokens;
            // Model-generated image metadata (download button) + final text after the backend rewrite (token markers stripped)
            if (images) updatedMsg.images = images;
            if (!error && content) updatedMsg.content = content;
            // Orphaned toolCalls: entries still in calling are marked as interrupted errors
            if (toolCalls.some((tc) => tc.state === 'calling')) {
              updatedMsg.toolCalls = toolCalls.map((tc): ToolCallDisplay =>
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
                agentSteps: steps.map((s, i: number): AgentStep => {
                  if (s.status !== 'running') return s;
                  // Interrupted tool_call steps stay consistent with their toolCall state (marked error);
                  // other running steps (including the last) are closed as complete
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
        // Sequential: first try auto-rename (first exchange in a non-default topic), then trigger history compaction
        setTimeout(async () => {
          await checkAndRename(assistant_id, topic_id);
          await checkAndSummarize();
        }, 500);
        return;
      }

      // Streaming append: batched per stream (CHAT-7) and applied via rAF-like windows
      // (CHAT-9: one setDatas per flush instead of per chunk)
      pushToBatch(assistant_id, topic_id, 'content', content);
    }),
    // Chain-of-thought chunk: accumulated into the message's reasoning field and the
    // agentSteps timeline. CHAT-9: batched through the same per-stream flush window —
    // previously each reasoning chunk cost two setDatas plus a full steps rebuild.
    listen<LlmReasoningPayload>(EVENTS.llmReasoning, (e) => {
      if (reasoningLevel() === 'off') return;
      const { assistant_id, topic_id, content } = e.payload;
      pushToBatch(assistant_id, topic_id, 'reasoning', content);
    }),
    // LLM tool-call event: show the "calling" bubble and build the agentSteps timeline
    listen<LlmToolCallPayload>(EVENTS.llmToolCall, (e) => {
      const { assistant_id, topic_id, tool_call_id, name, arguments: argsJson } = e.payload;
      const newTc: ToolCallDisplay = {
        id: tool_call_id,
        type: 'function',
        function: { name, arguments: argsJson },
        state: 'calling',
      };
      setDatas(
        'assistants',
        (a) => a.id === assistant_id,
        'topics',
        (t) => t.id === topic_id,
        'history',
        (h) => {
          const lastIdx = h.length - 1;
          if (lastIdx >= 0 && h[lastIdx]?.role === 'assistant') {
            const msg = h[lastIdx];
            // Keep the toolCalls array (used when rebuilding messages)
            const existing = msg.toolCalls || [];
            if (existing.find((tc) => tc.id === tool_call_id)) return h;
            const newToolCalls = [...existing, newTc];
            // Build agentSteps: close the previous step and add a new tool_call step
            const now = Date.now();
            const steps: AgentStep[] = msg.agentSteps || [];
            const lastStep = steps[steps.length - 1];
            const closed: AgentStep[] =
              lastStep && lastStep.status === 'running'
                ? [
                    ...steps.slice(0, -1),
                    { ...lastStep, status: 'complete', duration: now - lastStep.timestamp },
                  ]
                : steps;
            const newSteps: AgentStep[] = [
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
    // Tool execution result: update the matching toolCall state on the assistant message and the agentSteps timeline
    listen<LlmToolResultPayload>(EVENTS.llmToolResult, (e) => {
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
        (a) => a.id === assistant_id,
        'topics',
        (t) => t.id === topic_id,
        'history',
        (h) =>
          h.map((m) => {
            if (m.role !== 'assistant' || !m.toolCalls) return m;
            // CHAT-6: 未命中即原样返回，保持引用稳定——<For> 按引用比较，此前
            // 每个工具事件都无条件产出新数组/新对象，整列消息行重挂载
            // （AgentProcessBlock 被迫长出模块级渲染队列对抗它）
            const hitToolCall = m.toolCalls.some((tc) => tc.id === tool_call_id);
            const hitStep = (m.agentSteps || []).some(
              (s) => s.type === 'tool_call' && s.toolCall?.id === tool_call_id,
            );
            if (!hitToolCall && !hitStep) return m;
            const newToolCalls = m.toolCalls.map((tc): ToolCallDisplay =>
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
            // Also update the matching tool_call step in agentSteps
            const now = Date.now();
            const newSteps = (m.agentSteps || []).map((s): AgentStep => {
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
    // In-run context compaction event: only main-agent compaction updates conversation memory —
    // topic.summary is overwritten and history trimmed to the token budget so later rounds stop resending the compacted part
    listen<LlmCompressionPayload>(EVENTS.llmCompression, (e) => {
      const { assistant_id, topic_id, source, summary } = e.payload;
      if (!assistant_id || !topic_id) return;
      if (source === 'subagent') return; // subagent context compaction is unrelated to the main conversation
      if (!summary || typeof summary !== 'string' || summary.trim() === '') return;
      const asst = datas.assistants.find((a) => a.id === assistant_id);
      const topic = asst?.topics.find((t: Topic) => t.id === topic_id);
      if (!topic) return;
      // Aligned with the backend keep_recent_tokens: accumulate ~20k tokens from the tail, keeping at least the last 4 messages (one full exchange)
      const KEEP_TOKEN_BUDGET = 20_000;
      const estTokens = (m: Message): number => {
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
        (a) => a.id === assistant_id,
        'topics',
        (t: Topic) => t.id === topic_id,
        { history: trimmed, summary },
      );
      saveSingleAssistantToBackend(assistant_id);
    }),
    // A handoff doc was generated for unfinished work: record a traceable handoff step in the last assistant message's timeline
    // (the backend also appends a note to the message text; this listener provides the structured record and the doc path)
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
    // Tool-call approval request event: the backend needs user confirmation before running a tool (fields are snake_case to match the backend)
    listen<ToolApprovalRequestedPayload>(EVENTS.toolApprovalRequested, (e) => {
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

    // MCP-17: the backend approval wait timed out (or the channel closed) — drop the
    // bubble instead of leaving it clickable until a press hits "does not exist".
    listen<{ approval_id: string }>(EVENTS.toolApprovalExpired, (e) => {
      const { approval_id } = e.payload;
      setPendingApprovals((prev) => prev.filter((a) => a.approvalId !== approval_id));
    }),

    // Agent task board changed (create_tasks / update_task): mirror the snapshot (TaskBoardPanel
    // matches it against the current topic)
    listen<{ assistant_id: string; topic_id: string; tasks: AgentTaskState[] }>(
      EVENTS.agentTasksChanged,
      (e) => {
        const { assistant_id, topic_id, tasks } = e.payload;
        setAgentTaskBoard({
          assistantId: assistant_id,
          topicId: topic_id,
          tasks: tasks.map((tk) => ({ id: tk.id, title: tk.title, status: tk.status })),
        });
      },
    ),

    // Agent question request (ask_user): the loop pauses until the user answers via
    // respond_agent_question (AgentQuestionBubble)
    listen<{
      request_id: string;
      questions: {
        question_id: string;
        question: string;
        options: { label: string; description?: string; recommended?: boolean }[];
        allow_custom?: boolean;
      }[];
    }>(EVENTS.agentQuestionRequested, (e) => {
      const { request_id, questions } = e.payload;
      setPendingQuestions((prev) => [
        ...prev,
        {
          requestId: request_id,
          questions: questions.map((q) => ({
            questionId: q.question_id,
            question: q.question,
            options: q.options.map((o) => ({
              label: o.label,
              description: o.description,
              recommended: o.recommended,
            })),
            allowCustom: q.allow_custom !== false,
          })),
        },
      ]);
    }),

    // ===== Subagent event listeners =====

    // Subagent start: push a subagent step into the current assistant message's agentSteps
    listen<SubagentStartPayload>(EVENTS.subagentStart, (e) => {
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
        (a) => a.id === parent_assistant_id,
        'topics',
        (t) => t.id === parent_topic_id,
        'history',
        (h) => {
          const lastIdx = h.length - 1;
          if (lastIdx < 0 || h[lastIdx]?.role !== 'assistant') return h;
          const msg = h[lastIdx];
          const steps: AgentStep[] = msg.agentSteps || [];
          const newStep: AgentStep = {
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

    // Subagent step update: append the sub-agent's internal steps
    listen<SubagentStepPayload>(EVENTS.subagentStep, (e) => {
      const { parent_assistant_id, parent_topic_id, subagent_id, round, step_type, summary } =
        e.payload;
      setDatas(
        'assistants',
        (a) => a.id === parent_assistant_id,
        'topics',
        (t) => t.id === parent_topic_id,
        'history',
        (h) => {
          const lastIdx = h.length - 1;
          if (lastIdx < 0 || h[lastIdx]?.role !== 'assistant') return h;
          const msg = h[lastIdx];
          const steps: AgentStep[] = msg.agentSteps || [];
          const subIdx = steps.findIndex((s) => s.subagentId === subagent_id);
          if (subIdx < 0) return h;
          const subStep = steps[subIdx];
          const newSubStep = {
            id: `substep-${round}-${Date.now()}`,
            type: (step_type || '').startsWith('tool_result')
              ? ('tool_call' as const)
              : (step_type as SubagentStep['type']),
            timestamp: Date.now(),
            status: 'complete' as const,
            summary: summary || '',
            toolName: (step_type || '').startsWith('tool_result')
              ? (step_type as string).replace('tool_result:', '')
              : undefined,
            round,
          };
          const updatedSub = {
            ...subStep,
            subagentSteps: [...(subStep.subagentSteps || []), newSubStep],
          };
          return [
            ...h.slice(0, lastIdx),
            {
              ...msg,
              agentSteps: steps.map((s, i: number) => (i === subIdx ? updatedSub : s)),
            },
          ];
        },
      );
    }),

    // Subagent done: mark the subagent step complete and record the result
    listen<SubagentDonePayload>(EVENTS.subagentDone, (e) => {
      const { parent_assistant_id, parent_topic_id, subagent_id, result } = e.payload;
      const finishTime = Date.now();
      setDatas(
        'assistants',
        (a) => a.id === parent_assistant_id,
        'topics',
        (t) => t.id === parent_topic_id,
        'history',
        (h) => {
          const lastIdx = h.length - 1;
          if (lastIdx < 0 || h[lastIdx]?.role !== 'assistant') return h;
          const msg = h[lastIdx];
          const steps: AgentStep[] = msg.agentSteps || [];
          const subIdx = steps.findIndex((s) => s.subagentId === subagent_id);
          if (subIdx < 0) return h;
          const subStep = steps[subIdx];
          const updatedSub: AgentStep = {
            ...subStep,
            status: 'complete',
            duration: finishTime - subStep.timestamp,
            subagentResult: result || '',
          };
          return [
            ...h.slice(0, lastIdx),
            {
              ...msg,
              agentSteps: steps.map((s, i: number) => (i === subIdx ? updatedSub : s)),
            },
          ];
        },
      );
    }),

    // Subagent error
    listen<SubagentErrorPayload>(EVENTS.subagentError, (e) => {
      const { parent_assistant_id, parent_topic_id, subagent_id, error } = e.payload;
      setDatas(
        'assistants',
        (a) => a.id === parent_assistant_id,
        'topics',
        (t) => t.id === parent_topic_id,
        'history',
        (h) => {
          const lastIdx = h.length - 1;
          if (lastIdx < 0 || h[lastIdx]?.role !== 'assistant') return h;
          const msg = h[lastIdx];
          const steps: AgentStep[] = msg.agentSteps || [];
          const subIdx = steps.findIndex((s) => s.subagentId === subagent_id);
          if (subIdx < 0) return h;
          const subStep = steps[subIdx];
          const updatedSub: AgentStep = {
            ...subStep,
            status: 'error',
            duration: Date.now() - subStep.timestamp,
            subagentResult: error || t('agent.sub.error'),
          };
          return [
            ...h.slice(0, lastIdx),
            {
              ...msg,
              agentSteps: steps.map((s, i: number) => (i === subIdx ? updatedSub : s)),
            },
          ];
        },
      );
    }),
    // ---- Workflow events ----
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
      EVENTS.workflowStepComplete,
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
    // ---- Compression failure (LOOP-3): the backend now restores the early region and
    // carries on, so this is a non-fatal notice; log it so failures are never invisible.
    listen<{ error: string }>(EVENTS.llmCompressionFailed, (e) => {
      console.warn(
        '[compression] backend context compression failed (original context kept):',
        e.payload.error,
      );
    }),
  ];
}

/** Deps the ChatPage component provides to the page-scoped drag listeners. */
export interface ChatUiDeps {
  setIsDragging: Setter<boolean>;
  handleFileUpload: (path: string, kind: 'file' | 'image') => Promise<void>;
}

/**
 * Page-scoped UI listeners (drag-and-drop only): they touch nothing but the chat
 * surface's local state, so they stay owned by ChatPage and tear down with it.
 */
export function registerChatUiListeners(deps: ChatUiDeps): Promise<UnlistenFn>[] {
  const { setIsDragging, handleFileUpload } = deps;
  return [
    listen('tauri://drag-enter', () => setIsDragging(true)),
    listen('tauri://drag-leave', () => setIsDragging(false)),
    listen<{ paths: string[] }>('tauri://drag-drop', async (e) => {
      setIsDragging(false);
      for (const p of e.payload.paths) await handleFileUpload(p, 'file');
    }),
  ];
}

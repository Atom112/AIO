// Event listener registration set for ChatPage (18 listen calls) — extracted from ChatPage.onMount.
//
// Every dep comes from the ChatPage component scope (Solid setters/signals are stable references, safe to pass along).

import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { EVENTS } from '../../../core/api/events';
import { t } from '../../../core/i18n';
import type { Setter } from 'solid-js';
import { reasoningLevel } from '../../../core/store/store';
import {
  setDatas as storeSetDatas,
  type ActivatedModel,
  type AgentStep,
  type CompactOutcome,
  type DatasState,
  type Message,
  type SubagentStep,
  type Topic,
  type WorkflowState,
} from '../../../core/store/store';
import type { PendingApproval } from '../components/ToolApprovalBubble';
import { TOOL_INTERRUPTED } from '../constants';

/** Deps the ChatPage component provides to the listener set (concrete types: setDatas callback params are type-checked too). */
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
  /** Auto path (called after the conversation ends, without force, honoring the 75% threshold guard); the result is consumed only by the manual path */
  checkAndSummarize: (
    keepBudgetOverride?: number,
    opts?: { force?: boolean },
  ) => Promise<CompactOutcome>;
}

/**
 * Registers all ChatPage event listeners; returns an array of unlisten Promises (each called from onCleanup).
 * Note: listeners register before any invoke call, to avoid the race of losing the first chunk.
 */
export function registerChatEventListeners(deps: ChatEventDeps): Promise<UnlistenFn>[] {
  // Destructure deps so the names match those used in the listener bodies
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

  // Stream content batching state (50ms window) — used only by listeners in this module.
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
    // A new LLM round starts: multi-round agent work reuses the same assistant message
    listen<any>(EVENTS.llmRoundStart, (e) => {
      const { assistant_id, topic_id, round } = e.payload;

      // Later rounds: close the previous round's steps and set up the new round (no longer flattened wholesale into interimContent)
      if (round > 1) {
        const asst = datas.assistants.find((a) => a.id === assistant_id);
        const topic = asst?.topics.find((t: Topic) => t.id === topic_id);
        if (topic) {
          const lastIdx = topic.history.length - 1;
          const lastMsg = topic.history[lastIdx];
          if (lastMsg?.role === 'assistant') {
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
        // Flush any residual rAF-batched content
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
            // Save token usage
            if (input_tokens != null) updatedMsg.inputTokens = input_tokens;
            if (output_tokens != null) updatedMsg.outputTokens = output_tokens;
            if (context_tokens != null) updatedMsg.contextTokens = context_tokens;
            if (cached_input_tokens != null) updatedMsg.cachedInputTokens = cached_input_tokens;
            // Model-generated image metadata (download button) + final text after the backend rewrite (token markers stripped)
            if (images) updatedMsg.images = images;
            if (!error && content) updatedMsg.content = content;
            // Orphaned toolCalls: entries still in calling are marked as interrupted errors
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

      // Streaming append: batched via rAF to avoid a per-token typewriter effect
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
              // Apply the batched content in one shot (all tokens accumulated within the 50ms window)
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
    // Chain-of-thought chunk append: raw reasoning_content accumulates into the message's reasoning field
    // while also building thinking steps in the agentSteps timeline
    listen<any>(EVENTS.llmReasoning, (e) => {
      if (reasoningLevel() === 'off') return;
      const { assistant_id, topic_id, content } = e.payload;
      const asst = datas.assistants.find((a) => a.id === assistant_id);
      const topic = asst?.topics.find((t: Topic) => t.id === topic_id);
      if (topic) {
        const lastIdx = topic.history.length - 1;
        if (lastIdx >= 0) {
          // Append to the reasoning field (kept for backward compatibility)
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
          // Build the agentSteps timeline: create a new step if the last is not thinking, otherwise append
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
                // Append to the current thinking step
                return [
                  ...steps.slice(0, -1),
                  {
                    ...lastStep,
                    thinkingText: (lastStep.thinkingText || '') + content,
                  },
                ];
              }
              // Close the previous step (if still running) and create a new thinking step
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
    // LLM tool-call event: show the "calling" bubble and build the agentSteps timeline
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
            // Keep the toolCalls array (used when rebuilding messages)
            const existing = msg.toolCalls || [];
            if (existing.find((tc: any) => tc.id === tool_call_id)) return h;
            const newToolCalls = [...existing, newTc];
            // Build agentSteps: close the previous step and add a new tool_call step
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
    // Tool execution result: update the matching toolCall state on the assistant message and the agentSteps timeline
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
            // Also update the matching tool_call step in agentSteps
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
    // In-run context compaction event: only main-agent compaction updates conversation memory —
    // topic.summary is overwritten and history trimmed to the token budget so later rounds stop resending the compacted part
    listen<any>(EVENTS.llmCompression, (e) => {
      const { assistant_id, topic_id, source, summary } = e.payload;
      if (!assistant_id || !topic_id) return;
      if (source === 'subagent') return; // subagent context compaction is unrelated to the main conversation
      if (!summary || typeof summary !== 'string' || summary.trim() === '') return;
      const asst = datas.assistants.find((a: any) => a.id === assistant_id);
      const topic = asst?.topics.find((t: Topic) => t.id === topic_id);
      if (!topic) return;
      // Aligned with the backend keep_recent_tokens: accumulate ~20k tokens from the tail, keeping at least the last 4 messages (one full exchange)
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

    // ===== Subagent event listeners =====

    // Subagent start: push a subagent step into the current assistant message's agentSteps
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

    // Subagent step update: append the sub-agent's internal steps
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
              agentSteps: steps.map((s: any, i: number) => (i === subIdx ? updatedSub : s)),
            },
          ];
        },
      );
    }),

    // Subagent done: mark the subagent step complete and record the result
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

    // Subagent error
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

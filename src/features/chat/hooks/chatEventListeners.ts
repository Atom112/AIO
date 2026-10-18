// Event listener registration set for the chat UI.
//
// CHAT-12: the STREAM listeners (llm-chunk / reasoning / tool events / subagents /
// approvals …) register from the root Layout for the whole app lifetime —
// they used to live in ChatPage, so leaving the page mid-generation destroyed them
// while the backend kept streaming: chunks never reached the store, the `done`
// handler (which persists the assistant) never ran, and the turn was lost.
// Only the drag-and-drop UI trio stays page-scoped (registerChatUiListeners).
//
// Every state piece these listeners touch is a module-level store singleton
// (core/store/store + core/store/stream), so no page component needs to outlive them.

import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { produce } from 'solid-js/store';
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
  SubagentDeltaPayload,
  SubagentDonePayload,
  SubagentErrorPayload,
} from '../../../core/api/streamPayloads';
import { t } from '../../../core/i18n';
import type { Setter } from 'solid-js';
import { reasoningLevel } from '../../../core/store/store';
import {
  datas,
  persistMessagesIncrementally,
  saveSingleAssistantToBackend,
  selectedModel,
  setAgentTaskBoard,
  setCompactingTopicId,
  setDatas,
  setPendingApprovals,
  setPendingQuestions,
  setIsThinking,
  setTypingIndex,
  type AgentStep,
  type AgentTaskState,
  type DatasState,
  type Message,
  type SubagentStep,
  type Topic,
  type ToolCallDisplay,
} from '../../../core/store/store';
import { checkAndRename, checkAndSummarize, estimateMsgTokens } from './chatTurnEffects';
import { clearStreamKey } from '../../../core/api/chat';
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
    // UI-flicker fix (round-11 feedback): mutate the message IN PLACE via produce.
    // Solid path setters rebuild every object along the path, so the previous three
    // setDatas calls produced a fresh message reference per flush — the streaming
    // row remounted under <For> every ~80ms, replaying every CSS entrance animation
    // (stream fade-in, ThinkBlock expand-width) and resetting inner scroll positions:
    // the "thinking chain flickers per token" symptom. produce keeps the message
    // identity stable; fine-grained store proxies still notify every reader.
    setDatas(
      'assistants',
      (a) => a.id === batch.assistantId,
      'topics',
      (tp) => tp.id === batch.topicId,
      'history',
      idx,
      produce((msg) => {
        const m = msg as Message;
        if (batch.content)
          m.content = (typeof m.content === 'string' ? m.content : '') + batch.content;
        if (batch.reasoning)
          m.reasoning = (typeof m.reasoning === 'string' ? m.reasoning : '') + batch.reasoning;
        if (batch.agentStepsContent || batch.reasoning) {
          let steps: AgentStep[] = Array.isArray(m.agentSteps) ? m.agentSteps : [];
          if (batch.reasoning)
            steps = appendTimelineStep(steps, 'thinking', 'thinkingText', batch.reasoning);
          if (batch.agentStepsContent)
            steps = appendTimelineStep(steps, 'content', 'contentText', batch.agentStepsContent);
          m.agentSteps = steps;
        }
      }),
    );
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

  // ===== Subagent live streaming (harness round 11) =====
  // H-1 (perf): subagent deltas arrive per SSE token — same granularity as the main
  // llm-chunk stream, which batches at 80ms via pushToBatch. Applying each delta
  // straight to the store meant one setDatas(produce(...)) + a full subagentSteps
  // array copy per token (and ×N under maxConcurrentSubagents). The deltas funnel
  // through the same keyed batching here; the flush re-runs the append once per
  // window with the accumulated text.
  interface SubagentBatch {
    kind: 'content' | 'thinking';
    round: number;
    text: string;
    timer: number;
  }
  const subagentBatches = new Map<string, SubagentBatch>();
  const SUBAGENT_FLUSH_MS = 80;
  const subagentBatchKey = (
    assistantId: string,
    topicId: string,
    subagentId: string,
    kind: string,
  ) => `${assistantId}::${topicId}::${subagentId}::${kind}`;

  // Shared mutator: applies fn to the subagentSteps array of the given subagent
  // inside the last assistant message of the (assistant, topic). All subagent-*
  // listeners below funnel through it.
  // produce-based: mutates the message in place — the previous spread-rebuild
  // replaced the history item reference per delta, remounting the parent row and
  // replaying its entrance animations while a subagent streamed (per-token flicker).
  const mutateSubagentSteps = (
    parentAssistantId: string,
    parentTopicId: string,
    subagentId: string,
    mutate: (steps: SubagentStep[]) => SubagentStep[],
  ) => {
    setDatas(
      'assistants',
      (a) => a.id === parentAssistantId,
      'topics',
      (t) => t.id === parentTopicId,
      'history',
      produce((h) => {
        const list = h as Message[];
        const lastIdx = list.length - 1;
        if (lastIdx < 0 || list[lastIdx]?.role !== 'assistant') return;
        const msg = list[lastIdx];
        const steps: AgentStep[] = msg.agentSteps || [];
        const subIdx = steps.findIndex((s) => s.subagentId === subagentId);
        if (subIdx < 0) return;
        const subStep = steps[subIdx];
        steps[subIdx] = {
          ...subStep,
          subagentSteps: mutate(subStep.subagentSteps || []),
        };
      }),
    );
  };

  // Append accumulated text to the round's RUNNING stream step — one step per
  // (round, kind) accumulates the full text, giving the standalone subagent view
  // the same live fidelity as the main conversation.
  const applySubagentDelta = (
    parentAssistantId: string,
    parentTopicId: string,
    subagentId: string,
    kind: 'content' | 'thinking',
    round: number,
    content: string,
  ) => {
    if (!content) return;
    mutateSubagentSteps(parentAssistantId, parentTopicId, subagentId, (steps) => {
      for (let i = steps.length - 1; i >= 0; i--) {
        const s = steps[i];
        if (s.round !== round) break; // steps are appended in round order
        if (s.type === kind && s.status === 'running') {
          const next = steps.slice();
          next[i] = { ...next[i], summary: (next[i].summary || '') + content };
          return next;
        }
      }
      return [
        ...steps,
        {
          id: `sub${kind}-${round}-${Date.now()}`,
          type: kind,
          timestamp: Date.now(),
          status: 'running' as const,
          summary: content,
          round,
        },
      ];
    });
  };

  /** Flush one subagent batch (drops the batch even if the target vanished). */
  const flushSubagentBatch = (key: string) => {
    const batch = subagentBatches.get(key);
    if (!batch) return;
    subagentBatches.delete(key);
    window.clearTimeout(batch.timer);
    // key = assistant::topic::subagent::kind
    const [assistantId, topicId, subagentId, kind] = key.split('::');
    applySubagentDelta(
      assistantId,
      topicId,
      subagentId,
      kind as 'content' | 'thinking',
      batch.round,
      batch.text,
    );
  };

  /** Flush every batch of one subagent (done/error/status-change ordering). */
  const flushSubagentBatchesFor = (assistantId: string, topicId: string, subagentId: string) => {
    for (const key of [...subagentBatches.keys()]) {
      if (key.startsWith(`${assistantId}::${topicId}::${subagentId}::`)) {
        flushSubagentBatch(key);
      }
    }
  };

  // Live streaming delta (text / reasoning): batched per (assistant, topic,
  // subagent, kind); a round change flushes the previous round's batch first so
  // text lands in round order.
  const handleSubagentDelta =
    (kind: 'content' | 'thinking') =>
    (payload: {
      parent_assistant_id: string;
      parent_topic_id: string;
      subagent_id: string;
      round: number;
      content: string;
    }) => {
      const { parent_assistant_id, parent_topic_id, subagent_id, round, content } = payload;
      if (!content) return;
      const key = subagentBatchKey(parent_assistant_id, parent_topic_id, subagent_id, kind);
      const batch = subagentBatches.get(key);
      if (batch && batch.round !== round) {
        flushSubagentBatch(key);
      } else if (batch) {
        batch.text += content;
        return;
      }
      const fresh: SubagentBatch = { kind, round, text: content, timer: 0 };
      fresh.timer = window.setTimeout(
        () => flushSubagentBatch(key),
        SUBAGENT_FLUSH_MS,
      ) as unknown as number;
      subagentBatches.set(key, fresh);
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
            // Close the currently running step + reset content/reasoning for the new
            // round — one in-place produce (see flushStreamBatch for the remount rationale)
            const now = Date.now();
            setDatas(
              'assistants',
              (a) => a.id === assistant_id,
              'topics',
              (t) => t.id === topic_id,
              'history',
              lastIdx,
              produce((draft) => {
                const m = draft as Message;
                const steps: AgentStep[] = Array.isArray(m.agentSteps) ? m.agentSteps : [];
                const last = steps[steps.length - 1];
                if (last && last.status === 'running') {
                  steps[steps.length - 1] = {
                    ...last,
                    status: 'complete',
                    duration: now - last.timestamp,
                  };
                  m.agentSteps = steps;
                }
                m.content = '';
                m.reasoning = '';
              }),
            );
            // Typewriter index stays unchanged (still pointing at the same message)
            // Incremental persistence (H-2): each round boundary lands ONLY the
            // mutated message (message-scoped upsert) so a crash mid-turn keeps
            // the finished rounds in the visible history.
            if (lastMsg.id) persistMessagesIncrementally(assistant_id, topic_id, [lastMsg.id]);
            return;
          }
        }
      }

      // First round: create a new assistant placeholder message
      const currentMdl = selectedModel();
      const placeholderId = crypto.randomUUID();
      setDatas(
        'assistants',
        (a) => a.id === assistant_id,
        'topics',
        (t) => t.id === topic_id,
        'history',
        (h) => [
          ...h,
          {
            id: placeholderId,
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
      // Persist the placeholder immediately (H-2, message-scoped): a crash right at
      // turn start otherwise leaves the user message without any visible trace of
      // the turn. The user message itself was already appended at send time.
      persistMessagesIncrementally(assistant_id, topic_id, [placeholderId]);
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
        ttft_ms,
        images,
      } = e.payload;
      if (done) {
        // Flush this stream's residual batched content (CHAT-7: per-stream keying)
        flushStreamBatch(streamKey(assistant_id, topic_id));
        // The stream is over — drop its stop-button ownership key.
        clearStreamKey(assistant_id, topic_id);
        // The whole round is truly over (only the backend run_agent_turn epilogue emits done)
        if (error) {
          // Error: append the error text to the last assistant message. Role-guarded:
          // when the turn failed before any assistant placeholder exists (e.g. the
          // pre-stream POST never came back), gluing the error onto a USER message
          // would corrupt the user's own text — push a fresh assistant message instead.
          const asst = datas.assistants.find((a) => a.id === assistant_id);
          const topic = asst?.topics.find((t: Topic) => t.id === topic_id);
          if (topic) {
            const lastIdx = topic.history.length - 1;
            if (lastIdx >= 0 && topic.history[lastIdx]?.role === 'assistant') {
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
            } else {
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
                    content,
                    modelId: selectedModel()?.model_id,
                  },
                ],
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
            if (ttft_ms != null) updatedMsg.ttftMs = ttft_ms;
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
        // H-2: message-scoped persistence — the done handler only mutates the last
        // assistant message (final content/token stats/step closure; the rare
        // no-placeholder fallback appends a fresh one, which this read-back also
        // covers). Topic-level follow-ups (auto-rename / compaction at +500ms)
        // keep their own full saves.
        {
          const asstDone = datas.assistants.find((a) => a.id === assistant_id);
          const topicDone = asstDone?.topics.find((t: Topic) => t.id === topic_id);
          const lastDone = topicDone?.history[topicDone.history.length - 1];
          if (lastDone?.role === 'assistant' && lastDone.id) {
            persistMessagesIncrementally(assistant_id, topic_id, [lastDone.id]);
          }
        }
        setTypingIndex(null);
        // Sequential: first try auto-rename (first exchange in a non-default topic), then trigger history compaction
        setTimeout(async () => {
          await checkAndRename(assistant_id, topic_id);
          // In-record "compacting" pill for the auto path as well (mainstream feedback)
          setCompactingTopicId(topic_id);
          try {
            await checkAndSummarize();
          } finally {
            setCompactingTopicId(null);
          }
        }, 500);
        return;
      }
      // Round-level usage ping (agent loop emits one per completed round, no content):
      // update the running message's token stats in place so the context bar tracks the
      // growing prompt across a multi-round turn (mainstream: per-API-call updates).
      // produce() keeps message identity stable — no row remount, no animation replay.
      // ttft_ms alone also counts (a round may report latency without token usage totals).
      if (input_tokens != null || context_tokens != null || ttft_ms != null) {
        const asstPing = datas.assistants.find((a) => a.id === assistant_id);
        const topicPing = asstPing?.topics.find((t: Topic) => t.id === topic_id);
        const lastIdxPing = (topicPing?.history.length ?? 0) - 1;
        if (lastIdxPing >= 0) {
          setDatas(
            'assistants',
            (a) => a.id === assistant_id,
            'topics',
            (t) => t.id === topic_id,
            'history',
            lastIdxPing,
            produce((m) => {
              const msg = m as Message;
              if (input_tokens != null) msg.inputTokens = input_tokens;
              if (output_tokens != null) msg.outputTokens = output_tokens;
              if (context_tokens != null) msg.contextTokens = context_tokens;
              if (cached_input_tokens != null) msg.cachedInputTokens = cached_input_tokens;
              if (ttft_ms != null) msg.ttftMs = ttft_ms;
            }),
          );
        }
        return; // usage-only ping: nothing to append to the content stream
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
        produce((hDraft) => {
          // In-place (produce): keeps the message identity stable so the row does not
          // remount and replay its entrance animations on every tool event.
          const h = hDraft as Message[];
          const lastIdx = h.length - 1;
          if (lastIdx < 0 || h[lastIdx]?.role !== 'assistant') return;
          const msg = h[lastIdx];
          // Keep the toolCalls array (used when rebuilding messages)
          const existing = msg.toolCalls || [];
          if (existing.find((tc) => tc.id === tool_call_id)) return;
          const newToolCalls: ToolCallDisplay[] = [...existing, newTc];
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
          msg.toolCalls = newToolCalls;
          msg.agentSteps = [
            ...closed,
            {
              id: crypto.randomUUID(),
              type: 'tool_call',
              timestamp: now,
              status: 'running',
              toolCall: newTc,
            },
          ];
        }),
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
      // H-2: the id of the single message this event mutates (captured inside the
      // produce below) — drives the message-scoped persistence after it.
      let hitMessageId: string | undefined;
      setDatas(
        'assistants',
        (a) => a.id === assistant_id,
        'topics',
        (t) => t.id === topic_id,
        'history',
        produce((hDraft) => {
          // In-place: only the hit message mutates (field assignment on the draft);
          // no history/message reference churn, no row remounts.
          const h = hDraft as Message[];
          for (let i = h.length - 1; i >= 0; i--) {
            const m = h[i];
            if (m.role !== 'assistant' || !m.toolCalls) continue;
            const hitToolCall = m.toolCalls.some((tc) => tc.id === tool_call_id);
            const hitStep = (m.agentSteps || []).some(
              (s) => s.type === 'tool_call' && s.toolCall?.id === tool_call_id,
            );
            if (!hitToolCall && !hitStep) continue;
            if (m.id) hitMessageId = m.id;
            m.toolCalls = m.toolCalls.map((tc): ToolCallDisplay =>
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
            m.agentSteps = (m.agentSteps || []).map((s): AgentStep => {
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
            break; // tool_call_id lives in exactly one message
          }
        }),
      );
      // Incremental persistence (H-2, message-scoped): each landed tool result is
      // the natural "work landed" point — a crash mid-turn keeps every finished
      // step in the visible history. Only the one mutated message is upserted.
      if (hitMessageId) persistMessagesIncrementally(assistant_id, topic_id, [hitMessageId]);
    }),
    // In-run context compaction event: only main-agent compaction updates conversation memory —
    // topic.summary is overwritten and the context boundary moves so later turns stop
    // resending the compacted part; the displayed transcript is never modified
    listen<LlmCompressionPayload>(EVENTS.llmCompression, (e) => {
      const { assistant_id, topic_id, source, summary } = e.payload;
      if (!assistant_id || !topic_id) return;
      if (source === 'subagent') return; // subagent context compaction is unrelated to the main conversation
      if (!summary || typeof summary !== 'string' || summary.trim() === '') return;
      const asst = datas.assistants.find((a) => a.id === assistant_id);
      const topic = asst?.topics.find((t: Topic) => t.id === topic_id);
      if (!topic) return;
      // Aligned with the backend keep_recent_tokens: accumulate ~20k tokens from the tail,
      // keeping at least the last 4 messages (one full exchange), inside the context zone
      const KEEP_TOKEN_BUDGET = 20_000;
      const oldTrim = topic.contextTrimCount ?? 0;
      let keepCount = 0;
      let acc = 0;
      const hist = topic.history;
      for (let i = hist.length - 1; i >= oldTrim && i >= 0; i--) {
        acc += estimateMsgTokens(hist[i]);
        keepCount = hist.length - i;
        if (acc >= KEEP_TOKEN_BUDGET) break;
      }
      keepCount = Math.min(Math.max(keepCount, 4), hist.length - oldTrim);
      const newTrim = hist.length - keepCount;
      if (newTrim === oldTrim && topic.summary === summary) return;
      setDatas(
        'assistants',
        (a) => a.id === assistant_id,
        'topics',
        (t: Topic) => t.id === topic_id,
        { summary, contextTrimCount: newTrim },
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
          produce((hDraft) => {
            const h = hDraft as Message[];
            const lastIdx = h.length - 1;
            if (lastIdx < 0 || h[lastIdx]?.role !== 'assistant') return;
            const msg = h[lastIdx];
            const steps: AgentStep[] = msg.agentSteps ?? [];
            msg.agentSteps = [
              ...steps,
              {
                id: 'handoff-' + Date.now(),
                type: 'content',
                timestamp: Date.now(),
                status: 'complete',
                contentText: t('agent.handoff.note', { path }),
              },
            ];
          }),
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
        project_bound,
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
          projectBound: project_bound,
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

    // Subagent live deltas: text streams as 'content' steps, chain-of-thought as
    // 'thinking' steps (both accumulate into one running step per round).
    listen<SubagentDeltaPayload>(EVENTS.subagentChunk, (e) =>
      handleSubagentDelta('content')(e.payload),
    ),
    listen<SubagentDeltaPayload>(EVENTS.subagentReasoning, (e) =>
      handleSubagentDelta('thinking')(e.payload),
    ),

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
        produce((hDraft) => {
          const h = hDraft as Message[];
          const lastIdx = h.length - 1;
          if (lastIdx < 0 || h[lastIdx]?.role !== 'assistant') return;
          const msg = h[lastIdx];
          const steps: AgentStep[] = msg.agentSteps || [];
          msg.agentSteps = [
            ...steps,
            {
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
            },
          ];
        }),
      );
    }),

    // Subagent step update: round-end/tool events + giant-tool-arg progress heartbeats.
    // Harness round 11: with live delta streaming the round-end step additionally
    // completes the running streamed steps, and a `content` round-end summary is
    // deduped against the already-streamed text (kept only as a fallback when a
    // provider never streamed).
    listen<SubagentStepPayload>(EVENTS.subagentStep, (e) => {
      const { parent_assistant_id, parent_topic_id, subagent_id, round, step_type, summary } =
        e.payload;
      mutateSubagentSteps(parent_assistant_id, parent_topic_id, subagent_id, (steps) => {
        // Round end: complete this round's running streamed steps (thinking/content)
        const isRoundEnd = step_type === 'content' || step_type === 'tool_call';
        const base = isRoundEnd
          ? steps.map((s) =>
              s.round === round && s.status === 'running'
                ? { ...s, status: 'complete' as const }
                : s,
            )
          : steps;

        if (step_type === 'streaming') {
          // Heartbeat / giant tool-arg progress ("write_file: 12KB"): update the last
          // streaming step of this round in place instead of appending spam.
          for (let i = base.length - 1; i >= 0; i--) {
            const s = base[i];
            if (s.round !== round) break;
            if (s.type === 'streaming') {
              const next = base.slice();
              next[i] = { ...next[i], summary: summary || '', timestamp: Date.now() };
              return next;
            }
          }
          return [
            ...base,
            {
              id: `substep-${round}-${Date.now()}`,
              type: 'streaming' as const,
              timestamp: Date.now(),
              status: 'running' as const,
              summary: summary || '',
              round,
            },
          ];
        }

        if (step_type === 'content') {
          // Content round-end: the narration already streamed live — append only
          // when no streamed content step exists for the round (provider without
          // delta streaming; legacy shape).
          const streamed = base.some((s) => s.round === round && s.type === 'content');
          if (streamed) return base;
        }

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
        return [...base, newSubStep];
      });
    }),

    // Subagent done: mark the subagent step complete and record the result.
    // The step may live in ANY assistant message (background wait=false subagents
    // finish after the parent turn moved on) — search the whole history by id,
    // not just the last message, or background completions never flip to complete
    // and the status-bar spinner never stops.
    listen<SubagentDonePayload>(EVENTS.subagentDone, (e) => {
      const { parent_assistant_id, parent_topic_id, subagent_id, result } = e.payload;
      // H-1: flush pending delta batches first so the final partial text lands
      // synchronously with the completion flip (not 80ms later).
      flushSubagentBatchesFor(parent_assistant_id, parent_topic_id, subagent_id);
      const finishTime = Date.now();
      setDatas(
        'assistants',
        (a) => a.id === parent_assistant_id,
        'topics',
        (t) => t.id === parent_topic_id,
        'history',
        produce((hDraft) => {
          const h = hDraft as Message[];
          for (const msg of h) {
            if (!msg || msg.role !== 'assistant') continue;
            const steps: AgentStep[] = msg.agentSteps || [];
            const subIdx = steps.findIndex((s) => s.subagentId === subagent_id);
            if (subIdx < 0) continue;
            const subStep = steps[subIdx];
            steps[subIdx] = {
              ...subStep,
              status: 'complete',
              duration: finishTime - subStep.timestamp,
              subagentResult: result || '',
            };
            msg.agentSteps = steps;
            return;
          }
        }),
      );
    }),

    // Subagent error: same whole-history search as subagent-done — a background
    // subagent that errors after the parent turn moved on must still flip status,
    // or it stays 'running' forever and the status-bar spinner never stops.
    listen<SubagentErrorPayload>(EVENTS.subagentError, (e) => {
      const { parent_assistant_id, parent_topic_id, subagent_id, error } = e.payload;
      // H-1: flush pending delta batches first (same rationale as subagent-done).
      flushSubagentBatchesFor(parent_assistant_id, parent_topic_id, subagent_id);
      setDatas(
        'assistants',
        (a) => a.id === parent_assistant_id,
        'topics',
        (t) => t.id === parent_topic_id,
        'history',
        produce((hDraft) => {
          const h = hDraft as Message[];
          for (const msg of h) {
            if (!msg || msg.role !== 'assistant') continue;
            const steps: AgentStep[] = msg.agentSteps || [];
            const subIdx = steps.findIndex((s) => s.subagentId === subagent_id);
            if (subIdx < 0) continue;
            const subStep = steps[subIdx];
            steps[subIdx] = {
              ...subStep,
              status: 'error',
              duration: Date.now() - subStep.timestamp,
              subagentResult: error || t('agent.sub.error'),
            };
            msg.agentSteps = steps;
            return;
          }
        }),
      );
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

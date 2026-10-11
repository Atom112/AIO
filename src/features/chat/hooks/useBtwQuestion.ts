/**
 * /btw side-question overlay logic (阶段4 第2项拆分切片：ChatPage <900 程序).
 *
 * Owns the ephemeral overlay list + the streaming ask: a side question runs over
 * the existing conversation context (no tools, no history changes); chunks arrive
 * via the `btw-chunk` event and update the overlay in place. The unlisten fires on
 * the done/error event, avoiding a race between the invoke resolve and event delivery.
 */
import { createSignal } from 'solid-js';
import { listen } from '@tauri-apps/api/event';
import { askBtwQuestion as askBtwQuestionCmd, llmCreds } from '../../../core/api/chat';
import { EVENTS } from '../../../core/api/events';
import { buildApiMessages } from '../utils/apiMessages';
import type { Assistant, Topic } from '../../../core/store/types';

/** Deps the ChatPage component provides. */
export interface BtwQuestionDeps {
  selectedModel: () => any;
  currentAssistant: () => Assistant | undefined;
  activeTopic: () => Topic | null;
}

export interface BtwOverlay {
  id: string;
  question: string;
  answer: string | null;
  loading: boolean;
}

export function useBtwQuestion(deps: BtwQuestionDeps) {
  const { selectedModel, currentAssistant, activeTopic } = deps;
  const [btwOverlays, setBtwOverlays] = createSignal<BtwOverlay[]>([]);

  /** Streams one side-question answer into its overlay. */
  const askBtwQuestion = async (question: string, overlayId: string) => {
    const currentMdl = selectedModel();
    const asstObj = currentAssistant();
    const topicObj = activeTopic();
    if (!currentMdl || !asstObj || !topicObj) return;

    let resolved = false;

    // register the listener before invoke so the first chunk is never lost
    const unlisten = await listen<{
      overlay_id: string;
      content: string;
      done: boolean;
      error?: string;
    }>(EVENTS.btwChunk, (e) => {
      const { overlay_id, content, done, error } = e.payload;
      if (overlay_id !== overlayId) return;

      if (error) {
        resolved = true;
        setBtwOverlays((prev) =>
          prev.map((o) =>
            o.id === overlayId ? { ...o, answer: `[Error] ${error}`, loading: false } : o,
          ),
        );
        unlisten();
        return;
      }
      if (done) {
        resolved = true;
        setBtwOverlays((prev) =>
          prev.map((o) => (o.id === overlayId ? { ...o, loading: false } : o)),
        );
        unlisten();
        return;
      }
      setBtwOverlays((prev) =>
        prev.map((o) => (o.id === overlayId ? { ...o, answer: (o.answer || '') + content } : o)),
      );
    });

    try {
      // dedicated BTW system prompt: forbid continuing the main task, answer from context only
      const btwSystemPrompt = `[SIDE QUESTION — DO NOT CONTINUE THE MAIN TASK]
You are answering a brief side question about the conversation above.
Do NOT continue, repeat, or redo any work from the conversation.
Only answer the specific question asked. Be concise.

${asstObj.prompt}`;

      const messagesForAI: Record<string, unknown>[] = [
        { role: 'system', content: btwSystemPrompt },
        ...topicObj.history.flatMap((m: any) => buildApiMessages(m)),
        { role: 'user', content: question },
      ];

      await askBtwQuestionCmd(llmCreds(currentMdl), overlayId, JSON.stringify(messagesForAI));
    } catch (err) {
      if (!resolved) {
        setBtwOverlays((prev) =>
          prev.map((o) =>
            o.id === overlayId ? { ...o, answer: `[Error] ${err}`, loading: false } : o,
          ),
        );
        unlisten();
      }
    }
  };

  /** Opens a new overlay for the question and starts streaming its answer. */
  const openBtwQuestion = (question: string) => {
    const id = crypto.randomUUID();
    setBtwOverlays((prev) => [...prev, { id, question, answer: '', loading: true }]);
    void askBtwQuestion(question, id);
  };

  /** Dismisses one overlay. */
  const dismissBtwOverlay = (id: string) =>
    setBtwOverlays((prev) => prev.filter((o) => o.id !== id));

  return { btwOverlays, openBtwQuestion, dismissBtwOverlay };
}

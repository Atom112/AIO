/**
 * Agent question bubble (ask_user)
 *
 * When the agent hits a genuine decision point (ambiguous requirements, alternative
 * approaches), it calls ask_user and the loop pauses. This bubble renders the questions
 * with recommended options first, a free-form answer input per question, and a global
 * skip ("you decide"). Submitting calls respond_agent_question and the agent resumes.
 *
 * Visual language mirrors ToolApprovalBubble (same card style, CSS-variable colors).
 */
import { Component, createSignal, For, Show } from 'solid-js';
import { respondAgentQuestion, type QuestionAnswerPayload } from '../../../core/api/chat';
import Icon from '../../../shared/components/Icon';
import { t } from '../../../core/i18n';
import { btnPrimary, btnSecondary } from '../../../shared/components/buttonStyles';
import type {
  PendingQuestion,
  QuestionOptionPayload,
  QuestionPayload,
} from '../../../core/store/store';

// Types moved to core/store/stream.ts (CHAT-12: global stream state); re-exported
// here for existing importers of this module.
export type { PendingQuestion, QuestionOptionPayload, QuestionPayload };

interface AgentQuestionBubbleProps {
  question: PendingQuestion;
  onResolved: (requestId: string) => void;
}

const AgentQuestionBubble: Component<AgentQuestionBubbleProps> = (props) => {
  // Per-question interaction state: chosen option index (or null) + custom answer text
  const [selected, setSelected] = createSignal<Record<string, number | null>>({});
  const [custom, setCustom] = createSignal<Record<string, string>>({});
  const [submitting, setSubmitting] = createSignal(false);

  const customOf = (qid: string) => custom()[qid] ?? '';
  const selectedOf = (qid: string) => selected()[qid] ?? null;

  const pickOption = (qid: string, idx: number) => {
    setSelected((prev) => ({ ...prev, [qid]: idx }));
    setCustom((prev) => ({ ...prev, [qid]: '' })); // option choice clears the custom text
  };
  const setCustomText = (qid: string, text: string) => {
    setCustom((prev) => ({ ...prev, [qid]: text }));
    if (text) setSelected((prev) => ({ ...prev, [qid]: null })); // typing deselects options
  };

  const isAnswered = (q: QuestionPayload) =>
    customOf(q.questionId).trim().length > 0 || selectedOf(q.questionId) !== null;
  const allAnswered = () => props.question.questions.every(isAnswered);

  const handleSubmit = async () => {
    if (!allAnswered() || submitting()) return;
    setSubmitting(true);
    const answers: QuestionAnswerPayload[] = props.question.questions.map((q) => {
      const c = customOf(q.questionId).trim();
      if (c) return { questionId: q.questionId, answer: c, isCustom: true };
      const idx = selectedOf(q.questionId)!;
      return { questionId: q.questionId, answer: q.options[idx].label, isCustom: false };
    });
    try {
      await respondAgentQuestion(props.question.requestId, false, answers);
    } catch (e) {
      console.error('发送回答失败:', e);
    } finally {
      setSubmitting(false);
      props.onResolved(props.question.requestId);
    }
  };

  const handleSkip = async () => {
    if (submitting()) return;
    setSubmitting(true);
    try {
      await respondAgentQuestion(props.question.requestId, true, []);
    } catch (e) {
      console.error('发送跳过结果失败:', e);
    } finally {
      setSubmitting(false);
      props.onResolved(props.question.requestId);
    }
  };

  return (
    <div
      class="rounded-[10px] px-3.5 py-3 my-1 max-w-[420px] bg-[rgba(var(--surface-alt-bg),0.95)] border border-[rgba(var(--primary-rgb),0.15)] shadow-[0_4px_20px_rgba(0,0,0,0.3)]"
      style={{ animation: 'tool-approval-in 0.2s ease-out' }}
    >
      <div class="flex items-center gap-1.5 mb-2">
        <Icon name="chat" size={15} class="text-[var(--color-pri)]/60 shrink-0" />
        <span class="text-[13px] font-semibold text-white/85">{t('agent.question.title')}</span>
      </div>

      <div class="space-y-3 mb-3">
        <For each={props.question.questions}>
          {(q) => (
            <div>
              <div class="text-[12px] font-medium text-white/80 mb-1.5 leading-[1.45]">
                {q.question}
              </div>
              <div class="space-y-1">
                <For each={q.options}>
                  {(opt, idx) => (
                    <button
                      type="button"
                      class="w-full text-left rounded-md border px-2.5 py-1.5 text-xs transition-all duration-150 cursor-pointer"
                      classList={{
                        'border-[rgba(var(--primary-rgb),0.5)] bg-[rgba(var(--primary-rgb),0.1)] text-white':
                          selectedOf(q.questionId) === idx(),
                        'border-white/10 bg-black/20 text-white/65 hover:bg-white/[0.04] hover:text-white/85':
                          selectedOf(q.questionId) !== idx(),
                      }}
                      onClick={() => pickOption(q.questionId, idx())}
                    >
                      <span class="flex items-center gap-1.5">
                        <Show when={opt.recommended}>
                          <span class="text-[9px] font-semibold px-1 py-px rounded bg-[rgba(var(--primary-rgb),0.2)] text-[rgba(var(--primary-rgb),0.95)] shrink-0">
                            {t('agent.question.recommended')}
                          </span>
                        </Show>
                        <span class="font-medium">{opt.label}</span>
                      </span>
                      <Show when={opt.description}>
                        <span class="block text-white/45 text-[11px] mt-0.5 leading-[1.4]">
                          {opt.description}
                        </span>
                      </Show>
                    </button>
                  )}
                </For>
              </div>
              <Show when={q.allowCustom}>
                <input
                  type="text"
                  class="w-full mt-1.5 rounded-md bg-black/25 border border-white/10 px-2.5 py-1.5 text-xs text-white/85 outline-none transition-colors duration-150 placeholder:text-white/25 focus:border-[rgba(var(--primary-rgb),0.45)]"
                  placeholder={t('agent.question.customPlaceholder')}
                  value={customOf(q.questionId)}
                  onInput={(e) => setCustomText(q.questionId, e.currentTarget.value)}
                />
              </Show>
            </div>
          )}
        </For>
      </div>

      <div class="flex gap-2 justify-end">
        <button type="button" class={btnSecondary} onClick={() => void handleSkip()}>
          <span>{t('agent.question.skip')}</span>
        </button>
        <button
          type="button"
          class={btnPrimary + ' px-3 py-1.5 text-xs rounded-md'}
          disabled={!allAnswered() || submitting()}
          onClick={() => void handleSubmit()}
        >
          <span>{t('agent.question.submit')}</span>
        </button>
      </div>
    </div>
  );
};

export default AgentQuestionBubble;

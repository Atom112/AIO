/** One chat message row (bubble). */
import { For, Show, createMemo, type Component } from 'solid-js';
import { convertFileSrc } from '@tauri-apps/api/core';
import Markdown, { getLangIcon } from '../../../shared/components/Markdown';
import DiffView from '../../../shared/components/DiffView';
import Icon from '../../../shared/components/Icon';
import AgentProcessBlock from './AgentProcessBlock';
import {
  selectedModel,
  currentAssistantId,
  currentTopicId,
  setDatas,
} from '../../../core/store/store';
import { getLogo as getLogoByIds } from '../../../core/utils/modelLogo';
import { formatSize } from '../../../core/utils/format';
import { formatNumber, t } from '../../../core/i18n';
import { btnBadge, btnGhost } from '../../../shared/components/buttonStyles';
import { revertFileChangesBatch } from '../../../core/api/project';
import { deleteTopicMessage } from '../../../core/api/chat';
import type { FileChangeInfo, GeneratedImageInfo } from '../../../core/store/types';

/** Aggregated file-change entry; same shape as FileChangeInfo but deduplicated */
export type AggregatedFileChange = FileChangeInfo;

/** Aggregate all file changes from the message's toolCalls, deduplicated by filePath */
export function aggregateFileChanges(msg: any): AggregatedFileChange[] {
  const toolCalls: any[] = msg.toolCalls || [];
  const seen = new Map<string, FileChangeInfo>();
  for (const tc of toolCalls) {
    const changes: FileChangeInfo[] = tc.fileChanges || [];
    for (const fc of changes) {
      seen.set(fc.filePath, fc);
    }
  }
  return Array.from(seen.values());
}

/** Compute added/deleted line stats from the aggregated file changes */
export function aggregateDiffStats(changes: AggregatedFileChange[]): {
  added: number;
  deleted: number;
} {
  let added = 0;
  let deleted = 0;
  for (const c of changes) {
    const m = c.summary.match(/\+(\d+)\s*-(\d+)/);
    if (m) {
      added += parseInt(m[1], 10);
      deleted += parseInt(m[2], 10);
    }
  }
  return { added, deleted };
}

/** Deps the enclosing ChatInterface provides (all as lazy accessors; reactivity unchanged). */
export interface MessageBubbleProps {
  msg: any;
  index: () => number;
  /** Typewriter round of this message (parent's typingIndex). */
  parentTypingIndex: () => number | null;
  isThinking: () => boolean;
  isSelectingMessages: () => boolean;
  selectedMessageIds: () => Set<string>;
  onToggleMessage: (id: string) => void;
  /** Skip entrance animation for this row (resend/edit suppression). */
  skipMessageAnimation: () => boolean;
  /** Marks the message id as animated (entrance dedup across remounts). */
  markAnimated: (id: string) => void;
  /** True when the id already animated (entrance class guard). */
  hasAnimated: (id: string) => boolean;
  /** Registers the row DOM element (exit animations + deletion lookups). */
  registerEl: (id: string, el: HTMLElement) => void;
  /** Row DOM element for exit animation (undefined when none). */
  getEl: (id: string) => HTMLElement | undefined;
  lastUserMsgIndex: () => number;
  lastAssistantReply: () => any;
  handleStopGeneration: () => void | Promise<void>;
  /** CHAT-11: wait until the running generation actually stops. */
  awaitGenerationStopped: () => Promise<void>;
  onBranchFromMessage: (id: string) => void;
  /** Suppress the entrance animation of the re-sent row (parent list-level flag). */
  setSkipMessageAnimation: (v: boolean) => void;
  /** Sets the input text and sends (resend path). */
  resendWithInput: (text: string) => void;
  /** Sets the input text and focuses the textarea (edit path). */
  editWithInput: (text: string) => void;
  downloadGeneratedImage: (img: GeneratedImageInfo) => Promise<void>;
  /** Current project (revert gate + path resolution). */
  project: () => { id: string; name: string; path: string } | null | undefined;
  /** Whether the message's changes are already reverted. */
  isReverted: (msgId: string | undefined, index: number) => boolean;
  /** Marks a message's changes as reverted (local UI state). */
  markReverted: (key: string) => void;
  /** Whether undo is offered (revertible project + git branch). */
  showUndo: () => boolean;
}

const MessageBubble: Component<MessageBubbleProps> = (props) => {
  // Per-row components under <For> receive a stable message reference: the row's
  // identity is fixed for its lifetime (Solid runs the callback per item), so
  // destructuring once is safe — property reads through the store proxy stay
  // lazy/reactive for every nested field the JSX touches.
  // eslint-disable-next-line solid/reactivity
  const msg = props.msg; // stable per <For> item
  // eslint-disable-next-line solid/reactivity
  const index = props.index; // stable per <For> item

  const isActiveRound = createMemo(
    () => index() === props.parentTypingIndex() && props.isThinking(),
  );
  // Reactive: streaming updates now mutate the message in place (produce, no row
  // remount), so the aggregation must track msg.toolCalls itself instead of being
  // computed once at setup (it used to rely on remounts to refresh).
  const fileChanges = createMemo(() =>
    msg.role === 'assistant' ? aggregateFileChanges(msg) : ([] as AggregatedFileChange[]),
  );
  return (
    <Show when={msg.role !== 'tool'}>
      <div class="flex items-start gap-2 mb-3">
        {/* Selection mode: checkbox */}
        <Show when={props.isSelectingMessages() && msg.id}>
          <div
            class="flex-shrink-0 mt-1 cursor-pointer select-none"
            onClick={() => props.onToggleMessage(msg.id!)}
          >
            <div
              class="w-5 h-5 rounded border-2 flex items-center justify-center transition-all duration-150 hover:scale-110 active:scale-95"
              classList={{
                'bg-[rgba(var(--primary-rgb),0.25)] border-[rgba(var(--primary-rgb),0.5)]': props
                  .selectedMessageIds()
                  .has(msg.id!),
                'bg-transparent border-white/[0.15] hover:border-white/[0.35]': !props
                  .selectedMessageIds()
                  .has(msg.id!),
              }}
            >
              <Show when={props.selectedMessageIds().has(msg.id!)}>
                <Icon
                  name="check"
                  class="w-3 h-3"
                  style={{ color: 'rgba(var(--primary-rgb),0.9)' }}
                />
              </Show>
            </div>
          </div>
        </Show>
        <div
          ref={(el) => {
            if (msg.id) {
              props.markAnimated(msg.id);
              props.registerEl(msg.id, el);
            }
          }}
          class={`flex flex-col flex-1 pointer-events-auto min-w-0 ${msg.id && !props.hasAnimated(msg.id) && !props.skipMessageAnimation() ? 'animate-message-in' : ''} ${msg.role === 'assistant' ? 'items-start' : 'items-end'}`}
          style={{
            // content-visibility skips offscreen rendering for performance; contain-intrinsic-size
            // `auto <len>` remembers the element's last rendered height so it does not collapse
            // to the 100px placeholder when skipped; otherwise total height shrinks and the
            // browser clamps scrollTop upward (view jumps back toward the top on new output).
            'content-visibility': 'auto',
            'contain-intrinsic-size': 'auto 100px',
          }}
        >
          <div
            class={`flex gap-3 w-full ${msg.role === 'assistant' ? 'justify-start items-start' : 'justify-end items-start pr-[10px]'}`}
          >
            <Show when={msg.role === 'assistant'}>
              <div
                class="flex flex-shrink-0 items-center justify-center w-9 h-9 rounded-full overflow-hidden"
                style={{
                  background: 'rgb(var(--text-base-rgb))',
                  border: '1px solid var(--border-dim)',
                  'box-shadow': '0 2px 6px rgba(0,0,0,0.25)',
                }}
              >
                <Show
                  when={getLogoByIds(null, msg.modelId || selectedModel()?.model_id || '')}
                  fallback={
                    <span
                      class="text-[14px] font-bold select-none"
                      style={{ color: 'var(--color-dark-700)' }}
                    >
                      {(msg.modelId || selectedModel()?.model_id || 'AI').charAt(0).toUpperCase()}
                    </span>
                  }
                >
                  <img
                    src={getLogoByIds(null, msg.modelId || selectedModel()?.model_id || '')!}
                    alt={t('common.ai')}
                    class="w-[25px] h-[25px] rounded-full"
                  />
                </Show>
              </div>
            </Show>

            <div
              class={`flex flex-col max-w-[75%] ${msg.role === 'assistant' ? 'items-start' : 'items-end'}`}
            >
              <div
                class={`rounded-[10px] leading-relaxed max-w-full min-h-[1.5em] px-[14px] py-[10px] transition-[height] duration-200 break-words group relative ${
                  msg.role === 'assistant'
                    ? 'rounded-tl-[2px] text-white'
                    : 'rounded-tr-[2px] text-white'
                } ${isActiveRound() && msg.content ? 'message-streaming' : ''}`}
                style={{
                  background:
                    msg.role === 'assistant'
                      ? 'rgba(var(--text-base-rgb),0.04)'
                      : 'rgba(var(--primary-rgb),0.08)',
                  border:
                    msg.role === 'assistant'
                      ? '1px solid rgba(var(--text-base-rgb),0.13)'
                      : '1px solid rgba(var(--primary-rgb),0.16)',
                  'backdrop-filter': 'blur(6px)',
                }}
              >
                <Show when={msg.role === 'user' && msg.displayFiles && msg.displayFiles.length > 0}>
                  <div class="flex flex-col gap-2 mb-2 first:mt-3">
                    <For each={msg.displayFiles}>
                      {(file: any) => {
                        const isImage = (file.mimeType || '').startsWith('image/');
                        const ext = (file.name.split('.').pop() || '').toLowerCase();
                        return (
                          <div
                            class="flex items-center cursor-default gap-3 max-w-[300px] px-[14px] py-[5px] rounded-lg transition-all duration-200"
                            style={{
                              background: 'rgba(var(--primary-rgb),0.06)',
                              border: '1px solid rgba(var(--primary-rgb),0.12)',
                            }}
                          >
                            {/* Unified thumbnail/icon box: images and attachments share the same size */}
                            <Show
                              when={isImage && file.storagePath}
                              fallback={
                                <div
                                  class="flex flex-shrink-0 items-center justify-center w-7 h-7 rounded-md [&_svg]:w-4 [&_svg]:h-4"
                                  style={{
                                    background: 'rgba(var(--primary-rgb),0.08)',
                                    color: 'rgba(var(--primary-rgb),0.7)',
                                  }}
                                >
                                  <span
                                    ref={(node) => {
                                      node.innerHTML = getLangIcon(ext);
                                    }}
                                  />
                                </div>
                              }
                            >
                              <img
                                src={convertFileSrc(file.storagePath)}
                                loading="lazy"
                                class="w-7 h-7 flex-shrink-0 object-cover rounded-md border border-white/[0.08]"
                              />
                            </Show>
                            <div class="flex-grow overflow-hidden">
                              <div class="text-white text-[0.9rem] font-medium overflow-hidden text-ellipsis whitespace-nowrap">
                                {file.name}
                              </div>
                            </div>
                          </div>
                        );
                      }}
                    </For>
                  </div>
                </Show>

                <div class="mt-1">
                  {/* Agent process: reasoning + toolCalls + agentSteps in one collapsed block */}
                  <Show
                    when={
                      msg.role === 'assistant' &&
                      (msg.reasoning ||
                        msg.interimContent ||
                        msg.toolCalls?.length > 0 ||
                        msg.agentSteps?.length > 0)
                    }
                  >
                    <AgentProcessBlock
                      reasoning={msg.reasoning}
                      toolCalls={msg.toolCalls}
                      interimContent={msg.interimContent}
                      isActive={isActiveRound()}
                      startTime={msg.agentStartTime}
                      agentSteps={msg.agentSteps}
                    />
                  </Show>
                  <Show
                    when={
                      msg.role === 'assistant' &&
                      !msg.content &&
                      !msg.toolCalls?.length &&
                      !msg.reasoning
                    }
                    fallback={
                      <Show when={msg.role !== 'tool'}>
                        <div class={isActiveRound() && msg.content ? 'animate-stream-fade-in' : ''}>
                          <Markdown
                            content={
                              msg.role === 'user' && msg.displayText !== undefined
                                ? msg.displayText
                                : msg.content
                            }
                          />
                        </div>
                      </Show>
                    }
                  >
                    <div class="flex items-center gap-2 py-1 text-white/50 italic text-[14px] select-none">
                      <Icon
                        src="/icons/app-logo/loading.svg"
                        class="w-4 h-4 animate-spin opacity-50"
                      />
                      <span class="animate-pulse">{t('chat.thinking')}</span>
                    </div>
                  </Show>
                </div>
              </div>

              <Show when={msg.images && msg.images.length > 0}>
                <div class="flex flex-wrap gap-1.5 mt-2">
                  <For each={msg.images}>
                    {(img) => (
                      <button
                        class={btnBadge}
                        onClick={() => props.downloadGeneratedImage(img)}
                        title={t('common.download')}
                      >
                        <Icon name="download" class="w-3 h-3" />
                        <span class="max-w-[160px] truncate">{img.name}</span>
                        <span class="opacity-60">{formatSize(img.size)}</span>
                      </button>
                    )}
                  </For>
                </div>
              </Show>

              <Show
                when={
                  msg.role === 'assistant' &&
                  (msg.modelId || selectedModel()?.model_id || msg.inputTokens || msg.outputTokens)
                }
              >
                <div
                  style={{
                    color: 'rgba(var(--text-base-rgb),0.3)',
                    'font-family': 'monospace',
                    'font-size': '11px',
                    'margin-left': '4px',
                    'margin-top': '4px',
                    opacity: '0.7',
                    'user-select': 'none',
                    'text-align': 'left',
                    display: 'flex',
                    'align-items': 'center',
                    gap: '8px',
                  }}
                >
                  <Show when={msg.modelId || selectedModel()?.model_id}>
                    <span>{msg.modelId || selectedModel()?.model_id}</span>
                  </Show>
                  <Show when={msg.inputTokens || msg.outputTokens}>
                    <span style={{ color: 'rgba(var(--text-base-rgb),0.2)' }}>·</span>
                    <Show when={msg.inputTokens}>
                      <span title={t('chat.inputTokens')}>
                        ↗{' '}
                        {msg.inputTokens! >= 1000
                          ? `${(msg.inputTokens! / 1000).toFixed(1)}K`
                          : msg.inputTokens}
                      </span>
                    </Show>
                    <Show when={msg.outputTokens}>
                      <span title={t('chat.outputTokens')}>
                        ↘{' '}
                        {msg.outputTokens! >= 1000
                          ? `${(msg.outputTokens! / 1000).toFixed(1)}K`
                          : msg.outputTokens}
                      </span>
                    </Show>
                  </Show>
                </div>
              </Show>

              {/* Message action buttons: always visible */}
              <div
                class={`flex mt-1.5 px-[4px] z-[5] gap-1.5 flex-wrap ${msg.role === 'assistant' ? 'justify-start' : 'justify-end pr-[10px]'}`}
              >
                {/* Copy button (on every message) */}
                <button
                  class="group inline-flex items-center bg-transparent rounded-lg cursor-pointer text-xs py-1 px-1.5 transition-all duration-200 hover:bg-[rgba(var(--text-base-rgb),0.08)] hover:!p-[4px_8px]"
                  style={{
                    border: '1px solid rgba(var(--text-base-rgb),0.25)',
                    color: 'rgba(var(--text-base-rgb),0.6)',
                  }}
                  onClick={(e) => {
                    const currentBtn = e.currentTarget;
                    const text =
                      msg.role === 'user' && msg.displayText !== undefined
                        ? msg.displayText
                        : msg.content;
                    if (!text) return;
                    navigator.clipboard.writeText(text).then(() => {
                      const label = currentBtn.querySelector('.action-label') as HTMLElement | null;
                      if (label) {
                        const originalText = label.innerText;
                        currentBtn.style.color = 'var(--color-success)';
                        currentBtn.style.borderColor = 'var(--color-success)';
                        label.style.maxWidth = '60px';
                        label.style.opacity = '1';
                        label.innerText = t('common.copied');
                        setTimeout(() => {
                          currentBtn.style.color = '';
                          currentBtn.style.borderColor = '';
                          label.style.maxWidth = '';
                          label.style.opacity = '';
                          label.innerText = originalText;
                        }, 2000);
                      }
                    });
                  }}
                >
                  <Icon src="/icons/app-logo/clipboard-copy.svg" class="w-[13px] h-[13px]" />
                  <span class="action-label overflow-hidden whitespace-nowrap text-[11px] text-white max-w-0 opacity-0 transition-all duration-200 group-hover:max-w-[80px] group-hover:opacity-100">
                    {t('common.copy')}
                  </span>
                </button>

                {/* Branch button (assistant messages only) */}
                <Show when={msg.role === 'assistant' && !!msg.id}>
                  <button
                    class="group inline-flex items-center bg-transparent rounded-lg cursor-pointer text-xs py-1 px-1.5 transition-all duration-200 hover:bg-[rgba(var(--text-base-rgb),0.08)] hover:!p-[4px_8px]"
                    style={{
                      border: '1px solid rgba(var(--text-base-rgb),0.25)',
                      color: 'rgba(var(--text-base-rgb),0.6)',
                    }}
                    onClick={() => {
                      if (msg.id) props.onBranchFromMessage(msg.id);
                    }}
                    title={t('chat.branchFromHere')}
                  >
                    <Icon
                      name="git-branch"
                      size={13}
                      style={{ display: 'inline', 'vertical-align': 'middle' }}
                    />
                    <span class="action-label overflow-hidden whitespace-nowrap text-[11px] text-white max-w-0 opacity-0 transition-all duration-200 group-hover:max-w-[80px] group-hover:opacity-100">
                      {t('chat.branch')}
                    </span>
                  </button>
                </Show>

                {/* Last user message: resend + edit-and-resend */}
                <Show
                  when={
                    index() === props.lastUserMsgIndex() &&
                    msg.role === 'user' &&
                    (msg.content || msg.displayText)
                  }
                >
                  <button
                    class="group inline-flex items-center bg-transparent rounded-lg cursor-pointer text-xs py-1 px-1.5 transition-all duration-200 hover:bg-[rgba(var(--text-base-rgb),0.08)] hover:!p-[4px_8px]"
                    style={{
                      border: '1px solid rgba(var(--text-base-rgb),0.25)',
                      color: 'rgba(var(--text-base-rgb),0.6)',
                    }}
                    onClick={async () => {
                      const text = msg.displayText !== undefined ? msg.displayText : msg.content;
                      if (!text) return;
                      if (props.isThinking()) {
                        // CHAT-11: wait for the stream to actually stop
                        // (isThinking flips in the done handler) instead of
                        // sleeping a magic 150ms and racing the epilogue.
                        await props.handleStopGeneration();
                        await props.awaitGenerationStopped();
                      }
                      const asstId = currentAssistantId();
                      const tId = currentTopicId();
                      if (asstId && tId) {
                        const reply = props.lastAssistantReply();
                        // Skip the exit animation and delete directly; also suppress the entrance animation of the new message
                        props.setSkipMessageAnimation(true);
                        if (reply?.id) {
                          try {
                            await deleteTopicMessage(tId, reply.id!);
                          } catch (e) {
                            console.error('Failed to delete assistant reply:', e);
                          }
                          setDatas(
                            'assistants',
                            (a) => a.id === asstId,
                            'topics',
                            (t) => t.id === tId,
                            'history',
                            (h) => h.filter((m: any) => m.id !== reply.id),
                          );
                        }
                        if (msg.id) {
                          try {
                            await deleteTopicMessage(tId, msg.id!);
                          } catch (e) {
                            console.error('Failed to delete message:', e);
                          }
                          setDatas(
                            'assistants',
                            (a) => a.id === asstId,
                            'topics',
                            (t) => t.id === tId,
                            'history',
                            (h) => h.filter((m: any) => m.id !== msg.id),
                          );
                        }
                      }
                      props.resendWithInput(text);
                    }}
                  >
                    <svg
                      xmlns="http://www.w3.org/2000/svg"
                      fill="none"
                      viewBox="0 0 24 24"
                      stroke-width="1.5"
                      stroke="currentColor"
                      class="w-[13px] h-[13px]"
                    >
                      <path
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        d="M16.023 9.348h4.992v-.001M2.985 19.644v-4.992m0 0h4.992m-4.993 0l3.181 3.183a8.25 8.25 0 0013.803-3.7M4.031 9.865a8.25 8.25 0 0113.803-3.7l3.181 3.182"
                      />
                    </svg>
                    <span class="action-label overflow-hidden whitespace-nowrap text-[11px] text-white max-w-0 opacity-0 transition-all duration-200 group-hover:max-w-[80px] group-hover:opacity-100">
                      {t('chat.resendMessage')}
                    </span>
                  </button>
                  <button
                    class="group inline-flex items-center bg-transparent rounded-lg cursor-pointer text-xs py-1 px-1.5 transition-all duration-200 hover:bg-[rgba(var(--text-base-rgb),0.08)] hover:!p-[4px_8px]"
                    style={{
                      border: '1px solid rgba(var(--text-base-rgb),0.25)',
                      color: 'rgba(var(--text-base-rgb),0.6)',
                    }}
                    onClick={async () => {
                      const text = msg.displayText !== undefined ? msg.displayText : msg.content;
                      if (!text) return;
                      if (props.isThinking()) {
                        // CHAT-11: wait for the stream to actually stop
                        // (isThinking flips in the done handler) instead of
                        // sleeping a magic 150ms and racing the epilogue.
                        await props.handleStopGeneration();
                        await props.awaitGenerationStopped();
                      }
                      const asstId = currentAssistantId();
                      const tId = currentTopicId();
                      if (asstId && tId) {
                        const reply = props.lastAssistantReply();
                        // Skip the exit animation and delete directly; also suppress the entrance animation of the new message
                        props.setSkipMessageAnimation(true);
                        if (reply?.id) {
                          try {
                            await deleteTopicMessage(tId, reply.id!);
                          } catch (e) {
                            console.error('Failed to delete assistant reply:', e);
                          }
                          setDatas(
                            'assistants',
                            (a) => a.id === asstId,
                            'topics',
                            (t) => t.id === tId,
                            'history',
                            (h) => h.filter((m: any) => m.id !== reply.id),
                          );
                        }
                        if (msg.id) {
                          try {
                            await deleteTopicMessage(tId, msg.id!);
                          } catch (e) {
                            console.error('Failed to delete message:', e);
                          }
                          setDatas(
                            'assistants',
                            (a) => a.id === asstId,
                            'topics',
                            (t) => t.id === tId,
                            'history',
                            (h) => h.filter((m: any) => m.id !== msg.id),
                          );
                        }
                      }
                      props.editWithInput(text);
                    }}
                  >
                    <svg
                      xmlns="http://www.w3.org/2000/svg"
                      fill="none"
                      viewBox="0 0 24 24"
                      stroke-width="1.5"
                      stroke="currentColor"
                      class="w-[13px] h-[13px]"
                    >
                      <path
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        d="M16.862 4.487l1.687-1.688a1.875 1.875 0 112.652 2.652L10.582 16.07a4.5 4.5 0 01-1.897 1.13L6 18l.8-2.685a4.5 4.5 0 011.13-1.897l8.932-8.931zm0 0L19.5 7.125M18 14v4.75A2.25 2.25 0 0115.75 21H5.25A2.25 2.25 0 013 18.75V8.25A2.25 2.25 0 015.25 6H10"
                      />
                    </svg>
                    <span class="action-label overflow-hidden whitespace-nowrap text-[11px] text-white max-w-0 opacity-0 transition-all duration-200 group-hover:max-w-[80px] group-hover:opacity-100">
                      {t('common.edit')}
                    </span>
                  </button>
                </Show>

                {/* Delete button (on every message, placed rightmost) */}
                <button
                  class="group inline-flex items-center bg-transparent rounded-lg cursor-pointer text-xs py-1 px-1.5 transition-all duration-200 hover:bg-[rgba(248,113,113,0.1)] hover:!p-[4px_8px]"
                  style={{
                    border: '1px solid rgba(248,113,113,0.35)',
                    color: 'rgba(248,113,113,0.75)',
                  }}
                  onClick={async () => {
                    const asstId = currentAssistantId();
                    const tId = currentTopicId();
                    if (!asstId || !tId || !msg.id) return;
                    // Play the exit animation directly on the DOM, without triggering a reactive re-render
                    const el = props.getEl(msg.id);
                    if (el) el.classList.add('animate-message-out');
                    await new Promise((r) => setTimeout(r, 260));
                    try {
                      await deleteTopicMessage(tId, msg.id!);
                    } catch (e) {
                      console.error('Failed to delete message:', e);
                    }
                    setDatas(
                      'assistants',
                      (a) => a.id === asstId,
                      'topics',
                      (t) => t.id === tId,
                      'history',
                      (h) => h.filter((m: any) => m.id !== msg.id),
                    );
                  }}
                >
                  <svg
                    xmlns="http://www.w3.org/2000/svg"
                    fill="none"
                    viewBox="0 0 24 24"
                    stroke-width="1.5"
                    stroke="currentColor"
                    class="w-[13px] h-[13px]"
                  >
                    <path
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      d="M14.74 9l-.346 9m-4.788 0L9.26 9m9.968-3.21c.342.052.682.107 1.022.166m-1.022-.165L18.16 19.673a2.25 2.25 0 01-2.244 2.077H8.084a2.25 2.25 0 01-2.244-2.077L4.772 5.79m14.456 0a48.108 48.108 0 00-3.478-.397m-12 .562c.34-.059.68-.114 1.022-.165m0 0a48.11 48.11 0 013.478-.397m7.5 0v-.916c0-1.18-.91-2.164-2.09-2.201a51.964 51.964 0 00-3.32 0c-1.18.037-2.09 1.022-2.09 2.201v.916m7.5 0a48.667 48.667 0 00-7.5 0"
                    />
                  </svg>
                  <span class="action-label overflow-hidden whitespace-nowrap text-[11px] max-w-0 opacity-0 transition-all duration-200 group-hover:max-w-[60px] group-hover:opacity-100">
                    {t('chat.deleteMessage')}
                  </span>
                </button>
              </div>
            </div>
          </div>

          {/* File Changes box (assistant messages only, shown when there are file changes) */}
          <Show when={fileChanges().length > 0}>
            {(() => {
              const isReverted = () => props.isReverted(msg.id, index());
              const diffStats = () => aggregateDiffStats(fileChanges());
              const showUndo = () => !props.isReverted(msg.id, index()) && props.showUndo();
              const handleUndoAll = async () => {
                const projectPath = props.project()?.path;
                if (!projectPath) return;
                try {
                  await revertFileChangesBatch(
                    projectPath,
                    fileChanges().map((fc) => fc.filePath),
                  );
                  props.markReverted(msg.id ?? String(index()));
                } catch (err) {
                  console.error('Failed to batch-revert:', err);
                  alert(t('agent.diff.revertFailed'));
                }
              };
              return (
                <div
                  class="rounded-lg overflow-hidden transition-opacity duration-300 w-[80%] self-center mt-3"
                  classList={{ 'opacity-40': isReverted() }}
                  style={{
                    border: '1px solid var(--border-dim)',
                    background: 'rgba(var(--text-base-rgb),0.015)',
                  }}
                >
                  <div
                    class="flex items-center gap-2 px-3 py-1.5"
                    style={{
                      'border-bottom': '1px solid var(--border-dim)',
                      color: 'rgba(var(--text-base-rgb),0.4)',
                      'font-size': '14px',
                    }}
                  >
                    <svg
                      xmlns="http://www.w3.org/2000/svg"
                      fill="none"
                      viewBox="0 0 24 24"
                      stroke-width="1.5"
                      stroke="currentColor"
                      class="w-4 h-4"
                      style={{ opacity: '0.6' }}
                    >
                      <path
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        d="M19.5 14.25v-2.625a3.375 3.375 0 00-3.375-3.375h-1.5A1.125 1.125 0 0113.5 7.125v-1.5a3.375 3.375 0 00-3.375-3.375H8.25m0 12.75h7.5m-7.5 3H12M10.5 2.25H5.625c-.621 0-1.125.504-1.125 1.125v17.25c0 .621.504 1.125 1.125 1.125h12.75c.621 0 1.125-.504 1.125-1.125V11.25a9 9 0 00-9-9z"
                      />
                    </svg>
                    <Show
                      when={isReverted()}
                      fallback={<span class="font-medium">{t('agent.diff.title')}</span>}
                    >
                      <span class="font-medium" style={{ 'text-decoration': 'line-through' }}>
                        {t('agent.diff.title')}
                      </span>
                    </Show>
                    <span style={{ color: 'rgba(var(--text-base-rgb),0.25)' }}>
                      {t('agent.diff.files', {
                        count: formatNumber(fileChanges().length),
                      })}
                    </span>
                    <Show when={diffStats().added > 0 || diffStats().deleted > 0}>
                      <span class="font-mono">
                        {diffStats().added > 0 && (
                          <span style={{ color: 'var(--color-success)' }}>
                            +{diffStats().added}
                          </span>
                        )}
                        {diffStats().added > 0 && diffStats().deleted > 0 && (
                          <span style={{ color: 'rgba(var(--text-base-rgb),0.18)' }}> </span>
                        )}
                        {diffStats().deleted > 0 && (
                          <span style={{ color: 'var(--color-danger)' }}>
                            -{diffStats().deleted}
                          </span>
                        )}
                      </span>
                    </Show>
                    <div class="flex-1" />
                    <Show when={isReverted()}>
                      <span
                        class="px-1.5 py-px rounded text-[12px] font-medium"
                        style={{
                          color: 'rgba(80, 220, 100, 0.7)',
                          background: 'rgba(80, 220, 100, 0.08)',
                          border: '1px solid rgba(80, 220, 100, 0.15)',
                        }}
                      >
                        {t('agent.diff.reverted')}
                      </span>
                    </Show>
                    <Show when={showUndo()}>
                      <button
                        type="button"
                        class={'px-1.5 py-0.5 text-[12px] font-medium ' + btnGhost}
                        onClick={(e) => {
                          e.stopPropagation();
                          handleUndoAll();
                        }}
                        title={t('agent.diff.revertAllTitle')}
                      >
                        <svg
                          xmlns="http://www.w3.org/2000/svg"
                          fill="none"
                          viewBox="0 0 24 24"
                          stroke-width="2"
                          stroke="currentColor"
                          class="w-3.5 h-3.5"
                          style={{ opacity: '0.6' }}
                        >
                          <path
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            d="M9 15L3 9m0 0l6-6M3 9h12a6 6 0 010 12h-3"
                          />
                        </svg>
                        {t('agent.diff.revertAll')}
                      </button>
                    </Show>
                  </div>
                  <div class="p-2">
                    <DiffView changes={fileChanges()} allReverted={isReverted()} />
                  </div>
                </div>
              );
            })()}
          </Show>
        </div>
      </div>
    </Show>
  );
};

export default MessageBubble;

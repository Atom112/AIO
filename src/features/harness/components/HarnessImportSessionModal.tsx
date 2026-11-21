/**
 * Import-session modal — lists the agent's own sessions (`session/list`) and
 * adopts one as an AIO topic. The new topic stores the agent session id; the
 * normal connect path then replays its transcript via `session/load`.
 */
import { Component, createSignal, For, onMount, Show } from 'solid-js';
import Icon from '../../../shared/components/Icon';
import { btnClose, btnSecondary } from '../../../shared/components/buttonStyles';
import { formatNumber, t } from '../../../core/i18n';
import { harnessAdoptAgentSession, harnessListAgentSessions } from '../../../core/api/harness';
import type { HarnessAgentSessionInfo, HarnessTopic } from '../../../core/api/harnessPayloads';

interface HarnessImportSessionModalProps {
  toolId: string;
  workspacePath: string;
  onClose: () => void;
  onImported: (topic: HarnessTopic) => void;
}

const HarnessImportSessionModal: Component<HarnessImportSessionModalProps> = (props) => {
  const [loading, setLoading] = createSignal(true);
  const [supported, setSupported] = createSignal(true);
  const [sessions, setSessions] = createSignal<HarnessAgentSessionInfo[]>([]);
  const [error, setError] = createSignal('');
  const [importing, setImporting] = createSignal<string | null>(null);

  onMount(async () => {
    try {
      const list = await harnessListAgentSessions(props.toolId, props.workspacePath);
      setSupported(list.supported);
      setSessions(list.sessions);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  });

  const adopt = async (session: HarnessAgentSessionInfo) => {
    if (session.imported || importing()) return;
    setImporting(session.sessionId);
    setError('');
    try {
      const topic = await harnessAdoptAgentSession(
        props.toolId,
        props.workspacePath,
        session.sessionId,
        session.title?.trim() || `${t('harness.import.untitled')} ${session.sessionId.slice(0, 8)}`,
      );
      props.onImported(topic);
    } catch (e) {
      setError(String(e));
    } finally {
      setImporting(null);
    }
  };

  const updatedLabel = (session: HarnessAgentSessionInfo) => {
    if (!session.updatedAt) return '';
    const date = new Date(session.updatedAt);
    return Number.isNaN(date.getTime()) ? session.updatedAt : date.toLocaleString();
  };

  return (
    <div
      class="fixed inset-0 z-[1000] bg-black/25 flex items-center justify-center"
      onClick={() => props.onClose()}
    >
      <div
        class="rounded-xl px-6 py-5 max-w-[520px] w-[92%] relative flex flex-col max-h-[70vh]"
        style={{
          background: 'var(--dialog-bg)',
          'backdrop-filter': 'var(--popup-blur)',
          border: '1px solid var(--border-dim)',
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <button class={`absolute top-3 right-3 ${btnClose}`} onClick={() => props.onClose()}>
          <Icon name="x" class="w-4 h-4" />
        </button>

        <div class="text-sm font-medium text-white/85 mb-4">{t('harness.import.title')}</div>

        <div class="flex-1 min-h-0 overflow-y-auto -mx-1 px-1">
          <Show
            when={!loading()}
            fallback={<div class="text-xs text-white/40 py-4">{t('harness.import.loading')}</div>}
          >
            <Show
              when={supported()}
              fallback={
                <div class="text-xs text-white/40 py-4">{t('harness.import.unsupported')}</div>
              }
            >
              <Show
                when={sessions().length > 0}
                fallback={<div class="text-xs text-white/40 py-4">{t('harness.import.empty')}</div>}
              >
                <div class="space-y-1">
                  <For each={sessions()}>
                    {(session) => (
                      <button
                        class="w-full flex items-center gap-3 px-3 py-2 rounded-lg text-left border border-white/[0.06] bg-white/[0.03] transition-colors cursor-pointer disabled:cursor-default hover:bg-white/[0.07] disabled:hover:bg-white/[0.03]"
                        disabled={session.imported || importing() !== null}
                        title={updatedLabel(session)}
                        onClick={() => void adopt(session)}
                      >
                        <Icon name="chat" size={13} class="shrink-0 text-white/35" />
                        <span class="flex-1 min-w-0">
                          <span class="block text-xs text-white/80 truncate">
                            {session.title?.trim() || t('harness.import.untitled')}
                          </span>
                          <span class="block text-[10px] text-white/30 truncate font-mono">
                            {session.sessionId}
                          </span>
                        </span>
                        <span class="shrink-0 text-[10px] text-white/30">
                          <Show when={session.messageCount !== undefined}>
                            {t('harness.import.messageCount', {
                              count: formatNumber(session.messageCount!),
                            })}
                          </Show>
                        </span>
                        <span
                          class={`shrink-0 text-[11px] ${session.imported ? 'text-white/25' : 'text-pri'}`}
                        >
                          {session.imported
                            ? t('harness.import.imported')
                            : t('harness.import.confirm')}
                        </span>
                      </button>
                    )}
                  </For>
                </div>
              </Show>
            </Show>
          </Show>
        </div>

        <Show when={error()}>
          <div class="text-[11px] text-danger mt-3 break-all">{error()}</div>
        </Show>

        <div class="flex items-center justify-end mt-4">
          <button class={btnSecondary} onClick={() => props.onClose()}>
            {t('common.close')}
          </button>
        </div>
      </div>
    </div>
  );
};

export default HarnessImportSessionModal;

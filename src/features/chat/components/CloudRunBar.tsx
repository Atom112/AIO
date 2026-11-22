// Cloud run control bar: starts a team-server agent run for the current thread, shows its
// streamed state and resolves tool approvals.
//
// Runs are opt-in per message: the local agent loop stays the default, and this bar only
// renders when the installation is linked to a team workspace.
import { Component, For, Show, createMemo } from 'solid-js';
import { cloudRunArtifacts } from '../../../core/api/cloud';
import {
  cancelCloudRun,
  cloudRun,
  cloudSession,
  dismissCloudRun,
  resolveCloudApproval,
  startCloudRun,
} from '../../../core/store/cloud';
import { currentAssistantId, currentTopicId, datas } from '../../../core/store/store';
import { openSafeUrl } from '../../../core/utils/safeUrl';
import { t } from '../../../core/i18n';
import {
  btnDangerOutline,
  btnPrimaryOutlineSm,
  btnSecondarySm,
} from '../../../shared/components/buttonStyles';

interface CloudRunBarProps {
  /** Current composer text; a run consumes it exactly like a local send. */
  message: string;
  /** Clears the composer after a run starts. */
  onConsumed: () => void;
}

export const CloudRunBar: Component<CloudRunBarProps> = (props) => {
  const run = createMemo(() => cloudRun());
  const linked = createMemo(() => cloudSession().signedIn && Boolean(cloudSession().workspaceId));
  /** Only shared conversations we own can run on the server. */
  const topic = createMemo(() =>
    datas.assistants
      .flatMap((assistant) => assistant.topics)
      .find((entry) => entry.id === currentTopicId()),
  );
  const runnable = createMemo(
    () =>
      linked() &&
      topic()?.visibility === 'workspace' &&
      (!topic()?.ownerUserId || topic()?.ownerUserId === cloudSession().userId),
  );
  const active = createMemo(() => {
    const status = run()?.status;
    return status === 'queued' || status === 'running' || status === 'waiting_approval';
  });
  const canStart = createMemo(() => props.message.trim().length > 0);

  const onStart = async (): Promise<void> => {
    const threadId = currentTopicId();
    const assistantId = currentAssistantId();
    const message = props.message.trim();
    if (!threadId || !assistantId || message.length === 0) return;
    await startCloudRun({ threadId, assistantId, message });
    props.onConsumed();
  };

  return (
    <Show when={runnable()}>
      <div class="px-1 pb-1">
        <Show
          when={run()}
          fallback={
            <button
              type="button"
              class={btnSecondarySm}
              disabled={!canStart()}
              onClick={() => void onStart()}
            >
              {t('chat.cloud.run.start')}
            </button>
          }
        >
          {(current) => (
            <div class="flex flex-col gap-1.5 px-3 py-2 rounded-lg border border-[var(--border-dim)] bg-white/[0.04] text-xs">
              <div class="flex items-center justify-between gap-2">
                <span class="text-white/70">
                  {t('chat.cloud.run.status', { status: current().status })}
                </span>
                <div class="flex items-center gap-1.5">
                  <Show when={active()}>
                    <button
                      type="button"
                      class={btnDangerOutline}
                      onClick={() => void cancelCloudRun()}
                    >
                      {t('chat.cloud.run.cancel')}
                    </button>
                  </Show>
                  <Show when={!active()}>
                    <button type="button" class={btnSecondarySm} onClick={() => dismissCloudRun()}>
                      {t('chat.cloud.run.dismiss')}
                    </button>
                    <button
                      type="button"
                      class={btnSecondarySm}
                      onClick={() =>
                        void cloudRunArtifacts(current().runId).then((files) => {
                          for (const file of files) void openSafeUrl(file.downloadUrl);
                        })
                      }
                    >
                      {t('chat.cloud.run.artifacts')}
                    </button>
                  </Show>
                </div>
              </div>

              <Show when={current().text.length > 0}>
                <div class="text-white/85 whitespace-pre-wrap max-h-40 overflow-auto">
                  {current().text}
                </div>
              </Show>

              <Show when={current().error}>
                {(message) => <div class="text-red-300">{message()}</div>}
              </Show>

              <Show when={current().approvals.length > 0}>
                <div class="flex flex-col gap-1.5">
                  <For each={current().approvals}>
                    {(approval) => (
                      <div class="flex items-center justify-between gap-2 rounded-md border border-[var(--border-dim)] px-2 py-1.5">
                        <span class="text-white/70">
                          {t('chat.cloud.run.approval', { tool: approval.toolName })}
                        </span>
                        <div class="flex items-center gap-1.5">
                          <button
                            type="button"
                            class={btnPrimaryOutlineSm}
                            onClick={() =>
                              void resolveCloudApproval(approval.approvalId, 'approve')
                            }
                          >
                            {t('chat.cloud.run.approve')}
                          </button>
                          <button
                            type="button"
                            class={btnDangerOutline}
                            onClick={() => void resolveCloudApproval(approval.approvalId, 'deny')}
                          >
                            {t('chat.cloud.run.deny')}
                          </button>
                        </div>
                      </div>
                    )}
                  </For>
                </div>
              </Show>

              <Show when={!active() && current().status === 'succeeded'}>
                <div class="text-white/45">{t('chat.cloud.run.hint')}</div>
              </Show>
            </div>
          )}
        </Show>
      </div>
    </Show>
  );
};

export default CloudRunBar;

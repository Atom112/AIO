// Team cloud account card: server URL, GitHub device sign-in, workspace binding and
// sync controls. Mounted at the top of App settings; all state lives in the cloud store
// so the sync loop and the card never disagree.
import { Component, createMemo, createSignal, For, Show } from 'solid-js';
import { openSafeUrl } from '../../../core/utils/safeUrl';
import {
  acceptCloudInvitation,
  bindCloudWorkspace,
  cloudDevicePrompt,
  cloudInvitations,
  cloudServerUrl,
  invitationExpiresLabel,
  rejectCloudInvitation,
  cloudLastError,
  cloudPending,
  cloudSession,
  cloudStatus,
  cloudWorkspaces,
  initCloud,
  loadCloudWorkspaces,
  signOutCloud,
  startCloudSignIn,
  syncNow,
} from '../../../core/store/cloud';
import { t } from '../../../core/i18n';
import { Dropdown } from '../../../shared/components/Dropdown';
import { Icon } from '../../../shared/components/Icon';
import {
  btnDangerOutline,
  btnPrimaryOutlineSm,
  btnSecondarySm,
} from '../../../shared/components/buttonStyles';

type BusyKind = 'signin' | 'bind' | 'sync' | 'signout' | 'invitation';

/** Settings card for the optional team cloud account and conversation sync. */
export const CloudAccountCard: Component = () => {
  const [serverUrl, setServerUrl] = createSignal(
    cloudSession().serverUrl ?? cloudServerUrl() ?? '',
  );
  const [workspaceId, setWorkspaceId] = createSignal(cloudSession().workspaceId ?? '');
  const [busy, setBusy] = createSignal<
    'signin' | 'bind' | 'sync' | 'signout' | 'invitation' | null
  >(null);
  const [localError, setLocalError] = createSignal<string | null>(null);

  const bound = createMemo(() => Boolean(cloudSession().workspaceId));
  const signedIn = createMemo(() => cloudSession().signedIn);
  const prompt = createMemo(() => cloudDevicePrompt());

  const workspaceOptions = createMemo(() =>
    cloudWorkspaces().map((workspace) => ({
      value: workspace.id,
      label: `${workspace.name} (${workspace.role})`,
    })),
  );

  const statusLabel = createMemo(() => {
    switch (cloudStatus()) {
      case 'syncing':
        return t('settings.cloud.status.syncing');
      case 'idle':
        return t('settings.cloud.status.idle');
      case 'offline':
        return t('settings.cloud.status.offline');
      case 'error':
        return t('settings.cloud.status.error');
      default:
        return t('settings.cloud.status.signedOut');
    }
  });

  const run = async (kind: BusyKind, action: () => Promise<void>) => {
    setBusy(kind);
    setLocalError(null);
    try {
      await action();
    } catch (error) {
      setLocalError(String(error));
    } finally {
      setBusy(null);
    }
  };

  // Signals are read while building the click handler (tracked scope), never inside
  // the async callback, so the linter and the reactive graph agree on dependencies.
  const onSignIn = (): void => {
    const trimmed = serverUrl().trim();
    if (!trimmed) {
      setLocalError(t('settings.cloud.error.serverUrl'));
      return;
    }
    void run('signin', async () => {
      const started = await startCloudSignIn(trimmed);
      await openSafeUrl(started.verificationUri);
      await loadCloudWorkspaces(trimmed);
    });
  };

  const onBind = (): void => {
    const selected = workspaceId();
    const trimmed = serverUrl().trim();
    if (!selected) return;
    void run('bind', () => bindCloudWorkspace(trimmed, selected));
  };

  const onSignOut = () =>
    run('signout', async () => {
      await signOutCloud();
      setServerUrl('');
      setWorkspaceId('');
    });

  const onAcceptInvitation = (invitationId: string): void => {
    void run('invitation', () => acceptCloudInvitation(invitationId));
  };

  const onDeclineInvitation = (invitationId: string): void => {
    void run('invitation', () => rejectCloudInvitation(invitationId));
  };

  return (
    <div
      class="rounded-xl p-6 animate-row-in"
      style={{
        background: 'rgba(var(--text-base-rgb), 0.035)',
        'backdrop-filter': 'blur(16px)',
        border: '1px solid var(--border-dim)',
      }}
    >
      <div class="flex justify-between items-center mb-5">
        <h3 class="m-0 text-base text-white">{t('settings.cloud.title')}</h3>
        <span class="text-xs text-white/45 font-medium">{statusLabel()}</span>
      </div>

      <div class="flex justify-between items-center py-3 border-b border-white/5">
        <div class="pr-6">
          <span class="block text-[rgba(var(--text-base-rgb),0.93)] text-[14px]">
            {t('settings.cloud.serverUrl')}
          </span>
          <span class="block text-xs text-white/35 mt-1">{t('settings.cloud.serverUrlHint')}</span>
        </div>
        <input
          type="text"
          class="w-[280px] px-3 py-1.5 rounded-lg text-xs bg-transparent border border-[var(--border-dim)] text-white/85 outline-none transition-colors duration-150 hover:border-[rgba(var(--primary-rgb),0.5)] focus:border-[rgba(var(--primary-rgb),0.5)]"
          placeholder="https://cloud.example.com"
          value={serverUrl()}
          disabled={signedIn() || busy() !== null}
          onInput={(event) => setServerUrl(event.currentTarget.value)}
        />
      </div>

      <div class="flex justify-between items-center py-3 border-b border-white/5">
        <div class="pr-6">
          <span class="block text-[rgba(var(--text-base-rgb),0.93)] text-[14px]">
            {t('settings.cloud.account')}
          </span>
          <Show when={cloudSession().userLogin}>
            <span class="block text-xs text-white/35 mt-1">
              {t('settings.cloud.signedInAs', { user: cloudSession().userLogin ?? '' })}
            </span>
          </Show>
        </div>
        <Show
          when={signedIn()}
          fallback={
            <button
              type="button"
              class={`min-w-[140px] ${btnSecondarySm}`}
              disabled={busy() !== null}
              onClick={onSignIn}
            >
              <Icon src="/icons/app-logo/github.svg" class="h-3.5 w-3.5" />
              {t('settings.cloud.signIn')}
            </button>
          }
        >
          <button
            type="button"
            class={`min-w-[140px] ${btnDangerOutline}`}
            disabled={busy() !== null}
            onClick={() => void onSignOut()}
          >
            {t('settings.cloud.signOut')}
          </button>
        </Show>
      </div>

      <Show when={prompt()}>
        {(current) => (
          <div class="py-3 border-b border-white/5">
            <div class="text-xs text-white/70">
              {t('settings.cloud.deviceCode', { code: current().userCode })}
            </div>
            <div class="text-xs text-white/40 mt-1">{t('settings.cloud.deviceHint')}</div>
            <button
              type="button"
              class={`mt-2 ${btnSecondarySm}`}
              onClick={() => void openSafeUrl(current().verificationUri)}
            >
              {t('settings.cloud.openVerification')}
            </button>
          </div>
        )}
      </Show>

      <Show when={signedIn() && cloudInvitations().length > 0}>
        <div class="py-3 border-b border-white/5">
          <div class="text-[rgba(var(--text-base-rgb),0.93)] text-[14px] mb-2">
            {t('settings.cloud.invitations.title')}
          </div>
          <div class="flex flex-col gap-2">
            <For each={cloudInvitations()}>
              {(invitation) => (
                <div class="flex items-center justify-between gap-3 rounded-lg border border-[var(--border-dim)] bg-white/[0.06] px-3 py-2">
                  <div class="min-w-0">
                    <div class="text-xs text-white/90 truncate">
                      {invitation.organizationName}
                      <span class="ml-1.5 text-white/40">({invitation.role})</span>
                    </div>
                    <div class="text-[11px] text-white/40 mt-0.5">
                      {t('settings.cloud.invitations.expires', {
                        time: invitationExpiresLabel(invitation),
                      })}
                    </div>
                  </div>
                  <div class="flex items-center gap-2 shrink-0">
                    <button
                      type="button"
                      class={`min-w-[112px] ${btnPrimaryOutlineSm}`}
                      disabled={busy() !== null}
                      onClick={() => onAcceptInvitation(invitation.id)}
                    >
                      {t('settings.cloud.invitations.join')}
                    </button>
                    <button
                      type="button"
                      class={`min-w-[112px] ${btnSecondarySm}`}
                      disabled={busy() !== null}
                      onClick={() => onDeclineInvitation(invitation.id)}
                    >
                      {t('settings.cloud.invitations.decline')}
                    </button>
                  </div>
                </div>
              )}
            </For>
          </div>
        </div>
      </Show>

      <div class="flex justify-between items-center py-3 border-b border-white/5">
        <div class="pr-6">
          <span class="block text-[rgba(var(--text-base-rgb),0.93)] text-[14px]">
            {t('settings.cloud.workspace')}
          </span>
          <Show when={bound()}>
            <span class="block text-xs text-white/35 mt-1">
              {t('settings.cloud.workspaceLocked')}
            </span>
          </Show>
        </div>
        <div class="flex items-center gap-2">
          <Dropdown
            variant="ghost"
            class="w-[280px]"
            value={workspaceId()}
            placeholder={t('settings.cloud.selectWorkspace')}
            options={workspaceOptions()}
            disabled={bound() || signedIn() === false}
            onChange={(value) => setWorkspaceId(value)}
          />
          <Show when={!bound() && signedIn()}>
            <button
              type="button"
              class={`min-w-[112px] ${btnPrimaryOutlineSm}`}
              disabled={busy() !== null || !workspaceId()}
              onClick={onBind}
            >
              {t('settings.cloud.bind')}
            </button>
          </Show>
        </div>
      </div>

      <div class="flex justify-between items-center pt-3">
        <div class="text-xs text-white/45">
          {t('settings.cloud.pending', { count: cloudPending() })}
        </div>
        <div class="flex items-center gap-2">
          <button
            type="button"
            class={`min-w-[112px] ${btnSecondarySm}`}
            disabled={!bound() || busy() !== null}
            onClick={() => void run('sync', () => syncNow())}
          >
            {t('settings.cloud.syncNow')}
          </button>
          <button
            type="button"
            class={`h-[30px] ${btnSecondarySm}`}
            onClick={() => void initCloud()}
            title={t('settings.cloud.reload')}
          >
            <Icon name="refresh" size={13} />
          </button>
        </div>
      </div>

      <Show when={localError() ?? cloudLastError()}>
        {(message) => <div class="text-xs text-red-300 mt-3">{message()}</div>}
      </Show>
    </div>
  );
};

export default CloudAccountCard;

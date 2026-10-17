/**
 * Harness tool credentials section (settings detail page): auth-status
 * detection (the tool's own config/login probe files), the AIO-stored API key
 * (OS keychain → injected as the tool's provider env var at spawn), and the
 * official setup docs link.
 */
import { Component, createSignal, onMount, Show } from 'solid-js';
import Icon from '../../../shared/components/Icon';
import { btnDangerOutline, btnPrimaryOutlineSm } from '../../../shared/components/buttonStyles';
import {
  harnessSetToolApiKey,
  harnessToolSetupInfo,
  type HarnessToolSetupInfo,
} from '../../../core/api/harness';
import { t } from '../../../core/i18n';

interface HarnessToolCredentialsProps {
  toolId: string;
  toolName: string;
}

const HarnessToolCredentials: Component<HarnessToolCredentialsProps> = (props) => {
  const [info, setInfo] = createSignal<HarnessToolSetupInfo | null>(null);
  const [keyInput, setKeyInput] = createSignal('');
  const [savedTick, setSavedTick] = createSignal(false);
  const [error, setError] = createSignal('');

  const reload = async () => {
    try {
      setInfo(await harnessToolSetupInfo(props.toolId));
    } catch (e) {
      setError(String(e));
    }
  };
  onMount(() => void reload());

  const saveKey = async () => {
    setError('');
    try {
      await harnessSetToolApiKey(props.toolId, keyInput());
      setKeyInput('');
      setSavedTick(true);
      setTimeout(() => setSavedTick(false), 1500);
      await reload();
    } catch (e) {
      setError(String(e));
    }
  };

  const clearKey = async () => {
    setError('');
    try {
      await harnessSetToolApiKey(props.toolId, '');
      await reload();
    } catch (e) {
      setError(String(e));
    }
  };

  const inputClass =
    'w-full rounded-lg bg-[rgba(0,0,0,0.2)] border border-[var(--border-dim)] px-3 py-1.5 text-xs text-white placeholder:text-white/25 outline-none focus:border-pri-50 transition-colors';

  return (
    <div>
      <Show when={info()} fallback={<div class="text-xs text-white/40">{t('common.loading')}</div>}>
        {/* Tool's own auth/config detection */}
        <div class="flex items-center gap-2 text-xs mb-3">
          <Icon
            name={info()!.configured ? 'check-circle' : 'x-circle'}
            size={13}
            class={info()!.configured ? 'text-success' : 'text-white/35'}
          />
          <span class="text-white/70">
            {info()!.configured
              ? t('harness.setup.ownConfigFound')
              : t('harness.setup.ownConfigMissing')}
          </span>
        </div>

        {/* AIO-stored API key (tools with a provider env var) */}
        <Show when={info()!.apiKeyEnv}>
          <div class="text-xs text-white/60 mb-1.5">
            {t('harness.setup.apiKeyLabel', { env: info()!.apiKeyEnv! })}
          </div>
          <div class="flex items-center gap-2 mb-1.5">
            <input
              type="password"
              class={inputClass}
              placeholder={
                info()!.hasAioKey
                  ? t('harness.setup.apiKeyStored')
                  : t('harness.setup.apiKeyPlaceholder')
              }
              value={keyInput()}
              onInput={(e) => setKeyInput(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') void saveKey();
              }}
            />
            <button
              class={btnPrimaryOutlineSm}
              disabled={!keyInput().trim()}
              onClick={() => void saveKey()}
            >
              {savedTick() ? t('common.saved') : t('common.save')}
            </button>
            <Show when={info()!.hasAioKey}>
              <button class={btnDangerOutline} onClick={() => void clearKey()}>
                {t('common.delete')}
              </button>
            </Show>
          </div>
          <div class="text-[11px] text-white/35 mb-1">{t('harness.setup.apiKeyNote')}</div>
        </Show>

        {/* Docs link */}
        <Show when={info()!.authDocsUrl}>
          <a
            href={info()!.authDocsUrl}
            target="_blank"
            rel="noopener noreferrer"
            class="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-white/10 text-[rgba(var(--text-base-rgb),0.65)] hover:border-pri-30 hover:text-pri transition-all duration-200"
          >
            <Icon name="book" size={12} /> {t('harness.setup.docsLink')}
          </a>
        </Show>

        <Show when={error()}>
          <div class="text-[11px] text-danger mt-3 break-all">{error()}</div>
        </Show>
      </Show>
    </div>
  );
};

export default HarnessToolCredentials;

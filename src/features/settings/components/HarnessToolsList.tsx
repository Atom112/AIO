/**
 * Settings → CLI tools list page: one card per supported harness tool (logo,
 * availability state, adapter) linking to its per-tool configuration page.
 */
import { Component, For, onMount, Show } from 'solid-js';
import { A, useNavigate } from '@solidjs/router';
import Icon from '../../../shared/components/Icon';
import { harnessDetectTools } from '../../../core/api/harness';
import { setHarnessTools, harnessTools } from '../../../core/store/harness';
import { getHarnessLogo } from '../../harness/utils/harnessLogos';
import { t } from '../../../core/i18n';
import type { HarnessToolStatus } from '../../../core/api/harnessPayloads';

const stateLabel = (state: string) =>
  state === 'ready'
    ? t('harness.tool.ready')
    : state === 'installable'
      ? t('harness.tool.installable')
      : t('harness.tool.missing');

const stateDot = (state: string) =>
  state === 'ready'
    ? 'rgba(74,222,128,0.9)'
    : state === 'installable'
      ? 'rgba(250,204,21,0.85)'
      : 'rgba(var(--text-base-rgb),0.25)';

/** Initials chip fallback for tools without a logo (customs never list here). */
const ToolLogo: Component<{ tool: HarnessToolStatus; size: number }> = (p) => {
  const logo = () => getHarnessLogo(p.tool.id);
  return (
    <Show
      when={logo()}
      fallback={
        <span
          class="flex items-center justify-center rounded-[6px] bg-white/[0.08] text-white/70 font-semibold shrink-0"
          style={{ width: `${p.size}px`, height: `${p.size}px`, 'font-size': `${p.size * 0.42}px` }}
        >
          {p.tool.name.slice(0, 2).toUpperCase()}
        </span>
      }
    >
      <img
        src={logo()!}
        alt={p.tool.name}
        class="shrink-0 object-contain"
        style={{ width: `${p.size}px`, height: `${p.size}px` }}
      />
    </Show>
  );
};

const HarnessToolsList: Component = () => {
  const navigate = useNavigate();
  onMount(() => {
    if (harnessTools().length === 0) {
      void harnessDetectTools().then(setHarnessTools).catch(console.error);
    }
  });

  return (
    <div class="p-2">
      <div class="text-lg text-[rgba(var(--text-base-rgb),0.75)] font-medium mb-1">
        {t('settings.cliTools')}
      </div>
      <div class="text-xs text-[rgba(var(--text-base-rgb),0.4)] mb-4">
        {t('harness.conf.listSubtitle')}
      </div>

      <For each={harnessTools()}>
        {(tool) => (
          <A
            href={`/settings/cli-tools/${tool.id}`}
            class="flex items-center gap-3 p-3 my-1 rounded-xl cursor-pointer no-underline transition-all duration-200 border"
            style={{
              background: 'rgba(var(--text-base-rgb),0.03)',
              'border-color': 'var(--border-dim)',
              color: 'rgba(var(--text-base-rgb),0.8)',
            }}
            onMouseEnter={(e) =>
              (e.currentTarget.style.background = 'rgba(var(--primary-rgb),0.08)')
            }
            onMouseLeave={(e) =>
              (e.currentTarget.style.background = 'rgba(var(--text-base-rgb),0.03)')
            }
          >
            <ToolLogo tool={tool} size={26} />
            <div class="flex-1 min-w-0">
              <div class="text-sm font-medium text-white/85 truncate">{tool.name}</div>
              <div class="text-[11px] text-white/40 truncate">{tool.adapter}</div>
            </div>
            <span class="flex items-center gap-1.5 text-xs text-white/50 shrink-0">
              <span class="w-2 h-2 rounded-full" style={{ background: stateDot(tool.state) }} />
              {stateLabel(tool.state)}
            </span>
            <Icon name="chevron-right" size={14} class="text-white/25 shrink-0" />
          </A>
        )}
      </For>

      <Show when={harnessTools().length === 0}>
        <div class="text-xs text-white/40 p-3">{t('common.loading')}</div>
      </Show>

      {/* Custom ACP agents also get pages (no config slots; credentials only). */}
      <div class="text-[11px] text-white/30 mt-4 mb-2">{t('harness.conf.customHint')}</div>
      <button
        class="text-xs text-pri hover:underline cursor-pointer border-none bg-transparent"
        onClick={() => navigate('/harness')}
      >
        {t('harness.conf.goAddCustom')}
      </button>
    </div>
  );
};

export default HarnessToolsList;

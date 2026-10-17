/**
 * Settings → CLI tools → per-tool configuration page: overview (install state,
 * config-file slots, DSH profile stack), credentials (auth detection + API
 * key + docs), and the validated raw editors for the tool's user-level config
 * files (the CLI's own files are the source of truth).
 */
import { Component, createSignal, For, onMount, Show, type JSX } from 'solid-js';
import { A, useParams } from '@solidjs/router';
import Icon from '../../../shared/components/Icon';
import {
  harnessDetectTools,
  harnessToolConfigSlots,
  harnessToolDshProfiles,
  type DshProfileInfo,
  type HarnessToolConfigSlot,
} from '../../../core/api/harness';
import { harnessTools, setHarnessTools } from '../../../core/store/harness';
import { getHarnessLogo } from '../../harness/utils/harnessLogos';
import { t } from '../../../core/i18n';
import HarnessToolCredentials from './HarnessToolCredentials';
import HarnessConfigFileEditor from './HarnessConfigFileEditor';
import HarnessToolStructuredForm from './HarnessToolStructuredForm';

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

const Section: Component<{ title: string; children: JSX.Element }> = (props) => (
  <div class="mb-6">
    <div class="text-sm font-semibold text-white/85 mb-2">{props.title}</div>
    {props.children}
  </div>
);

const HarnessToolConfigPage: Component = () => {
  const params = useParams();
  const toolId = () => params.toolId as string;
  const [slots, setSlots] = createSignal<HarnessToolConfigSlot[]>([]);
  const [dshProfiles, setDshProfiles] = createSignal<DshProfileInfo[]>([]);
  const [notFound, setNotFound] = createSignal(false);

  const tool = () => harnessTools().find((x) => x.id === toolId());
  const logo = () => getHarnessLogo(toolId());

  onMount(async () => {
    if (harnessTools().length === 0) {
      try {
        setHarnessTools(await harnessDetectTools());
      } catch (e) {
        console.error('[cli-tools] detect failed:', e);
      }
    }
    if (!tool()) setNotFound(true);
    try {
      setSlots(await harnessToolConfigSlots(toolId()));
    } catch (e) {
      console.error('[cli-tools] slots failed:', e);
    }
    if (toolId() === 'dsh') {
      try {
        setDshProfiles(await harnessToolDshProfiles());
      } catch {
        /* DSH_HOME absent */
      }
    }
  });

  return (
    <Show
      when={!notFound()}
      fallback={<div class="p-6 text-xs text-white/40">{t('harness.conf.notFound')}</div>}
    >
      <div class="p-2 pb-8">
        {/* Back + header */}
        <A
          href="/settings/cli-tools"
          class="inline-flex items-center gap-1.5 text-xs text-white/45 hover:text-white/85 transition-colors no-underline mb-4"
        >
          <Icon name="chevron-left" size={14} />
          {t('harness.conf.back')}
        </A>
        <div class="flex items-center gap-3 mb-6">
          <Show
            when={logo()}
            fallback={
              <span class="flex items-center justify-center rounded-[6px] bg-white/[0.08] text-white/70 font-semibold shrink-0 w-[30px] h-[30px] text-[13px]">
                {tool()?.name.slice(0, 2).toUpperCase()}
              </span>
            }
          >
            <img
              src={logo()!}
              alt={tool()?.name}
              class="w-[30px] h-[30px] object-contain shrink-0"
            />
          </Show>
          <div class="flex-1 min-w-0">
            <div class="text-lg text-[rgba(var(--text-base-rgb),0.75)] font-medium">
              {tool()?.name ?? toolId()}
            </div>
            <div class="flex items-center gap-1.5 text-xs text-white/45">
              <Show when={tool()}>
                <span
                  class="w-2 h-2 rounded-full"
                  style={{ background: stateDot(tool()!.state) }}
                />
                {stateLabel(tool()!.state)} · {tool()!.adapter}
              </Show>
            </div>
          </div>
        </div>

        {/* ===== 概览 ===== */}
        <Section title={t('harness.conf.overview')}>
          <div
            class="rounded-xl p-4 text-xs"
            style={{
              background: 'rgba(var(--text-base-rgb),0.03)',
              border: '1px solid var(--border-dim)',
            }}
          >
            <div class="text-white/50 mb-2">{t('harness.conf.configFiles')}</div>
            <For each={slots()}>
              {(slot) => (
                <div class="flex items-center gap-2 py-1">
                  <Icon
                    name={slot.exists ? 'check-circle' : 'x-circle'}
                    size={12}
                    class={slot.exists ? 'text-success' : 'text-white/25'}
                  />
                  <span class="text-white/70">{slot.label}</span>
                  <span class="text-[10px] font-mono text-white/30 uppercase px-1 rounded border border-white/10">
                    {slot.format}
                  </span>
                  <span class="flex-1 truncate text-white/30 font-mono" title={slot.path}>
                    {slot.path}
                  </span>
                  <Show when={slot.hasBackup}>
                    <span class="flex items-center shrink-0" title={t('harness.conf.hasBackup')}>
                      <Icon name="rotate-ccw" size={11} class="text-white/30" />
                    </span>
                  </Show>
                </div>
              )}
            </For>
            <Show when={slots().length === 0}>
              <div class="text-white/30 py-1">{t('harness.conf.noConfigFiles')}</div>
            </Show>

            {/* DSH profile stack (read-only overview) */}
            <Show when={toolId() === 'dsh'}>
              <div class="text-white/50 mt-3 mb-1">{t('harness.conf.dshProfiles')}</div>
              <For each={dshProfiles()}>
                {(p) => (
                  <div class="flex items-center gap-2 py-0.5 text-white/60">
                    <Icon name="layers" size={11} class="text-white/30" />
                    <span>{p.name}</span>
                    <span class="flex-1 truncate text-white/25 font-mono" title={p.path}>
                      {p.path}
                    </span>
                  </div>
                )}
              </For>
              <Show when={dshProfiles().length === 0}>
                <div class="text-white/30 py-0.5">{t('harness.conf.dshNoProfiles')}</div>
              </Show>
            </Show>
          </div>
        </Section>

        {/* ===== 常用配置（结构化表单） ===== */}
        <Section title={t('harness.conf.common')}>
          <div class="text-[11px] text-white/35 mb-2">{t('harness.conf.commonNote')}</div>
          <HarnessToolStructuredForm toolId={toolId()} />
        </Section>

        {/* ===== 凭据 ===== */}
        <Section title={t('harness.conf.credentials')}>
          <div
            class="rounded-xl p-4"
            style={{
              background: 'rgba(var(--text-base-rgb),0.03)',
              border: '1px solid var(--border-dim)',
            }}
          >
            <HarnessToolCredentials toolId={toolId()} toolName={tool()?.name ?? toolId()} />
          </div>
        </Section>

        {/* ===== 高级配置（原始编辑器） ===== */}
        <Section title={t('harness.conf.advanced')}>
          <div class="text-[11px] text-white/35 mb-2">{t('harness.conf.advancedNote')}</div>
          <div class="flex flex-col gap-3">
            <For each={slots()}>
              {(slot) => <HarnessConfigFileEditor toolId={toolId()} slot={slot} />}
            </For>
          </div>
        </Section>
      </div>
    </Show>
  );
};

export default HarnessToolConfigPage;

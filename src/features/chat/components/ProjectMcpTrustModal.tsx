import { Component, For, Show } from 'solid-js';
import {
  confirmProjectMcpServer,
  dismissProjectMcpServer,
  pendingProjectMcpServers,
} from '../../../core/store/store';
import type { McpHttpTransport, McpStdioTransport, McpTransport } from '../../../core/types/mcp';
import Icon from '../../../shared/components/Icon';
import {
  btnClose,
  btnPrimaryOutlineSm,
  btnSecondarySm,
} from '../../../shared/components/buttonStyles';
import { t } from '../../../core/i18n';

/** One label + value row; values are security-relevant and shown in full with wrapping. */
const DetailRow: Component<{ label: string; value: string }> = (props) => (
  <div class="flex flex-col gap-0.5 min-w-0">
    <span class="text-[10px] uppercase tracking-[1px] font-semibold text-white/45 shrink-0">
      {props.label}
    </span>
    <span
      class="text-[11px] break-all whitespace-pre-wrap"
      style={{ color: 'rgba(var(--text-base-rgb),0.62)' }}
    >
      {props.value}
    </span>
  </div>
);

/** Full transport details (command/args/env or url/headers) so trust decisions are informed. */
const TransportDetails: Component<{ transport: McpTransport }> = (props) => {
  const asStdio = () => props.transport as McpStdioTransport;
  const asHttp = () => props.transport as McpHttpTransport;
  const pairsText = (map: Record<string, string>) =>
    Object.entries(map)
      .map(([k, v]) => `${k}=${v}`)
      .join('\n');
  return (
    <div class="flex flex-col gap-1.5 min-w-0">
      <Show when={props.transport.transport === 'stdio'}>
        <DetailRow label={t('mcp.trust.command')} value={asStdio().command} />
        <Show when={asStdio().args.length > 0}>
          <DetailRow label={t('mcp.trust.args')} value={asStdio().args.join(' ')} />
        </Show>
        <Show when={Object.keys(asStdio().env).length > 0}>
          <DetailRow label={t('mcp.trust.env')} value={pairsText(asStdio().env)} />
        </Show>
      </Show>
      <Show when={props.transport.transport !== 'stdio'}>
        <DetailRow label={t('mcp.trust.url')} value={asHttp().url} />
        <Show when={Object.keys(asHttp().headers).length > 0}>
          <DetailRow label={t('mcp.trust.headers')} value={pairsText(asHttp().headers)} />
        </Show>
      </Show>
    </div>
  );
};

/**
 * Project-level MCP server trust prompt (P0-1 frontend side).
 * Repo-shipped `.aio/mcp-servers.json` servers never auto-start; the user must
 * confirm trust here (persisted outside the repo, fingerprinted so repo changes
 * revoke it). Dismissing is session-only — the prompt returns on next app start.
 */
const ProjectMcpTrustModal: Component = () => {
  const servers = () => pendingProjectMcpServers();
  const dismissAll = () => servers().forEach((cfg) => dismissProjectMcpServer(cfg.id));

  return (
    <Show when={servers().length > 0}>
      <div class="fixed inset-0 z-[1100] flex items-center justify-center bg-black/25 rounded-lg">
        <div
          class="rounded-lg p-6 w-[92%] max-w-[560px] max-h-[80vh] overflow-y-auto flex flex-col gap-4"
          style={{
            background: 'var(--dialog-bg)',
            'backdrop-filter': 'var(--popup-blur)',
            border: '1px solid var(--border-dim)',
          }}
        >
          <div class="flex justify-between items-center border-b border-[rgba(var(--text-base-rgb),0.27)] pb-3">
            <h2 class="m-0 text-xl">{t('mcp.trust.title')}</h2>
            <button onClick={dismissAll} class={btnClose} aria-label={t('common.close')}>
              <Icon name="x" size={18} class="stroke-[2.5]" />
            </button>
          </div>
          <p class="m-0 text-xs" style={{ color: 'rgba(var(--text-base-rgb),0.55)' }}>
            {t('mcp.trust.description')}
          </p>
          <div class="flex flex-col gap-3">
            <For each={servers()}>
              {(cfg) => (
                <div class="rounded-lg border border-dark-100 p-3 flex flex-col gap-2.5">
                  <div class="flex items-center gap-2">
                    <div class="flex-1 min-w-0 text-sm text-white font-medium truncate">
                      {cfg.displayName || cfg.id}
                    </div>
                    <Show when={cfg.autoStart}>
                      <span
                        class="px-1.5 py-0.5 rounded text-[10px] shrink-0"
                        style={{
                          background: 'rgba(var(--primary-rgb),0.15)',
                          color: 'rgba(var(--primary-rgb),1)',
                        }}
                      >
                        {t('mcp.trust.autoStart')}
                      </span>
                    </Show>
                  </div>
                  <TransportDetails transport={cfg.transport} />
                  <div class="flex justify-end gap-2">
                    <button class={btnSecondarySm} onClick={() => dismissProjectMcpServer(cfg.id)}>
                      {t('mcp.trust.skip')}
                    </button>
                    <button
                      class={btnPrimaryOutlineSm}
                      onClick={() => void confirmProjectMcpServer(cfg)}
                    >
                      {t('mcp.trust.trustAndStart')}
                    </button>
                  </div>
                </div>
              )}
            </For>
          </div>
        </div>
      </div>
    </Show>
  );
};

export default ProjectMcpTrustModal;

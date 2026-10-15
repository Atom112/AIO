/**
 * Tool-call approval bubble component
 *
 * When an agent in normal/plan mode needs user confirmation for a tool call,
 * this bubble is shown inline in the chat with approve/deny actions.
 */
import { Component, Show } from 'solid-js';
import { respondToolApproval } from '../../../core/api/mcp';
import Icon from '../../../shared/components/Icon';
import { t } from '../../../core/i18n';
import { btnDanger, btnSuccess } from '../../../shared/components/buttonStyles';
import type { PendingApproval } from '../../../core/store/store';

// Type moved to core/store/stream.ts (CHAT-12: global stream state); re-exported
// here for existing importers of this module.
export type { PendingApproval };

interface ToolApprovalBubbleProps {
  approval: PendingApproval;
  onResolved: (approvalId: string) => void;
}

const ToolApprovalBubble: Component<ToolApprovalBubbleProps> = (props) => {
  const handleApprove = async () => {
    try {
      await respondToolApproval(props.approval.approvalId, true);
    } catch (e) {
      console.error('Failed to send approval result:', e);
    } finally {
      props.onResolved(props.approval.approvalId);
    }
  };

  // Always allow — project-bound calls persist the derived rule into .aio/permissions.json
  // (never asks again, survives restarts); project-less calls fall back to session scope.
  // Derived rule: execute_command → first command token; file writes → same file; MCP → same server + tool.
  const persistable = () => props.approval.projectBound === true;
  const handleAlwaysAllow = async () => {
    try {
      await respondToolApproval(
        props.approval.approvalId,
        true,
        persistable() ? 'always' : 'session',
      );
    } catch (e) {
      console.error('Failed to send always-allow result:', e);
    } finally {
      props.onResolved(props.approval.approvalId);
    }
  };

  const handleReject = async () => {
    try {
      await respondToolApproval(props.approval.approvalId, false);
    } catch (e) {
      console.error('Failed to send denial result:', e);
    } finally {
      props.onResolved(props.approval.approvalId);
    }
  };

  // MCP tool calls carry the originating server id; builtin file/command tools use the
  // reserved builtin ids (mirrors the filter in McpServerList) and get no chip.
  const BUILTIN_SERVER_IDS = new Set(['__builtin__', '__aio-filesystem__']);
  const serverLabel = () =>
    BUILTIN_SERVER_IDS.has(props.approval.serverId) ? null : props.approval.serverId;

  // Format arguments for display
  const formatArgs = () => {
    const args = props.approval.arguments;
    if (!args || Object.keys(args).length === 0) return t('agent.approval.noArguments');
    const parts: string[] = [];
    // Put the path argument first
    if (args.path) {
      parts.push(`path: "${args.path}"`);
    }
    for (const [key, val] of Object.entries(args)) {
      if (key === 'path') continue;
      if (key === 'content') {
        const str = String(val);
        parts.push(`content: "${str.slice(0, 80)}${str.length > 80 ? '...' : ''}"`);
      } else {
        parts.push(`${key}: ${JSON.stringify(val)}`);
      }
    }
    return parts.join(', ');
  };

  // Choose an icon based on the tool name
  const toolIcon = () => {
    const name = props.approval.toolName;
    if (name === 'write_file') return 'edit';
    if (name === 'delete_file') return 'trash';
    if (name === 'make_directory') return 'folder-plus';
    if (name === 'read_file') return 'file';
    if (name.startsWith('search')) return 'search';
    return 'wrench';
  };

  // Determine danger level from the tool name
  const isDangerous = () => {
    const name = props.approval.toolName;
    return name === 'delete_file';
  };

  return (
    <div
      class="rounded-[10px] px-3.5 py-3 my-1 max-w-[420px] bg-[rgba(var(--surface-alt-bg),0.95)] border border-[rgba(var(--primary-rgb),0.15)] shadow-[0_4px_20px_rgba(0,0,0,0.3)]"
      classList={{ 'border-[rgba(255,77,77,0.25)]': isDangerous() }}
      style={{ animation: 'tool-approval-in 0.2s ease-out' }}
    >
      <div class="flex items-center gap-1.5 mb-1.5">
        <Icon name={toolIcon() as any} size={15} class="text-[var(--color-pri)]/60 shrink-0" />
        <span class="text-[13px] font-semibold text-white/85 font-mono">
          {props.approval.toolName}
        </span>
        <Show when={serverLabel()}>
          {(sid) => (
            <span
              class="text-[10px] text-white/35 font-mono rounded bg-white/5 px-1 py-px truncate max-w-[160px]"
              title={sid()}
            >
              {sid()}
            </span>
          )}
        </Show>
        <Show when={isDangerous()}>
          <span class="text-[10px] font-semibold py-px px-1.5 rounded bg-[var(--color-danger)]/15 text-[var(--color-danger)]/80 uppercase tracking-[0.3px]">
            {t('agent.approval.dangerous')}
          </span>
        </Show>
      </div>
      <div class="text-[11px] text-white/55 bg-black/20 rounded-md px-2 py-1.5 mb-1.5 overflow-hidden text-ellipsis whitespace-nowrap">
        <code>{formatArgs()}</code>
      </div>
      {/* Backend reason carries the specifics (untrusted MCP server name, danger
          annotations, sandbox notices); the static title is only the fallback. */}
      <div class="text-[11px] text-white/45 mb-2.5 leading-[1.4] whitespace-pre-wrap break-words max-h-32 overflow-y-auto">
        {props.approval.reason || t('agent.approval.title')}
      </div>
      <Show when={props.approval.previewDiff}>
        <div
          class="mb-2.5 rounded-md overflow-hidden border border-white/[0.06]"
          style={{ background: 'rgba(0,0,0,0.3)' }}
        >
          <div
            class="text-[10px] text-white/35 px-2 py-1 border-b border-white/[0.04]"
            style={{ 'font-family': 'monospace' }}
          >
            {props.approval.filePath || t('agent.approval.diffPreview')}
          </div>
          <pre
            class="text-[11px] leading-[1.5] p-2 m-0 overflow-x-auto text-white/60"
            style={{
              'font-family': "'JetBrains Mono', 'Fira Code', monospace",
              'max-height': '200px',
            }}
          >
            {props.approval.previewDiff}
          </pre>
        </div>
      </Show>
      <div class="flex gap-2 justify-end">
        <button type="button" class={btnDanger} onClick={handleReject}>
          <Icon name="x" size={13} />
          <span>{t('agent.approval.deny')}</span>
        </button>
        <button
          type="button"
          class={btnSuccess}
          onClick={handleAlwaysAllow}
          title={persistable() ? t('agent.approval.allowAlwaysPersistHint') : undefined}
        >
          <Icon name="check" size={13} />
          <span>
            {persistable()
              ? t('agent.approval.allowAlwaysPersist')
              : t('agent.approval.allowAlways')}
          </span>
        </button>
        <button type="button" class={btnSuccess} onClick={handleApprove}>
          <Icon name="check" size={13} />
          <span>{t('agent.approval.approve')}</span>
        </button>
      </div>
    </div>
  );
};

export default ToolApprovalBubble;

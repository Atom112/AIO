/**
 * 工具调用审批气泡组件
 *
 * 当 agent 处于 normal/plan 模式，工具调用需要用户确认时，
 * 在 chat 中内联显示此气泡，提供批准/拒绝操作。
 */
import { Component, Show } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import Icon from '../../../shared/components/Icon';
import { t } from '../../../core/i18n';
import { btnDanger, btnSuccess } from '../../../shared/components/buttonStyles';

export interface PendingApproval {
  approvalId: string;
  serverId: string;
  toolName: string;
  arguments: any;
  reason: string;
  /** 文件变更预览（unified diff） */
  previewDiff?: string;
  /** 受影响的文件路径 */
  filePath?: string;
}

interface ToolApprovalBubbleProps {
  approval: PendingApproval;
  onResolved: (approvalId: string) => void;
}

const ToolApprovalBubble: Component<ToolApprovalBubbleProps> = (props) => {
  const handleApprove = async () => {
    try {
      await invoke('respond_tool_approval', {
        approvalId: props.approval.approvalId,
        approved: true,
      });
    } catch (e) {
      console.error('发送批准结果失败:', e);
    } finally {
      props.onResolved(props.approval.approvalId);
    }
  };

  const handleReject = async () => {
    try {
      await invoke('respond_tool_approval', {
        approvalId: props.approval.approvalId,
        approved: false,
      });
    } catch (e) {
      console.error('发送拒绝结果失败:', e);
    } finally {
      props.onResolved(props.approval.approvalId);
    }
  };

  // 格式化参数显示
  const formatArgs = () => {
    const args = props.approval.arguments;
    if (!args || Object.keys(args).length === 0) return t('agent.approval.noArguments');
    const parts: string[] = [];
    // path 参数放最前面
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

  // 根据工具名选择图标
  const toolIcon = () => {
    const name = props.approval.toolName;
    if (name === 'write_file') return 'edit';
    if (name === 'delete_file') return 'trash';
    if (name === 'make_directory') return 'folder-plus';
    if (name === 'read_file') return 'file';
    if (name.startsWith('search')) return 'search';
    return 'wrench';
  };

  // 根据工具名判断危险等级
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
        <Icon name={toolIcon() as any} size={15} class="text-[#7c9abf]/60 shrink-0" />
        <span class="text-[13px] font-semibold text-white/85 font-mono">
          {props.approval.toolName}
        </span>
        <Show when={isDangerous()}>
          <span class="text-[10px] font-semibold py-px px-1.5 rounded bg-[#ff4d4d]/15 text-[#ff4d4d]/80 uppercase tracking-[0.3px]">
            {t('agent.approval.dangerous')}
          </span>
        </Show>
      </div>
      <div class="text-[11px] text-white/55 bg-black/20 rounded-md px-2 py-1.5 mb-1.5 overflow-hidden text-ellipsis whitespace-nowrap">
        <code>{formatArgs()}</code>
      </div>
      <div class="text-[11px] text-white/45 mb-2.5 leading-[1.4]">{t('agent.approval.title')}</div>
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
        <button type="button" class={btnSuccess} onClick={handleApprove}>
          <Icon name="check" size={13} />
          <span>{t('agent.approval.approve')}</span>
        </button>
      </div>
    </div>
  );
};

export default ToolApprovalBubble;

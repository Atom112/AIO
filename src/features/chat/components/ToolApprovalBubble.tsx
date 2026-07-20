/**
 * 工具调用审批气泡组件
 *
 * 当 agent 处于 normal/plan 模式，工具调用需要用户确认时，
 * 在 chat 中内联显示此气泡，提供批准/拒绝操作。
 */
import { Component, Show } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import Icon from '../../../shared/components/Icon';

export interface PendingApproval {
    approvalId: string;
    serverId: string;
    toolName: string;
    arguments: any;
    reason: string;
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
        if (!args || Object.keys(args).length === 0) return '无参数';
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
            class="rounded-[10px] px-3.5 py-3 my-1 max-w-[420px] bg-[rgba(22,28,46,0.95)] border border-[rgba(124,154,191,0.15)] shadow-[0_4px_20px_rgba(0,0,0,0.3)]"
            classList={{ 'border-[rgba(255,77,77,0.25)]': isDangerous() }}
            style={{ animation: 'tool-approval-in 0.2s ease-out' }}
        >
            <div class="flex items-center gap-1.5 mb-1.5">
                <Icon name={toolIcon() as any} size={15} class="text-[#7c9abf]/60 shrink-0" />
                <span class="text-[13px] font-semibold text-white/85 font-mono">{props.approval.toolName}</span>
                <Show when={isDangerous()}>
                    <span class="text-[10px] font-semibold py-px px-1.5 rounded bg-[#ff4d4d]/15 text-[#ff4d4d]/80 uppercase tracking-[0.3px]">危险操作</span>
                </Show>
            </div>
            <div class="text-[11px] text-white/55 bg-black/20 rounded-md px-2 py-1.5 mb-1.5 overflow-hidden text-ellipsis whitespace-nowrap">
                <code>{formatArgs()}</code>
            </div>
            <div class="text-[11px] text-white/45 mb-2.5 leading-[1.4]">
                {props.approval.reason}
            </div>
            <div class="flex gap-2 justify-end">
                <button
                    type="button"
                    class="flex items-center gap-1 px-3 py-[5px] rounded-md text-xs font-medium border-none cursor-pointer transition-all duration-150 bg-white/[0.06] text-white/60 hover:bg-[#ff4d4d]/15 hover:text-[#ff4d4d]/80"
                    onClick={handleReject}
                >
                    <Icon name="x" size={13} />
                    <span>拒绝</span>
                </button>
                <button
                    type="button"
                    class="flex items-center gap-1 px-3 py-[5px] rounded-md text-xs font-medium border-none cursor-pointer transition-all duration-150 bg-[#4af908]/10 text-[#4af908]/70 hover:bg-[#4af908]/20 hover:text-[#4af908]/90"
                    onClick={handleApprove}
                >
                    <Icon name="check" size={13} />
                    <span>批准</span>
                </button>
            </div>
        </div>
    );
};

export default ToolApprovalBubble;

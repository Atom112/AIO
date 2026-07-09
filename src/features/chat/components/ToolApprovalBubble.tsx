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
            class="tool-approval-bubble"
            classList={{ 'is-dangerous': isDangerous() }}
        >
            <div class="tool-approval-header">
                <Icon name={toolIcon() as any} size={15} class="tool-approval-icon" />
                <span class="tool-approval-toolname">{props.approval.toolName}</span>
                <Show when={isDangerous()}>
                    <span class="tool-approval-badge">危险操作</span>
                </Show>
            </div>
            <div class="tool-approval-args">
                <code>{formatArgs()}</code>
            </div>
            <div class="tool-approval-reason">
                {props.approval.reason}
            </div>
            <div class="tool-approval-actions">
                <button
                    type="button"
                    class="tool-approval-btn tool-approval-btn-reject"
                    onClick={handleReject}
                >
                    <Icon name="x" size={13} />
                    <span>拒绝</span>
                </button>
                <button
                    type="button"
                    class="tool-approval-btn tool-approval-btn-approve"
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

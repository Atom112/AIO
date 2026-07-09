/**
 * @file ProjectCreateModal.tsx
 * @description 新建项目弹窗：输入名称 + 选择目录。
 */
import { createSignal } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import type { Project } from '../../../core/store/store';

interface Props {
    onClose: () => void;
    onCreated: (projectId: string) => void;
}

export default function ProjectCreateModal(props: Props) {
    const [name, setName] = createSignal('');
    const [path, setPath] = createSignal('');
    const [error, setError] = createSignal('');
    const [loading, setLoading] = createSignal(false);

    const handleSelectDir = async () => {
        try {
            const selected = await open({ directory: true, multiple: false });
            if (selected && typeof selected === 'string') {
                setPath(selected);
                // 自动填充名称为目录名
                const dirName = selected.split(/[/\\]/).filter(Boolean).pop() || '';
                if (!name()) setName(dirName);

                // 验证路径
                try {
                    const result = await invoke<{ valid: boolean; reason: string; canonicalPath: string }>(
                        'validate_project_path',
                        { path: selected }
                    );
                    if (!result.valid) {
                        setError(result.reason);
                    } else {
                        setPath(result.canonicalPath || selected);
                        setError('');
                    }
                } catch {
                    setError('');
                }
            }
        } catch (e) {
            console.warn('选择目录失败:', e);
        }
    };

    const handleCreate = async () => {
        if (!name().trim()) {
            setError('请输入项目名称');
            return;
        }
        if (!path()) {
            setError('请选择项目目录');
            return;
        }
        setLoading(true);
        setError('');
        try {
            const project = await invoke<Project>('create_project', {
                name: name().trim(),
                path: path(),
            });
            props.onCreated(project.id);
        } catch (e) {
            setError(String(e));
        } finally {
            setLoading(false);
        }
    };

    return (
        <div
            class="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
            onClick={props.onClose}
        >
            <div
                class="bg-[#1a2540] border border-white/15 rounded-xl shadow-2xl w-full max-w-md p-6"
                onClick={(e) => e.stopPropagation()}
            >
                <h2 class="text-lg font-semibold text-white mb-4">新建项目</h2>

                {/* 项目名称 */}
                <label class="block text-sm text-white/60 mb-1.5">项目名称</label>
                <input
                    type="text"
                    class="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white
                           text-sm placeholder-white/30 focus:outline-none focus:border-[#7c9abf] mb-4"
                    placeholder="输入项目名称..."
                    value={name()}
                    onInput={(e) => setName(e.currentTarget.value)}
                    onKeyDown={(e) => e.key === 'Enter' && handleCreate()}
                />

                {/* 项目目录 */}
                <label class="block text-sm text-white/60 mb-1.5">项目目录</label>
                <div class="flex gap-2 mb-1">
                    <input
                        type="text"
                        class="flex-1 px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white
                               text-sm placeholder-white/30 focus:outline-none focus:border-[#7c9abf]"
                        placeholder="选择文件系统目录..."
                        value={path()}
                        readOnly
                    />
                    <button
                        class="px-3 py-2 bg-white/10 hover:bg-white/20 text-white/80 text-sm
                               rounded-lg transition-colors border border-white/10 shrink-0"
                        onClick={handleSelectDir}
                    >
                        浏览...
                    </button>
                </div>

                {/* 错误信息 */}
                {error() && (
                    <p class="text-red-400 text-xs mb-3">{error()}</p>
                )}

                <p class="text-white/30 text-xs mb-4">
                    项目目录下将自动创建 <code class="bg-white/10 px-1 rounded">.aio/</code> 文件夹，存放项目专属的 Skills 和 MCP 配置。
                </p>

                {/* 按钮 */}
                <div class="flex justify-end gap-3">
                    <button
                        class="px-4 py-2 text-sm text-white/60 hover:text-white/90 transition-colors"
                        onClick={props.onClose}
                    >
                        取消
                    </button>
                    <button
                        class="px-5 py-2 text-sm bg-[#7c9abf] hover:bg-[#6b8aaf] text-white
                               rounded-lg transition-colors disabled:opacity-50"
                        onClick={handleCreate}
                        disabled={loading()}
                    >
                        {loading() ? '创建中...' : '创建项目'}
                    </button>
                </div>
            </div>
        </div>
    );
}

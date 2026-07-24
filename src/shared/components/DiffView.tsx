import { createSignal, For, Show } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { currentProject } from '../../core/store/store';
import type { FileChangeInfo } from '../../core/store/store';

interface DiffViewProps {
    changes: FileChangeInfo[];
    /** 当整轮文件变更已被批量撤销时为 true，所有卡片变暗且隐藏单项撤销按钮 */
    allReverted?: boolean;
}

const ACTION_COLORS: Record<FileChangeInfo['action'], string> = {
    create: 'rgba(80, 220, 100, 0.8)',
    modify: 'rgba(252, 196, 72, 0.8)',
    delete: 'rgba(255, 80, 80, 0.8)',
};

const ACTION_LABELS: Record<FileChangeInfo['action'], string> = {
    create: 'New',
    modify: 'Mod',
    delete: 'Del',
};

function parseDiffLines(diff: string): { type: 'ctx' | 'add' | 'del' | 'hdr'; text: string }[] {
    const lines: { type: 'ctx' | 'add' | 'del' | 'hdr'; text: string }[] = [];
    for (const line of diff.split('\n')) {
        if (line.startsWith('@@')) {
            lines.push({ type: 'hdr', text: line });
        } else if (line.startsWith('+') && !line.startsWith('+++')) {
            lines.push({ type: 'add', text: line });
        } else if (line.startsWith('-') && !line.startsWith('---')) {
            lines.push({ type: 'del', text: line });
        } else {
            lines.push({ type: 'ctx', text: line });
        }
    }
    return lines;
}

/** 解析 hunk header `@@ -1,5 +1,6 @@` 为红绿分段 */
function parseHunkHeader(text: string): { text: string; color?: string }[] {
    const segs: { text: string; color?: string }[] = [];
    // regex: match leading @@, then -N[,M], then +N[,M], then trailing @@
    const m = text.match(/^(@@\s*)(-\d+(?:,\d+)?)(\s+)(\+\d+(?:,\d+)?)(\s*@@)/);
    if (m) {
        segs.push({ text: m[1] });
        segs.push({ text: m[2], color: '#ff5050' });      // red for removed line range
        segs.push({ text: m[3] });
        segs.push({ text: m[4], color: '#50dc64' });      // green for added line range
        segs.push({ text: m[5] });
    } else {
        segs.push({ text });
    }
    return segs;
}

/** 解析 summary `+N -M` 为红绿分段 */
function parseSummary(summary: string): { text: string; color?: string }[] {
    const segs: { text: string; color?: string }[] = [];
    // split on whitespace, preserve delimiters
    const parts = summary.split(/(\s+)/g);
    for (const p of parts) {
        if (p.startsWith('+')) {
            segs.push({ text: p, color: '#50dc64' });
        } else if (p.startsWith('-')) {
            segs.push({ text: p, color: '#ff5050' });
        } else {
            segs.push({ text: p });
        }
    }
    return segs;
}

const DiffView = (props: DiffViewProps) => {
    return (
        <For each={props.changes}>
            {(change) => {
                const [expanded, setExpanded] = createSignal(false);
                const diffLines = () => parseDiffLines(change.diff);

                const handleRevert = async () => {
                    try {
                        const projectPath = currentProject()?.path;
                        if (!projectPath) return;
                        await invoke('revert_file_change', {
                            projectPath,
                            filePath: change.filePath,
                        });
                    } catch (err) {
                        console.error('Revert failed:', err);
                    }
                };
                return (
                    <div class="rounded-md overflow-hidden mb-1.5 border border-white/[0.06] bg-white/[0.01] transition-opacity duration-300" classList={{ 'opacity-40': !!props.allReverted }}>
                        <button
                            type="button"
                            class="flex items-center gap-2 w-full px-2.5 py-1.5 cursor-pointer select-none bg-white/[0.03] border-none text-[13px] transition-colors hover:bg-white/[0.05]"
                            onClick={() => setExpanded(!expanded())}
                        >
                            <span class="flex items-center justify-center w-4 h-4 rounded bg-white/[0.05] shrink-0" style="font-size: 11px;">
                                {'<>'}
                            </span>
                            <span class="truncate font-mono text-white/70 flex-1 min-w-0 text-left" style={props.allReverted ? 'text-decoration: line-through;' : ''}>
                                {change.filePath}
                            </span>
                            <span
                                class="px-1.5 py-px rounded text-[12px] font-medium shrink-0"
                                style={{
                                    color: ACTION_COLORS[change.action],
                                    background: 'rgba(255,255,255,0.04)',
                                    border: `1px solid ${ACTION_COLORS[change.action]}33`,
                                }}
                            >
                                {ACTION_LABELS[change.action]}
                            </span>
                            <span class="font-mono text-[12px] shrink-0">
                                <For each={parseSummary(change.summary)}>
                                    {(seg) => (
                                        <span style={seg.color ? { color: seg.color } : { color: 'rgba(255,255,255,0.35)' }}>
                                            {seg.text}
                                        </span>
                                    )}
                                </For>
                            </span>
                            <Show when={!props.allReverted}>
                                <span
                                    class="flex items-center justify-center w-5 h-5 rounded hover:bg-white/[0.08] shrink-0 text-white/25 hover:text-red-400 transition-colors cursor-pointer"
                                    title="Revert this change"
                                    onClick={(e) => { e.stopPropagation(); handleRevert(); }}
                                >
                                    <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="2" stroke="currentColor" class="w-3.5 h-3.5">
                                        <path stroke-linecap="round" stroke-linejoin="round" d="M9 15L3 9m0 0l6-6M3 9h12a6 6 0 010 12h-3" />
                                    </svg>
                                </span>
                            </Show>
                            <span
                                class="flex items-center justify-center shrink-0 text-white/20 transition-transform duration-150 w-4 h-4"
                                style={{ transform: expanded() ? 'rotate(180deg)' : 'rotate(0deg)' }}
                            >
                                <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="2" stroke="currentColor" class="w-3 h-3">
                                    <path stroke-linecap="round" stroke-linejoin="round" d="M19.5 8.25l-7.5 7.5-7.5-7.5" />
                                </svg>
                            </span>
                        </button>
                        <div
                            style={{
                                'max-height': expanded() ? '320px' : '0px',
                                opacity: expanded() ? '1' : '0',
                                transition: 'max-height 0.2s ease, opacity 0.15s ease',
                                overflow: 'hidden',
                            }}
                        >
                            <div
                                class="overflow-x-auto overflow-y-auto"
                                style={{
                                    'max-height': '320px',
                                    'font-family': "'Cascadia Code', 'Fira Code', 'JetBrains Mono', Consolas, monospace",
                                    'font-size': '12px',
                                    'line-height': '1.5',
                                }}
                            >
                                <For each={diffLines()}>
                                    {(dl) => {
                                        const hunkSegs = () => dl.type === 'hdr' ? parseHunkHeader(dl.text) : null;
                                        const addColor = 'rgba(80, 220, 100, 0.7)';
                                        const delColor = 'rgba(255, 80, 80, 0.7)';
                                        return (
                                            <div
                                                class="px-2.5 py-px whitespace-pre"
                                                classList={{
                                                    'bg-emerald-500/[0.08]': dl.type === 'add',
                                                    'bg-red-500/[0.08]': dl.type === 'del',
                                                    'text-white/20 font-medium': dl.type === 'hdr',
                                                    'text-white/40': dl.type === 'ctx',
                                                }}
                                                style={{
                                                    color:
                                                        dl.type === 'add' ? addColor :
                                                        dl.type === 'del' ? delColor :
                                                        undefined,
                                                }}
                                            >
                                                <Show when={dl.type === 'hdr' && hunkSegs()} fallback={dl.text || ' '}>
                                                    <For each={hunkSegs()!}>
                                                        {(seg) => (
                                                            <span style={seg.color ? { color: seg.color } : undefined}>{seg.text}</span>
                                                        )}
                                                    </For>
                                                </Show>
                                            </div>
                                        );
                                    }}
                                </For>
                            </div>
                        </div>
                    </div>
                );
            }}
        </For>
    );
};

export default DiffView;
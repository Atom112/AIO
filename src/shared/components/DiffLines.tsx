/**
 * @file DiffLines.tsx
 * @description Unified-diff parsing plus line rendering, shared by the inline diff
 * cards (chat file-change bubbles, harness approval preview) and the chat side-panel
 * file viewer. Rendering lives here so both surfaces stay pixel-identical.
 */
import { For, Show } from 'solid-js';
import { t } from '../../core/i18n';
import type { FileChangeInfo } from '../../core/store/types';

/** Action-chip colors per file-change action. */
export const ACTION_COLORS: Record<FileChangeInfo['action'], string> = {
  create: 'rgba(80, 220, 100, 0.8)',
  modify: 'rgba(252, 196, 72, 0.8)',
  delete: 'rgba(255, 80, 80, 0.8)',
};

/**
 * FE-06: resolved lazily per render — the previous module-level object froze
 * `t()` at module-load locale, leaving these labels stuck in the old language
 * after a switch.
 */
export const actionLabel = (action: FileChangeInfo['action']): string => {
  switch (action) {
    case 'create':
      return t('diff.new');
    case 'modify':
      return t('diff.mod');
    case 'delete':
      return t('diff.del');
  }
};

/** Parsed unified-diff line: hunk header, added, removed or context. */
export interface ParsedDiffLine {
  type: 'ctx' | 'add' | 'del' | 'hdr';
  text: string;
}

/** Split a unified diff into renderable lines (file headers `+++`/`---` stay context). */
export function parseDiffLines(diff: string): ParsedDiffLine[] {
  const lines: ParsedDiffLine[] = [];
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

/** Parse hunk header `@@ -1,5 +1,6 @@` into red/green segments. */
export function parseHunkHeader(text: string): { text: string; color?: string }[] {
  const segs: { text: string; color?: string }[] = [];
  // regex: match leading @@, then -N[,M], then +N[,M], then trailing @@
  const m = text.match(/^(@@\s*)(-\d+(?:,\d+)?)(\s+)(\+\d+(?:,\d+)?)(\s*@@)/);
  if (m) {
    segs.push({ text: m[1] });
    segs.push({ text: m[2], color: 'var(--color-danger)' }); // red for removed line range
    segs.push({ text: m[3] });
    segs.push({ text: m[4], color: 'var(--color-success)' }); // green for added line range
    segs.push({ text: m[5] });
  } else {
    segs.push({ text });
  }
  return segs;
}

/** Parse summary `+N -M` into red/green segments. */
export function parseSummary(summary: string): { text: string; color?: string }[] {
  const segs: { text: string; color?: string }[] = [];
  // split on whitespace, preserve delimiters
  const parts = summary.split(/(\s+)/g);
  for (const p of parts) {
    if (p.startsWith('+')) {
      segs.push({ text: p, color: 'var(--color-success)' });
    } else if (p.startsWith('-')) {
      segs.push({ text: p, color: 'var(--color-danger)' });
    } else {
      segs.push({ text: p });
    }
  }
  return segs;
}

/**
 * Render one unified diff as colored lines.
 * @param props.diff raw unified diff text
 */
const DiffLines = (props: { diff: string }) => {
  return (
    <For each={parseDiffLines(props.diff)}>
      {(dl) => {
        const hunkSegs = () => (dl.type === 'hdr' ? parseHunkHeader(dl.text) : null);
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
              color: dl.type === 'add' ? addColor : dl.type === 'del' ? delColor : undefined,
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
  );
};

export default DiffLines;

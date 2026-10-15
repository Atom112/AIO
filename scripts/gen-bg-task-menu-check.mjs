// Visual check: agent status-bar right group — background-task pull-up menu (app
// background jobs: memory extraction/consolidation, reindex, code index, model
// downloads) LEFT of the subagent pull-up menu, both themes. Usage:
// node scripts/gen-bg-task-menu-check.mjs [outfile]
import { readFileSync, writeFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const out = process.argv[2] || 'bg-task-menu-check.html';
const assetsDir = 'dist/assets';
const cssFile = readdirSync(assetsDir).find((f) => /^index-.*\.css$/.test(f));
const css = readFileSync(join(assetsDir, cssFile), 'utf8');

// icon path snippets mirroring shared/components/Icon.tsx PATHS entries
const ICONS = {
  brain:
    '<path d="M9 4a3 3 0 0 0-3 3v0a3 3 0 0 0-2 5 3 3 0 0 0 1 4 3 3 0 0 0 3 3 3 3 0 0 0 3-1l1-1V7l-1-1a3 3 0 0 0-3-2Z"/><path d="M15 4a3 3 0 0 1 3 3v0a3 3 0 0 1 2 5 3 3 0 0 1-1 4 3 3 0 0 1-3 3 3 3 0 0 1-3-1l-1-1V7l1-1a3 3 0 0 1 3-2Z"/>',
  layers: '<path d="m3 12 9-7 9 7-9 7-9-7Z"/><path d="m3 16 9 5 9-5"/><path d="m3 8 9 5 9-5"/>',
  'rotate-ccw': '<path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5"/>',
  code: '<path d="m9 8-4 4 4 4M15 8l4 4-4 4"/>',
  download: '<path d="M12 4v12M7 11l5 5 5-5"/><path d="M4 20h16"/>',
  x: '<path d="M6 6l12 12M18 6 6 18"/>',
  spinner: '<path d="M12 3a9 9 0 1 0 9 9"/>',
  'chevron-up': '<path d="m6 14 6-6 6 6"/>',
  terminal: '<path d="M4 17l6-5-6-5"/><path d="M12 19h8"/>',
};

const svg = (name, size, color, extra = '') =>
  `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" width="${size}" height="${size}" style="color:${color};flex-shrink:0" ${extra}>${ICONS[name]}</svg>`;

// One menu row per task kind, mirroring BackgroundTaskMenu's row structure:
// kind icon chip / title + scope + detail / progress bar / elapsed / error / status.
const row = ({ icon, title, scope, detail, progressPct, progressText, elapsed, error, right }) => `
  <div class="flex items-start gap-2 px-3 py-2" style="text-align:left">
    <span class="flex items-center justify-center rounded shrink-0" style="width:20px;height:20px;background:rgba(var(--text-base-rgb),0.06)">
      ${svg(icon, 12, 'rgba(var(--text-base-rgb),0.55)')}
    </span>
    <span class="flex-1 min-w-0" style="text-align:left">
      <span class="flex items-center gap-1.5">
        <span class="truncate" style="font-size:12px;font-weight:500;color:rgba(var(--text-base-rgb),0.8)">${title}</span>
        <span class="truncate shrink-0" style="font-size:10px;color:rgba(var(--text-base-rgb),0.35)">${scope}</span>
      </span>
      ${detail ? `<span class="block truncate" style="font-size:11px;color:rgba(var(--text-base-rgb),0.35)">${detail}</span>` : ''}
      ${
        progressPct !== undefined
          ? `<span class="flex items-center gap-1.5" style="margin-top:4px">
              <span class="rounded-full overflow-hidden" style="height:4px;flex:1;background:rgba(var(--text-base-rgb),0.1)">
                <span class="block rounded-full" style="height:100%;width:${progressPct}%;background:rgba(var(--primary-rgb),0.65);transition:width 0.2s ease-out"></span>
              </span>
              <span class="shrink-0" style="font-family:ui-monospace,monospace;font-size:10px;color:rgba(var(--text-base-rgb),0.4)">${progressText}</span>
            </span>`
          : ''
      }
      ${
        elapsed !== undefined
          ? `<span class="block" style="font-family:ui-monospace,monospace;font-size:10px;margin-top:2px;color:rgba(var(--text-base-rgb),0.4)">已运行 ${elapsed}</span>`
          : ''
      }
      ${error ? `<span class="block truncate" style="font-size:11px;color:var(--color-danger)">${error}</span>` : ''}
    </span>
    <span class="flex items-center gap-1.5 shrink-0" style="padding-top:2px">${right}</span>
  </div>`;

const spinner = svg('spinner', 12, 'inherit', 'class="animate-spin"');
const statusRight = (status) =>
  status === 'running'
    ? spinner
    : status === 'error'
      ? svg('x', 12, 'rgba(248,113,113,0.7)')
      : `<span style="font-size:10px;color:rgba(var(--text-base-rgb),0.45)">${status}</span>`;

const ROWS = [
  row({
    icon: 'brain',
    title: '记忆提取',
    scope: 'AIO',
    right: statusRight('running'),
  }),
  row({
    icon: 'layers',
    title: '记忆整理',
    scope: 'AIO',
    right: statusRight('running'),
  }),
  row({
    icon: 'rotate-ccw',
    title: '索引重建',
    scope: 'AIO',
    progressPct: 62,
    progressText: '31 / 50',
    right: statusRight('running'),
  }),
  row({
    icon: 'code',
    title: '代码索引',
    scope: 'demo-project',
    right: statusRight('2m05s'),
  }),
  row({
    icon: 'download',
    title: '模型下载',
    scope: 'bge-m3',
    detail: 'onnx/model_quantized.onnx',
    progressPct: 37,
    progressText: '384.2 / 1024.0 MB',
    right: statusRight('running'),
  }),
  row({
    icon: 'download',
    title: '本地模型下载',
    scope: 'qwen3-4b-instruct',
    detail: 'qwen3-4b-instruct-q4_k_m.gguf',
    progressPct: 82,
    progressText: '2.1 / 2.6 GB',
    right: statusRight('running'),
  }),
  row({
    icon: 'download',
    title: '模型下载',
    scope: 'bge-m3',
    error: '下载中断: connection reset',
    right: statusRight('error'),
  }),
  row({
    icon: 'terminal',
    title: '命令执行',
    scope: 'cargo build --release',
    elapsed: '1m22s',
    right: statusRight('running'),
  }),
  row({
    icon: 'terminal',
    title: '命令执行',
    scope: 'node scripts/watch.mjs',
    right: statusRight('已取消'),
  }),
];

const popup = (title, rows) => `
  <div class="absolute bottom-full right-0 mb-2 z-[41] w-[340px] rounded-xl overflow-hidden" style="background:var(--popup-bg);border:1px solid var(--border-dim);backdrop-filter:var(--popup-blur);box-shadow:0 -8px 30px rgba(0,0,0,0.4)">
    <div class="px-3 py-2 uppercase tracking-widest" style="font-size:11px;font-weight:700;color:rgba(var(--text-base-rgb),0.35);background:rgba(var(--text-base-rgb),0.04);border-bottom:1px solid var(--border-dim)">${title}</div>
    <div class="py-1 max-h-[280px] overflow-y-auto">${rows}</div>
  </div>`;

const trigger = (label, count, spinning) => `
  <button type="button" class="px-1.5 py-0.5 rounded-md" style="font-size:11px;background:rgba(var(--surface-alt-bg),0.5);color:rgba(var(--text-base-rgb),0.55);font-weight:400;border:none;cursor:pointer">
    ${label}${spinning ? svg('spinner', 11, 'inherit', 'class="animate-spin" style="margin:0 2px;display:inline-block;vertical-align:-1px;color:inherit"') : ''} ${count} ${svg('chevron-up', 10, 'inherit', 'style="margin-left:2px;display:inline-block;vertical-align:-1px;color:inherit"')}
  </button>`;

// Status-bar frame mirroring the agent status row: status text left, pull-up group right.
const statusBar = (open) => `
  <div class="rounded-2xl" style="display:flex;align-items:center;gap:8px;padding:0 12px;height:32px;position:relative;width:640px;background:rgba(var(--surface-bg),0.5);border:1px solid rgba(var(--border-rgb),0.15)">
    ${svg('code', 14, 'rgba(var(--text-base-rgb),0.6)')}
    <span style="font-size:12px;color:rgba(var(--text-base-rgb),0.7)">Agent 工作区模式 · demo-project</span>
    <div class="ml-auto flex items-center gap-2">
      <div class="relative inline-block">${trigger('后台任务', '9 个', true)}
        ${open ? popup('后台任务', ROWS.join('')) : ''}
      </div>
      <div class="relative inline-block">${trigger('子智能体', '2 个', false)}
        ${
          open
            ? popup(
                '子智能体',
                row({
                  icon: 'brain',
                  title: '探索者',
                  scope: 'explorer',
                  right: statusRight('1m42s'),
                }),
              )
            : ''
        }
      </div>
    </div>
  </div>`;

const col = (theme, title) => `
  <div style="flex:1;position:relative;min-height:640px" ${theme === 'dark' ? 'data-theme="dark"' : ''}>
    <div style="position:absolute;inset:0;overflow:hidden;background:var(--app-bg)"></div>
    <div style="position:relative;z-index:1;padding:16px;color:rgb(var(--text-base-rgb))">
      <div style="font-weight:600;margin-bottom:24px">${title}</div>
      <div style="margin-bottom:420px">${statusBar(false)}</div>
      <div style="margin-top:420px">${statusBar(true)}</div>
    </div>
  </div>`;

const html = `<!doctype html><html><head><meta charset="utf-8"><style>${css}</style>
<style>html,body{margin:0}</style></head><body>
<div style="display:flex">
  ${col('dark', '深色')}
  ${col('light', '浅色')}
</div>
</body></html>`;

writeFileSync(out, html);
console.log(`Wrote ${out}`);

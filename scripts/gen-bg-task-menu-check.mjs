// Visual check: agent status-bar right group — background-task pull-up menu (fire-and-forget
// delegates, running row carries a stop button) LEFT of the subagent pull-up menu, both themes.
// Usage: node scripts/gen-bg-task-menu-check.mjs [outfile]
import { readFileSync, writeFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const out = process.argv[2] || 'bg-task-menu-check.html';
const assetsDir = 'dist/assets';
const cssFile = readdirSync(assetsDir).find((f) => /^index-.*\.css$/.test(f));
const css = readFileSync(join(assetsDir, cssFile), 'utf8');

// icon path snippets mirroring shared/components/Icon.tsx PATHS entries
const ICONS = {
  search: '<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>',
  code: '<path d="m9 8-4 4 4 4M15 8l4 4-4 4"/>',
  check: '<path d="m5 12 5 5L20 7"/>',
  x: '<path d="M6 6l12 12M18 6 6 18"/>',
  stop: '<rect x="6" y="6" width="12" height="12" rx="1"/>',
  spinner: '<path d="M12 3a9 9 0 1 0 9 9"/>',
  'chevron-up': '<path d="m6 14 6-6 6 6"/>',
};

const svg = (name, size, color, extra = '') =>
  `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" width="${size}" height="${size}" style="color:${color};flex-shrink:0" ${extra}>${ICONS[name]}</svg>`;

const chip = (icon, role) => `
  <span class="relative flex items-center justify-center rounded" style="width:20px;height:20px;flex-shrink:0;background:rgba(var(--role-${role}-rgb),0.1);border:1px solid rgba(var(--role-${role}-rgb),0.3);color:rgba(var(--role-${role}-rgb),0.9)">
    ${svg(icon, 11, 'inherit')}
  </span>`;

const statusIcon = (status) =>
  status === 'running'
    ? svg('spinner', 12, 'inherit', 'class="animate-spin"')
    : status === 'error'
      ? svg('x', 12, 'rgba(248,113,113,0.7)')
      : svg('check', 12, 'rgba(255,255,255,0.3)');

const taskRow = (icon, role, name, profile, task, status, right) => `
  <div class="w-full flex items-center gap-2 px-3 py-2 cursor-pointer hover:bg-[rgba(var(--text-base-rgb),0.05)]" style="color:rgba(var(--text-base-rgb),0.75);text-align:left">
    ${chip(icon, role)}
    <span class="flex items-center justify-center" style="width:14px;height:14px;flex-shrink:0">${statusIcon(status)}</span>
    <span class="flex-1 min-w-0" style="text-align:left">
      <span class="flex items-center gap-1.5">
        <span class="truncate" style="font-size:12px;font-weight:500">${name}</span>
        <span class="shrink-0" style="font-size:10px;color:rgba(255,255,255,0.35)">${profile}</span>
      </span>
      <span class="block truncate" style="font-size:11px;color:rgba(255,255,255,0.35)">${task}</span>
    </span>
    ${right}
  </div>`;

const stopBtn = `
  <button type="button" class="flex items-center justify-center rounded" style="width:20px;height:20px;flex-shrink:0;background:transparent;border:none;cursor:pointer;color:rgba(255,255,255,0.4)" title="停止此后台任务">
    ${svg('stop', 11, 'inherit')}
  </button>`;

const dur = (t) =>
  `<span class="shrink-0" style="font-family:ui-monospace,monospace;font-size:10px;color:rgba(255,255,255,0.25)">${t}</span>`;

const popup = (title, rows) => `
  <div class="absolute bottom-full right-0 mb-2 z-[41] w-[320px] rounded-xl overflow-hidden" style="background:var(--popup-bg);border:1px solid var(--border-dim);backdrop-filter:var(--popup-blur);box-shadow:0 -8px 30px rgba(0,0,0,0.4)">
    <div class="px-3 py-2 uppercase tracking-widest" style="font-size:11px;font-weight:700;color:rgba(var(--text-base-rgb),0.35);background:rgba(var(--text-base-rgb),0.04);border-bottom:1px solid var(--border-dim)">${title}</div>
    <div class="py-1 max-h-[280px] overflow-y-auto">${rows}</div>
  </div>`;

const trigger = (label, count, spinning) => `
  <button type="button" class="px-1.5 py-0.5 rounded-md" style="font-size:11px;background:rgba(var(--surface-alt-bg),0.5);color:rgba(var(--text-base-rgb),0.55);font-weight:400;border:none;cursor:pointer">
    ${label}${spinning ? svg('spinner', 11, 'inherit', 'class="animate-spin" style="margin:0 2px;display:inline-block;vertical-align:-1px;color:inherit"') : ''} ${count} ${svg('chevron-up', 10, 'inherit', 'style="margin-left:2px;display:inline-block;vertical-align:-1px;color:inherit"')}
  </button>`;

// Status-bar frame mirroring the agent status row: status text left, pull-up group right.
// Both menus open to verify side-by-side popups (background left, subagents right).
const statusBar = `
  <div class="rounded-2xl" style="display:flex;align-items:center;gap:8px;padding:0 12px;height:32px;position:relative;width:640px;background:rgba(var(--surface-bg),0.5);border:1px solid rgba(var(--border-rgb),0.15)">
    ${svg('code', 14, 'rgba(var(--text-base-rgb),0.6)')}
    <span style="font-size:12px;color:rgba(var(--text-base-rgb),0.7)">Agent 工作区模式 · demo-project</span>
    <div class="ml-auto flex items-center gap-2">
      <div class="relative inline-block">${trigger('后台任务', '3 个', true)}
        ${popup(
          '后台任务',
          taskRow(
            'search',
            'explorer',
            '探索者',
            'explorer',
            '搜索 workspace 下的模块',
            'running',
            stopBtn,
          ) +
            taskRow('code', 'coder', '实现者', 'coder', '实现下载重试逻辑', 'running', stopBtn) +
            taskRow(
              'search',
              'explorer',
              '探索者',
              'explorer',
              '检查 embedding 目录结构',
              'complete',
              dur('1m42s'),
            ) +
            taskRow('code', 'coder', '实现者', 'coder', '写单元测试', 'error', dur('0m31s')),
        )}
      </div>
      <div class="relative inline-block">${trigger('子智能体', '1 个', false)}
        ${popup('子智能体', taskRow('search', 'explorer', '探索者', 'explorer', '梳理依赖关系图', 'complete', dur('2m05s')))}
      </div>
    </div>
  </div>`;

// Trigger-only variant (menus closed): the two chips sit adjacent on the right.
const statusBarClosed = `
  <div class="rounded-2xl" style="display:flex;align-items:center;gap:8px;padding:0 12px;height:32px;width:640px;background:rgba(var(--surface-bg),0.5);border:1px solid rgba(var(--border-rgb),0.15)">
    ${svg('code', 14, 'rgba(var(--text-base-rgb),0.6)')}
    <span style="font-size:12px;color:rgba(var(--text-base-rgb),0.7)">Agent 工作区模式 · demo-project</span>
    <div class="ml-auto flex items-center gap-2">
      <div class="relative inline-block">${trigger('后台任务', '3 个', true)}</div>
      <div class="relative inline-block">${trigger('子智能体', '1 个', false)}</div>
    </div>
  </div>`;

const col = (theme, title) => `
  <div style="flex:1;position:relative;min-height:560px" ${theme === 'dark' ? 'data-theme="dark"' : ''}>
    <div style="position:absolute;inset:0;overflow:hidden;background:var(--app-bg)"></div>
    <div style="position:relative;z-index:1;padding:16px;color:rgb(var(--text-base-rgb))">
      <div style="font-weight:600;margin-bottom:24px">${title}</div>
      <div style="margin-bottom:200px">${statusBarClosed}</div>
      <div style="margin-top:200px">${statusBar}</div>
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

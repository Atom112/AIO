// Visual check: MCP server-row action buttons (stop/edit/delete) at one height +
// edit-dialog inputs at rounded-lg (matching the ghost Dropdown trigger).
// Usage: node scripts/gen-mcp-row-check.mjs [outfile]
import { readFileSync, writeFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const out = process.argv[2] || 'mcp-row-check.html';
const assetsDir = 'dist/assets';
const cssFile = readdirSync(assetsDir).find((f) => /^index-.*\.css$/.test(f));
const css = readFileSync(join(assetsDir, cssFile), 'utf8');

// Mirrors buttonStyles.ts constants
const btnDanger =
  'inline-flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium border border-danger/30 bg-danger/10 text-danger';
const btnSecondarySm =
  'inline-flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium bg-white/[0.06] border border-white/10 text-white/60';
const btnSuccess =
  'inline-flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium border border-green-400/30 bg-green-400/10 text-green-400';

// Mirrors McpServerList row
const serverRow = `
<div class="flex items-center justify-between" style="width:560px;padding:12px 14px;border:1px solid var(--border-dim);border-radius:10px;background:rgba(var(--text-base-rgb),0.02)">
  <div>
    <div style="font-size:13px;color:rgba(var(--text-base-rgb),0.9)">Getlead</div>
    <div style="font-size:12px;color:rgba(var(--text-base-rgb),0.5);margin-top:4px">streamable_http · 已连接</div>
  </div>
  <div class="flex items-center gap-2 ml-3">
    <button class="${btnDanger}">停止</button>
    <button class="${btnSecondarySm}">编辑</button>
    <button class="${btnDanger}">删除</button>
  </div>
</div>
<div class="flex items-center justify-between" style="width:560px;padding:12px 14px;border:1px solid var(--border-dim);border-radius:10px;background:rgba(var(--text-base-rgb),0.02)">
  <div>
    <div style="font-size:13px;color:rgba(var(--text-base-rgb),0.9)">MS Learn</div>
    <div style="font-size:12px;color:rgba(var(--text-base-rgb),0.5);margin-top:4px">http · 未连接</div>
  </div>
  <div class="flex items-center gap-2 ml-3">
    <button class="${btnSuccess}">启动</button>
    <button class="${btnSecondarySm}">编辑</button>
    <button class="${btnDanger}">删除</button>
  </div>
</div>`;

// Mirrors McpServerDetail inputs (rounded-lg now) next to the ghost Dropdown trigger
const formRows = `
<div style="width:460px;border-radius:12px;padding:16px;background:var(--dialog-bg);backdrop-filter:var(--popup-blur);border:1px solid var(--border-dim)">
  <div style="font-size:12px;color:rgba(var(--text-base-rgb),0.6);margin-bottom:6px">传输类型</div>
  <button class="w-full flex items-center justify-between gap-2 px-3 py-1.5 rounded-lg text-xs text-left border border-[var(--border-dim)]" style="color:rgba(var(--text-base-rgb),0.8);background:transparent">stdio（本地子进程）</button>
  <div style="font-size:12px;color:rgba(var(--text-base-rgb),0.6);margin:12px 0 6px">命令</div>
  <div class="px-3 py-1.5 rounded-lg" style="font-size:13px;background:rgba(0,0,0,0.3);border:1px solid var(--border-dim);color:rgba(var(--text-base-rgb),0.85)">npx</div>
  <div style="font-size:12px;color:rgba(var(--text-base-rgb),0.6);margin:12px 0 6px">参数</div>
  <div class="flex items-center gap-1">
    <div class="flex-1 px-3 py-1.5 rounded-lg" style="font-size:13px;background:rgba(0,0,0,0.3);border:1px solid var(--border-dim);color:rgba(var(--text-base-rgb),0.85)">-y</div>
    <button class="inline-flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium border border-danger/50 bg-white/[0.06] text-white/70"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" width="12" height="12"><path d="M6 6l12 12M18 6 6 18"/></svg></button>
  </div>
</div>`;

const col = (theme, title) => `
<div style="flex:1;position:relative;min-height:430px" ${theme === 'dark' ? 'data-theme="dark"' : ''}>
  <div style="position:absolute;inset:0;overflow:hidden;background:var(--app-bg)"></div>
  <div style="position:relative;z-index:1;padding:16px;color:rgb(var(--text-base-rgb))">
    <div style="font-weight:600;margin-bottom:12px">${title}</div>
    <div style="display:flex;flex-direction:column;gap:10px;margin-bottom:18px">${serverRow}</div>
    ${formRows}
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

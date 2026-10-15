// Visual check: checkpoint-resume button left of the send button — collapsed state
// vs hover-expanded state (the message-history copy-button animation classes applied
// statically), both themes. Usage: node scripts/gen-checkpoint-resume-check.mjs [outfile]
import { readFileSync, writeFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const out = process.argv[2] || 'checkpoint-resume-check.html';
const assetsDir = 'dist/assets';
const cssFile = readdirSync(assetsDir).find((f) => /^index-.*\.css$/.test(f));
const css = readFileSync(join(assetsDir, cssFile), 'utf8');

const rotateCcw =
  '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" width="13" height="13"><path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/></svg>';
const sendSvg =
  '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" width="18" height="18"><path d="m22 2-7 20-4-9-9-4 20-7Z"/></svg>';

// Mirrors ChatInterface.tsx exactly: group + max-w-0/opacity-0 label; the expanded
// copy applies the group-hover classes statically for the screenshot.
const resumeBtn = (expanded, label) => `
<button type="button"
  class="group inline-flex items-center bg-transparent rounded-lg cursor-pointer text-xs py-1 px-1.5 transition-all duration-200 ${expanded ? 'bg-[rgba(var(--text-base-rgb),0.08)]' : ''}"
  style="border:1px solid rgba(var(--primary-rgb),0.45);color:rgb(var(--primary-rgb));${expanded ? 'padding:4px 8px' : ''}">
  ${rotateCcw}
  <span class="overflow-hidden whitespace-nowrap text-[11px] transition-all duration-200 ${expanded ? 'max-w-[110px] opacity-100 ml-1' : 'max-w-0 opacity-0'}">${label}</span>
</button>`;

const sendBtn = `
<button class="flex items-center justify-center border-none rounded-lg cursor-pointer h-8 w-8 text-white" style="background:rgba(var(--primary-rgb),0.3)">${sendSvg}</button>`;

const inputBar = (expanded, label, title) => `
<div style="margin-bottom:6px;font-size:12px;color:rgba(var(--text-base-rgb),0.5)">${title}</div>
<div style="display:flex;align-items:center;justify-content:flex-end;gap:8px;border:1px solid var(--border-dim);border-radius:12px;padding:10px 12px;background:var(--dialog-bg);backdrop-filter:var(--popup-blur)">
  ${resumeBtn(expanded, label)}
  ${sendBtn}
</div>`;

const col = (theme, title, label) => `
<div style="flex:1;position:relative;min-height:300px" ${theme === 'dark' ? 'data-theme="dark"' : ''}>
  <div style="position:absolute;inset:0;overflow:hidden;background:var(--app-bg)"></div>
  <div style="position:relative;z-index:1;padding:16px;color:rgb(var(--text-base-rgb))">
    <div style="font-weight:600;margin-bottom:12px">${title}</div>
    <div style="display:flex;flex-direction:column;gap:20px">
      ${inputBar(false, label, '默认（仅图标）')}
      ${inputBar(true, label, '悬停（文字展开）')}
    </div>
  </div>
</div>`;

const html = `<!doctype html><html><head><meta charset="utf-8"><style>${css}</style>
<style>html,body{margin:0}</style></head><body>
<div style="display:flex">
  ${col('dark', '深色', '从断点继续')}
  ${col('light', '浅色', '从断点继续')}
</div>
</body></html>`;

writeFileSync(out, html);
console.log(`Wrote ${out}`);

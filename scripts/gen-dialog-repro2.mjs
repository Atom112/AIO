// Repro 2: isolate the REAL difference — a dialog nested INSIDE an acrylic ancestor
// (backdrop-filter blur(4px), like the chat input bar) vs the SAME dialog moved OUT
// to body level (what Portal does). If A is see-through and B is opaque, Portal is
// the correct fix and the current code is right. Usage: node scripts/gen-dialog-repro2.mjs [outfile]
import { readFileSync, writeFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const out = process.argv[2] || 'dialog-repro2.html';
const assetsDir = 'dist/assets';
const cssFile = readdirSync(assetsDir).find((f) => /^index-.*\.css$/.test(f));
const css = readFileSync(join(assetsDir, cssFile), 'utf8');

const busy = Array.from(
  { length: 40 },
  (_, i) =>
    `<div style="padding:2px 8px;font-size:15px;white-space:nowrap">背景文字 行 ${i} ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz 0123456789</div>`,
).join('');

const panel = (label) => `
<div style="position:fixed;top:120px;z-index:50;width:340px;padding:20px;border-radius:12px;background:var(--dialog-bg);border:1px solid var(--border-dim);backdrop-filter:var(--popup-blur)">
  <div style="font-size:17px;font-weight:600;margin-bottom:10px">${label}</div>
  <div style="font-size:13px;line-height:1.6">弹窗内容文字。背景是否透出看这里。</div>
</div>`;

const html = `<!doctype html><html><head><meta charset="utf-8"><style>${css}</style>
<style>html,body{margin:0}#root{background:var(--app-bg)}</style></head>
<body data-theme="dark"><div id="root">
<div style="position:absolute;inset:0;overflow:hidden">${busy}</div>
<div style="position:relative;z-index:1;display:flex;gap:0">
  <!-- LEFT: dialog nested inside an acrylic ancestor (chat-input bar like) -->
  <div style="width:440px">
    <div style="margin:60px 16px;padding:12px;border-radius:12px;background:var(--dialog-bg);backdrop-filter:blur(4px);border:1px solid var(--border-dim)">
      <div style="font-size:13px;color:rgb(var(--text-base-rgb))">聊天输入条祖先（blur(4px) 亚克力）</div>
      <div style="position:relative">
        ${panel('A. 嵌在亚克力祖先内').replace('top:120px', 'top:20px;left:16px')}
      </div>
    </div>
  </div>
  <!-- RIGHT: same dialog at body level (Portal equivalent) -->
  <div style="width:440px">
    <div style="margin:60px 16px;padding:12px;font-size:13px;color:rgb(var(--text-base-rgb))">body 层（Portal 等价，无亚克力祖先）</div>
    ${panel('B. Portal 到 body').replace('top:120px', 'top:120px;left:0')}
  </div>
</div>
</div></body></html>`;

writeFileSync(out, html);
console.log(`Wrote ${out}`);

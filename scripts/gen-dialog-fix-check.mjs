// Verify: the self-sufficient opaque layered background hides behind-text in both
// themes regardless of backdrop-filter. Usage: node scripts/gen-dialog-fix-check.mjs [outfile]
import { readFileSync, writeFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const out = process.argv[2] || 'dialog-fix-check.html';
const assetsDir = 'dist/assets';
const cssFile = readdirSync(assetsDir).find((f) => /^index-.*\.css$/.test(f));
const css = readFileSync(join(assetsDir, cssFile), 'utf8');

const busy = Array.from(
  { length: 40 },
  (_, i) =>
    `<div style="padding:2px 8px;font-size:15px;white-space:nowrap">背景文字 行 ${i} ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz 0123456789</div>`,
).join('');

const FIX_BG =
  'linear-gradient(rgba(var(--surface-bg),0.96), rgba(var(--surface-bg),0.96)), var(--dialog-bg)';

const panel = (bg, blur, label) => `
<div style="width:340px;padding:20px;border-radius:12px;background:${bg};border:1px solid var(--border-dim);${blur}">
  <div style="font-size:17px;font-weight:600;margin-bottom:10px">${label}</div>
  <div style="font-size:13px;line-height:1.6">弹窗内容文字。背景是否透出看这里。</div>
</div>`;

const col = (theme, title) => `
<div style="flex:1;position:relative;min-height:340px" ${theme === 'dark' ? 'data-theme="dark"' : ''}>
  <div style="position:absolute;inset:0;overflow:hidden;background:var(--app-bg)">${busy}</div>
  <div style="position:relative;z-index:1;padding:16px;color:rgb(var(--text-base-rgb))">
    <div style="font-weight:600;margin-bottom:12px">${title}</div>
    <div style="display:flex;flex-direction:column;gap:16px">
      ${panel('var(--dialog-bg)', 'backdrop-filter:var(--popup-blur)', '旧（仅 tint + blur）')}
      ${panel(FIX_BG, 'backdrop-filter:var(--popup-blur)', '新（不透明基底 + tint + blur）')}
    </div>
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

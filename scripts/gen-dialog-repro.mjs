// Repro: does the mode-warning dialog's keyframe animation (modalPanelIn) break
// backdrop-filter vs a static ProjectSettingsModal-style panel? Screenshot both in
// dark mode over busy text. Usage: node scripts/gen-dialog-repro.mjs [outfile]
import { readFileSync, writeFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const out = process.argv[2] || 'dialog-repro.html';
const assetsDir = 'dist/assets';
const cssFile = readdirSync(assetsDir).find((f) => /^index-.*\.css$/.test(f));
const css = readFileSync(join(assetsDir, cssFile), 'utf8');

// Busy text backdrop so blur (or its absence) is obvious.
const busy = Array.from(
  { length: 40 },
  (_, i) =>
    `<div style="padding:2px 8px;font-size:15px;white-space:nowrap">背景文字 行 ${i} ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz 0123456789 测试可见性</div>`,
).join('');

const panel = (extraStyle, anim, label) => `
<div style="position:relative;z-index:2">
  <div style="width:340px;padding:20px;border-radius:12px;background:var(--dialog-bg);border:1px solid var(--border-dim);backdrop-filter:var(--popup-blur);${extraStyle};${anim}">
    <div style="font-size:17px;font-weight:600;margin-bottom:10px">${label}</div>
    <div style="font-size:13px;line-height:1.6">本行是弹窗内容文字。下面这段文字用于观察背景是否透出。</div>
  </div>
</div>`;

const html = `<!doctype html><html><head><meta charset="utf-8"><style>${css}</style>
<style>html,body{margin:0}#root{background:var(--app-bg)}.col{width:420px;padding:20px;color:rgb(var(--text-base-rgb));position:relative}</style>
</head><body data-theme="dark"><div id="root">
<div style="position:absolute;inset:0;overflow:hidden">${busy}</div>
<div style="position:relative;z-index:1;display:flex;gap:40px">
  <div class="col">
    <div style="font-weight:600;margin-bottom:12px">A. 项目设置样式（静态，无 keyframe 动画）</div>
    ${panel('transition:transform .3s', '', '静态面板')}
  </div>
  <div class="col">
    <div style="font-weight:600;margin-bottom:12px">B. 模式警告样式（modalPanelIn both）</div>
    ${panel('', 'animation:modalPanelIn 0.25s cubic-bezier(0.16,1,0.3,1) both', '动画面板')}
  </div>
</div>
</div></body></html>`;

writeFileSync(out, html);
console.log(`Wrote ${out} (css ${(css.length / 1024).toFixed(0)}KB)`);

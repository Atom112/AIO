// Visual check: approval bubble full-width in a status-bar-width container,
// semantic outline buttons (border-only color), MCP dialog panel on dialog-bg.
// Usage: node scripts/gen-approval-outline-check.mjs [outfile]
import { readFileSync, writeFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const out = process.argv[2] || 'approval-outline-check.html';
const assetsDir = 'dist/assets';
const cssFile = readdirSync(assetsDir).find((f) => /^index-.*\.css$/.test(f));
const css = readFileSync(join(assetsDir, cssFile), 'utf8');

const busy = Array.from(
  { length: 60 },
  (_, i) =>
    `<div style="padding:2px 8px;font-size:15px;white-space:nowrap">背景文字 行 ${i} ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz 0123456789</div>`,
).join('');

// class lists mirrored from shared/components/buttonStyles.ts (outline family:
// semantic border + text, neutral body)
const btnSuccessOutline =
  'inline-flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium border border-success/50 bg-white/[0.06] text-success';
const btnDangerOutline =
  'inline-flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium border border-danger/50 bg-white/[0.06] text-danger';
const btnPrimaryOutline =
  'inline-flex items-center justify-center gap-1.5 px-4 py-2 rounded-lg text-sm font-medium border border-pri-50 bg-white/[0.06] text-pri';
const btnPrimaryOutlineSm =
  'inline-flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium border border-pri-50 bg-white/[0.06] text-pri';
const btnWarnOutline =
  'inline-flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium border border-warn/50 bg-white/[0.06] text-warn';
const btnSecondary =
  'inline-flex items-center justify-center gap-1.5 px-4 py-2 rounded-lg text-sm font-medium bg-white/[0.06] border border-white/10 text-white/60';
const btnSecondarySm =
  'inline-flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium bg-white/[0.06] border border-white/10 text-white/60';

const iconX =
  '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" width="13" height="13"><path d="M6 6l12 12M18 6 6 18"/></svg>';
const iconCheck =
  '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" width="13" height="13"><path d="m5 12 5 5L20 7"/></svg>';

const bubble = `
<div class="rounded-[10px] px-3.5 py-3 my-1 w-full bg-[rgba(var(--surface-alt-bg),0.95)] border border-[rgba(var(--primary-rgb),0.15)]" style="box-sizing:border-box">
  <div style="display:flex;align-items:center;gap:6px;margin-bottom:6px">
    <span style="font-size:13px;font-weight:600;color:rgba(255,255,255,0.85);font-family:monospace">get_account</span>
    <span class="text-[10px] rounded bg-white/5 px-1 py-px" style="color:rgba(255,255,255,0.35);font-family:monospace">registry-ad-getle-leads</span>
  </div>
  <div class="text-[11px] bg-black/20 rounded-md px-2 py-1.5 mb-1.5" style="color:rgba(255,255,255,0.55)"><code>无参数</code></div>
  <div class="text-[11px] mb-2.5" style="color:rgba(255,255,255,0.45);line-height:1.4">工具 'get_account' 来自未受信任的 MCP server「Getlead」，请在确认安全后执行（全局 server 可在 MCP 设置中标记为受信任以跳过确认）</div>
  <div style="display:flex;gap:8px;justify-content:flex-end">
    <button class="${btnDangerOutline}">${iconX} 拒绝</button>
    <button class="${btnSuccessOutline}">${iconCheck} 始终允许</button>
    <button class="${btnSuccessOutline}">${iconCheck} 批准</button>
  </div>
</div>`;

const dialog = `
<div style="width:560px;border-radius:12px;padding:20px;background:var(--dialog-bg);backdrop-filter:var(--popup-blur);border:1px solid var(--border-dim);color:rgba(255,255,255,0.88)">
  <div style="font-size:15px;font-weight:600;margin-bottom:12px">编辑 MCP 服务器</div>
  <div style="font-size:12px;margin-bottom:6px;color:rgba(255,255,255,0.6)">请求头 (headers)</div>
  <div style="display:flex;align-items:center;gap:4px;margin-bottom:14px">
    <div style="width:33%;padding:6px 8px;border-radius:4px;font-size:12px;background:rgba(0,0,0,0.3);border:1px solid var(--border-dim)">Authorization</div>
    <div style="flex:1;padding:6px 8px;border-radius:4px;font-size:12px;background:rgba(0,0,0,0.3);border:1px solid var(--border-dim);color:rgba(255,255,255,0.55)">[密钥·已存keyring]</div>
    <button class="${btnWarnOutline}">存密钥</button>
    <button class="${btnDangerOutline}">×</button>
  </div>
  <div style="display:flex;gap:8px;margin-bottom:14px"><button class="${btnSuccessOutline}">测试连接</button></div>
  <div style="display:flex;gap:8px;justify-content:flex-end">
    <button class="${btnSecondarySm}">取消</button>
    <button class="${btnPrimaryOutlineSm}">保存</button>
  </div>
  <div style="display:flex;gap:8px;justify-content:flex-end;margin-top:10px">
    <button class="${btnSecondary}">取消</button>
    <button class="${btnPrimaryOutline}">确定</button>
  </div>
</div>`;

const col = (theme, title) => `
<div style="flex:1;position:relative;min-height:560px" ${theme === 'dark' ? 'data-theme="dark"' : ''}>
  <div style="position:absolute;inset:0;overflow:hidden;background:var(--app-bg)">${busy}</div>
  <div style="position:relative;z-index:1;padding:16px;color:rgb(var(--text-base-rgb))">
    <div style="font-weight:600;margin-bottom:12px">${title}</div>
    <div style="display:flex;flex-direction:column;gap:16px;align-items:flex-start">
      <div style="width:100%;max-width:640px">${bubble}</div>
      ${dialog}
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

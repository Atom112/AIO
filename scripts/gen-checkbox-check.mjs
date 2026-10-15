// Visual check: shared Checkbox (checked/unchecked/rich-label) + icon delete buttons
// in the MCP edit dialog rows, both themes.
// Usage: node scripts/gen-checkbox-check.mjs [outfile]
import { readFileSync, writeFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const out = process.argv[2] || 'checkbox-check.html';
const assetsDir = 'dist/assets';
const cssFile = readdirSync(assetsDir).find((f) => /^index-.*\.css$/.test(f));
const css = readFileSync(join(assetsDir, cssFile), 'utf8');

const checkSvg =
  '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="width:10px;height:10px;color:rgba(var(--primary-rgb),0.9)"><path d="m5 12 5 5L20 7"/></svg>';
const xSvg =
  '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" width="12" height="12"><path d="M6 6l12 12M18 6 6 18"/></svg>';

// Mirrors shared/components/Checkbox.tsx
const checkbox = (checked, label) => `
<label class="flex items-center gap-1.5 cursor-pointer select-none transition-colors" style="font-size:13px">
  <div class="w-4 h-4 rounded border flex items-center justify-center shrink-0 transition-all duration-150 ${
    checked
      ? 'bg-[rgba(var(--primary-rgb),0.25)] border-[rgba(var(--primary-rgb),0.4)]'
      : 'bg-transparent border-white/[0.15]'
  }">${checked ? checkSvg : ''}</div>
  <span>${label}</span>
</label>`;

// Mirrors buttonStyles.btnDangerOutline + Icon x (MCP detail delete buttons)
const delBtn = `<button class="inline-flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium border border-danger/50 bg-white/[0.06] text-white/70">${xSvg}</button>`;

const envRow = `
<div class="flex items-center gap-1" style="width:460px">
  <div style="width:33%;padding:6px 8px;border-radius:4px;font-size:12px;background:rgba(0,0,0,0.3);border:1px solid var(--border-dim);color:rgba(var(--text-base-rgb),0.85)">API_KEY</div>
  <div style="flex:1;padding:6px 8px;border-radius:4px;font-size:12px;background:rgba(0,0,0,0.3);border:1px solid var(--border-dim);color:rgba(var(--text-base-rgb),0.55)">[密钥·已存keyring]</div>
  ${delBtn}
</div>`;

const whitelistRow = (checked, name, desc) => `
<label class="flex items-center items-start gap-2 cursor-pointer select-none transition-colors" style="font-size:12px;padding:4px 8px;border-radius:4px;background:rgba(var(--text-base-rgb),0.03)">
  <div class="w-4 h-4 rounded border flex items-center justify-center shrink-0 transition-all duration-150 ${
    checked
      ? 'bg-[rgba(var(--primary-rgb),0.25)] border-[rgba(var(--primary-rgb),0.4)]'
      : 'bg-transparent border-white/[0.15]'
  }">${checked ? checkSvg : ''}</div>
  <div style="flex:1;min-width:0">
    <div class="font-mono" style="color:rgba(var(--text-base-rgb),0.85)">${name}</div>
    <div style="color:rgba(var(--text-base-rgb),0.5)">${desc}</div>
  </div>
</label>`;

const dialog = `
<div style="width:560px;border-radius:12px;padding:20px;background:var(--dialog-bg);backdrop-filter:var(--popup-blur);border:1px solid var(--border-dim);color:rgba(var(--text-base-rgb),0.88)">
  <div style="font-size:15px;font-weight:600;margin-bottom:12px">编辑 MCP 服务器</div>
  <div style="display:flex;align-items:center;gap:12px;margin-bottom:10px">
    ${checkbox(true, '启动时自动连接')}
    <span style="font-size:11px;color:rgba(var(--text-base-rgb),0.4)">应用启动时自动启动该服务器</span>
  </div>
  <div style="display:flex;align-items:center;gap:12px;margin-bottom:14px">
    ${checkbox(false, '受信任（跳过工具审批）')}
    <span style="font-size:11px;color:rgba(var(--text-base-rgb),0.4)">信任后其工具调用不再逐次确认</span>
  </div>
  <div style="font-size:12px;margin-bottom:6px;color:rgba(var(--text-base-rgb),0.6)">环境变量 (env)</div>
  ${envRow}
  <div style="font-size:12px;margin:14px 0 6px;color:rgba(var(--text-base-rgb),0.6)">工具白名单</div>
  ${whitelistRow(true, 'get_account', '获取账户信息')}
  ${whitelistRow(false, 'delete_lead', '删除线索（危险操作）')}
</div>`;

const col = (theme, title) => `
<div style="flex:1;position:relative;min-height:430px" ${theme === 'dark' ? 'data-theme="dark"' : ''}>
  <div style="position:absolute;inset:0;overflow:hidden;background:var(--app-bg)"></div>
  <div style="position:relative;z-index:1;padding:16px;color:rgb(var(--text-base-rgb))">
    <div style="font-weight:600;margin-bottom:12px">${title}</div>
    ${dialog}
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

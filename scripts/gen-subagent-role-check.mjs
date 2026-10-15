// Visual check: subagent model-settings rows with per-role icon chips (left of the
// profile name), both themes. Usage: node scripts/gen-subagent-role-check.mjs [outfile]
import { readFileSync, writeFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const out = process.argv[2] || 'subagent-role-check.html';
const assetsDir = 'dist/assets';
const cssFile = readdirSync(assetsDir).find((f) => /^index-.*\.css$/.test(f));
const css = readFileSync(join(assetsDir, cssFile), 'utf8');

// icon path snippets mirroring shared/components/Icon.tsx PATHS entries
const ICONS = {
  search: '<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>',
  code: '<path d="m9 8-4 4 4 4M15 8l4 4-4 4"/>',
  'review-search':
    '<circle cx="10.5" cy="10.5" r="7.5"/><path d="M15.5 15.5 21 21"/><path d="m7.5 12 2.2-5.5 3 7 1.4-3.2"/>',
  bug: '<path d="M8 2 6 5M16 2l2 3M9 6h6v6a3 3 0 0 1-3 3 3 3 0 0 1-3-3V6Z"/><path d="M12 15v3M5 10H2M22 10h-3M5 7 3 5M19 7l2-2M5 13l-2 2M19 13l2 2"/>',
  'pen-line': '<path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z"/>',
  'flask-check':
    '<path d="M9 3h6M10 3v6L4.5 18a2 2 0 0 0 1.7 3h11.6a2 2 0 0 0 1.7-3L14 9V3"/><path d="M7 14h10"/><path d="m9.5 17 1.5 1.5 3-3"/>',
  lightbulb:
    '<path d="M9 18h6M10 22h4"/><path d="M12 2a7 7 0 0 0-4 12.7c.7.7 1 1.6 1 2.5V18h6v-.8c0-.9.3-1.8 1-2.5A7 7 0 0 0 12 2Z"/>',
  compass: '<circle cx="12" cy="12" r="9"/><path d="m15 9-2 5-4 2 2-5 4-2Z"/>',
  eye: '<path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7Z"/><circle cx="12" cy="12" r="3"/>',
  sparkles:
    '<path d="M12 3v3M12 18v3M3 12h3M18 12h3M5.6 5.6l2.1 2.1M16.3 16.3l2.1 2.1M5.6 18.4l2.1-2.1M16.3 7.7l2.1-2.1"/>',
};

const svg = (name, size, color) =>
  `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" width="${size}" height="${size}" style="color:${color}">${ICONS[name]}</svg>`;

const role = (id, icon) => ({
  icon,
  color: `rgba(var(--role-${id}-rgb),0.9)`,
  bgColor: `rgba(var(--role-${id}-rgb),0.1)`,
  borderColor: `rgba(var(--role-${id}-rgb),0.3)`,
});
const ROWS = [
  [role('explorer', 'search'), '文件浏览器', 'explorer'],
  [role('coder', 'code'), '代码编辑', 'coder'],
  [role('architect', 'compass'), '系统架构', 'architect'],
  [role('debugger', 'bug'), '调试专家', 'debugger'],
  [role('reviewer', 'review-search'), '代码审查', 'reviewer'],
  [role('writer', 'pen-line'), '文档撰写', 'writer'],
  [role('tester', 'flask-check'), '测试工程', 'tester'],
  [role('requirements', 'lightbulb'), '需求分析', 'requirements'],
  [role('vision', 'eye'), '视觉分析', 'vision'],
  [role('general', 'sparkles'), '通用助手', 'general'],
  [
    {
      icon: 'sparkles',
      color: 'rgba(var(--role-default-rgb),0.75)',
      bgColor: 'rgba(var(--role-default-rgb),0.06)',
      borderColor: 'rgba(var(--role-default-rgb),0.2)',
    },
    '自定义角色',
    'my-custom',
  ],
];

const row = (ps, name, id) => `
<div class="bg-white/[0.03] rounded-xl border border-[var(--border-dim)]" style="padding:14px 16px;display:flex;align-items:center;justify-content:space-between">
  <div style="display:flex;align-items:center;gap:12px;flex:1;min-width:0">
    <div class="rounded-lg" style="width:32px;height:32px;display:flex;align-items:center;justify-content:center;flex-shrink:0;border:1px solid ${ps.borderColor};background:${ps.bgColor}">
      ${svg(ps.icon, 15, ps.color)}
    </div>
    <div style="min-width:0">
      <div style="display:flex;align-items:center;gap:6px">
        <span style="font-size:13px;font-weight:600;color:rgba(255,255,255,0.95)">${name}</span>
        <span class="bg-pri-20 text-pri" style="font-size:10px;padding:1px 6px;border-radius:4px;font-family:monospace">${id}</span>
      </div>
      <p style="font-size:11px;color:rgba(255,255,255,0.4);margin:4px 0 0">跟随主模型（未设置覆盖）</p>
    </div>
  </div>
  <div class="bg-white/[0.06] border border-white/10 text-white/60 rounded-md" style="display:inline-flex;align-items:center;gap:6px;padding:6px 12px;font-size:12px;margin-left:12px;flex-shrink:0">
    跟随主模型
    <svg viewBox="0 0 24 24" width="10" height="10" fill="currentColor"><path d="M4 6l6 6-6 6V6zm7 0l6 6-6 6V6z" opacity="0"/></svg>
  </div>
</div>`;

const col = (theme, title) => `
<div style="flex:1;position:relative;min-height:640px" ${theme === 'dark' ? 'data-theme="dark"' : ''}>
  <div style="position:absolute;inset:0;overflow:hidden;background:var(--app-bg)"></div>
  <div style="position:relative;z-index:1;padding:16px;color:rgb(var(--text-base-rgb))">
    <div style="font-weight:600;margin-bottom:12px">${title}</div>
    <div style="display:flex;flex-direction:column;gap:8px;max-width:560px">
      ${ROWS.map(([ps, name, id]) => row(ps, name, id)).join('')}
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

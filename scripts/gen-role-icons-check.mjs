// Visual check: all 10 subagent role chips (icon + color) + the work-state orbit
// animation, dark & light themes. Usage: node scripts/gen-role-icons-check.mjs [outfile]
import { readFileSync, writeFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const out = process.argv[2] || 'role-icons-check.html';
const assetsDir = 'dist/assets';
const cssFile = readdirSync(assetsDir).find((f) => /^index-.*\.css$/.test(f));
const css = readFileSync(join(assetsDir, cssFile), 'utf8');

// Recreate Icon.tsx paths inline (kept in sync — quick visual review only).
const ICONS = {
  search: `<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>`,
  code: `<path d="m9 8-4 4 4 4M15 8l4 4-4 4"/>`,
  'review-search': `<path d="M14 3H7a2 2 0 0 0-2 2v9"/><path d="M14 3v5h5"/><circle cx="14.5" cy="16.5" r="3.5"/><path d="m20.5 22-3-3"/>`,
  bug: `<path d="m8 2 1.5 1.5M14.5 3.5 16 2"/><path d="M9 7.1a4 4 0 0 1 6 0l1.5 2.1a3 3 0 0 1 .5 1.7V13a5 5 0 0 1-10 0v-2.1a3 3 0 0 1 .5-1.7Z"/><path d="M9 13h6M12 13v6"/><path d="m3 9 2.5 1M18.5 10 21 9M4 15l2-.5M18 14.5l2 .5"/>`,
  'pen-line': `<path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z"/>`,
  'flask-check': `<path d="M10 2v6L4.5 18a2 2 0 0 0 1.8 3h8.2"/><path d="M14 2v6l2.3 4.1"/><path d="M9 2h6"/><path d="m14 17 2.5 2.5L21 15"/>`,
  lightbulb: `<path d="M9 18h6M10 22h4"/><path d="M12 2a7 7 0 0 0-4 12.7c.7.7 1 1.6 1 2.5V18h6v-.8c0-.9.3-1.8 1-2.5A7 7 0 0 0 12 2Z"/>`,
  compass: `<path d="m16.1 3.9 4 4L8.5 19.5l-4.4 1.1 1.1-4.4Z"/><path d="m12 8 4 4"/><circle cx="12" cy="12" r="9"/>`,
  eye: `<path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7Z"/><circle cx="12" cy="12" r="3"/>`,
  sparkles: `<path d="M12 3v3M12 18v3M3 12h3M18 12h3M5.6 5.6l2.1 2.1M16.3 16.3l2.1 2.1M5.6 18.4l2.1-2.1M16.3 7.7l2.1-2.1"/>`,
};

const ROLES = [
  ['explorer', 'search', '探索者'],
  ['coder', 'code', '实现者'],
  ['reviewer', 'review-search', '审查者'],
  ['debugger', 'bug', '诊断者'],
  ['writer', 'pen-line', '写作者'],
  ['tester', 'flask-check', '测试者'],
  ['requirements', 'lightbulb', '需求分析'],
  ['architect', 'compass', '架构师'],
  ['vision', 'eye', '看图分析'],
  ['general', 'sparkles', '全能力'],
];

const icon = (name, size = 13) =>
  `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">${ICONS[name]}</svg>`;

const chip = (id, iconName, label, running) => `
<span style="position:relative;display:inline-flex;align-items:center;justify-content:center;width:26px;height:26px;border-radius:5px;background:rgba(var(--role-${id}-rgb),0.1);border:1px solid rgba(var(--role-${id}-rgb),0.3);color:rgba(var(--role-${id}-rgb),0.9);${running ? 'animation:subagentPulse 1.6s ease-in-out infinite' : ''}">
  ${icon(iconName)}
  ${running ? `<span class="subagent-work-orbit" style="color:rgba(var(--role-${id}-rgb),0.9)"></span>` : ''}
</span><span style="font-size:12px;margin-left:6px">${label}${running ? '（工作中）' : ''}</span>`;

const grid = ROLES.map(
  ([id, ic, label], i) =>
    `<div style="display:flex;align-items:center;padding:4px 8px">${chip(id, ic, label, i < 5)}</div>`,
).join('');

const col = (theme, title) => `
<div style="flex:1;padding:16px;${theme === 'dark' ? 'background:#0a0c10' : 'background:#2f2f2f'}" ${theme === 'dark' ? 'data-theme="dark"' : ''}>
  <div style="font-weight:600;margin-bottom:12px;color:${theme === 'dark' ? '#eee' : '#fff'}">${title}</div>
  ${grid}
</div>`;

const html = `<!doctype html><html><head><meta charset="utf-8"><style>${css}</style>
<style>html,body{margin:0;font-family:system-ui}</style></head><body>
<div style="display:flex">${col('dark', '深色主题')}${col('light', '浅色主题')}</div>
</body></html>`;

writeFileSync(out, html);
console.log(`Wrote ${out}`);

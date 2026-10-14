// Generate a static preview page of key UI surfaces (dropdowns, popups, modals)
// rendered with the app's real compiled CSS, in both themes side by side, for
// headless-browser screenshots. Not part of the app — a dev review tool.
// Usage: node scripts/gen-theme-preview.mjs [outfile]
import { readFileSync, writeFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const out = process.argv[2] || 'theme-preview.html';
const assetsDir = 'dist/assets';
const cssFile = readdirSync(assetsDir).find((f) => /^index-.*\.css$/.test(f));
const css = readFileSync(join(assetsDir, cssFile), 'utf8');

// Representative surfaces copied from the real components (class strings kept verbatim):
const dropdownMenu = () => `
<div class="relative inline-block" style="vertical-align:top">
  <div class="absolute z-[100] min-w-full mt-1 rounded-[10px] p-1" style="position:static;width:200px;background:var(--popup-bg);backdrop-filter:var(--popup-blur);border:1px solid var(--border-dim);box-shadow:0 12px 40px rgba(0,0,0,0.45)">
    <div class="flex items-center gap-1.5 px-2.5 py-[7px] rounded-md text-white/[0.78] select-none"><span class="grow truncate">简体中文</span></div>
    <div class="flex items-center gap-1.5 px-2.5 py-[7px] rounded-md text-white/[0.78] !bg-[rgba(var(--primary-rgb),0.12)] select-none"><span class="grow truncate">English</span></div>
    <div class="flex items-center gap-1.5 px-2.5 py-[7px] rounded-md text-white/[0.78] select-none"><span class="grow truncate">日本語</span></div>
  </div>
</div>`;

const modeMenu = `
<div class="rounded-xl overflow-hidden" style="width:200px;background:var(--popup-bg);border:1px solid var(--border-dim);backdrop-filter:var(--popup-blur);box-shadow:0 -8px 30px rgba(0,0,0,0.4)">
  <div class="px-3 py-2 text-[11px] font-bold uppercase tracking-widest" style="color:rgba(var(--text-base-rgb),0.35);background:rgba(var(--text-base-rgb),0.04);border-bottom:1px solid var(--border-dim)">工作模式</div>
  <div class="py-1">
    <button type="button" class="w-full flex items-center gap-2.5 px-3 py-2 text-left border-none" style="color:rgba(var(--text-base-rgb),0.75)"><span class="flex-1 min-w-0 text-sm font-medium">只读</span></button>
    <button type="button" class="w-full flex items-center gap-2.5 px-3 py-2 text-left border-none !bg-[rgba(var(--primary-rgb),0.12)]" style="color:rgba(var(--text-base-rgb),0.75)"><span class="flex-1 min-w-0 text-sm font-medium">工作区内修改</span></button>
    <button type="button" class="w-full flex items-center gap-2.5 px-3 py-2 text-left border-none" style="color:rgba(var(--text-base-rgb),0.75)"><span class="flex-1 min-w-0 text-sm font-medium">完全权限</span></button>
    <button type="button" class="w-full flex items-center gap-2.5 px-3 py-2 text-left border-none" style="color:rgba(var(--text-base-rgb),0.75)"><span class="flex-1 min-w-0 text-sm font-medium">计划</span></button>
  </div>
</div>`;

const reasoningPopup = `
<div class="w-[240px] rounded-xl p-1.5" style="background:var(--popup-bg);backdrop-filter:var(--popup-blur);border:1px solid var(--border-dim);box-shadow:0 12px 40px rgba(0,0,0,0.45)">
  <div class="flex items-center gap-2 px-3 pt-2 pb-1 text-[13px] font-semibold text-white/85">推理</div>
  <div class="relative mx-3 h-8 select-none">
    <div class="absolute left-0 right-0 top-1/2 h-2 -translate-y-1/2 rounded-full bg-[rgba(var(--text-base-rgb),0.12)]">
      <div class="absolute left-0 top-0 h-full overflow-hidden rounded-full" style="width:66.7%;background:linear-gradient(90deg, color-mix(in srgb, var(--color-reasoning-medium) 30%, transparent), var(--color-reasoning-medium))">
        <div class="absolute inset-0" style="background-image:radial-gradient(rgba(var(--text-base-rgb),0.95) 0.6px, transparent 1px),radial-gradient(rgba(var(--text-base-rgb),0.55) 0.6px, transparent 1px),radial-gradient(rgba(var(--text-base-rgb),0.75) 0.5px, transparent 1px);background-size:11px 11px, 17px 17px, 23px 23px;background-position:0 1px, 5px 6px, 11px 3px"></div>
      </div>
    </div>
    <div class="absolute top-1/2 h-4 w-4 -translate-x-1/2 -translate-y-1/2 rounded-full bg-[rgb(var(--text-base-rgb))]" style="left:66.7%;box-shadow:0 0 0 2px rgba(var(--surface-bg),0.9), 0 0 12px 2px color-mix(in srgb, var(--color-reasoning-medium) 60%, transparent)"></div>
  </div>
  <div class="relative mx-3 h-5 pb-1">
    <span class="absolute top-0 -translate-x-1/2 px-1 py-0.5 text-[11px] text-white/45" style="left:0%">关闭</span>
    <span class="absolute top-0 -translate-x-1/2 px-1 py-0.5 text-[11px] text-white/45" style="left:33.3%">低</span>
    <span class="absolute top-0 -translate-x-1/2 px-1 py-0.5 text-[11px] font-semibold" style="left:66.7%;color:var(--color-reasoning-medium)">中</span>
    <span class="absolute top-0 -translate-x-1/2 px-1 py-0.5 text-[11px] text-white/45" style="left:100%">高</span>
  </div>
</div>`;

const modal = `
<div class="rounded-xl w-full max-w-md p-6" style="background:var(--dialog-bg);background-attachment:fixed;backdrop-filter:var(--popup-blur);border:1px solid var(--border-dim);box-shadow:0 8px 32px rgba(0,0,0,0.5)">
  <h2 class="text-lg font-semibold text-white mb-4">新建项目</h2>
  <label class="block text-sm text-white/60 mb-1.5">项目名称</label>
  <input type="text" class="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-sm mb-4" value="my-project" readonly />
  <label class="block text-sm text-white/60 mb-1.5">项目路径</label>
  <div class="flex gap-2 mb-1">
    <input type="text" class="flex-1 px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-sm" value="D:\\base\\my-project" readonly />
    <button class="inline-flex items-center justify-center gap-1.5 px-4 py-2 rounded-lg text-sm font-medium bg-white/[0.06] border border-white/10 text-white/60">浏览</button>
  </div>
  <p class="text-white/30 text-xs mb-4 mt-2">项目下会创建 .aio 文件夹</p>
  <div class="flex justify-end gap-3">
    <button class="px-4 py-2 text-sm inline-flex items-center gap-1 text-white/60">取消</button>
    <button class="inline-flex items-center justify-center gap-1.5 px-4 py-2 rounded-lg text-sm font-medium text-white" style="background:rgba(var(--primary-rgb),0.35);border:1px solid rgba(var(--primary-rgb),0.5)">创建</button>
  </div>
</div>`;

const modalDark700 = `
<div class="backdrop-blur-[24px] border border-white/15 rounded-xl shadow-2xl w-full max-w-md p-6" style="background:var(--dialog-bg);background-attachment:fixed">
  <h2 class="text-lg font-semibold text-white mb-4">新建项目（项目创建弹窗）</h2>
  <input type="text" class="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-sm mb-4" value="my-project" readonly />
  <div class="flex justify-end gap-3">
    <button class="px-4 py-2 text-sm inline-flex items-center gap-1 text-white/60">取消</button>
    <button class="inline-flex items-center justify-center gap-1.5 px-4 py-2 rounded-lg text-sm font-medium text-white" style="background:rgba(var(--primary-rgb),0.35);border:1px solid rgba(var(--primary-rgb),0.5)">创建</button>
  </div>
</div>`;

const taskBoard = `
<div class="rounded-2xl" style="background:rgba(var(--surface-bg),0.5);border:1px solid var(--border-dim);width:340px">
  <div class="border-b" style="border-color:rgba(var(--border-rgb), 0.15)">
    <button type="button" class="flex items-center gap-2 w-full px-3 py-1.5 text-left rounded-t-2xl bg-transparent border-none">
      <span class="font-medium truncate" style="color:rgba(var(--text-base-rgb),0.7)">任务清单：3项 已完成1项 待处理1项</span>
      <span class="shrink-0 ml-auto" style="color:rgba(var(--text-base-rgb),0.4)">&#9660;</span>
    </button>
    <div class="px-2 pb-2 space-y-1">
      <div class="flex items-center gap-2 rounded-md px-2 py-1 text-[11px] select-none" style="background:rgba(var(--text-base-rgb),0.03);border-left:2px solid transparent">
        <span class="truncate" style="color:rgba(var(--text-base-rgb),0.35);text-decoration:line-through">读取项目结构</span>
      </div>
      <div class="flex items-center gap-2 rounded-md px-2 py-1 text-[11px] select-none" style="background:rgba(var(--text-base-rgb),0.03);border-left:2px solid rgba(var(--primary-rgb),0.55)">
        <span class="w-2 h-2 rounded-full shrink-0 animate-pulse" style="background:rgba(var(--primary-rgb),0.8)"></span>
        <span class="truncate" style="color:rgba(var(--text-base-rgb),0.75)">修改 ReasoningButton 组件</span>
      </div>
      <div class="flex items-center gap-2 rounded-md px-2 py-1 text-[11px] select-none opacity-55" style="background:rgba(var(--text-base-rgb),0.03);border-left:2px solid transparent">
        <span class="w-2 h-2 rounded-full shrink-0" style="background:transparent;border:1px solid rgba(var(--text-base-rgb),0.35)"></span>
        <span class="truncate" style="color:rgba(var(--text-base-rgb),0.75)">运行测试</span>
      </div>
    </div>
  </div>
</div>`;

const slashMenu = `
<div class="rounded-xl overflow-hidden" style="width:300px;background:var(--popup-bg);backdrop-filter:var(--popup-blur);border:1px solid var(--border-dim);box-shadow:0 12px 40px rgba(0,0,0,0.45)">
  <div class="flex items-center justify-between gap-3 px-2.5 py-[7px] rounded-md bg-white/[0.10]">
    <span class="text-[13px] font-semibold text-white/85 font-mono">/compact</span>
    <span class="text-[11px] text-white/30 italic">压缩上下文</span>
  </div>
  <div class="flex items-center justify-between gap-3 px-2.5 py-[7px] rounded-md">
    <span class="text-[13px] font-semibold text-white/85 font-mono">/clear</span>
    <span class="text-[11px] text-white/30 italic">清空对话</span>
  </div>
  <div class="flex items-center justify-between gap-3 px-2.5 py-[7px] rounded-md">
    <span class="text-[13px] font-semibold text-white/85 font-mono">/settings</span>
    <span class="text-[11px] text-white/30 italic">打开设置</span>
  </div>
</div>`;

const section = (title, ...blocks) => `
<div style="margin-bottom:28px">
  <div style="font-size:12px;letter-spacing:1px;text-transform:uppercase;color:rgba(var(--text-base-rgb),0.4);margin-bottom:10px">${title}</div>
  <div style="display:flex;gap:24px;flex-wrap:wrap;align-items:flex-start">${blocks.join('')}</div>
</div>`;

const column = (theme, title) => `
<div data-theme="${theme}" style="flex:1;min-width:0;padding:32px;border-radius:16px;background:var(--app-bg);color:rgba(var(--text-base-rgb),0.87)">
  <div style="font-size:18px;font-weight:700;margin-bottom:24px">${title}</div>
  ${section('下拉菜单 Dropdown', dropdownMenu())}
  ${section('模式菜单 Mode menu', modeMenu)}
  ${section('推理滑杆弹层 Reasoning popup', reasoningPopup)}
  ${section('弹窗 Modal（dialog 亚克力）', modal)}
  ${section('弹窗 Modal（项目创建同款）', modalDark700)}
  ${section('任务清单 Task board', taskBoard)}
  ${section('斜杠菜单 Slash menu', slashMenu)}
</div>`;

const html = `<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8" />
<style>${css}</style>
<style>
  body { margin:0; padding:24px; background:#222; font-family:system-ui,sans-serif; }
  .row { display:flex; gap:24px; align-items:flex-start; }
</style>
</head>
<body>
<div class="row">
${column('light', '浅色模式 Light')}
${column('dark', '深色模式 Dark')}
</div>
</body>
</html>`;

writeFileSync(out, html);
console.log('wrote', out, 'css:', cssFile);

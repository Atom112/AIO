// Visual check: redesigned embedding settings page — new "embedding" icon (dashed
// vector space + points), "arrow-down-up" rerank icon, section chips, standardized
// buttons (save/download primary outline), both themes.
// Usage: node scripts/gen-embedding-check.mjs [outfile]
import { readFileSync, writeFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const out = process.argv[2] || 'embedding-check.html';
const assetsDir = 'dist/assets';
const cssFile = readdirSync(assetsDir).find((f) => /^index-.*\.css$/.test(f));
const css = readFileSync(join(assetsDir, cssFile), 'utf8');

const svg = (inner, size, color, extra = '') =>
  `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" width="${size}" height="${size}" style="color:${color}" ${extra}>${inner}</svg>`;

const ICONS = {
  embedding:
    '<rect x="3" y="5" width="18" height="14" rx="4" stroke-dasharray="3 3"/><path d="M8 10h.01" stroke-width="3.5"/><path d="M13.5 15.5h.01" stroke-width="3.5"/><path d="M16.5 8.5h.01" stroke-width="3.5"/>',
  rerank:
    '<path d="M7 4v16M7 20l-3.5-3.5M7 20l3.5-3.5"/><path d="M17 20V4M17 4l-3.5 3.5M17 4l3.5 3.5"/>',
  folder: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7Z"/>',
  check: '<path d="m5 12 5 5L20 7"/>',
  download: '<path d="M12 4v12M7 11l5 5 5-5"/><path d="M4 20h16"/>',
  play: '<path d="M7 5v14l12-7-12-7Z"/>',
  trash:
    '<path d="M4 7h16M9 7V4h6v3M6 7v13a2 2 0 0 0 2 2h8a2 2 0 0 0 2-2V7"/><path d="M10 11v6M14 11v6"/>',
};

const btnPrimaryOutlineSm =
  'inline-flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium border border-pri-50 bg-white/[0.06] text-pri';
const btnSecondarySm =
  'inline-flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium bg-white/[0.06] border border-white/10 text-white/60';
const btnDangerOutline =
  'inline-flex items-center justify-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium border border-danger/50 bg-white/[0.06] text-danger';

const sectionHeader = (iconKey, title, desc) => `
<div style="display:flex;align-items:center;gap:12px;margin-bottom:12px">
  <div class="rounded-lg" style="width:32px;height:32px;display:flex;align-items:center;justify-content:center;flex-shrink:0;border:1px solid rgba(var(--text-base-rgb),0.12);background:rgba(var(--text-base-rgb),0.05);color:rgba(var(--text-base-rgb),0.55)">
    ${svg(ICONS[iconKey], 15, 'currentColor')}
  </div>
  <div style="min-width:0">
    <div style="font-size:13px;font-weight:500;color:rgba(255,255,255,0.95)">${title}</div>
    <div style="font-size:11px;margin-top:2px;color:rgba(var(--text-base-rgb),0.4)">${desc}</div>
  </div>
</div>`;

const card = (delay, inner) => `
<div class="bg-white/[0.03] rounded-xl border border-[var(--border-dim)]" style="padding:16px">
  ${inner}
</div>`;

const modelRow = (iconKey, id, meta, right, bundled) => `
<div class="rounded-[10px] border border-white/[0.05] bg-white/[0.02]" style="display:flex;align-items:center;gap:12px;padding:10px 12px">
  <div class="rounded-lg" style="width:32px;height:32px;display:flex;align-items:center;justify-content:center;flex-shrink:0;background:${bundled ? 'rgba(var(--primary-rgb),0.15)' : 'rgba(var(--text-base-rgb),0.06)'};color:${bundled ? 'var(--primary-color)' : 'rgba(var(--text-base-rgb),0.5)'}">
    ${svg(ICONS[iconKey], 16, 'currentColor')}
  </div>
  <div style="flex:1;min-width:0">
    <div style="display:flex;align-items:center;gap:8px">
      <span style="font-size:13px;font-weight:500;color:rgba(255,255,255,0.85);font-family:monospace">${id}</span>
      ${bundled ? '<span class="bg-pri-20 text-pri" style="font-size:10px;padding:1px 6px;border-radius:4px">内置</span>' : ''}
    </div>
    <div style="font-size:10px;margin-top:2px;color:rgba(var(--text-base-rgb),0.4)">${meta}</div>
  </div>
  <div style="display:flex;align-items:center;gap:6px;flex-shrink:0">${right}</div>
</div>`;

const dirCard = card(
  30,
  sectionHeader('folder', '模型安装目录', '下载的模型文件存放位置（重排模型固定于应用数据目录）') +
    `
<div style="display:flex;align-items:center;gap:8px">
  <div class="rounded-lg" style="flex:1;min-width:0;padding:8px 12px;font-size:12px;background:rgba(0,0,0,0.25);border:1px solid rgba(255,255,255,0.08);color:rgba(var(--text-base-rgb),0.88)">C:\\Users\\me\\AppData\\Roaming\\com.loch.aio\\models\\embed</div>
  <button class="${btnSecondarySm}">浏览</button>
  <button class="${btnSecondarySm}">重置</button>
  <button class="${btnPrimaryOutlineSm}">保存</button>
</div>`,
);

const embedCard = card(
  60,
  sectionHeader('embedding', '嵌入模型', '项目记忆 RAG 的向量化模型；内置模型开箱即用') +
    modelRow(
      'embedding',
      'all-MiniLM-L6-v2',
      'all-MiniLM-L6-v2 · 384 dims · 22.5 MB',
      `<span class="text-success" style="display:inline-flex;align-items:center;gap:4px;font-size:11px">${svg(ICONS.check, 10, 'currentColor')} 已安装</span>
       <span style="width:40px;height:22px;border-radius:999px;background:rgba(var(--primary-rgb),0.7);border:1px solid rgba(var(--primary-rgb),0.5);display:inline-flex;align-items:center;justify-content:flex-end;padding-right:3px"><span style="width:16px;height:16px;border-radius:999px;background:#fff"></span></span>`,
      true,
    ) +
    modelRow(
      'embedding',
      'bge-m3',
      'BAAI/bge-m3 · 1024 dims · 560 MB',
      `<button class="${btnPrimaryOutlineSm}">${svg(ICONS.download, 12, 'currentColor')} 下载</button>`,
      false,
    ),
);

const rerankCard = card(
  90,
  sectionHeader('rerank', '重排序模型', '本地交叉编码器（bge-reranker 家族），用于检索结果精排') +
    modelRow(
      'rerank',
      'bge-reranker-v2-m3',
      'BAAI/bge-reranker-v2-m3 · 568 MB',
      `<span class="text-success" style="display:inline-flex;align-items:center;gap:4px;font-size:11px">${svg(ICONS.check, 10, 'currentColor')} 已安装</span>
       <button class="${btnDangerOutline}">${svg(ICONS.trash, 11, 'currentColor')} 删除</button>
       <span style="width:40px;height:22px;border-radius:999px;background:rgba(255,255,255,0.08);border:1px solid rgba(255,255,255,0.08);display:inline-flex;align-items:center;padding-left:3px"><span style="width:16px;height:16px;border-radius:999px;background:#fff"></span></span>`,
      false,
    ),
);

const navPreview = `
<div style="display:flex;align-items:center;gap:10px;padding:0 4px 10px;border-bottom:1px solid var(--border-dim);margin-bottom:12px">
  ${svg(ICONS.embedding, 20, 'rgba(var(--text-base-rgb),0.7)')}
  <span style="font-size:13px;font-weight:500;color:rgba(var(--text-base-rgb),0.85)">嵌入模型（设置导航图标：向量空间 + 散点）</span>
  ${svg(ICONS.rerank, 20, 'rgba(var(--text-base-rgb),0.7)')}
  <span style="font-size:13px;font-weight:500;color:rgba(var(--text-base-rgb),0.85)">重排序（对向箭头）</span>
</div>`;

const col = (theme, title) => `
<div style="flex:1;position:relative;min-height:600px" ${theme === 'dark' ? 'data-theme="dark"' : ''}>
  <div style="position:absolute;inset:0;overflow:hidden;background:var(--app-bg)"></div>
  <div style="position:relative;z-index:1;padding:16px;color:rgb(var(--text-base-rgb))">
    <div style="font-weight:600;margin-bottom:12px">${title}</div>
    ${navPreview}
    <div style="display:flex;flex-direction:column;gap:8px;max-width:600px">
      ${dirCard}${embedCard}${rerankCard}
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

// Build a reproduction page for the usage-heatmap scrollbar flash (v2):
// models the REAL nesting — settings scroll area > h-full flex column > content
// column with overflow-y-auto — and toggles the ModelBreakdown section into
// existence (the empty-day -> used-day switch). scrollHeight is sampled every
// frame for ~900ms after the click to find the exact overflowing frame.
// Usage: node scripts/gen-heatmap-repro.mjs [outfile]
import { writeFileSync } from 'node:fs';

const out = process.argv[2] || 'heatmap-repro.html';

const html = `<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8" />
<style>
  body { margin:0; background:#222; color:#fff; font-family:system-ui,sans-serif; }
  /* settings main area (SettingsPage.tsx:129): the outer scroll container */
  .settings-scroll { width: 760px; height: 640px; overflow-y: auto; overflow-x: hidden; margin: 16px auto; border: 1px solid #555; }
  /* UsageSettings.tsx:87-91 */
  .page { width: 100%; height: 100%; }
  .page-inner { width: 100%; height: 100%; border-radius: 12px; padding: 32px; display: flex; flex-direction: column; }
  .page-header { padding-bottom: 12px; margin-bottom: 24px; flex-shrink: 0; border-bottom: 1px solid #444; font-weight: 700; }
  /* UsageSettings.tsx:165 — the content column */
  .content { display: flex; flex-direction: column; gap: 24px; overflow-y: auto; }
  .card { background: rgba(255,255,255,0.035); border: 1px solid rgba(255,255,255,0.16); border-radius: 12px; padding: 20px; }
  @keyframes rowIn { from { opacity: 0; transform: translateY(6px); } to { opacity: 1; transform: translateY(0); } }
  .animate-row-in { animation: rowIn 0.32s cubic-bezier(0.2, 0.8, 0.2, 1) backwards; }
  @keyframes fadeOnly { from { opacity: 0; } to { opacity: 1; } }
  .animate-fade-only { animation: fadeOnly 0.25s ease-out backwards; }
  .heatmap { display: flex; gap: 2px; }
  .cell { width: 12px; height: 12px; border-radius: 2px; background: rgba(124,154,191,0.5); cursor: pointer; }
  .row { display: flex; align-items: center; gap: 12px; padding: 12px 16px; background: rgba(255,255,255,0.035); border: 1px solid rgba(255,255,255,0.16); border-radius: 8px; margin-bottom: 4px; }
  .bar { flex: 1; height: 8px; border-radius: 999px; background: rgba(255,255,255,0.04); overflow: hidden; }
  .bar > div { height: 100%; border-radius: 999px; background: rgba(124,154,191,0.55); }
  #log { position: fixed; left: 8px; top: 8px; font: 11px monospace; white-space: pre; pointer-events: none; color: #7fd; z-index: 99; }
  #clickTarget { position: fixed; right: 16px; top: 8px; padding: 6px 12px; z-index: 99; }
</style>
</head>
<body>
<div id="log"></div>
<button id="clickTarget">select used day</button>
<div class="settings-scroll" id="outer">
  <div class="page">
    <div class="page-inner">
      <div class="page-header">Usage</div>
      <div class="content" id="content">
        <div class="card animate-row-in" style="height:90px;animation-delay:60ms">Summary cards</div>
        <div class="card animate-row-in" style="animation-delay:90ms">
          <div style="font-size:11px;margin-bottom:12px">DAILY ACTIVITY</div>
          <div class="heatmap" id="heatmap"></div>
        </div>
        <!-- ModelBreakdown section: mounted only when the day has usage -->
        <div id="bdSlot"></div>
      </div>
    </div>
  </div>
</div>
<script>
  const hm = document.getElementById('heatmap');
  for (let c = 0; c < 53; c++) {
    const col = document.createElement('div');
    col.style.display = 'flex'; col.style.flexDirection = 'column'; col.style.gap = '2px';
    for (let r = 0; r < 7; r++) { const cell = document.createElement('div'); cell.className = 'cell'; col.appendChild(cell); }
    hm.appendChild(col);
  }

  const outer = document.getElementById('outer');
  const content = document.getElementById('content');
  const log = document.getElementById('log');
  let frames = [], sampling = false;

  function mountBreakdown() {
    const card = document.createElement('div');
    // Variant A (default): animate-row-in like the app; Variant B (?fade=1): opacity-only
    card.className = 'card ' + (location.search.includes('fade=1') ? 'animate-fade-only' : 'animate-row-in');
    card.style.animationDelay = '120ms';
    let rows = '';
    for (let i = 0; i < 3; i++) {
      rows += '<div class="row"><span style="flex:1">model-' + i + '</span><div class="bar"><div style="width:' + (90 - i * 20) + '%"></div></div></div>';
    }
    card.innerHTML = '<div style="font-size:11px;margin-bottom:12px;display:flex;gap:8px;align-items:center"><span>2026-10-14 BREAKDOWN</span><button style="width:16px;height:16px">x</button></div>' + rows;
    document.getElementById('bdSlot').appendChild(card);
  }

  function sample() {
    if (!sampling) return;
    const overOuter = outer.scrollHeight > outer.clientHeight;
    const overContent = content.scrollHeight > content.clientHeight;
    frames.push(
      'outer=' + outer.scrollHeight + '/' + outer.clientHeight + (overOuter ? ' OVERFLOW' : ' ok') +
      '  content=' + content.scrollHeight + '/' + content.clientHeight + (overContent ? ' OVERFLOW' : ' ok')
    );
    requestAnimationFrame(sample);
  }

  document.getElementById('clickTarget').onclick = () => {
    frames = []; sampling = true;
    requestAnimationFrame(sample);
    setTimeout(mountBreakdown, 80); // IPC-like async data arrival
    setTimeout(() => {
      sampling = false;
      log.textContent = 'frames after select (outer / content):\\n' + frames.join('\\n');
      window.__reproResult = frames;
    }, 900);
  };

  // report the resting state so the driver can size the page to the edge case
  window.__metrics = () => ({
    outer: outer.scrollHeight + '/' + outer.clientHeight,
    content: content.scrollHeight + '/' + content.clientHeight,
    bodyH: document.body.scrollHeight,
  });
</script>
</body>
</html>`;

writeFileSync(out, html);
console.log('wrote', out);

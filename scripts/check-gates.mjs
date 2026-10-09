// Architecture and style governance gates: keep anti-patterns cleaned up in past
// reviews from creeping back; each failing gate lists the exact offending locations.
// Gate list:
//   1. Raw Chinese JSX leaks (user-facing copy must go through i18n)
//   2. Hardcoded color ban (new colors must use CSS variables; allowlist covers built-in defaults and the standalone export template)
//   3. `any` ratchet (only the existing baseline is allowed; new `any` raises the count and fails)
//   4. Frontend/backend layering (features/ must not call raw invoke; services/plugins/core/utils must not depend on commands)
//   5. File-size caps (component/module line counts)
//   6. Backend command registration consistency (every #[tauri::command] must be registered in lib.rs)
import { readFile, readdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = fileURLToPath(new URL('..', import.meta.url));
const errors = [];
const push = (gate, file, msg) => errors.push(`[${gate}] ${file}: ${msg}`);

async function* walk(dir, ext) {
  let entries;
  try {
    entries = await readdir(dir, { withFileTypes: true });
  } catch {
    return;
  }
  for (const e of entries) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) {
      if (['node_modules', 'dist', 'target', '.git', '.claude', '.aio'].includes(e.name)) continue;
      yield* walk(p, ext);
    } else if (ext.some((x) => e.name.endsWith(x))) {
      yield p;
    }
  }
}

const rel = (p) => path.relative(root, p).replaceAll('\\', '/');
const read = async (p) => readFile(p, 'utf8');

// ---------- Gate 1: raw Chinese JSX ----------
const JSX_CHINESE = /^[^<>{}/*"'`=()]*[\u4e00-\u9fff]/;
for await (const p of walk(path.join(root, 'src'), ['.tsx'])) {
  const lines = (await read(p)).split('\n');
  for (let i = 0; i < lines.length; i++) {
    const t = lines[i].trim();
    if (t && JSX_CHINESE.test(t) && !t.startsWith('{') && !t.includes('t(')) {
      // Plain-text JSX node (not inside a {} expression) containing Chinese -> leak
      if (/^[\u4e00-\u9fff]/.test(t) || /^[^<>{}]*[\u4e00-\u9fff][^<>{}]*$/.test(t)) {
        if (!t.includes('t(') && !t.includes('{') && !t.includes('//')) {
          push('i18n-jsx', rel(p), `L${i + 1} 裸中文 JSX: ${t.slice(0, 40)}`);
        }
      }
    }
  }
}

// ---------- Gate 2: hardcoded colors ----------
// Allowed: index.css (token definitions), the export template (standalone HTML where var() is unresolvable), ui-prefs themeColor default
const COLOR_ALLOWLIST = new Set([
  'src/index.css',
  'src/core/utils/exportConversation.ts',
  'src/core/store/ui-prefs.ts',
]);
const HEX_RE = /#[0-9a-fA-F]{3}(?:[0-9a-fA-F]{3,5})?\b/;
for await (const p of walk(path.join(root, 'src'), ['.tsx', '.ts'])) {
  const r = rel(p);
  if (COLOR_ALLOWLIST.has(r)) continue;
  const lines = (await read(p)).split('\n');
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    // Inline exemption: gate-allow: color <reason> on the same line or one/two lines above
    // (a JSX element's class line sits one line after the opening tag, so the exemption must cover the previous 2 lines)
    if (line.includes('gate-allow: color')) continue;
    if (i > 0 && lines[i - 1].includes('gate-allow: color')) continue;
    if (i > 1 && lines[i - 2].includes('gate-allow: color')) continue;
    if (HEX_RE.test(line)) {
      push('color', r, `L${i + 1} 硬编码颜色（应走 CSS 变量）: ${line.trim().slice(0, 60)}`);
    }
  }
}

// ---------- Gate 3: `any` ratchet (baseline cap, may only go down) ----------
// Baseline: the `any` count recorded when this gate first went green after the split.
// New code must not add `any`; lower the baseline as the backlog is cleaned up.
// Known hotspots: chatEventListeners.ts / ChatPage.tsx (listen<any> payloads pending typing).
const ANY_BASELINE = 163;
let anyCount = 0;
const anyFiles = [];
for await (const p of walk(path.join(root, 'src'), ['.tsx', '.ts'])) {
  const c = await read(p);
  const n = (c.match(/\b(?:as\s+any|:\s*any\b|<any>|any\[\]|Record<string,\s*any>)/g) || []).length;
  if (n > 0) {
    anyCount += n;
    if (n >= 5) anyFiles.push(`${rel(p)}(${n})`);
  }
}
if (anyCount > ANY_BASELINE) {
  push(
    'any-ratchet',
    '(global)',
    `any 计数 ${anyCount} 超过基准 ${ANY_BASELINE}；最大来源: ${anyFiles.slice(0, 5).join(', ')}`,
  );
}

// ---------- Gate 4: layering ----------
// Frontend: features/ components must not call @tauri-apps/api/core invoke directly (must go through core/api)
for await (const p of walk(path.join(root, 'src/features'), ['.tsx', '.ts'])) {
  const c = await read(p);
  if (/\binvoke[<(]/.test(c)) {
    push('layering-fe', rel(p), '组件层裸 invoke—— 必须经 core/api/ 域封装');
  }
}
// Backend: lower layers must not depend back on the commands layer
for await (const p of walk(path.join(root, 'src-tauri/src'), ['.rs'])) {
  const r = rel(p);
  if (/[\\/](?:services|plugins|core|utils)[\\/]/.test(r) && !/[\\/]commands[\\/]/.test(r)) {
    const c = await read(p);
    if (/crate::commands::/.test(c)) {
      push('layering-be', r, '低层（services/plugins/core/utils）反向依赖 commands 层');
    }
  }
}

// ---------- Gate 5: file-size caps ----------
// Size caps are growth gates (they pin the current state and ratchet down as components get split):
//   .tsx current max ChatPage 2016 -> cap 2050 (split target <900)
//   .rs  current max llm/mod.rs 2518 -> cap 2550 (target <1100)
const SIZE_LIMITS = [
  { dir: 'src/features', ext: ['.tsx'], max: 2050 },
  { dir: 'src/core', ext: ['.ts'], max: 900 },
  { dir: 'src/shared', ext: ['.tsx'], max: 900 },
  { dir: 'src-tauri/src', ext: ['.rs'], max: 2550 },
];
for (const { dir, ext, max } of SIZE_LIMITS) {
  for await (const p of walk(path.join(root, dir), ext)) {
    const n = (await read(p)).split('\n').length;
    if (n > max) push('file-size', rel(p), `${n} 行 > 上限 ${max}`);
  }
}

// ---------- Gate 6: backend command registration ----------
{
  const lib = await read(path.join(root, 'src-tauri/src/lib.rs'));
  // Collect full registered command paths (2-3 segments after commands::, e.g. config::save_assistant or llm::btw::ask_btw_question)
  const registered = new Set([...lib.matchAll(/commands::(\w+(?:::\w+)+)/g)].map((m) => m[1]));
  const missing = [];
  for await (const p of walk(path.join(root, 'src-tauri/src/commands'), ['.rs'])) {
    const c = await read(p);
    // Derive the module path from the file's relative path (commands/llm/btw.rs -> llm::btw)
    const relPath = path
      .relative(path.join(root, 'src-tauri/src/commands'), p)
      .replace(/\.rs$/, '')
      .replace(/[\\/]/g, '::')
      .replace(/::mod$/, '');
    for (const m of c.matchAll(/#\[tauri::command\][\s\S]*?pub (?:async )?fn (\w+)/g)) {
      const cmd = `${relPath}::${m[1]}`;
      if (!registered.has(cmd) && !missing.includes(cmd)) missing.push(cmd);
    }
  }
  if (missing.length)
    push('cmd-dispatch', 'src-tauri/src/lib.rs', '未注册的命令: ' + missing.join(', '));
}

// ---------- Output ----------
if (errors.length) {
  console.error('架构门禁失败（' + errors.length + ' 项）:');
  for (const e of errors) console.error('  ' + e);
  process.exit(1);
}
console.log(
  'gates passed: i18n-jsx=0, color=0, any=' +
    anyCount +
    ' (≤' +
    ANY_BASELINE +
    '), layering=0, file-size=0, cmd-dispatch=0',
);

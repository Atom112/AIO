// 架构与样式门禁（governance gates）—— 代码审查 M6 阶段产出。
// 目标：防止本次清理过的反模式回潮。每条门禁失败都会列出具体位置。
// 门禁清单：
//   1. 裸中文 JSX 泄漏（面向用户文案必须走 i18n）
//   2. 硬编码颜色禁令（新颜色必须走 CSS 变量；白名单允许内置默认与独立导出模板）
//   3. `any` 闸门（只允许既有存量，新增 any 会让计数上涨并失败）
//   4. 前端/后端分层断言（features/ 不得裸 invoke；services/plugins/core/utils 不得依赖 commands）
//   5. 文件规模上限（组件/模块文件行数）
//   6. 后端命令注册一致性（每个 #[tauri::command] 必须在 lib.rs 注册）
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

// ---------- 门禁 1：裸中文 JSX ----------
const JSX_CHINESE = /^[^<>{}/*"'`=()]*[\u4e00-\u9fff]/;
for await (const p of walk(path.join(root, 'src'), ['.tsx'])) {
  const lines = (await read(p)).split('\n');
  for (let i = 0; i < lines.length; i++) {
    const t = lines[i].trim();
    if (t && JSX_CHINESE.test(t) && !t.startsWith('{') && !t.includes('t(')) {
      // 纯文本 JSX 节点（不在 {} 表达式中）且含中文 → 泄漏
      if (/^[\u4e00-\u9fff]/.test(t) || /^[^<>{}]*[\u4e00-\u9fff][^<>{}]*$/.test(t)) {
        if (!t.includes('t(') && !t.includes('{') && !t.includes('//')) {
          push('i18n-jsx', rel(p), `L${i + 1} 裸中文 JSX: ${t.slice(0, 40)}`);
        }
      }
    }
  }
}

// ---------- 门禁 2：硬编码颜色 ----------
// 允许：index.css（令牌定义处）、导出模板（独立 HTML，var() 不可解析）、ui-prefs 的 themeColor 默认值
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
    // 内联豁免：同行或前一行/前两行含 gate-allow: color <原因>
    //（JSX 元素的 class 行在开标签后一行，故豁免需覆盖前 2 行）
    if (line.includes('gate-allow: color')) continue;
    if (i > 0 && lines[i - 1].includes('gate-allow: color')) continue;
    if (i > 1 && lines[i - 2].includes('gate-allow: color')) continue;
    if (HEX_RE.test(line)) {
      push('color', r, `L${i + 1} 硬编码颜色（应走 CSS 变量）: ${line.trim().slice(0, 60)}`);
    }
  }
}

// ---------- 门禁 3：any 闸门（存量封顶，只降不升）----------
// 基准：拆分后首次绿时的 any 计数。新代码不得新增 any；存量随清理逐步下调本基准。
// 拆分后首次绿时的实测基准（随清理逐步下调；只降不升）。
// 已知大头：chatEventListeners.ts / ChatPage.tsx（listen<any> 载荷待类型化）。
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

// ---------- 门禁 4：分层断言 ----------
// 前端：features/ 组件不得直接调用 @tauri-apps/api/core 的 invoke（必须走 core/api）
for await (const p of walk(path.join(root, 'src/features'), ['.tsx', '.ts'])) {
  const c = await read(p);
  if (/\binvoke[<(]/.test(c)) {
    push('layering-fe', rel(p), '组件层裸 invoke—— 必须经 core/api/ 域封装');
  }
}
// 后端：低层不得反向依赖命令层
for await (const p of walk(path.join(root, 'src-tauri/src'), ['.rs'])) {
  const r = rel(p);
  if (/[\\/](?:services|plugins|core|utils)[\\/]/.test(r) && !/[\\/]commands[\\/]/.test(r)) {
    const c = await read(p);
    if (/crate::commands::/.test(c)) {
      push('layering-be', r, '低层（services/plugins/core/utils）反向依赖 commands 层');
    }
  }
}

// ---------- 门禁 5：文件规模上限 ----------
// 规模上限为"防增长闸门"（封顶现状，随 M5e2 组件拆分下调）：
//   .tsx 当前最大 ChatPage 2016 → 封 2050（拆分目标 <900）
//   .rs  当前最大 llm/mod.rs 2518 → 封 2550（目标 <1100）
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

// ---------- 门禁 6：后端命令注册一致性 ----------
{
  const lib = await read(path.join(root, 'src-tauri/src/lib.rs'));
  // 收集注册的命令完整路径（commands:: 之后的 2~3 段：config::save_assistant 或 llm::btw::ask_btw_question）
  const registered = new Set([...lib.matchAll(/commands::(\w+(?:::\w+)+)/g)].map((m) => m[1]));
  const missing = [];
  for await (const p of walk(path.join(root, 'src-tauri/src/commands'), ['.rs'])) {
    const c = await read(p);
    // 由文件相对路径推导模块路径（commands/llm/btw.rs -> llm::btw）
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

// ---------- 输出 ----------
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

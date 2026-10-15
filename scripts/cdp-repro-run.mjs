// CDP driver for the heatmap repro page: launches nothing itself — expects a
// headless Edge already running with --remote-debugging-port. Clicks the
// "select used day" button and samples scroll metrics per frame.
// Usage: node scripts/cdp-repro-run.mjs --port <n> --url <file-url>
const args = process.argv.slice(2);
const opt = (name, dflt) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : dflt;
};
const port = Number(opt('--port', '9666'));
const url = opt('--url', '');

const newTarget = await (
  await fetch(`http://127.0.0.1:${port}/json/new?${encodeURIComponent(url)}`, { method: 'PUT' })
).json();

const ws = new WebSocket(newTarget.webSocketDebuggerUrl);
let id = 0;
const pending = new Map();
const send = (method, params = {}) =>
  new Promise((resolve, reject) => {
    const mid = ++id;
    pending.set(mid, { resolve, reject });
    ws.send(JSON.stringify({ id: mid, method, params }));
  });
ws.onmessage = (ev) => {
  const msg = JSON.parse(ev.data);
  if (msg.id && pending.has(msg.id)) {
    const { resolve, reject } = pending.get(msg.id);
    pending.delete(msg.id);
    if (msg.error) {
      reject(new Error(JSON.stringify(msg.error)));
    } else {
      resolve(msg.result);
    }
  }
};
await new Promise((r) => (ws.onopen = r));
await send('Page.enable');
await send('Runtime.enable');
await new Promise((r) => setTimeout(r, 1200)); // let entrance animations finish

const before = await send('Runtime.evaluate', {
  expression: 'JSON.stringify(window.__metrics())',
  returnByValue: true,
});
console.log('before click:', before.result.value);

await send('Runtime.evaluate', {
  expression: "document.getElementById('clickTarget').click()",
});
await new Promise((r) => setTimeout(r, 1400));

const after = await send('Runtime.evaluate', {
  expression: 'JSON.stringify({metrics: window.__metrics(), frames: window.__reproResult})',
  returnByValue: true,
});
console.log('after click:', after.result.value);

await send('Target.closeTarget', { targetId: newTarget.id }).catch(() => {});
ws.close();
process.exit(0);

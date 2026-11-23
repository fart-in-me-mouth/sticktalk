// Can a frame inside the app reach the Windows app's host-message handler?
// (docs/features/perf.md, task `headroom`.) The handler listens only to
// the server's app document (`Policy::is_app_page`, unit-tested); this shows,
// in the real WebView2, which documents' `window.ipc.postMessage` arrive at
// the handler at all. Runs the app with `--smoke`: never shown, a throwaway
// profile, no window on screen. A local page on 127.0.0.1 stands in for the
// server: the top document, a same-origin frame, and two sandboxed frames
// like an activity's (an opaque origin), each try the host channel.
//
// Run: node crates/desktop/tests/frame-ipc.mjs  (Windows; needs the built
// sticktalk-desktop.exe: $STICKTALK_DESKTOP_EXE or $CARGO_TARGET_DIR/debug).
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { once } from 'node:events';
import { execFile } from 'node:child_process';
import { join } from 'node:path';
import { existsSync } from 'node:fs';

const exe = process.env.STICKTALK_DESKTOP_EXE ?? join(process.env.CARGO_TARGET_DIR ?? 'target', 'debug', 'sticktalk-desktop.exe');
assert(existsSync(exe), `build the Windows app first: ${exe}`);

// What a frame reports about itself, and its attempt to reach the host.
const probe = (who) => `
  const seen = { who: ${JSON.stringify(who)}, ipc: typeof window.ipc, webview: typeof window.chrome?.webview };
  try { window.ipc?.postMessage(JSON.stringify({ smoke: seen.who })); seen.sent = !!window.ipc; } catch (e) { seen.error = String(e); }
  try { window.chrome?.webview?.postMessage(JSON.stringify({ smoke: seen.who + '-webview' })); } catch (e) { seen.webviewError = String(e); }
  parent.postMessage({ frameIpc: seen }, '*');`;
const frame = (who) => `<!doctype html><meta charset="utf-8"><script>${probe(who)}</script>`;
const top = `<!doctype html><meta charset="utf-8"><title>waiting</title>
<script>
  const results = [];
  addEventListener('message', (e) => {
    if (!e.data?.frameIpc) return;
    results.push(e.data.frameIpc);
    if (results.length === 3) document.title = JSON.stringify(results);
  });
  try { window.ipc?.postMessage(JSON.stringify({ smoke: 'top' })); } catch {}
</script>
<iframe src="/frame-same"></iframe>
<iframe src="/frame-sandboxed" sandbox="allow-scripts"></iframe>
<iframe srcdoc="${frame('srcdoc-sandboxed').replaceAll('"', '&quot;')}" sandbox="allow-scripts"></iframe>`;

const server = createServer((req, res) => {
  const body = req.url === '/' ? top
    : req.url === '/frame-same' ? frame('same-origin')
    : req.url === '/frame-sandboxed' ? frame('sandboxed') : null;
  if (body === null) { res.writeHead(404); return res.end(); }
  res.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store' });
  res.end(body);
});
server.listen(0, '127.0.0.1');
await once(server, 'listening');
const origin = `http://127.0.0.1:${server.address().port}`;
try {
  const { stdout } = await new Promise((done, fail) =>
    execFile(exe, ['--smoke', `${origin}/`, '--smoke-eval', 'void 0', '--smoke-wait', '3000'], { windowsHide: true, timeout: 60000 },
      (error, stdout, stderr) => (error && !stdout ? fail(Object.assign(error, { stderr })) : done({ stdout }))));
  const line = JSON.parse(stdout.trim().split(/\r?\n/).at(-1));
  assert.equal(line.ok, true, stdout);
  const frames = JSON.parse(line.title);
  const ipc = line.ipc ?? [];
  console.log(JSON.stringify({ frames, ipc }, null, 1));
  // The app's own document reaches the handler, which listens to it.
  assert(ipc.some((m) => m.accepted && new URL(m.uri).pathname === '/'), 'the top document reaches the handler');
  // Nothing from a frame is ever listened to.
  assert(!ipc.some((m) => m.accepted && new URL(m.uri).pathname !== '/'), 'no frame is listened to');
  // Every frame tried (WebView2 gives frames the host script too).
  assert.equal(frames.length, 3);
  assert(frames.every((f) => f.sent), JSON.stringify(frames));
  // Measured on WebView2 (2026-11-23): no frame's message reaches the handler
  // at all; only the top document's does. The URI check stays as defence in depth.
  assert(ipc.every((m) => m.uri === `${origin}/`), `only the app document reached the handler: ${JSON.stringify(ipc)}`);
  // Sandboxed frames like an activity's: whether they can reach the handler at all.
  for (const f of frames.filter((f) => f.who !== 'same-origin')) {
    const arrived = ipc.some((m) => !m.accepted && (m.uri === 'null' || m.uri.startsWith('about:') || m.uri.includes('frame-sandboxed')));
    console.log(`${f.who}: window.ipc is ${f.ipc}, chrome.webview is ${f.webview}; a message from it ${arrived ? 'reached the handler (refused there)' : 'never reached the handler'}`);
  }
  console.log('Frame IPC: only the app document is listened to; passed');
} finally {
  server.close();
}

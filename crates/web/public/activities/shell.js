// The shell a registered activity runs in (/activities/frame/custom-<n>).
// It passes SDK messages between the StickTalk page and the activity's own
// frame, and nothing else. Its policy (frame-src) allows only the activity's
// registered origin, so the activity frame cannot be navigated anywhere else.
// Both frames are sandboxed without allow-same-origin: their origins are
// opaque, so a message is recognized by the window that sent it.
// The shell loads this before its frame is parsed (no `defer`), so the relay
// is listening before the activity can send its one `ready`; the frame is
// looked up when a message arrives.
(() => {
  'use strict';
  const child = () => document.getElementById('activity')?.contentWindow;
  addEventListener('message', (e) => {
    const m = e.data;
    if (!m || typeof m !== 'object' || m.sticktalk !== 1) return;
    if (e.source === window.parent) child()?.postMessage(m, '*');
    else if (e.source && e.source === child()) window.parent.postMessage(m, '*');
  });
})();

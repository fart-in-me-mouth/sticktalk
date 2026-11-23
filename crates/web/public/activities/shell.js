// The shell a registered activity runs in (/activities/frame/custom-<n>).
// It passes SDK messages between the StickTalk page and the activity's own
// frame, and nothing else. Its policy (frame-src) allows only the activity's
// registered origin, so the activity frame cannot be navigated anywhere else.
// Both frames are sandboxed without allow-same-origin: their origins are
// opaque, so a message is recognized by the window that sent it.
(() => {
  'use strict';
  const child = document.getElementById('activity');
  addEventListener('message', (e) => {
    const m = e.data;
    if (!m || typeof m !== 'object' || m.sticktalk !== 1) return;
    if (e.source === window.parent) child.contentWindow?.postMessage(m, '*');
    else if (e.source === child.contentWindow) window.parent.postMessage(m, '*');
  });
})();

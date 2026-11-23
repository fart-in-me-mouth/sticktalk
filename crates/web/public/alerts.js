// Alerts for new messages, mentions, direct messages and reactions: a short
// sound and, while StickTalk is not focused, a desktop notification. The
// server decides what alerts (mutes, levels, Do Not Disturb, ignore); the app
// only applies this device's preferences. Nothing exists until an alert
// plays: one AudioContext is made for a chime and closed right after it, and
// a desktop notification is shown only if the person already allowed them.
//
// Tests and embedders replace both with `globalThis.__sticktalkAlert`, a
// function given { kind, title, body, tag, desktop, sound } or { badge }.
// With it, nothing plays and nothing is shown.

const CHIMES = {
  message: [[660, 0, 0.09]],
  direct: [[740, 0, 0.09], [988, 0.11, 0.14]],
  mention: [[880, 0, 0.09], [1175, 0.11, 0.16]],
  reaction: [[1047, 0, 0.07]],
};

let chiming = null;

function chime(kind) {
  const notes = CHIMES[kind] ?? CHIMES.message;
  if (!globalThis.AudioContext || chiming) return;
  const ctx = new AudioContext({ latencyHint: 'interactive' });
  chiming = ctx;
  const out = ctx.createGain();
  out.gain.value = 0.08;
  out.connect(ctx.destination);
  const begin = ctx.currentTime + 0.02;
  let end = begin;
  for (const [frequency, offset, length] of notes) {
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    const start = begin + offset;
    osc.type = 'sine';
    osc.frequency.value = frequency;
    gain.gain.setValueAtTime(0, start);
    gain.gain.linearRampToValueAtTime(1, start + 0.01);
    gain.gain.linearRampToValueAtTime(0, start + length);
    osc.connect(gain).connect(out);
    osc.start(start);
    osc.stop(start + length);
    end = Math.max(end, start + length);
    // The context closes when the last note ends: no timer keeps it alive.
    osc.onended = () => {
      if (ctx.currentTime + 0.001 >= end && chiming === ctx) {
        chiming = null;
        ctx.close().catch(() => {});
      }
    };
  }
}

function desktop(title, body, tag) {
  if (!('Notification' in globalThis) || Notification.permission !== 'granted') return;
  try {
    const n = new Notification(title, { body, tag: `sticktalk-${tag}`, silent: true });
    n.onclick = () => {
      window.focus();
      window.dispatchEvent(new CustomEvent('sticktalk-request', { detail: `channel:${tag}` }));
      n.close();
    };
  } catch {
    // Some browsers allow notifications only from a service worker.
  }
}

export function alert(kind, title, body, tag, showDesktop, sound) {
  const mock = globalThis.__sticktalkAlert;
  if (typeof mock === 'function') {
    mock({ kind, title, body, tag, desktop: showDesktop, sound });
    return;
  }
  if (sound) chime(kind);
  if (showDesktop) desktop(title, body, tag);
}

const BASE_TITLE = 'StickTalk';
export function badge(count) {
  const mock = globalThis.__sticktalkAlert;
  if (typeof mock === 'function') mock({ badge: count });
  const title = document.title.replace(/^\(\d+\+?\) /, '') || BASE_TITLE;
  document.title = count > 0 ? `(${count > 99 ? '99+' : count}) ${title}` : title;
  try {
    if (count > 0) navigator.setAppBadge?.(count)?.catch?.(() => {});
    else navigator.clearAppBadge?.()?.catch?.(() => {});
  } catch {
    // App badges are optional.
  }
}

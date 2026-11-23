// Ring sounds for direct and group calls. Nothing exists until a ring starts:
// then one AudioContext plays a pattern scheduled ahead on the audio clock (no
// JavaScript timers), and it is closed the moment the ring stops.
//
// Tests and embedders replace the sound with `globalThis.__sticktalkRingtone`,
// a function given "incoming", "outgoing" or "" (stop). With it, nothing plays.
const SETTING = 'sticktalk-call-ringtone';
const DEVICES = 'sticktalk-call-devices';
// The server stops a ring after 45 seconds; the pattern never outlasts that.
const RING_SECONDS = 45;

export function ringtoneEnabled() {
  try { return localStorage.getItem(SETTING) !== 'off'; } catch { return true; }
}
export function setRingtoneEnabled(on) {
  try {
    if (on) localStorage.removeItem(SETTING);
    else localStorage.setItem(SETTING, 'off');
  } catch { /* Storage may be disabled in a private browser. */ }
  if (!on) stop();
}

let context = null;
let playing = '';

function stop() {
  context?.close().catch(() => {});
  context = null;
}

// One tone burst: a soft attack and release, so it never clicks.
function tone(ctx, out, frequency, start, length) {
  const osc = ctx.createOscillator();
  const gain = ctx.createGain();
  osc.type = 'sine';
  osc.frequency.value = frequency;
  gain.gain.setValueAtTime(0, start);
  gain.gain.linearRampToValueAtTime(1, start + 0.02);
  gain.gain.setValueAtTime(1, start + length - 0.04);
  gain.gain.linearRampToValueAtTime(0, start + length);
  osc.connect(gain).connect(out);
  osc.start(start);
  osc.stop(start + length);
}

// Incoming: a bright two-note chime, twice, every three seconds.
// Outgoing: a low, unhurried ringback, a second on and three off.
const PATTERNS = {
  incoming: { period: 3, notes: [[784, 0, 0.18], [1047, 0.2, 0.28], [784, 0.7, 0.18], [1047, 0.9, 0.28]], level: 0.12 },
  outgoing: { period: 4, notes: [[440, 0, 1], [480, 0, 1]], level: 0.05 },
};

export function ring(kind) {
  if (kind === playing) return;
  playing = kind;
  const mock = globalThis.__sticktalkRingtone;
  if (typeof mock === 'function') {
    stop();
    mock(kind);
    return;
  }
  stop();
  const pattern = PATTERNS[kind];
  if (!pattern || !ringtoneEnabled() || !globalThis.AudioContext) return;
  const ctx = new AudioContext({ latencyHint: 'playback' });
  context = ctx;
  // The speaker chosen for calls, when this browser can pick one.
  try {
    const output = JSON.parse(localStorage.getItem(DEVICES) ?? '{}').output;
    if (typeof output === 'string' && output && ctx.setSinkId) ctx.setSinkId(output).catch(() => {});
  } catch { /* The default speaker. */ }
  const out = ctx.createGain();
  out.gain.value = pattern.level;
  out.connect(ctx.destination);
  const begin = ctx.currentTime + 0.05;
  for (let at = 0; at < RING_SECONDS; at += pattern.period)
    for (const [frequency, offset, length] of pattern.notes)
      tone(ctx, out, frequency, begin + at + offset, length);
  // A page that has not been interacted with may start suspended; it resumes
  // on the next interaction rather than polling.
  if (ctx.state === 'suspended') {
    const resume = () => { if (context === ctx) ctx.resume().catch(() => {}); };
    addEventListener('pointerdown', resume, { once: true, capture: true });
    addEventListener('keydown', resume, { once: true, capture: true });
  }
}

// Call extras, loaded with media.js: soundboard playback, entrance sounds, the
// voice channel's Opus bitrate, stream and camera quality, what each stream is
// actually delivering, clip indicators, and the call's "More" menu. The panels
// (soundboard.js), camera backgrounds (backgrounds.js) and clips (clips.js) are
// imported only when someone opens or turns them on.
//
// Nothing here runs while unused: no timers, no audio graph and no model. The
// soundboard's AudioContext exists only after a sound is heard in this call, is
// suspended whenever nothing plays and closed when the call ends.
let host = null;
export function bind(h) {
  host = h;
}
// Shared activities (activities.js) follow the call's output device, deafen,
// speaking and its end. Nothing is registered unless an activity runs.
const followers = new Set();
export function follow(fn) { followers.add(fn); return () => followers.delete(fn); }
export function callHost() { return host; }
const tellFollowers = (what, value) => { for (const f of followers) try { f(what, value); } catch {} };
export function speaking(data) { if (followers.size) tellFollowers('speaking', data); }
const $ = (s) => document.querySelector(s);
const emit = (action) => window.dispatchEvent(new CustomEvent('sticktalk-live', { detail: action }));
const key = (s) => `${s.user_id}:${s.connection_id}`;
// Tests measure the paths before these extras by switching them off.
const legacy = () => globalThis.__sticktalkLegacyQuality === true;

// ---- preferences, on this device ------------------------------------------
const PREFS = 'sticktalk-call-extras';
export const HEIGHTS = [[480, '480p'], [720, '720p'], [1080, '1080p'], [1440, '1440p'], [2160, '4K / Source']];
export const RATES = [15, 30, 60];
export const CLIP_SECONDS = [30, 60, 120];
const pick = (value, allowed, fallback) => (allowed.includes(value) ? value : fallback);
export function normalizePrefs(raw) {
  const v = raw && typeof raw === 'object' ? raw : {};
  const heights = HEIGHTS.map(([h]) => h);
  const quality = (q, height, fps) => ({
    height: pick(q?.height, heights, height),
    fps: pick(q?.fps, RATES, fps),
    mode: q?.mode === 'detail' ? 'detail' : 'motion',
  });
  const volume = Number(v.soundVolume);
  return {
    soundVolume: Number.isFinite(volume) ? Math.min(100, Math.max(0, Math.round(volume))) : 100,
    soundMuted: v.soundMuted === true,
    entrances: v.entrances !== false,
    camera: quality(v.camera, 720, 30),
    screen: quality(v.screen, 1080, 30),
    background:
      typeof v.background === 'string' && /^(none|blur-light|blur-strong|scene:[a-z]{2,16}|custom:\d{1,20})$/.test(v.background)
        ? v.background : 'none',
    clips: { enabled: v.clips?.enabled === true, seconds: pick(v.clips?.seconds, CLIP_SECONDS, 30) },
  };
}
export let prefs = normalizePrefs(null);
try {
  const raw = localStorage.getItem(PREFS);
  if (raw && raw.length <= 4096) prefs = normalizePrefs(JSON.parse(raw));
} catch { /* Storage may be disabled in a private browser. */ }
export function savePrefs(next) {
  prefs = normalizePrefs({ ...prefs, ...next });
  try { localStorage.setItem(PREFS, JSON.stringify(prefs)); } catch {}
  return prefs;
}

// ---- REST, with Snowflake IDs as strings ------------------------------------
export function csrf() {
  return document.cookie.split(';').map((v) => v.trim())
    .find((v) => v.startsWith('__Host-sticktalk_csrf='))?.split('=').slice(1).join('=') ?? '';
}
const MESSAGES = {
  Forbidden: 'You don’t have permission to do that.',
  NotFound: 'That is no longer available.',
  LimitExceeded: 'The limit has been reached.',
  RateLimited: 'Slow down a little, then try again.',
  PayloadTooLarge: 'That file is too large.',
};
export async function api(path, { method = 'GET', body, json } = {}) {
  const headers = {};
  if (method !== 'GET') headers['X-StickTalk-CSRF'] = csrf();
  if (json !== undefined) headers['Content-Type'] = 'application/json';
  const response = await fetch(path, {
    method, credentials: 'same-origin', cache: 'no-store', headers,
    body: json !== undefined ? JSON.stringify(json) : body,
    signal: AbortSignal.timeout(30000),
  });
  if (!response.ok) {
    let message = '';
    try {
      const error = await response.json();
      message = error.message && !/^[A-Z][A-Za-z]+$/.test(error.message) ? error.message : MESSAGES[error.code] ?? '';
    } catch {}
    throw new Error(message || `The server could not do that (${response.status}).`);
  }
  return response.status === 204 ? null : response.json();
}

// ---- soundboard playback ------------------------------------------------------
// A sound is a signalled event: each listener plays its own copy of the clip
// through the call's speaker, at full quality, never through a microphone.
let soundContext = null;
const playing = new Set();
// Decoded clips for this call: at most 24, and at most 48 MB of samples.
const clipCache = new Map();
const CACHE_ENTRIES = 24, CACHE_BYTES = 48 * 1024 * 1024;
function context() {
  if (!soundContext) {
    soundContext = new AudioContext({ latencyHint: 'interactive' });
    if (host?.output) soundContext.setSinkId?.(host.output).catch(() => {});
  }
  return soundContext;
}
function decoded(id) {
  const hit = clipCache.get(id);
  if (hit) {
    clipCache.delete(id);
    clipCache.set(id, hit);
    return hit.buffer;
  }
  const entry = { bytes: 0 };
  entry.buffer = fetch(`/api/v1/soundboard/sounds/${id}/audio`, { credentials: 'same-origin', cache: 'no-store' })
    .then((r) => {
      if (!r.ok) throw new Error('missing');
      return r.arrayBuffer();
    })
    .then((bytes) => context().decodeAudioData(bytes))
    .then((buffer) => {
      entry.bytes = buffer.length * buffer.numberOfChannels * 4;
      let total = 0;
      for (const e of clipCache.values()) total += e.bytes;
      while (clipCache.size > CACHE_ENTRIES || total > CACHE_BYTES) {
        const [oldest, e] = clipCache.entries().next().value;
        if (oldest === id) break;
        total -= e.bytes;
        clipCache.delete(oldest);
      }
      return buffer;
    })
    .catch(() => {
      clipCache.delete(id);
      return null;
    });
  clipCache.set(id, entry);
  return entry.buffer;
}
function silenced() {
  return prefs.soundMuted || host?.deafened;
}
/** Plays one sound for this listener, unless they are deafened or muted it. */
export async function play(sound, { entrance = false, preview = false } = {}) {
  if (!preview && (silenced() || (entrance && !prefs.entrances))) return false;
  const gain = (Number(sound.volume ?? 100) / 100) * (prefs.soundVolume / 100);
  // Tests (and any embedder that must stay silent) replace the speaker.
  if (globalThis.__sticktalkSoundboard) {
    globalThis.__sticktalkSoundboard({ id: String(sound.sound_id ?? sound.id), gain, entrance, preview });
    return true;
  }
  const buffer = await decoded(String(sound.sound_id ?? sound.id));
  if (!buffer || (!preview && silenced())) return false;
  const ctx = context();
  if (ctx.state === 'suspended') await ctx.resume().catch(() => {});
  // At most four at once: a fifth replaces the oldest.
  while (playing.size >= 4) {
    const oldest = playing.values().next().value;
    playing.delete(oldest);
    try { oldest.stop(); } catch {}
  }
  const source = ctx.createBufferSource(), level = ctx.createGain();
  source.buffer = buffer;
  level.gain.value = gain;
  source.connect(level).connect(ctx.destination);
  playing.add(source);
  source.onended = () => {
    playing.delete(source);
    level.disconnect();
    // Nothing playing: the audio thread goes quiet until the next sound.
    if (!playing.size && soundContext === ctx) ctx.suspend().catch(() => {});
  };
  source.start();
  return true;
}
function stopSounds() {
  for (const s of playing) try { s.stop(); } catch {}
  playing.clear();
}

// ---- notices: who played a sound, who is clipping -----------------------------
const clipping = new Map(); // session key -> session
let toasts = null, strip = null;
function overlay() {
  const view = $('.voice-view');
  if (!view) return;
  toasts ??= Object.assign(document.createElement('div'), { id: 'live-call-toasts', className: 'call-toasts' });
  toasts.setAttribute('role', 'status');
  toasts.setAttribute('aria-live', 'polite');
  strip ??= Object.assign(document.createElement('p'), { id: 'live-clipping-notice', className: 'call-clipping-notice' });
  const stage = $('#live-media-stage');
  if (toasts.parentElement !== view) view.append(toasts);
  if (strip.parentElement !== view) view.insertBefore(strip, stage ?? null);
}
function toast(text, icon, ms = 3500) {
  overlay();
  if (!toasts) return;
  while (toasts.childElementCount >= 4) toasts.firstElementChild.remove();
  const item = document.createElement('div');
  item.className = 'call-toast';
  if (icon) {
    const glyph = document.createElement('span');
    glyph.className = 'call-toast-icon';
    glyph.setAttribute('aria-hidden', 'true');
    glyph.textContent = icon;
    item.append(glyph);
  }
  item.append(document.createTextNode(text));
  toasts.append(item);
  setTimeout(() => item.remove(), ms);
}
function nameOf(session) {
  return session.user_id === host?.self() ? 'You' : host?.person(session.user_id).name ?? 'Someone';
}
function describeClipping() {
  overlay();
  const names = [...clipping.values()].map(nameOf);
  if (strip) {
    strip.hidden = !names.length;
    strip.textContent = !names.length ? ''
      : names.length === 1 && names[0] === 'You' ? 'You are keeping clips of this call’s streams.'
      : `Clips on: ${names.join(', ')}. Streams in this call can be clipped.`;
  }
  for (const e of host?.elements().values() ?? [])
    e.tile.classList.toggle('is-clipping', clipping.has(e.tile.dataset.participant) ||
      (e.tile.dataset.self === 'true' && clipsActive));
}
let clipsActive = false;
/** clips.js says whether this user's own buffer is running. */
export function ownClipping(active, session) {
  clipsActive = active;
  // Your own state is kept here; the server's echo of it is not counted twice.
  if (active && session) clipping.set('self', session);
  else clipping.delete('self');
  describeClipping();
  for (const b of document.querySelectorAll('[data-live="media:clip"]')) {
    b.hidden = !active;
    b.closest('.call-clip-slot')?.toggleAttribute('hidden', !active);
  }
}

// ---- events from the call -------------------------------------------------------
export function event(e) {
  const data = e.data;
  switch (e.type) {
    case 'sound_played': {
      const by = nameOf(data.by), s = data.sound;
      const heard = !silenced() && !(data.entrance && !prefs.entrances);
      play(s, { entrance: data.entrance }).catch(() => {});
      if (!data.entrance || heard)
        toast(data.entrance ? `${by} joined with ${s.name}` : `${by} played ${s.name}`, s.emoji || '🔊',
          Math.min(8000, Number(s.duration_ms) + 2500));
      // The tile of whoever played it lights up for as long as it lasts.
      for (const el of host?.elements().values() ?? [])
        if (el.tile.dataset.participant === key(data.by) && el.kind !== 'screen') {
          el.tile.dataset.sound = s.emoji || '🔊';
          setTimeout(() => { if (el.tile.dataset.sound) delete el.tile.dataset.sound; }, Math.min(6000, Number(s.duration_ms) + 500));
        }
      break;
    }
    case 'clipping':
      if (data.session.user_id === host?.self() && clipsActive) break;
      if (data.enabled) clipping.set(key(data.session), data.session);
      else clipping.delete(key(data.session));
      describeClipping();
      if (data.enabled && data.session.user_id !== host?.self())
        toast(`${nameOf(data.session)} turned on clips.`, '🎬');
      break;
    case 'clip_taken':
      toast(data.session.user_id === host?.self() ? 'Clip saved.' : `${nameOf(data.session)} saved a clip.`, '🎬');
      break;
    case 'audio_quality':
      setAudioBitrate(Number(data.bitrate));
      break;
    case 'left':
      clipping.delete(key(data.session));
      describeClipping();
      if (prefs.clips.enabled || clipsModule) clipsChanged();
      break;
    case 'participant':
      if (prefs.clips.enabled || clipsModule) clipsChanged();
      break;
    default:
      break;
  }
}

// ---- voice bitrate --------------------------------------------------------------
let audioBitrate = 0, answeredBitrate = 0, pendingBitrate = 0;
function setAudioBitrate(bps) {
  if (legacy()) {
    // Until it is switched back on: then the call's bitrate applies.
    pendingBitrate = bps;
    return;
  }
  if (!(bps >= 8000 && bps <= 512000) || bps === audioBitrate) return;
  audioBitrate = bps;
  host.enqueue(async () => {
    const p = host.publication('audio');
    if (!p) return;
    const params = p.transceiver.sender.getParameters();
    if (!params.encodings?.length) return;
    for (const e of params.encodings) {
      e.maxBitrate = audioBitrate;
      e.priority = 'high';
      e.networkPriority = 'high';
    }
    await p.transceiver.sender.setParameters(params).catch(() => {});
    // The encoder's target comes from the answer's Opus parameters.
    if (answeredBitrate !== audioBitrate) await host.negotiate();
  });
}
/** The SFU's answer, with the call's Opus bitrate for this browser's encoder. */
export function answer(sdp) {
  if (pendingBitrate && !legacy()) {
    const bps = pendingBitrate;
    pendingBitrate = 0;
    queueMicrotask(() => setAudioBitrate(bps));
  }
  if (!audioBitrate || legacy()) return sdp;
  const pts = new Set([...sdp.matchAll(/^a=rtpmap:(\d+) opus\/48000/gim)].map((m) => m[1]));
  let lines = sdp.split('\r\n');
  for (const pt of pts) {
    const at = lines.findIndex((l) => l.startsWith(`a=fmtp:${pt} `));
    if (at >= 0) {
      const params = lines[at].slice(`a=fmtp:${pt} `.length).split(';').filter((p) => p && !/^\s*maxaveragebitrate=/.test(p));
      lines[at] = `a=fmtp:${pt} ${[...params, `maxaveragebitrate=${audioBitrate}`].join(';')}`;
    } else {
      const map = lines.findIndex((l) => l.startsWith(`a=rtpmap:${pt} `));
      if (map >= 0) lines.splice(map + 1, 0, `a=fmtp:${pt} maxaveragebitrate=${audioBitrate}`);
    }
  }
  answeredBitrate = audioBitrate;
  return lines.join('\r\n');
}

// ---- stream and camera quality --------------------------------------------------
// Payload bitrates for H.264 at 30 fps; 60 fps needs about 1.6 times as much.
const BASE = { 480: 1_200_000, 720: 2_500_000, 1080: 4_500_000, 1440: 8_000_000, 2160: 14_000_000 };
const sfuLimits = new Map();
export function capFor(kind, q = kind === 'camera' ? prefs.camera : prefs.screen) {
  const factor = q.fps >= 60 ? 1.6 : q.fps <= 15 ? 0.6 : 1;
  return Math.round(BASE[q.height] * factor);
}
/** The SFU's per-track allowance, never above what the chosen quality needs. */
export function videoCap(p, bps) {
  if (!p || legacy() || (p.kind !== 'camera' && p.kind !== 'screen')) return bps;
  if (p.transceiver.mid) sfuLimits.set(p.transceiver.mid, bps);
  return Math.min(bps, capFor(p.kind));
}
export function cameraConstraints() {
  const q = prefs.camera;
  return {
    width: { ideal: Math.round((q.height * 16) / 9) },
    height: { ideal: q.height },
    frameRate: { ideal: q.fps, max: q.fps },
  };
}
/** Encoder settings for a published camera: its cap, frame rate and what gives
 *  way first when the network or the CPU cannot keep up. A shared screen has no
 *  choices (media.js: 30 fps, as sharp as the connection carries). */
export function tune(kind) {
  if (legacy() || kind !== 'camera') return;
  host.enqueue(async () => {
    const p = host.publication(kind);
    if (!p) return;
    const q = kind === 'camera' ? prefs.camera : prefs.screen;
    if (kind === 'screen') p.track.contentHint = q.mode === 'detail' ? 'detail' : 'motion';
    const sender = p.transceiver.sender, params = sender.getParameters();
    if (!params.encodings?.length) return;
    const cap = capFor(kind), limit = sfuLimits.get(p.transceiver.mid);
    for (const e of params.encodings) {
      e.maxBitrate = limit ? Math.min(limit, cap) : cap;
      e.maxFramerate = q.fps;
      e.scaleResolutionDownBy = 1;
    }
    params.degradationPreference = kind === 'screen' && q.mode === 'detail' ? 'maintain-resolution'
      : kind === 'screen' ? 'maintain-framerate' : 'balanced';
    try { await sender.setParameters(params); } catch {
      delete params.degradationPreference;
      await sender.setParameters(params).catch(() => {});
    }
    // Tell the SFU what this track needs, so others' smaller needs leave it room.
    if (p.transceiver.mid) host.signal('track_quality', { mid: p.transceiver.mid, max_bps: cap });
  });
}
function selectField(label, options, value, change) {
  const field = document.createElement('label');
  field.className = 'field';
  field.textContent = label;
  const select = document.createElement('select');
  select.setAttribute('aria-label', label);
  for (const [v, text] of options) select.add(new Option(text, String(v)));
  select.value = String(value);
  select.onchange = () => change(select.value);
  field.append(select);
  return field;
}
function qualityFields(onCamera) {
  const box = document.createElement('div');
  box.className = 'call-quality-fields';
  const heading = (text) => Object.assign(document.createElement('h3'), { className: 'category mt-4', textContent: text });
  box.append(
    heading('Camera'),
    selectField('Camera Resolution', HEIGHTS.slice(0, 4), prefs.camera.height, (v) => { savePrefs({ camera: { ...prefs.camera, height: Number(v) } }); onCamera?.(); }),
    selectField('Camera Frame Rate', RATES.map((n) => [n, `${n} fps`]), prefs.camera.fps, (v) => { savePrefs({ camera: { ...prefs.camera, fps: Number(v) } }); onCamera?.(); }),
  );
  const note = document.createElement('p');
  note.className = 'muted-copy';
  note.textContent = 'StickTalk sends the camera quality you choose when your device and connection allow it, and steps down smoothly when they don’t. A shared screen always runs at 30 fps, as sharp as your connection allows. Everyone sees what is actually being delivered on each stream.';
  box.append(note);
  return box;
}
function liveQualityChanged() {
  if (host.publication('camera')) host.restartCamera();
}

// ---- what each stream really delivers ---------------------------------------------
let statsAt = 0;
const counters = new Map();
export function stats(report) {
  const now = performance.now();
  if (now - statsAt < 950) return;
  statsAt = now;
  const inbound = new Map(), outbound = new Map();
  for (const row of report.values()) {
    if (row.kind !== 'video') continue;
    if (row.type === 'inbound-rtp' && row.trackIdentifier) inbound.set(row.trackIdentifier, row);
    if (row.type === 'outbound-rtp' && row.mid) outbound.set(row.mid, row);
  }
  for (const [id, e] of host.elements()) {
    if (e.kind !== 'camera' && e.kind !== 'screen') continue;
    let text = '', row;
    if (e.tile.dataset.self === 'true') {
      const p = host.publication(e.kind);
      row = p && outbound.get(p.transceiver.mid);
      if (row?.frameWidth) {
        const last = counters.get(id), bits = last ? ((row.bytesSent - last.bytes) * 8) / ((row.timestamp - last.at) / 1000) : 0;
        counters.set(id, { bytes: row.bytesSent, at: row.timestamp });
        const limited = row.qualityLimitationReason && row.qualityLimitationReason !== 'none'
          ? ` · limited by ${row.qualityLimitationReason === 'cpu' ? 'your device' : 'the connection'}` : '';
        text = `Sending ${row.frameWidth}×${row.frameHeight} · ${Math.round(row.framesPerSecond ?? 0)} fps${bits > 0 ? ` · ${(bits / 1e6).toFixed(1)} Mbps` : ''}${limited}`;
      }
    } else {
      const track = e.media.srcObject?.getVideoTracks?.()[0];
      row = track && inbound.get(track.id);
      if (row?.frameWidth) text = `${row.frameWidth}×${row.frameHeight} · ${Math.round(row.framesPerSecond ?? 0)} fps`;
    }
    let badge = e.tile.querySelector('.tile-quality');
    if (!text) { badge?.remove(); continue; }
    if (!badge) {
      badge = document.createElement('span');
      badge.className = 'tile-quality';
      e.tile.append(badge);
    }
    if (badge.textContent !== text) badge.textContent = text;
    e.tile.dataset.delivered = row ? `${row.frameWidth}x${row.frameHeight}@${Math.round(row.framesPerSecond ?? 0)}` : '';
  }
}

// ---- the call's More menu and dialogs ---------------------------------------------
export function dialog(title, { className = '', label } = {}) {
  const d = document.createElement('dialog');
  d.className = `live-share-options call-extras-dialog ${className}`;
  const h = document.createElement('h2');
  h.id = `call-extras-title-${Math.random().toString(36).slice(2)}`;
  h.textContent = title;
  d.setAttribute('aria-labelledby', h.id);
  if (label) d.setAttribute('aria-label', label);
  d.addEventListener('close', () => d.remove());
  d.addEventListener('keydown', (e) => e.stopPropagation());
  d.append(h);
  document.body.append(d);
  return d;
}
export function button(text, className, onclick) {
  const b = document.createElement('button');
  b.type = 'button';
  b.className = className;
  b.textContent = text;
  b.onclick = onclick;
  return b;
}
function more() {
  $('.call-more-menu')?.close();
  const d = dialog('More', { className: 'call-more-menu' });
  const items = [
    ['🔊', 'Soundboard', 'soundboard'],
    ['✨', 'Video Effects', 'backgrounds'],
    ['🎚', 'Camera Quality', 'quality'],
    ...(clipsActive ? [['🎬', 'Save Clip', 'clip']] : []),
    ['🎞', prefs.clips.enabled ? 'Clips' : 'Turn On Clips', 'clips'],
    ['🎧', 'Audio Devices', 'output'],
  ];
  const list = document.createElement('div');
  list.className = 'call-more-items';
  list.setAttribute('role', 'menu');
  for (const [glyph, text, command] of items) {
    const b = button('', 'call-more-item', () => { d.close(); host.action(command); });
    b.setAttribute('role', 'menuitem');
    b.append(Object.assign(document.createElement('span'), { textContent: glyph, ariaHidden: 'true' }), text);
    list.append(b);
  }
  d.append(list, button('Close', 'button secondary', () => d.close()));
  d.showModal();
  list.querySelector('button')?.focus();
}
function qualityDialog() {
  const d = dialog('Camera Quality');
  d.append(qualityFields(() => liveQualityChanged()),
    button('Done', 'button primary', () => d.close()));
  d.showModal();
}

// ---- Voice & Video settings, and a voice channel's bitrate in channel settings ----
function checkbox(label, checked, change) {
  const row = document.createElement('label');
  row.className = 'live-checkbox';
  const text = document.createElement('span');
  text.textContent = label;
  const input = document.createElement('input');
  input.type = 'checkbox';
  input.checked = checked;
  input.onchange = () => change(input.checked);
  row.append(text, input);
  return row;
}
export function soundControls(onChange) {
  const box = document.createElement('div');
  box.className = 'soundboard-controls';
  const field = document.createElement('label');
  field.className = 'field';
  field.textContent = 'Soundboard Volume';
  const slider = document.createElement('input');
  slider.type = 'range';
  slider.min = '0';
  slider.max = '100';
  slider.value = String(prefs.soundVolume);
  slider.setAttribute('aria-valuetext', `${prefs.soundVolume}%`);
  slider.oninput = () => {
    savePrefs({ soundVolume: Number(slider.value) });
    slider.setAttribute('aria-valuetext', `${slider.value}%`);
    onChange?.();
  };
  field.append(slider);
  box.append(field,
    checkbox('Mute Soundboard', prefs.soundMuted, (v) => { savePrefs({ soundMuted: v }); if (v) stopSounds(); onChange?.(); }),
    checkbox('Hear Entrance Sounds', prefs.entrances, (v) => { savePrefs({ entrances: v }); onChange?.(); }));
  return box;
}
/** Sections appended to Voice & Video settings. */
export function settings() {
  const root = document.createElement('section');
  root.className = 'call-extras-settings';
  root.id = 'live-call-extras-settings';
  const heading = (text) => Object.assign(document.createElement('h3'), { className: 'category mt-8', textContent: text });
  const entrance = document.createElement('div');
  entrance.className = 'entrance-settings';
  entrance.textContent = 'Loading…';
  import('./soundboard.js').then((m) => m.entranceSettings(entrance)).catch(() => { entrance.textContent = 'Couldn’t load entrance sounds.'; });
  const background = document.createElement('div');
  background.className = 'mt-2 flex flex-wrap gap-2';
  background.append(button('Video Effects…', 'button secondary', () => action('backgrounds')));
  const clips = document.createElement('div');
  clips.append(
    checkbox('Keep Clips of Streams', prefs.clips.enabled, (v) => { savePrefs({ clips: { ...prefs.clips, enabled: v } }); clipsChanged(); }),
    selectField('Clip Length', CLIP_SECONDS.map((s) => [s, `Last ${s} seconds`]), prefs.clips.seconds,
      (v) => { savePrefs({ clips: { ...prefs.clips, seconds: Number(v) } }); clipsChanged(); }),
    Object.assign(document.createElement('p'), { className: 'muted-copy', textContent: 'Off by default. While it is on and a stream is playing in a call, StickTalk keeps the last moments in memory on this device so you can save them. Everyone in the call sees that clips are on.' }),
    button('Clip Library…', 'button secondary', () => action('library')));
  root.append(heading('Soundboard'), soundControls(), heading('Entrance Sound'), entrance,
    heading('Video Quality'), qualityFields(() => host?.publication('camera') && liveQualityChanged()),
    heading('Camera Background'), background, heading('Clips'), clips);
  return root;
}
async function bitrateSection(section) {
  if (section.dataset.ready) return;
  section.dataset.ready = 'true';
  const channel = section.dataset.voiceBitrate;
  const heading = Object.assign(document.createElement('h3'), { className: 'category', textContent: 'Voice Quality' });
  const field = document.createElement('label');
  field.className = 'field';
  field.textContent = 'Bitrate';
  const slider = Object.assign(document.createElement('input'), { type: 'range', min: '8', max: '384', step: '8', disabled: true });
  const readout = Object.assign(document.createElement('output'), { className: 'text-sm text-foreground' });
  const note = Object.assign(document.createElement('small'), { textContent: 'Higher sounds better and uses more data. Direct and group calls always use the best quality.' });
  const save = button('Save Bitrate', 'button secondary mt-2', null);
  save.disabled = true;
  const status = Object.assign(document.createElement('p'), { className: 'muted-copy', role: 'status' });
  const show = () => { readout.textContent = `${slider.value} kbps`; slider.setAttribute('aria-valuetext', `${slider.value} kbps`); };
  slider.oninput = () => { show(); save.disabled = section.dataset.canManage !== 'true'; status.textContent = ''; };
  field.append(slider, readout, note);
  section.replaceChildren(heading, field, save, status);
  try {
    const q = await api(`/api/v1/channels/${channel}/voice-quality`);
    slider.value = String(Math.round(q.bitrate / 1000));
    slider.disabled = section.dataset.canManage !== 'true';
    show();
  } catch (e) {
    status.textContent = e.message;
  }
  save.onclick = async () => {
    save.disabled = true;
    try {
      await api(`/api/v1/channels/${channel}/voice-quality`, { method: 'PUT', json: { bitrate: Number(slider.value) * 1000 } });
      status.textContent = 'Saved. People in this channel switch now.';
    } catch (e) {
      status.textContent = e.message;
      save.disabled = false;
    }
  };
}

// ---- lazily loaded parts -------------------------------------------------------------
let clipsModule = null, backgroundsModule = null;
async function clips() {
  clipsModule ??= await import('./clips.js');
  clipsModule.bind(host, { ownClipping, toast, dialog, button, prefs: () => prefs, savePrefs });
  return clipsModule;
}
function clipsChanged() {
  if (prefs.clips.enabled || clipsModule) clips().then((m) => m.update()).catch(host?.report ?? (() => {}));
}
async function backgrounds() {
  backgroundsModule ??= await import('./backgrounds.js');
  backgroundsModule.bind(host, { dialog, button, api, prefs: () => prefs, savePrefs, toast });
  return backgroundsModule;
}
/** The camera stream to publish: with the chosen background, or as it is. */
export async function camera(stream) {
  if (prefs.background === 'none') return stream;
  try {
    return await (await backgrounds()).process(stream);
  } catch (e) {
    host.report(new Error(`Your camera is on without its background effect. ${e.message ?? ''}`.trim()));
    return stream;
  }
}
export function cameraStopped() {
  backgroundsModule?.release();
}
/** A stream started or stopped, or a tile came or went: clips follow. */
export function streamsChanged() {
  if (prefs.clips.enabled || clipsModule) clipsChanged();
}

// ---- hooks from media.js -------------------------------------------------------------
export function deafen(on) {
  if (on) stopSounds();
  tellFollowers('deafen', on);
}
export function output(id) {
  soundContext?.setSinkId?.(id).catch(() => {});
  tellFollowers('output', id);
}
export function mount() {
  if ($('.voice-view')) {
    overlay();
    describeClipping();
    for (const b of document.querySelectorAll('[data-live="media:clip"]')) b.hidden = !clipsActive;
  }
  for (const section of document.querySelectorAll('[data-voice-bitrate]')) bitrateSection(section);
}
/** The call ended: everything here stops and lets go of its memory. */
export function stop() {
  tellFollowers('stop');
  stopSounds();
  soundContext?.close().catch(() => {});
  soundContext = null;
  clipCache.clear();
  clipping.clear();
  sfuLimits.clear();
  counters.clear();
  audioBitrate = answeredBitrate = 0;
  backgroundsModule?.release();
  clipsModule?.stop();
  toasts?.remove();
  strip?.remove();
  toasts = strip = null;
}
export function action(name) {
  switch (name) {
    case 'more': more(); break;
    case 'soundboard':
      import('./soundboard.js').then((m) => m.open(host, { play, api, dialog, button, soundControls, prefs: () => prefs }))
        .catch(host.report);
      break;
    case 'backgrounds': backgrounds().then((m) => m.open()).catch(host.report); break;
    case 'quality': qualityDialog(); break;
    case 'clips':
      if (!prefs.clips.enabled) { savePrefs({ clips: { ...prefs.clips, enabled: true } }); clipsChanged(); }
      clips().then((m) => m.settingsDialog()).catch(host.report);
      break;
    case 'clip': clips().then((m) => m.take()).catch(host.report); break;
    case 'library': clips().then((m) => m.library()).catch(host.report); break;
    case 'voice-bitrate': mount(); break;
    default: break;
  }
}
// Diagnostics for tests: what this module believes, never media data.
export function diagnostics() {
  return {
    audioBitrate, answeredBitrate, prefs, clipping: [...clipping.keys()], clipsActive,
    soundContext: soundContext?.state ?? null, cachedSounds: clipCache.size,
    backgrounds: backgroundsModule?.diagnostics() ?? null, clips: clipsModule?.diagnostics() ?? null,
  };
}

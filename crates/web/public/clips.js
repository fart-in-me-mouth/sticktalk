// Clips: an opt-in rolling buffer of the last 30, 60 or 120 seconds of a stream
// you share or watch in a call, kept only in this tab's memory. "Save Clip"
// (or Alt+C) freezes it; trim the start and end, choose whether the call's
// sound is included, name it, then download it, keep it in the local library,
// or share it to the call's chat as a video.
//
// The buffer exists only while the setting is on and a stream is playing. It
// holds already-encoded H.264 and Opus (WebCodecs, hardware encoder where there
// is one) bounded by time and by bytes, so its memory never grows with length.
// Everyone in the call is told while clips are on, and when one is saved.
let host = null, t = null;
export function bind(h, tools) {
  host = h;
  t = tools;
}
const element = (tag, props = {}, ...children) => {
  const e = Object.assign(document.createElement(tag), props);
  e.append(...children);
  return e;
};
/** Memory for the whole buffer, video and sound together. */
export const BUFFER_BYTES = 96 * 1024 * 1024;
export function supported() {
  return !!(globalThis.VideoEncoder && globalThis.AudioEncoder && globalThis.MediaStreamTrackProcessor);
}

// ---- which stream is clipped ---------------------------------------------------
function source() {
  const own = host.publication('screen');
  if (own) return { track: own.track, who: host.self(), label: 'your stream', kind: 'screen' };
  const screens = [...host.elements().values()].filter((e) => e.kind === 'screen' && e.tile.dataset.self !== 'true' && !e.tile.classList.contains('is-closed'));
  const watched = screens.find((e) => e.tile.classList.contains('is-focused')) ?? screens[0];
  const track = watched?.media.srcObject?.getVideoTracks()[0];
  if (!track || track.readyState !== 'live') return null;
  return { track, who: watched.user, label: `${watched.who}'s stream`, kind: 'screen' };
}
/** Every sound of the call: the others, your microphone and the streams. */
function callAudio() {
  const tracks = [];
  for (const e of host.elements().values()) {
    if (e.tile.dataset.self === 'true') continue;
    const tr = e.media.srcObject?.getAudioTracks?.()[0];
    if (tr && tr.readyState === 'live') tracks.push(tr);
  }
  for (const kind of ['audio', 'system_audio']) {
    const p = host.publication(kind);
    if (p?.track.readyState === 'live') tracks.push(p.track);
  }
  return tracks;
}

// ---- the rolling buffer ------------------------------------------------------------
class Buffer {
  constructor(seconds) {
    this.seconds = seconds;
    this.video = [];
    this.audio = [];
    this.bytes = 0;
    this.config = null;
    this.audioConfig = null;
  }
  push(list, chunk) {
    const data = new Uint8Array(chunk.byteLength);
    chunk.copyTo(data);
    list.push({ ts: chunk.timestamp, key: chunk.type === 'key', data });
    this.bytes += data.byteLength;
    this.evict();
  }
  evict() {
    const window = (this.seconds + 1) * 1e6;
    const last = this.video.at(-1)?.ts ?? 0;
    // Whole groups of pictures leave together, so the buffer starts on a key frame.
    while (this.video.length > 1 && (last - this.video[0].ts > window || this.bytes > BUFFER_BYTES)) {
      do this.bytes -= this.video.shift().data.byteLength;
      while (this.video.length > 1 && !this.video[0].key);
    }
    const first = this.video[0]?.ts ?? last;
    while (this.audio.length && (this.audio[0].ts < first - 500_000 || (this.bytes > BUFFER_BYTES && this.audio.length > 1)))
      this.bytes -= this.audio.shift().data.byteLength;
  }
  snapshot() {
    return { video: this.video.slice(), audio: this.audio.slice(), config: this.config, audioConfig: this.audioConfig };
  }
}
const even = (n) => Math.max(2, Math.round(n / 2) * 2);
class Recorder {
  constructor(src, seconds) {
    this.src = src;
    this.buffer = new Buffer(seconds);
    this.stopped = false;
  }
  async start() {
    const track = this.src.track.clone();
    this.track = track;
    const s = track.getSettings();
    const w0 = s.width || 1280, h0 = s.height || 720, k = Math.min(1, 1920 / w0, 1080 / h0);
    const width = even(w0 * k), height = even(h0 * k), fps = Math.min(60, s.frameRate || 30);
    // Enough quality for the length the buffer has to hold within its memory.
    const bitrate = Math.min(fps > 30 ? 10_000_000 : 7_000_000, Math.floor((BUFFER_BYTES * 8 * 0.9) / (this.buffer.seconds + 2)));
    let config = null;
    for (const codec of [height > 720 || fps > 30 ? 'avc1.64002a' : 'avc1.640028', 'avc1.4d002a', 'avc1.42e02a']) {
      const c = { codec, width, height, bitrate, framerate: fps, avc: { format: 'avc' }, latencyMode: 'realtime', hardwareAcceleration: 'prefer-hardware' };
      const support = await VideoEncoder.isConfigSupported(c).catch(() => null);
      if (support?.supported) { config = support.config; break; }
      delete c.hardwareAcceleration;
      const fallback = await VideoEncoder.isConfigSupported(c).catch(() => null);
      if (fallback?.supported) { config = fallback.config; break; }
    }
    if (!config) throw new Error('This device cannot record clips.');
    this.config = config;
    this.encoder = new VideoEncoder({
      output: (chunk, meta) => {
        if (meta?.decoderConfig) this.buffer.config = { ...meta.decoderConfig, width, height };
        this.buffer.push(this.buffer.video, chunk);
      },
      error: () => this.fail(),
    });
    this.encoder.configure(config);
    this.reader = new MediaStreamTrackProcessor({ track, maxBufferSize: 4 }).readable.getReader();
    this.pumpVideo(fps);
    this.startAudio();
  }
  async pumpVideo(fps) {
    let lastKey = -Infinity, last = 0;
    const gap = 1e6 / fps * 0.5;
    while (!this.stopped) {
      const { value: frame, done } = await this.reader.read().catch(() => ({ done: true }));
      if (done) break;
      const now = Math.round(performance.now() * 1000);
      // Never queue behind the encoder: a late frame is dropped, not delayed.
      if (this.stopped || this.encoder.state !== 'configured' || this.encoder.encodeQueueSize > 3 || now - last < gap) {
        frame.close();
        continue;
      }
      last = now;
      const stamped = new VideoFrame(frame, { timestamp: now });
      frame.close();
      const key = now - lastKey >= 1_000_000;
      if (key) lastKey = now;
      this.encoder.encode(stamped, { keyFrame: key });
      stamped.close();
    }
  }
  startAudio() {
    const tracks = callAudio();
    this.context = new AudioContext({ sampleRate: 48000 });
    this.mix = this.context.createMediaStreamDestination();
    this.sources = new Map();
    this.wire(tracks);
    const output = this.mix.stream.getAudioTracks()[0];
    this.audioEncoder = new AudioEncoder({
      output: (chunk, meta) => {
        if (meta?.decoderConfig) this.buffer.audioConfig = meta.decoderConfig;
        this.buffer.push(this.buffer.audio, { byteLength: chunk.byteLength, copyTo: (d) => chunk.copyTo(d), type: 'key', timestamp: chunk.timestamp + this.audioOffset });
      },
      error: () => {},
    });
    this.audioEncoder.configure({ codec: 'opus', sampleRate: 48000, numberOfChannels: 2, bitrate: 128000 });
    this.audioReader = new MediaStreamTrackProcessor({ track: output }).readable.getReader();
    (async () => {
      while (!this.stopped) {
        const { value: data, done } = await this.audioReader.read().catch(() => ({ done: true }));
        if (done) break;
        // One clock for picture and sound: when each arrived here.
        this.audioOffset ??= Math.round(performance.now() * 1000) - data.timestamp;
        if (this.audioEncoder.state === 'configured' && this.audioEncoder.encodeQueueSize < 20) this.audioEncoder.encode(data);
        data.close();
      }
    })();
  }
  /** Follow who is in the call: new voices join the mix, departed ones leave. */
  wire(tracks = callAudio()) {
    if (!this.context) return;
    const live = new Set(tracks.map((tr) => tr.id));
    for (const [id, node] of this.sources) if (!live.has(id)) { node.disconnect(); this.sources.delete(id); }
    for (const tr of tracks) {
      if (this.sources.has(tr.id)) continue;
      const node = this.context.createMediaStreamSource(new MediaStream([tr]));
      node.connect(this.mix);
      this.sources.set(tr.id, node);
    }
  }
  fail() {
    this.stop();
    rec = null;
    t.ownClipping(false);
    t.toast('Clips stopped: the encoder failed on this device.', '🎬');
  }
  stop() {
    if (this.stopped) return;
    this.stopped = true;
    this.reader?.cancel().catch(() => {});
    this.audioReader?.cancel().catch(() => {});
    this.track?.stop();
    for (const node of this.sources?.values() ?? []) node.disconnect();
    this.context?.close().catch(() => {});
    if (this.encoder?.state !== 'closed') this.encoder?.close();
    if (this.audioEncoder?.state !== 'closed') this.audioEncoder?.close();
    this.buffer = null;
  }
}

let rec = null, keys = false;
/** Start, retarget or stop the buffer to match the setting and the call. */
export function update() {
  host.enqueue(async () => {
    const on = t.prefs().clips.enabled && host.joined && supported();
    const src = on ? source() : null;
    if (rec && (!src || rec.src.track !== src.track || rec.buffer?.seconds !== t.prefs().clips.seconds)) {
      rec.stop();
      rec = null;
    }
    if (src && !rec) {
      rec = new Recorder(src, t.prefs().clips.seconds);
      try {
        await rec.start();
      } catch (e) {
        rec.stop();
        rec = null;
        t.toast(e.message, '🎬', 5000);
      }
    } else rec?.wire();
    const active = !!rec;
    // The server tells everyone in the call; it is told only when this changes.
    if (active !== announced && host.joined) {
      announced = active;
      host.signal('set_clipping', { enabled: active });
    }
    t.ownClipping(active, active ? { user_id: host.self(), connection_id: 'self' } : null);
    shortcut(active);
  });
}
let announced = false;
export function stop() {
  rec?.stop();
  rec = null;
  announced = false;
  shortcut(false);
  t?.ownClipping(false);
}
function onKey(e) {
  if (e.altKey && !e.ctrlKey && !e.metaKey && e.code === 'KeyC' && !e.target.closest?.('input,textarea,[contenteditable]')) {
    e.preventDefault();
    take();
  }
}
function shortcut(on) {
  if (on === keys) return;
  keys = on;
  if (on) document.addEventListener('keydown', onKey);
  else document.removeEventListener('keydown', onKey);
}

// ---- MP4 (H.264 + Opus), written here: no muxing library -------------------------
const text = (s) => new TextEncoder().encode(s);
function u(n, bytes) {
  const out = new Uint8Array(bytes);
  for (let i = bytes - 1; i >= 0; i--) { out[i] = n & 255; n = Math.floor(n / 256); }
  return out;
}
function cat(parts) {
  const size = parts.reduce((n, p) => n + p.byteLength, 0), out = new Uint8Array(size);
  let at = 0;
  for (const p of parts) { out.set(p, at); at += p.byteLength; }
  return out;
}
const box = (type, ...parts) => {
  const body = cat(parts.flat());
  return cat([u(body.byteLength + 8, 4), text(type), body]);
};
const full = (type, version, flags, ...parts) => box(type, u(version, 1), u(flags, 3), ...parts);
const MATRIX = cat([u(0x10000, 4), u(0, 4), u(0, 4), u(0, 4), u(0x10000, 4), u(0, 4), u(0, 4), u(0, 4), u(0x40000000, 4)]);
function runs(deltas) {
  const out = [];
  for (const d of deltas) {
    if (out.length && out.at(-1)[1] === d) out.at(-1)[0]++;
    else out.push([1, d]);
  }
  return out;
}
function track(id, kind, timescale, duration, movieDuration, samples, offset, entry, width = 0, height = 0) {
  const deltas = samples.map((s) => s.delta), stts = runs(deltas);
  const sync = kind === 'video' ? samples.map((s, i) => (s.key ? i + 1 : 0)).filter(Boolean) : null;
  const stbl = box('stbl',
    full('stsd', 0, 0, u(1, 4), entry),
    full('stts', 0, 0, u(stts.length, 4), ...stts.map(([n, d]) => cat([u(n, 4), u(d, 4)]))),
    ...(sync ? [full('stss', 0, 0, u(sync.length, 4), ...sync.map((n) => u(n, 4)))] : []),
    full('stsc', 0, 0, u(1, 4), u(1, 4), u(samples.length, 4), u(1, 4)),
    full('stsz', 0, 0, u(0, 4), u(samples.length, 4), ...samples.map((s) => u(s.data.byteLength, 4))),
    full('stco', 0, 0, u(1, 4), u(offset, 4)));
  const header = kind === 'video' ? full('vmhd', 0, 1, u(0, 8)) : full('smhd', 0, 0, u(0, 4));
  return box('trak',
    full('tkhd', 0, 3, u(0, 4), u(0, 4), u(id, 4), u(0, 4), u(movieDuration, 4), u(0, 8), u(0, 2), u(0, 2),
      u(kind === 'audio' ? 0x100 : 0, 2), u(0, 2), MATRIX, u(width * 65536, 4), u(height * 65536, 4)),
    box('mdia',
      full('mdhd', 0, 0, u(0, 4), u(0, 4), u(timescale, 4), u(duration, 4), u(0x55c4, 2), u(0, 2)),
      full('hdlr', 0, 0, u(0, 4), text(kind === 'video' ? 'vide' : 'soun'), u(0, 12), text(kind === 'video' ? 'Video\0' : 'Sound\0')),
      box('minf', header, box('dinf', full('dref', 0, 0, u(1, 4), full('url ', 0, 1))), stbl)));
}
/** A playable MP4 of [start, end] (microseconds into the snapshot). */
export function mp4(shot, start, end, withAudio) {
  const v = shot.video;
  if (!v.length || !shot.config?.description) throw new Error('Nothing has been recorded yet.');
  const base = v[0].ts;
  // The clip starts on the last key frame at or before the chosen start.
  let first = 0;
  for (let i = 0; i < v.length; i++) if (v[i].key && v[i].ts - base <= start) first = i;
  const picked = v.slice(first).filter((s) => s.ts - base <= end);
  const t0 = picked[0].ts;
  const vt = 90000, at = 48000;
  const video = picked.map((s, i) => ({
    ...s,
    delta: Math.max(1, Math.round((((picked[i + 1]?.ts ?? s.ts + 33_333) - s.ts) * vt) / 1e6)),
  }));
  const tEnd = picked.at(-1).ts + 33_333;
  const audio = withAudio && shot.audioConfig
    ? shot.audio.filter((s) => s.ts >= t0 && s.ts < tEnd).map((s) => ({ ...s, delta: 960 }))
    : [];
  const vDur = video.reduce((n, s) => n + s.delta, 0), aDur = audio.length * 960;
  const movie = Math.round(Math.max(vDur / vt, aDur / at) * 1000);
  const { width, height } = shot.config;
  const avc1 = box('avc1', u(0, 6), u(1, 2), u(0, 2), u(0, 2), u(0, 12), u(width, 2), u(height, 2),
    u(0x480000, 4), u(0x480000, 4), u(0, 4), u(1, 2), u(0, 32), u(0x18, 2), u(0xffff, 2),
    box('avcC', new Uint8Array(shot.config.description)));
  const head = shot.audioConfig?.description ? new Uint8Array(shot.audioConfig.description) : null;
  const channels = head?.[9] ?? 2, preSkip = head ? head[10] | (head[11] << 8) : 312;
  const opus = box('Opus', u(0, 6), u(1, 2), u(0, 8), u(channels, 2), u(16, 2), u(0, 2), u(0, 2), u(48000 * 65536, 4),
    box('dOps', u(0, 1), u(channels, 1), u(preSkip, 2), u(48000, 4), u(0, 2), u(0, 1)));
  const ftyp = box('ftyp', text('isom'), u(512, 4), text('isomiso2avc1mp41'));
  const build = (vOffset, aOffset) => box('moov',
    full('mvhd', 0, 0, u(0, 4), u(0, 4), u(1000, 4), u(movie, 4), u(0x10000, 4), u(0x100, 2), u(0, 10), MATRIX, u(0, 24), u(audio.length ? 3 : 2, 4)),
    track(1, 'video', vt, vDur, movie, video, vOffset, avc1, width, height),
    ...(audio.length ? [track(2, 'audio', at, aDur, movie, audio, aOffset, opus)] : []));
  const size = build(0, 0).byteLength;
  const vOffset = ftyp.byteLength + size + 8;
  const vBytes = video.reduce((n, s) => n + s.data.byteLength, 0);
  const moov = build(vOffset, vOffset + vBytes);
  const payload = [...video.map((s) => s.data), ...audio.map((s) => s.data)];
  const mdatSize = 8 + vBytes + audio.reduce((n, s) => n + s.data.byteLength, 0);
  return new Blob([ftyp, moov, u(mdatSize, 4), text('mdat'), ...payload], { type: 'video/mp4' });
}

// ---- the local library (IndexedDB), bounded -----------------------------------
export const LIBRARY_CLIPS = 50, LIBRARY_BYTES = 1024 * 1024 * 1024;
function db() {
  return new Promise((resolve, reject) => {
    const r = indexedDB.open('sticktalk-clips', 1);
    r.onupgradeneeded = () => r.result.createObjectStore('clips', { keyPath: 'id' });
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
}
async function store(mode, fn) {
  const d = await db();
  try {
    return await new Promise((resolve, reject) => {
      const tx = d.transaction('clips', mode), s = tx.objectStore('clips');
      let result;
      Promise.resolve(fn(s)).then((v) => { result = v; });
      tx.oncomplete = () => resolve(result);
      tx.onerror = () => reject(tx.error);
    });
  } finally { d.close(); }
}
const all = (s) => new Promise((resolve) => { const r = s.getAll(); r.onsuccess = () => resolve(r.result); });
export async function clipsList() {
  const list = await store('readonly', all);
  return list.sort((a, b) => b.created - a.created);
}
async function keep(clip) {
  await store('readwrite', async (s) => {
    s.put(clip);
    // Oldest clips leave first when the library is full.
    const items = (await all(s)).sort((a, b) => a.created - b.created);
    let bytes = items.reduce((n, c) => n + c.size, 0), count = items.length;
    for (const c of items) {
      if (count <= LIBRARY_CLIPS && bytes <= LIBRARY_BYTES) break;
      if (c.id === clip.id) continue;
      s.delete(c.id);
      count--;
      bytes -= c.size;
    }
  });
}
const forget = (id) => store('readwrite', (s) => s.delete(id));
function download(blob, name) {
  const a = element('a', { href: URL.createObjectURL(blob), download: `${name.replace(/[\\/:*?"<>|]+/g, '_').slice(0, 80) || 'clip'}.mp4` });
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 60_000);
}
/** Attach the clip to the call's chat composer, as if it had been chosen. */
async function share(blob, name) {
  const channel = host.channel;
  const file = new File([blob], `${name.replace(/[\\/:*?"<>|]+/g, '_').slice(0, 80) || 'clip'}.mp4`, { type: 'video/mp4' });
  const open = () => document.querySelector(`#live-messages[data-channel="${channel}"]`) && document.querySelector('#live-file');
  if (!open()) window.dispatchEvent(new CustomEvent('sticktalk-live', { detail: `call:chat,${channel}` }));
  for (let i = 0; i < 40 && !open(); i++) await new Promise((r) => requestAnimationFrame(r));
  const picker = open();
  if (!picker) throw new Error('Open the call’s chat, then share the clip again.');
  const files = new DataTransfer();
  files.items.add(file);
  picker.files = files.files;
  picker.dispatchEvent(new Event('change', { bubbles: true }));
}
const MAX_SHARE = 25 * 1024 * 1024;
const time = (us) => `${Math.floor(us / 60e6)}:${String(Math.floor(us / 1e6) % 60).padStart(2, '0')}`;

// ---- Save Clip: trim, sound, name, then download, keep or share --------------------
export async function take() {
  if (!rec?.buffer?.video.length) {
    t.toast(rec ? 'Nothing to clip yet: the stream just started.' : 'Clips are off, or nothing is streaming.', '🎬');
    return;
  }
  const shot = rec.buffer.snapshot();
  host.signal('clip_taken');
  editor(shot);
}
function editor(shot) {
  const d = t.dialog('Save Clip', { className: 'clip-editor' });
  const length = shot.video.at(-1).ts - shot.video[0].ts;
  const preview = element('video', { className: 'clip-preview', controls: true, playsInline: true, muted: true });
  preview.setAttribute('aria-label', 'Clip preview');
  const range = (label, value) => {
    const input = element('input', { type: 'range', min: '0', max: String(Math.round(length / 1e5)), value: String(value) });
    input.setAttribute('aria-label', label);
    return input;
  };
  const start = range('Clip start', 0), end = range('Clip end', Math.round(length / 1e5));
  const readout = element('p', { className: 'muted-copy clip-readout' });
  const sound = element('input', { type: 'checkbox', checked: !!shot.audioConfig, disabled: !shot.audioConfig });
  const name = element('input', { type: 'text', maxLength: 80, value: `Clip ${new Date().toLocaleString()}` });
  const status = element('p', { className: 'muted-copy' });
  status.setAttribute('role', 'status');
  let blob = null, url = null, timer = 0;
  const rebuild = () => {
    clearTimeout(timer);
    timer = setTimeout(() => {
      try {
        const a = Math.min(Number(start.value), Number(end.value) - 10) * 1e5, b = Number(end.value) * 1e5;
        blob = mp4(shot, a, b, sound.checked);
        if (url) URL.revokeObjectURL(url);
        url = URL.createObjectURL(blob);
        preview.src = url;
        readout.textContent = `${time(a)} – ${time(b)} · ${(blob.size / 1048576).toFixed(1)} MB${blob.size > MAX_SHARE ? ' · too large to share in chat: trim it' : ''}`;
        shareButton.disabled = blob.size > MAX_SHARE;
      } catch (e) { status.textContent = e.message; }
    }, 150);
  };
  for (const c of [start, end, sound]) c.addEventListener('input', rebuild);
  const keepButton = t.button('Save to Library', 'button primary', async () => {
    try {
      await keep({ id: crypto.randomUUID(), name: name.value.trim() || 'Clip', created: Date.now(), size: blob.size, duration: Number(end.value - start.value) / 10, blob });
      status.textContent = 'Saved to your clip library on this device.';
    } catch (e) { status.textContent = `Couldn’t save: ${e.message}`; }
  });
  const shareButton = t.button('Share to Chat', 'button secondary', async () => {
    try {
      await share(blob, name.value.trim());
      d.close();
    } catch (e) { status.textContent = e.message; }
  });
  d.addEventListener('close', () => { if (url) URL.revokeObjectURL(url); preview.removeAttribute('src'); });
  d.append(preview,
    element('label', { className: 'field' }, 'Start', start),
    element('label', { className: 'field' }, 'End', end), readout,
    element('label', { className: 'live-checkbox' }, element('span', { textContent: 'Include Call Audio' }), sound),
    element('label', { className: 'field' }, 'Name', name), status,
    element('div', { className: 'clip-actions' }, keepButton,
      t.button('Download', 'button secondary', () => blob && download(blob, name.value.trim())), shareButton,
      t.button('Close', 'button secondary', () => d.close())));
  d.showModal();
  rebuild();
}
export async function library() {
  const d = t.dialog('Clip Library', { className: 'clip-library' });
  const list = element('div', { className: 'clip-list' });
  const note = element('p', { className: 'muted-copy', textContent: `Clips stay on this device only: up to ${LIBRARY_CLIPS} clips or 1 GB, oldest removed first.` });
  d.append(note, list, t.button('Close', 'button secondary', () => d.close()));
  const urls = [];
  d.addEventListener('close', () => urls.forEach((u) => URL.revokeObjectURL(u)));
  const render = async () => {
    let clips = [];
    try { clips = await clipsList(); } catch (e) { list.textContent = e.message; return; }
    list.replaceChildren();
    if (!clips.length) list.append(element('p', { className: 'muted-copy', textContent: 'No clips yet. Turn on clips, then press Save Clip (Alt+C) while watching or sharing a stream.' }));
    for (const c of clips) {
      const url = URL.createObjectURL(c.blob);
      urls.push(url);
      const video = element('video', { src: url, controls: true, playsInline: true, preload: 'metadata', className: 'clip-thumb' });
      video.setAttribute('aria-label', c.name);
      list.append(element('div', { className: 'clip-row' }, video,
        element('div', { className: 'clip-meta' }, element('strong', { textContent: c.name }),
          element('small', { textContent: `${new Date(c.created).toLocaleString()} · ${(c.size / 1048576).toFixed(1)} MB` })),
        element('div', { className: 'clip-actions' },
          t.button('Download', 'button secondary', () => download(c.blob, c.name)),
          ...(host.joined ? [t.button('Share to Chat', 'button secondary', () => share(c.blob, c.name).then(() => d.close()).catch((e) => { note.textContent = e.message; }))] : []),
          t.button('Delete', 'button danger-text', async () => { await forget(c.id); render(); }))));
    }
  };
  d.showModal();
  render();
}
export function settingsDialog() {
  const d = t.dialog('Clips');
  const on = element('input', { type: 'checkbox', checked: t.prefs().clips.enabled });
  const seconds = element('select');
  seconds.setAttribute('aria-label', 'Clip length');
  for (const s of [30, 60, 120]) seconds.add(new Option(`Last ${s} seconds`, String(s)));
  seconds.value = String(t.prefs().clips.seconds);
  const status = element('p', { className: 'muted-copy' });
  status.setAttribute('role', 'status');
  const describe = () => {
    status.textContent = !supported() ? 'This browser cannot record clips. Use Chrome, Edge or the StickTalk Windows app.'
      : rec ? `Keeping the last ${t.prefs().clips.seconds} seconds of ${rec.src.label}. Press Save Clip or Alt+C.`
      : t.prefs().clips.enabled ? 'On. Clips start when someone streams in this call.' : 'Off.';
  };
  on.onchange = () => { t.savePrefs({ clips: { ...t.prefs().clips, enabled: on.checked } }); update(); setTimeout(describe, 300); };
  seconds.onchange = () => { t.savePrefs({ clips: { ...t.prefs().clips, seconds: Number(seconds.value) } }); update(); setTimeout(describe, 300); };
  d.append(element('label', { className: 'live-checkbox' }, element('span', { textContent: 'Keep Clips of Streams' }), on),
    element('label', { className: 'field' }, 'Clip Length', seconds), status,
    element('p', { className: 'muted-copy', textContent: 'The last moments are kept in memory on this device only, and only while a stream is playing. Everyone in the call sees that clips are on, and when one is saved.' }),
    t.button('Save Clip Now', 'button primary', () => { d.close(); take(); }),
    t.button('Clip Library', 'button secondary', () => { d.close(); library(); }),
    t.button('Done', 'button secondary', () => d.close()));
  describe();
  d.showModal();
}
export function diagnostics() {
  return rec?.buffer
    ? { active: true, seconds: rec.buffer.seconds, videoChunks: rec.buffer.video.length, audioChunks: rec.buffer.audio.length, bytes: rec.buffer.bytes, codec: rec.config?.codec }
    : { active: false };
}

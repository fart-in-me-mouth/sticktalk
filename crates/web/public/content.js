// Voice messages: recording (click to start and stop; on a touch screen hold
// to record, slide left to cancel, slide up to lock) and playback (play/pause,
// seek, speed). Loaded only when a voice control is first used. A microphone,
// an AudioContext or a timer exists only while recording; playback uses one
// <audio> element and its own events, never a timer or an animation loop.
const emit = (action) => window.dispatchEvent(new CustomEvent('sticktalk-live', { detail: action }));
const MAX_MS = 20 * 60 * 1000;
const MIN_MS = 1000;
const SPEEDS = [1, 1.5, 2];
const clock = (ms) => { const s = Math.ceil(Math.max(0, ms) / 1000); return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`; };
const sprite = (name) => `<svg class="icon size-5" viewBox="0 0 24 24" aria-hidden="true"><use href="#i-${name}"></use></svg>`;

// ---- Playback -------------------------------------------------------------
let audio = null, playing = null, speed = 1, watcher = null, seekTo = null;
const box = (id) => document.querySelector(`[data-voice="${CSS.escape(id ?? '')}"]`);
function length(el) {
  const fromFile = audio && playing === el?.dataset.voice && Number.isFinite(audio.duration) ? audio.duration * 1000 : 0;
  return fromFile || Number(el?.dataset.duration) || 0;
}
function paint() {
  for (const el of document.querySelectorAll('.voice-message.is-playing')) if (el.dataset.voice !== playing) reset(el);
  const el = box(playing);
  if (!el || !audio) return;
  const total = length(el), now = audio.currentTime * 1000, part = total ? Math.min(1, now / total) : 0;
  el.classList.add('is-playing');
  el.classList.toggle('is-paused', audio.paused);
  const bars = el.querySelectorAll('.voice-bar');
  const lit = Math.round(part * bars.length);
  bars.forEach((bar, i) => bar.classList.toggle('played', i < lit));
  const time = el.querySelector('.voice-time'); if (time) time.textContent = clock(audio.paused && now === 0 ? total : total - now);
  const seek = el.querySelector('.voice-wave');
  if (seek) { seek.setAttribute('aria-valuenow', String(Math.floor(now / 1000))); seek.setAttribute('aria-valuetext', `${clock(now)} of ${clock(total)}`); }
  const button = el.querySelector('.voice-play'); if (button) button.setAttribute('aria-label', audio.paused ? 'Play voice message' : 'Pause voice message');
  el.querySelector('.voice-icon-play')?.classList.toggle('hidden', !audio.paused);
  el.querySelector('.voice-icon-pause')?.classList.toggle('hidden', audio.paused);
  const rate = el.querySelector('.voice-speed'); if (rate) { rate.textContent = `${speed}×`; rate.setAttribute('aria-label', `Playback speed ${speed}×`); }
}
function reset(el) {
  el.classList.remove('is-playing', 'is-paused');
  el.querySelectorAll('.voice-bar.played').forEach((bar) => bar.classList.remove('played'));
  const time = el.querySelector('.voice-time'); if (time) time.textContent = clock(Number(el.dataset.duration));
  el.querySelector('.voice-icon-play')?.classList.remove('hidden');
  el.querySelector('.voice-icon-pause')?.classList.add('hidden');
  el.querySelector('.voice-play')?.setAttribute('aria-label', 'Play voice message');
}
function stopPlayback() {
  const was = box(playing);
  if (audio) { audio.pause(); audio.removeAttribute('src'); audio.load(); }
  audio = null; playing = null; seekTo = null;
  watcher?.disconnect(); watcher = null;
  if (was) reset(was);
}
function play(el, at) {
  const id = el.dataset.voice;
  if (playing === id && audio) {
    if (at !== undefined) { const total = length(el); if (total) audio.currentTime = at * total / 1000; if (audio.paused) void audio.play().catch(fail); }
    else if (audio.paused) void audio.play().catch(fail); else audio.pause();
    paint();
    return;
  }
  stopPlayback();
  stopRecording(false);
  playing = id;
  audio = new Audio(el.dataset.src);
  audio.preload = 'auto';
  audio.playbackRate = speed;
  seekTo = at ?? null;
  for (const type of ['timeupdate', 'play', 'pause', 'seeked']) audio.addEventListener(type, paint);
  audio.addEventListener('loadedmetadata', () => { if (seekTo !== null && Number.isFinite(audio.duration)) audio.currentTime = seekTo * audio.duration; seekTo = null; paint(); });
  audio.addEventListener('ended', stopPlayback);
  audio.addEventListener('error', () => { stopPlayback(); emit('voice-error:That voice message could not be played.'); });
  // The app re-renders its view; repaint the playing message only while one plays.
  watcher = new MutationObserver(() => { if (!box(playing)?.classList.contains('is-playing')) paint(); });
  const root = document.getElementById('sticktalk');
  if (root) watcher.observe(root, { childList: true, subtree: true });
  void audio.play().catch(fail);
  paint();
}
function fail(e) { if (e?.name !== 'AbortError') { stopPlayback(); emit('voice-error:That voice message could not be played.'); } }

// ---- Recording ------------------------------------------------------------
let rec = null, lastPress = 0;
function panel() {
  const composer = document.querySelector('.composer');
  let el = document.getElementById('voice-recorder');
  if (!el) {
    el = document.createElement('div');
    el.id = 'voice-recorder';
    el.className = 'voice-recorder fixed z-[60] flex items-center gap-3 rounded-lg border border-primary bg-popover px-3 text-sm shadow-lg';
    el.setAttribute('role', 'group');
    el.setAttribute('aria-label', 'Recording a voice message');
    el.innerHTML = `<span class="rec-dot size-2.5 flex-none rounded-full bg-destructive" aria-hidden="true"></span><span class="rec-time w-12 flex-none font-semibold tabular-nums" role="timer" aria-live="off">0:00</span><span class="rec-hint min-w-0 flex-1 truncate text-subtle"></span><button type="button" class="icon-button rec-cancel" aria-label="Cancel recording" title="Cancel">${sprite('trash-2')}</button><button type="button" class="icon-button rec-send text-primary" aria-label="Send voice message" title="Send">${sprite('send')}</button>`;
    el.querySelector('.rec-cancel').addEventListener('click', () => stopRecording(false));
    el.querySelector('.rec-send').addEventListener('click', () => stopRecording(true));
    document.body.append(el);
  }
  if (composer) {
    const r = composer.getBoundingClientRect();
    Object.assign(el.style, { left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: `${Math.max(48, r.height)}px` });
  }
  const hold = rec?.mode === 'hold' && !rec.locked;
  el.querySelector('.rec-hint').textContent = hold ? '‹ Slide to cancel · slide up to lock' : 'Recording… Send or cancel';
  el.querySelector('.rec-send').hidden = hold;
  el.querySelector('.rec-cancel').hidden = hold;
  el.dataset.mode = hold ? 'hold' : 'locked';
  return el;
}
function tick() {
  if (!rec?.analyser) return;
  rec.analyser.getByteTimeDomainData(rec.buffer);
  let sum = 0;
  for (const v of rec.buffer) { const x = (v - 128) / 128; sum += x * x; }
  const level = Math.min(255, Math.round(Math.sqrt(Math.sqrt(sum / rec.buffer.length)) * 255 * 1.2));
  if (rec.samples.length < MAX_MS / 100) rec.samples.push(level);
  const elapsed = performance.now() - rec.started;
  const time = document.querySelector('#voice-recorder .rec-time'); if (time) time.textContent = clock(elapsed);
  if (elapsed >= MAX_MS) stopRecording(true);
}
async function startRecording(mode) {
  if (rec) return;
  if (!navigator.mediaDevices?.getUserMedia || !window.MediaRecorder) { emit('voice-error:This browser cannot record voice messages.'); return; }
  stopPlayback();
  rec = { mode, locked: mode === 'click', cancelled: false, pending: true, samples: [], chunks: [] };
  const mine = rec;
  let stream;
  try {
    stream = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true, channelCount: 1 } });
  } catch {
    if (rec === mine) rec = null;
    emit('voice-error:Allow microphone access to record a voice message.');
    return;
  }
  // Released, cancelled or replaced while the browser was asking.
  if (rec !== mine || mine.cancelled) { stream.getTracks().forEach((t) => t.stop()); if (rec === mine) rec = null; return; }
  const type = ['audio/ogg;codecs=opus', 'audio/webm;codecs=opus'].find((t) => MediaRecorder.isTypeSupported(t)) ?? '';
  try {
    mine.recorder = new MediaRecorder(stream, type ? { mimeType: type, audioBitsPerSecond: 48000 } : {});
  } catch {
    stream.getTracks().forEach((t) => t.stop()); rec = null;
    emit('voice-error:This browser cannot record voice messages.');
    return;
  }
  mine.stream = stream;
  mine.type = mine.recorder.mimeType || type || 'audio/webm';
  mine.recorder.addEventListener('dataavailable', (e) => { if (e.data.size) mine.chunks.push(e.data); });
  mine.context = new AudioContext();
  const source = mine.context.createMediaStreamSource(stream);
  mine.analyser = mine.context.createAnalyser();
  mine.analyser.fftSize = 1024;
  mine.buffer = new Uint8Array(mine.analyser.fftSize);
  source.connect(mine.analyser);
  mine.recorder.start(1000);
  mine.started = performance.now();
  mine.pending = false;
  mine.timer = setInterval(tick, 100);
  mine.escape = (e) => { if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); stopRecording(false); } };
  mine.resize = () => panel();
  addEventListener('keydown', mine.escape, true);
  addEventListener('resize', mine.resize);
  panel();
  document.querySelector('.voice-record')?.classList.add('is-recording');
}
function stopRecording(send) {
  const r = rec;
  if (!r) return;
  rec = null;
  r.cancelled = true;
  clearInterval(r.timer);
  removeEventListener('keydown', r.escape, true);
  removeEventListener('resize', r.resize);
  window.removeEventListener('pointermove', r.move);
  window.removeEventListener('pointerup', r.up);
  window.removeEventListener('pointercancel', r.cancel);
  document.getElementById('voice-recorder')?.remove();
  document.querySelector('.voice-record')?.classList.remove('is-recording');
  const elapsed = r.started ? performance.now() - r.started : 0;
  const finish = () => {
    r.stream?.getTracks().forEach((t) => t.stop());
    void r.context?.close();
    if (!send) return;
    if (elapsed < MIN_MS) { emit(`voice-error:${r.mode === 'hold' ? 'Hold the microphone to record, then let go to send.' : 'That recording was too short.'}`); return; }
    deliver(new Blob(r.chunks, { type: r.type }), Math.min(MAX_MS, Math.round(elapsed)), r.samples, r.type);
  };
  if (r.recorder && r.recorder.state !== 'inactive') { r.recorder.addEventListener('stop', finish, { once: true }); r.recorder.stop(); }
  else finish();
}
function deliver(blob, duration, samples, type) {
  const n = Math.min(256, samples.length) || 1;
  const wave = [];
  for (let i = 0; i < n; i++) {
    const part = samples.slice(Math.floor(i * samples.length / n), Math.max(Math.floor((i + 1) * samples.length / n), Math.floor(i * samples.length / n) + 1));
    wave.push(Math.max(1, ...part, 0));
  }
  const ext = type.includes('ogg') ? 'ogg' : type.includes('mp4') ? 'm4a' : 'webm';
  const file = new File([blob], `voice-message-${Date.now()}.${ext}`, { type: type.split(';')[0] });
  const input = document.getElementById('live-voice-file');
  if (!input) return;
  const files = new DataTransfer(); files.items.add(file);
  input.files = files.files;
  input.dataset.duration = String(duration);
  input.dataset.waveform = wave.join(',');
  emit('voice-upload');
}

// ---- Entry points (from live.js) -------------------------------------------
/** A press that may become a hold-to-record gesture (touch and pen). */
export function press(el, e) {
  if (el.dataset.content !== 'voice-record' || e.pointerType === 'mouse') return;
  lastPress = performance.now();
  if (rec) { if (rec.locked) stopRecording(true); return; }
  const start = { x: e.clientX, y: e.clientY, id: e.pointerId };
  void startRecording('hold');
  const r = rec;
  if (!r) return;
  r.move = (m) => {
    if (m.pointerId !== start.id || rec !== r || r.locked) return;
    if (start.x - m.clientX > 96) stopRecording(false);
    else if (start.y - m.clientY > 72) { r.locked = true; panel(); }
  };
  r.up = (u) => { if (u.pointerId === start.id && rec === r && !r.locked) stopRecording(true); };
  r.cancel = (c) => { if (c.pointerId === start.id && rec === r && !r.locked) stopRecording(false); };
  window.addEventListener('pointermove', r.move);
  window.addEventListener('pointerup', r.up);
  window.addEventListener('pointercancel', r.cancel);
}
export function click(el, e) {
  const kind = el.dataset.content;
  if (kind === 'voice-record') {
    if (performance.now() - lastPress < 800) return;
    if (rec) stopRecording(true); else void startRecording('click');
    return;
  }
  const message = el.closest('[data-voice]');
  if (!message) return;
  if (kind === 'voice-play') play(message);
  else if (kind === 'voice-seek') {
    const r = el.getBoundingClientRect();
    play(message, Math.min(1, Math.max(0, (e.clientX - r.left) / Math.max(1, r.width))));
  } else if (kind === 'voice-speed') {
    speed = SPEEDS[(SPEEDS.indexOf(speed) + 1) % SPEEDS.length];
    if (audio) audio.playbackRate = speed;
    el.textContent = `${speed}×`;
    el.setAttribute('aria-label', `Playback speed ${speed}×`);
    paint();
  }
}
/** Arrow keys, Home and End on a waveform seek by 5 seconds or to an end. */
export function key(el, e) {
  const message = el.closest('[data-voice]');
  if (!message) return;
  const total = length(message);
  const now = playing === message.dataset.voice && audio ? audio.currentTime * 1000 : 0;
  const next = e.key === 'Home' ? 0 : e.key === 'End' ? total - 100 : now + (e.key === 'ArrowLeft' ? -5000 : 5000);
  play(message, total ? Math.min(1, Math.max(0, next / total)) : 0);
}
export function action(name) {
  if (name === 'stop') { stopPlayback(); stopRecording(false); }
}

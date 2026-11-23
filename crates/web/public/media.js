// Loaded only when a user joins a call or opens Voice & Video settings. One SFU
// peer connection; no mesh, external ICE service, or media work while disconnected.
import * as ptt from './ptt.js';
import { ringtoneEnabled, setRingtoneEnabled } from './calls.js';
import * as extras from './callextras.js';
import * as stageList from './stage.js';
const emit = (action) =>
  window.dispatchEvent(new CustomEvent("sticktalk-live", { detail: action }));
const signal = (type, data) =>
  emit(
    `signal:${JSON.stringify(data === undefined ? { type } : { type, data })}`,
  );
const $ = (s) => document.querySelector(s);
let pc,
  local,
  joined = false,
  epoch = 0,
  muted = false,
  deafened = false,
  output = "",
  input = "",
  cameraDevice = "",
  levelsTimer,
  deviceListener = false,
  joinSounds = true;
let idleReceivers = [];
// End-to-end encryption (e2ee.js): loaded when the server welcomes this device
// into a call that can be encrypted; its events run in order.
let e2ee = null, e2eeLoading = null, e2eeQueue = Promise.resolve(), e2eeReady = null;
// The chosen microphone, camera and speaker, remembered on this device.
const DEVICES = 'sticktalk-call-devices';
try {
  const stored = JSON.parse(localStorage.getItem(DEVICES) ?? '{}');
  const id = (value) => (typeof value === 'string' && value.length <= 512 ? value : '');
  input = id(stored.input);
  output = id(stored.output);
  cameraDevice = id(stored.camera);
  joinSounds = stored.joinSound !== false;
} catch { /* Storage may be disabled in a private browser. */ }
function rememberDevices() {
  try {
    localStorage.setItem(DEVICES, JSON.stringify({ input, output, camera: cameraDevice, joinSound: joinSounds }));
  } catch {}
}
// What the browser calls the connection, in words for people.
const CALL_STATES = {
  new: 'Connecting…',
  connecting: 'Connecting…',
  connected: 'Voice Connected',
  disconnected: 'Reconnecting…',
  failed: 'Connection failed',
  closed: 'Disconnected',
};
let publications = new Map(),
  senders = new Map(),
  participants = new Map(),
  subscriptions = new Map(),
  elements = new Map(),
  pendingAnswer,
  queue = Promise.resolve(),
  status = "Disconnected";
const trace = [];
let wantCall=false;
// A video call, or a call answered with video: the camera starts once connected.
let wantCamera=false;
// A Stage's audience opens no microphone and runs no input processing: it
// only listens, through receive slots negotiated up front, so a new speaker
// never makes the whole audience renegotiate (stage.md).
let listenOnly = false, pendingListen = false;
const LISTEN_SLOTS = 4;
let callChannel = '', focusedTile = null;
const requests=new Map();
let nextRequest=crypto.getRandomValues(new Uint32Array(1))[0]||1;
function requestSignal(type,data) {
  if(requests.size>=32)return Promise.reject(new Error('Too many pending media operations.'));
  while(requests.has(String(nextRequest)))nextRequest=(nextRequest+1)>>>0||1;
  const id=String(nextRequest);nextRequest=(nextRequest+1)>>>0||1;
  return new Promise((resolve,reject)=>{
    const timer=setTimeout(()=>{requests.delete(id);reject(new Error('Media subscription timed out.'));},5000);
    requests.set(id,{resolve:()=>{clearTimeout(timer);requests.delete(id);resolve();},reject:error=>{clearTimeout(timer);requests.delete(id);reject(error);}});
    emit(`signal-request:${id}:${JSON.stringify({type,data})}`);
  });
}
const speaking = new Set();
// Listening volume per person and per stream, remembered on this device only.
// Entries are [`${userId}:voice|stream`, {volume, muted}]; defaults are not stored.
const VOLUMES = 'sticktalk-call-volumes';
export function normalizeVolumes(value) {
  if (!Array.isArray(value)) return [];
  return value
    .filter((row) => Array.isArray(row) && /^\d{1,20}:(voice|stream)$/.test(row[0]))
    .slice(-64)
    .map(([id, v]) => {
      const n = Number(v?.volume);
      return [id, { volume: Number.isFinite(n) ? Math.min(2, Math.max(0, n)) : 1, muted: v?.muted === true }];
    });
}
let volumes = new Map();
try {
  const raw = localStorage.getItem(VOLUMES);
  if (raw && raw.length <= 8192) volumes = new Map(normalizeVolumes(JSON.parse(raw)));
} catch { /* Storage may be disabled in a private browser. */ }
const volumeKey = (p, kind) =>
  `${p.session.user_id}:${kind === 'screen' || kind === 'system_audio' ? 'stream' : 'voice'}`;
const volumeOf = (id) => ({ ...(volumes.get(id) ?? { volume: 1, muted: false }) });
function setVolume(id, next) {
  volumes.delete(id);
  if (next.muted || next.volume !== 1) volumes.set(id, next);
  while (volumes.size > 64) volumes.delete(volumes.keys().next().value);
  try { localStorage.setItem(VOLUMES, JSON.stringify([...volumes])); } catch {}
  for (const e of elements.values()) if (e.volumeKey === id) applyAudio(e);
}
// Louder than 100%: a media element cannot go above its source, so that part
// plays through a gain node instead (and the element itself stays silent).
let audioContext;
function boost(entry, gain) {
  const stream = entry.media.srcObject;
  if (!gain || !stream) {
    entry.boost?.source.disconnect();
    entry.boost?.node.disconnect();
    entry.boost = null;
    restAudio();
    return;
  }
  wakeAudio().catch(() => {});
  if (!entry.boost || entry.boost.stream !== stream) {
    entry.boost?.source.disconnect();
    entry.boost?.node.disconnect();
    const source = audioContext.createMediaStreamSource(stream), node = audioContext.createGain();
    source.connect(node).connect(audioContext.destination);
    entry.boost = { stream, source, node };
  }
  entry.boost.node.gain.value = gain;
}
function discard(id) {
  const entry = elements.get(id);
  if (!entry) return;
  if (entry.media === pip) leavePip();
  boost(entry, 0);
  entry.tile.remove();
  elements.delete(id);
}
// The audio engine runs only while a boosted voice or a chime is playing.
// The browser applies resume and suspend in the order asked, so the last one
// stands; `awake` is what was asked last (the reported state lags behind).
let awake = false, chiming = 0, chimes = 0, chimeAt = 0;
function wakeAudio() {
  if (!audioContext) {
    audioContext = new AudioContext();
    // The same speaker the rest of the call plays through.
    if (output) audioContext.setSinkId?.(output).catch(() => {});
  }
  awake = true;
  return audioContext.resume();
}
function restAudio() {
  if (!awake || chiming) return;
  for (const e of elements.values()) if (e.boost) return;
  awake = false;
  audioContext.suspend().catch(() => {});
}
// Someone joined your call: two soft rising notes, made here instead of
// downloaded. People arriving together make one sound.
async function joinSound() {
  if (!joinSounds || !globalThis.AudioContext || performance.now() - chimeAt < 600) return;
  chimeAt = performance.now();
  chiming++;
  try {
    // A browser that will not start audio yet stays silent instead of chiming late.
    await Promise.race([wakeAudio(), new Promise((_, fail) => setTimeout(fail, 1000))]);
    const context = audioContext, t = context.currentTime + 0.02;
    let last;
    for (const [hz, at] of [[587.33, 0], [880, 0.11]]) {
      const tone = context.createOscillator(), level = context.createGain();
      tone.frequency.value = hz;
      level.gain.setValueAtTime(0, t + at);
      level.gain.linearRampToValueAtTime(0.14, t + at + 0.012);
      level.gain.exponentialRampToValueAtTime(0.0001, t + at + 0.32);
      tone.connect(level).connect(context.destination);
      tone.onended = () => level.disconnect();
      tone.start(t + at);
      tone.stop(t + at + 0.34);
      last = tone;
    }
    chimes++;
    await new Promise((done) => last.addEventListener('ended', done));
  } catch { /* No sound is better than a late one. */ }
  chiming--;
  restAudio();
}
// The one place that decides what a tile's media element plays.
function applyAudio(entry) {
  const v = volumeOf(entry.volumeKey);
  const video = entry.media.tagName === 'VIDEO';
  // A browser that cannot send a gain node to the chosen speaker stops at 100%.
  const limit = !output || globalThis.AudioContext?.prototype.setSinkId ? 2 : 1;
  if (entry.volume) entry.volume.slider.max = String(limit * 100);
  if (v.volume > limit) v.volume = limit;
  const boosted = !video && v.volume > 1 && !deafened && !v.muted;
  entry.media.muted = video || deafened || v.muted || boosted;
  if (!video) {
    entry.media.volume = Math.min(1, v.volume);
    boost(entry, boosted ? v.volume : 0);
  }
  if (!entry.volume) return;
  const silent = v.muted || v.volume === 0;
  entry.volume.icon.setAttribute('href', silent ? '#i-volume-x' : '#i-volume-2');
  entry.volume.button.setAttribute('aria-pressed', String(v.muted));
  entry.volume.button.setAttribute('aria-label', `${v.muted ? 'Unmute' : 'Mute'} ${entry.volume.name}`);
  const percent = Math.round(v.volume * 100);
  entry.volume.slider.value = String(percent);
  entry.volume.slider.setAttribute('aria-valuetext', `${percent}%`);
  entry.volume.readout.textContent = `${percent}%`;
}
function volumeControl(id, name) {
  const root = document.createElement('div');
  root.className = 'tile-volume';
  // The tile itself is a button that focuses the tile.
  for (const type of ['click', 'keydown', 'pointerdown'])
    root.addEventListener(type, (e) => e.stopPropagation());
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'icon-button';
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.classList.add('icon');
  svg.setAttribute('aria-hidden', 'true');
  const icon = document.createElementNS(svg.namespaceURI, 'use');
  svg.append(icon);
  button.append(svg);
  button.onclick = () => {
    const v = volumeOf(id);
    // Unmuting a slider left at zero would stay silent.
    setVolume(id, { volume: v.muted && v.volume === 0 ? 1 : v.volume, muted: !v.muted });
  };
  const slider = document.createElement('input');
  slider.type = 'range';
  slider.min = '0';
  slider.max = '200';
  slider.setAttribute('aria-label', `${name} volume`);
  slider.oninput = () => setVolume(id, { volume: Number(slider.value) / 100, muted: false });
  const readout = document.createElement('span');
  readout.className = 'tile-volume-value';
  readout.setAttribute('aria-hidden', 'true');
  root.append(button, slider, readout);
  return { root, button, icon, slider, readout, name };
}
export function displayAudio(supported) {
  // Raw capture for music and games. Where the browser can, leave out what this
  // page itself plays (the other people in the call), so viewers never hear
  // themselves echoed back through the stream.
  return {
    echoCancellation: false,
    noiseSuppression: false,
    autoGainControl: false,
    ...(supported?.restrictOwnAudio ? { restrictOwnAudio: true } : {}),
  };
}
// A shared screen is sent at 30 frames a second, in the sharpest picture the
// connection and the encoder can carry at that rate. There is nothing to choose.
export const STREAM_FPS = 30;
export function displayVideo() {
  // The source's own size and shape. 3840×2160 is as large as H.264 encoders and
  // decoders commonly go; a larger screen is captured scaled to fit inside it.
  return {
    frameRate: { ideal: STREAM_FPS, max: STREAM_FPS },
    width: { max: 3840 },
    height: { max: 2160 },
  };
}
// When the full picture cannot be carried, the browser sends it smaller instead
// of dropping frames, and returns to full size by itself when there is room.
// `bps` is the SFU's ceiling for the track, when it has sent one.
export function streamEncoding(params, bps) {
  params.degradationPreference = "maintain-framerate";
  for (const e of params.encodings ?? []) {
    e.maxFramerate = STREAM_FPS;
    if (bps) e.maxBitrate = bps;
  }
  return params;
}
export function retainSpeaking(states, sessionKey, active) {
  if (!active) states.delete(sessionKey);
  else if (states.size < 64 || states.has(sessionKey)) states.add(sessionKey);
}
const record = (type, data) => {
  trace.push({ type, data });
  if (trace.length > 64) trace.shift();
};
const self = () => $("#sticktalk")?.dataset.selfId;
const key = (s) => `${s.user_id}:${s.connection_id}`;
function report(error) {
  record("error", error.message ?? String(error));
  emit(`media-error:${error.message ?? error}`);
}
function enqueue(task) {
  const generation = epoch;
  queue = queue
    .then(() => {
      if (generation === epoch) return task();
    })
    .catch((error) => {
      if (error.message !== "Call ended.") report(error);
    });
  return queue;
}
export function h264Codecs(codecs) {
  // The SFU does not transcode. Prefer constrained baseline, packetization-mode 1.
  const h264 = codecs.filter(
    (c) =>
      c.mimeType.toLowerCase() === "video/h264" &&
      /packetization-mode=1/.test(c.sdpFmtpLine ?? ""),
  );
  h264.sort(
    (a, b) =>
      Number(/profile-level-id=42/.test(b.sdpFmtpLine)) -
      Number(/profile-level-id=42/.test(a.sdpFmtpLine)),
  );
  if (!h264.length) throw Error("This browser does not support H.264 video.");
  // RTX is a repair codec, not an alternative video encoder. Filtering it out
  // disables both NACK repair and str0m's bandwidth-probe padding. An audio-only
  // TWCC estimate can then strand a new camera at its minimum bitrate: the IDR
  // queue expires, and without probes there is no feedback to recover capacity.
  return [...h264, ...codecs.filter(c => c.mimeType.toLowerCase() === 'video/rtx')];
}
function preferH264(transceiver) {
  transceiver.setCodecPreferences(h264Codecs(RTCRtpReceiver.getCapabilities('video')?.codecs ?? []));
}
function preferOpus(transceiver) {
  // Match the SFU's Opus-only audio contract; do not offer RED/G.722/DTMF
  // codecs whose extra fmtp mappings differ across browser versions.
  const codecs=RTCRtpReceiver.getCapabilities('audio')?.codecs.filter(c=>c.mimeType.toLowerCase()==='audio/opus')??[];
  if(!codecs.length)throw Error('This browser does not support Opus audio.');
  transceiver.setCodecPreferences(codecs);
}
async function negotiate(beforeRemote) {
  const current = pc;
  if (!current || !joined) return;
  const offer = await current.createOffer();
  await current.setLocalDescription(offer);
  // Bundle complete local candidates. Avoid signaling ICE before the first offer.
  if (current.iceGatheringState !== "complete")
    await new Promise((resolve) => {
      const timer = setTimeout(done, 2000);
      function done() {
        clearTimeout(timer);
        current.removeEventListener("icegatheringstatechange", check);
        resolve();
      }
      function check() {
        if (current.iceGatheringState === "complete") done();
      }
      current.addEventListener("icegatheringstatechange", check);
    });
  if (current !== pc) return;
  const answer = new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      pendingAnswer = null;
      reject(Error("Media negotiation timed out."));
    }, 10000);
    pendingAnswer = {
      resolve: (sdp) => {
        clearTimeout(timer);
        resolve(sdp);
      },
      reject: () => {
        clearTimeout(timer);
        reject(Error("Call ended."));
      },
    };
  });
  const tracks = [...publications.values()]
    .filter((p) => !p.track.ended && p.transceiver.direction !== "inactive")
    .map((p) => ({
      mid: p.transceiver.mid,
      kind: p.kind,
      codec: p.track.kind === "video" ? "h264" : null,
      layers: [],
      surface:
        p.kind === "screen"
          ? p.track.getSettings().displaySurface === "monitor"
            ? "monitor"
            : "window"
          : null,
    }));
  signal("offer", { sdp: normalizeOffer(current.localDescription.sdp), tracks });
  const sdp = await answer;
  if (current !== pc) return;
  // Subscribe before enabling a new receiver. Otherwise the SFU's temporary
  // answer stream can bind Chromium to an SSRC which Subscribe discards.
  await beforeRemote?.();
  if (current !== pc) return;
  // The SFU allocates a fresh forwarding SSRC *after* answering Subscribe.
  // Do not pin Chromium's video receiver to the answer's discarded default SSRC.
  // Its generated send-stream MSIDs also need the wildcard semantic declaration.
  const browserSdp = extras.answer(normalizeAnswer(sdp));
  await current.setRemoteDescription({ type: "answer", sdp: browserSdp });
  bindRemote();
}
export function normalizeOffer(sdp) {
  // Chromium gathers TCP; Firefox also gathers component 2 (separate RTCP),
  // even with rtcp-mux required. The SFU is UDP + RTCP-mux only. Keep component
  // 1 UDP candidates; leave local SDP, fingerprints and codec fields unchanged.
  return sdp.split(/\r?\n/).filter(line=>{
    if(!line.startsWith('a=candidate:'))return true;
    const fields=line.split(/\s+/);
    return fields[1]==='1'&&fields[2]?.toLowerCase()==='udp';
  }).join('\r\n');
}
export function normalizeAnswer(sdp) {
  return sdp
    .split(/\r?\n/)
    .filter(
      (line) =>
        !line.startsWith("a=ssrc:") && !line.startsWith("a=ssrc-group:"),
    )
    .map((line) =>
      line.startsWith("a=msid-semantic:") ? "a=msid-semantic: WMS *" : line,
    )
    .join("\r\n");
}
function bindRemote() {
  for (const s of subscriptions.values()) {
    const entry = elements.get(s.key),
      track = s.transceiver.receiver.track;
    if (!entry || !track) continue;
    const old = entry.media.srcObject?.getTracks()[0];
    if (old !== track) entry.media.srcObject = new MediaStream([track]);
    applyAudio(entry);
    if (entry.media.paused) entry.media.play().catch(() => {});
  }
}
// Push to talk: the microphone's sender carries the track only while the key is
// held, so nothing at all is sent otherwise (not silence). Voice Activity always carries it.
function gateAudio() {
  const p = publications.get("audio");
  if (!p) return Promise.resolve();
  const want = ptt.transmitting() ? p.track : null;
  if (p.transceiver.sender.track === want) return Promise.resolve();
  return p.transceiver.sender.replaceTrack(want).catch(report);
}
async function publish(track, kind, stream) {
  let transceiver = senders.get(kind);
  if (transceiver) {
    await transceiver.sender.replaceTrack(track);
    transceiver.direction = "sendonly";
  } else {
    transceiver = pc.addTransceiver(track, {
      direction: "sendonly",
      streams: [stream],
    });
    senders.set(kind, transceiver);
  }
  if (track.kind === "video") preferH264(transceiver);
  else preferOpus(transceiver);
  e2ee?.sender(transceiver.sender, kind);
  if (kind === "screen") {
    // Chromium treats a captured screen as text, which it never sends smaller:
    // on a slow connection the frame rate falls instead. "motion" is the opposite.
    track.contentHint = "motion";
    try {
      const sender = transceiver.sender;
      await sender.setParameters(streamEncoding(sender.getParameters()));
    } catch {
      // Not before the first negotiation in every browser. The SFU's first
      // bitrate for the track sets the same parameters.
    }
  }
  publications.set(kind, { kind, track, stream, transceiver });
  return transceiver;
}
// The microphone: capture (with echo cancellation, noise suppression and
// gain control), publication, push to talk and your own tile.
async function microphone(generation) {
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: {
      echoCancellation: true,
      noiseSuppression: true,
      autoGainControl: true,
      ...(input ? { deviceId: { ideal: input } } : {}),
    },
    video: false,
  });
  if (generation !== epoch) {
    stream.getTracks().forEach((t) => t.stop());
    return false;
  }
  local = stream;
  await publish(stream.getAudioTracks()[0], "audio", stream);
  ptt.attach(() => { gateAudio(); }, KEY_USAGE);
  await gateAudio();
  elements.set(
    "self-audio",
    createTile(
      "self-audio",
      { session: { user_id: self(), connection_id: "self" } },
      "audio",
    ),
  );
  applyState();
  return true;
}
// On a Stage: from the audience onto the Stage. Only now is the microphone opened.
async function speak() {
  if (!pc || !joined || !listenOnly) { listenOnly = false; return; }
  listenOnly = false;
  try {
    if (!(await microphone(epoch))) return;
  } catch (e) {
    listenOnly = true;
    if (e.name === 'NotAllowedError') throw new Error('Microphone access was denied. Allow it in your browser’s site settings to speak.');
    throw e;
  }
  await negotiate();
  if (!levelsTimer) levelsTimer = setTimeout(levels, 200);
  mount();
}
// On a Stage: back to the audience. Every capture stops and nothing is sent.
async function quiet() {
  if (listenOnly) return;
  listenOnly = true;
  if (!pc || !joined) return;
  ptt.detach();
  clearTimeout(levelsTimer);
  levelsTimer = null;
  for (const [kind, p] of [...publications]) {
    p.stream?.getTracks().forEach((t) => t.stop());
    p.track.stop();
    await p.transceiver.sender.replaceTrack(null).catch(() => {});
    p.transceiver.direction = "inactive";
    publications.delete(kind);
    if (kind === "camera") extras.cameraStopped();
  }
  local?.getTracks().forEach((t) => t.stop());
  local = null;
  for (const id of [...elements.keys()]) if (id.startsWith("self-")) discard(id);
  await negotiate();
  mount();
}
async function start(data) {
  stop(false, true);
  if (!globalThis.RTCPeerConnection || !navigator.mediaDevices?.getUserMedia) {
    signal('leave');
    wantCall = false;
    throw Error('Voice and video are unavailable in this browser. Use Safari, Chrome or Firefox with media support.');
  }
  joined = true;
  const generation = ++epoch;
  if (e2eeReady) await e2eeReady.catch(report);
  if (generation !== epoch) return;
  participants = new Map(data.participants.map((p) => [key(p.session), p]));
  pc = new RTCPeerConnection({
    iceServers: [],
    bundlePolicy: "max-bundle",
    rtcpMuxPolicy: "require",
    ...(e2ee?.pcConfig() ?? {}),
  });
  const current = pc;
  current.oniceconnectionstatechange = () =>
    record("ice", current.iceConnectionState);
  current.onicecandidateerror = (e) => record("ice-error", e.errorText);
  current.onconnectionstatechange = () => {
    record("connection", current.connectionState);
    status = CALL_STATES[current.connectionState] ?? "Connecting…";
    if (current.connectionState === "failed")
      report("Media connection failed. Leave and rejoin.");
    mount();
  };
  current.ontrack = (e) => {
    const subscription = [...subscriptions.values()].find(
      (s) =>
        s.transceiver === e.transceiver ||
        (s.transceiver.mid !== null && s.transceiver.mid === e.transceiver.mid),
    );
    if (!subscription) return;
    const id = subscription.key;
    let entry = elements.get(id);
    if (!entry) {
      entry = createTile(id, subscription.participant, subscription.kind);
      elements.set(id, entry);
    }
    const stream = new MediaStream([e.track]);
    entry.media.srcObject = stream;
    applyAudio(entry);
    entry.media.play().catch(() => {
      entry.blocked = true;
      describe(entry);
      entry.tile.onclick = () =>
        entry.media.play().then(() => { entry.blocked = false; describe(entry); }).catch(report);
    });
    // A remembered speaker that is no longer plugged in falls back to the default.
    if (output && entry.media.setSinkId)
      entry.media.setSinkId(output).catch(() => {});
    mount();
  };
  pendingListen = false;
  try {
    if (listenOnly) {
      for (let i = 0; i < LISTEN_SLOTS; i++) {
        const slot = current.addTransceiver("audio", { direction: "recvonly" });
        preferOpus(slot);
        idleReceivers.push(slot);
      }
      applyState();
    } else if (!(await microphone(generation))) return;
    await negotiate();
    for (const participant of participants.values())
      await updateParticipant(participant);
    if (!listenOnly) levelsTimer = setTimeout(levels, 200);
    if (wantCamera) {
      wantCamera = false;
      if (!publications.has("camera")) enqueue(toggleCamera);
    }
    // You are in: an earlier "microphone denied" notice no longer applies.
    if ($('.live-error')?.textContent.includes('Microphone')) emit('dismiss-error');
    mount();
  } catch (e) {
    stop(true);
    if(e.name==='NotAllowedError')throw new Error('Microphone access was denied. Allow it in your browser’s site settings, then rejoin.');
    throw e;
  }
}
// Names and pictures come from the call screen's own list (the Rust view), and
// from the member list for anyone that list does not have yet.
function person(userId) {
  const row = document.querySelector(`#live-call-people [data-user="${userId}"]`);
  return {
    name:
      row?.dataset.name ??
      $(`[data-member-name="${userId}"]`)?.textContent ??
      (userId === self() ? "You" : "Participant"),
    avatar: row?.dataset.avatar ?? "",
    tag: row?.dataset.tag ?? "",
  };
}
function glyph(name) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.classList.add("icon");
  svg.setAttribute("aria-hidden", "true");
  const use = document.createElementNS(svg.namespaceURI, "use");
  use.setAttribute("href", `#i-${name}`);
  svg.append(use);
  return svg;
}
function createTile(id, p, kind) {
  const tile = document.createElement("div");
  tile.className = "participant-tile";
  tile.tabIndex = 0;
  tile.setAttribute('role', 'button');
  let zoomGestureUntil = 0;
  const focus = () => {
    if (performance.now() < zoomGestureUntil) return;
    focusedTile = focusedTile === id ? null : id;
    mount();
  };
  tile.addEventListener('click', focus);
  tile.addEventListener('keydown', e => {
    if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); focus(); }
  });
  tile.dataset.participant = key(p.session);
  tile.classList.toggle('is-speaking', kind !== 'screen' && speaking.has(key(p.session)));
  tile.dataset.kind = kind;
  if (p.session.user_id === self()) tile.dataset.self = "true";
  const label = document.createElement("div");
  label.className = "participant-label";
  const name = document.createElement("span");
  const state = document.createElement("span");
  state.className = "participant-state";
  label.append(name, state);
  const media = document.createElement(
    kind === "audio" || kind === "system_audio" ? "audio" : "video",
  );
  media.autoplay = true;
  media.playsInline = true;
  // Set these before attaching a stream: iOS requires inline, muted video for
  // gesture-free remote video playback (audio remains a separate media track).
  if (media.tagName === 'VIDEO') {
    media.setAttribute('playsinline', '');
    media.muted = true;
  }
  if (media.tagName === "VIDEO") media.className = "call-video";
  else {
    const placeholder = document.createElement("span");
    placeholder.className = "call-avatar";
    tile.append(placeholder);
  }
  tile.append(media, label);
  if (kind === 'screen') {
    bindScreenZoom(tile, media, () => { zoomGestureUntil = performance.now() + 500; });
    const live = document.createElement('span');
    live.className = 'tile-live';
    live.textContent = 'LIVE';
    tile.append(live);
    if (tile.requestFullscreen) {
      const full = document.createElement('button');
      full.type = 'button';
      full.className = 'icon-button tile-fullscreen';
      full.setAttribute('aria-label', 'Full Screen');
      full.title = 'Full Screen';
      full.append(glyph('maximize'));
      for (const type of ['click', 'keydown', 'pointerdown'])
        full.addEventListener(type, (e) => e.stopPropagation());
      full.onclick = () => {
        if (document.fullscreenElement === tile) document.exitFullscreen();
        else tile.requestFullscreen().catch(() => {});
      };
      tile.append(full);
    }
  }
  // Someone else's video can move to a window of its own.
  if (media.tagName === 'VIDEO' && p.session.user_id !== self() && canPopOut()) {
    const pop = document.createElement('button');
    pop.type = 'button';
    pop.className = 'icon-button tile-popout';
    pop.setAttribute('aria-label', 'Pop Out');
    pop.title = 'Pop Out';
    pop.append(glyph('external-link'));
    for (const type of ['click', 'keydown', 'pointerdown'])
      pop.addEventListener(type, (e) => e.stopPropagation());
    pop.onclick = () => popOut(id).catch(report);
    tile.append(pop);
  }
  const entry = { tile, label, name, state, media, user: p.session.user_id, kind, volumeKey: volumeKey(p, kind) };
  if (kind === 'screen' && p.session.user_id !== self()) {
    entry.control = document.createElement('button');
    entry.control.type = 'button';
    entry.control.className = 'button secondary tile-control';
    for (const type of ['click', 'keydown', 'pointerdown'])
      entry.control.addEventListener(type, (e) => e.stopPropagation());
    entry.control.onclick = () => {
      if (controlling?.tile === entry) stopControl();
      else enqueue(() => requestControl(entry));
    };
    tile.append(entry.control);
    bindControl(entry);
  }
  describe(entry);
  // A stream's sound arrives as its own track; its control sits on the screen tile.
  if (p.session.user_id !== self() && kind !== 'system_audio') {
    entry.volume = volumeControl(entry.volumeKey, kind === 'screen' ? `${entry.who}'s stream` : entry.who);
    tile.append(entry.volume.root);
  }
  applyAudio(entry);
  return entry;
}
// A tile's name, picture and mute state, kept current as the call changes.
function describe(entry) {
  const who = person(entry.user), mine = entry.user === self();
  entry.who = who.name;
  const text =
    (entry.kind !== "screen" ? who.name : mine ? "Your screen" : `${who.name}'s screen`) +
    (entry.blocked ? " · Tap to play" : "");
  if (entry.name.textContent !== text) entry.name.textContent = text;
  // The server tag the person wears (identity.css draws it after the name).
  if ((entry.name.dataset.tag ?? "") !== who.tag) entry.name.dataset.tag = who.tag;
  entry.tile.setAttribute("aria-label", `Focus ${text}`);
  const avatar = entry.tile.querySelector(".call-avatar");
  if (avatar && who.avatar) {
    let img = avatar.querySelector("img");
    if (!img) {
      img = document.createElement("img");
      img.alt = "";
      // A picture that cannot load leaves the initials.
      img.onerror = () => { avatar.textContent = who.name.slice(0, 2); };
      avatar.replaceChildren(img);
    }
    if (img.getAttribute("src") !== who.avatar) img.src = who.avatar;
  } else if (avatar && !avatar.querySelector("img")) avatar.textContent = who.name.slice(0, 2);
  // Muted or deafened, as that person (or you) last set it.
  const other = participants.get(entry.tile.dataset.participant);
  const off =
    entry.kind === "screen" || entry.kind === "system_audio" ? ""
      : (mine ? deafened : other?.deafened) ? "headphone-off"
      : (mine ? muted : other?.muted) ? "mic-off"
      : "";
  if (entry.state.dataset.icon !== off) {
    entry.state.dataset.icon = off;
    entry.state.replaceChildren(...(off ? [glyph(off)] : []));
    if (off) entry.state.setAttribute("aria-label", off === "mic-off" ? "Muted" : "Deafened");
    else entry.state.removeAttribute("aria-label");
  }
  if (entry.control) {
    const mine = controlling?.tile === entry, active = mine && controlling.state === "active";
    const text = active ? "Stop Controlling" : mine ? "Asking…" : "Request Control";
    if (entry.control.textContent !== text) entry.control.textContent = text;
    entry.control.disabled = (mine && !active) || (!mine && !!controlling);
    entry.tile.classList.toggle("is-controlling", active);
  }
}
// In-app fullscreen preserves playsinline on iPhone. Pinch/pan only intercepts
// the focused screen tile; ordinary call scrolling and controls stay native.
export function bindScreenZoom(tile, video, onGesture = () => {}) {
  const points = new Map();
  let scale = 1, x = 0, y = 0, start;
  const snapshot = () => {
    const p = [...points.values()];
    return { scale, x, y, cx: p.reduce((n, v) => n + v.x, 0) / p.length,
      cy: p.reduce((n, v) => n + v.y, 0) / p.length,
      distance: p.length === 2 ? Math.hypot(p[0].x - p[1].x, p[0].y - p[1].y) : 0 };
  };
  video.addEventListener('pointerdown', e => {
    if (!tile.classList.contains('is-focused') || e.pointerType !== 'touch' || points.size >= 2) return;
    video.setPointerCapture(e.pointerId);
    points.set(e.pointerId, { x: e.clientX, y: e.clientY }); start = snapshot();
  });
  video.addEventListener('pointermove', e => {
    if (!points.has(e.pointerId)) return;
    points.set(e.pointerId, { x: e.clientX, y: e.clientY });
    const next = snapshot();
    if (points.size === 2 && start.distance) {
      scale = Math.max(1, Math.min(4, start.scale * next.distance / start.distance));
      x = start.x + next.cx - start.cx; y = start.y + next.cy - start.cy;
      onGesture();
    } else if (scale > 1) { x = start.x + next.cx - start.cx; y = start.y + next.cy - start.cy; onGesture(); }
    const bounds = video.getBoundingClientRect();
    x = Math.max(-bounds.width / 2, Math.min(bounds.width / 2, x));
    y = Math.max(-bounds.height / 2, Math.min(bounds.height / 2, y));
    video.style.transform = `translate(${x}px,${y}px) scale(${scale})`;
    video.dataset.zoom = String(scale);
  });
  const release = e => { points.delete(e.pointerId); start = points.size ? snapshot() : undefined; };
  for (const event of ['pointerup', 'pointercancel', 'lostpointercapture']) video.addEventListener(event, release);
  tile.resetZoom = () => { points.clear(); start = undefined; scale = 1; x = y = 0; video.style.transform = ''; delete video.dataset.zoom; };
}
async function updateParticipant(participant) {
  if (!pc || participant.session.user_id === self()) return;
  // Not for the people who were already here when you joined.
  if (!participants.has(key(participant.session))) joinSound();
  participants.set(key(participant.session), participant);
  const active = new Set(
    participant.tracks.map((t) => `${key(participant.session)}:${t.mid}`),
  );
  for (const [id, s] of subscriptions) {
    if (
      key(s.participant.session) === key(participant.session) &&
      !active.has(id)
    ) {
      signal("unsubscribe", { receiver_mid: s.transceiver.mid });
      if (!listenOnly) s.transceiver.direction = "inactive";
      idleReceivers.push(s.transceiver);
      subscriptions.delete(id);
      discard(id);
    }
  }
  const added = [];
  for (const track of participant.tracks) {
    const id = `${key(participant.session)}:${track.mid}`;
    if (subscriptions.has(id)) continue;
    if (subscriptions.size >= 28) break;
    const kind =
      track.kind === "audio" || track.kind === "system_audio"
        ? "audio"
        : "video";
    const reusable = idleReceivers.findIndex(
      (t) => t.receiver.track.kind === kind,
    );
    const transceiver =
      reusable < 0
        ? pc.addTransceiver(kind, { direction: "recvonly" })
        : idleReceivers.splice(reusable, 1)[0];
    // A slot already negotiated as recvonly needs no new offer (a Stage's audience).
    const ready = listenOnly && reusable >= 0 && transceiver.mid !== null &&
      transceiver.currentDirection === "recvonly";
    transceiver.direction = "recvonly";
    if (track.codec) preferH264(transceiver);
    else preferOpus(transceiver);
    e2ee?.receiver(transceiver, participant.session, track.kind);
    const subscription = {
      key: id,
      participant,
      kind: track.kind,
      transceiver,
      source_mid: track.mid,
      ready,
    };
    subscriptions.set(id, subscription);
    added.push(subscription);
    // Someone started sharing: show the stream large unless you chose another tile.
    if (track.kind === "screen" && !focusedTile) focusedTile = id;
    if (!elements.has(id)) {
      const entry = createTile(id, participant, track.kind);
      entry.media.srcObject = new MediaStream([transceiver.receiver.track]);
      applyAudio(entry);
      entry.media.play().catch(() => {});
      elements.set(id, entry);
    }
  }
  if (added.length) {
    const subscribe = () => Promise.all(added.map(s=>requestSignal("subscribe", {
        publisher: participant.session,
        source_mid: s.source_mid,
        receiver_mid: s.transceiver.mid,
        rid: null,
      })));
    if (added.every((s) => s.ready)) {
      await subscribe();
      bindRemote();
    } else await negotiate(subscribe);
  }
  mount();
}
export function event(event) {
  const data = event.data;
  record("event", event.type === "answer" ? { type: "answer" } : event);
  const request=requests.get(String(event.request_nonce));
  if(request&&event.type==='ack'){request.resolve();return;}
  if(request&&event.type==='error'){request.reject(new Error(data.message??'Media subscription failed.'));return;}
  switch (event.type) {
    case "joined":
      if(wantCall)enqueue(() => start(data));
      break;
    case "answer":
      pendingAnswer?.resolve(data.sdp);
      pendingAnswer = null;
      break;
    case "participant":
      if (joined) enqueue(() => updateParticipant(data));
      break;
    case "left": {
      const prefix = key(data.session);
      speaking.delete(prefix);
      participants.delete(prefix);
      if (controlling && key(controlling.sharer) === prefix) endControlling();
      if (controlled && key(controlled.requester) === prefix) endControlled();
      if (agentControl && [agentControl.sharer, agentControl.controller].some((s) => key(s) === prefix)) endAgentControl();
      for (const [id, s] of subscriptions)
        if (key(s.participant.session) === prefix) {
          signal("unsubscribe", { receiver_mid: s.transceiver.mid });
          if (!listenOnly) s.transceiver.direction = "inactive";
          idleReceivers.push(s.transceiver);
          subscriptions.delete(id);
          discard(id);
        }
      mount();
      break;
    }
    case "speaking":
      retainSpeaking(speaking, key(data.session), data.speaking);
      extras.speaking(data);
      for (const entry of elements.values())
        if (entry.tile.dataset.participant === key(data.session) && entry.kind !== "screen")
          entry.tile.classList.toggle("is-speaking", data.speaking);
      break;
    case "bitrate": {
      // setLocal/RemoteDescription invalidates sender parameter transactions in
      // Chromium and Firefox. Serialize with negotiation, and acquire a fresh
      // transaction only after its answer is applied (never during renegotiation).
      enqueue(async () => {
        const p = [...publications.values()].find(
          (p) => p.transceiver.mid === data.mid,
        );
        if (!p) return;
        const params = p.transceiver.sender.getParameters();
        if (!params.encodings?.length) return;
        if (p.kind === "screen") streamEncoding(params, Number(data.bps));
        else for (const e of params.encodings) e.maxBitrate = extras.videoCap(p, Number(data.bps));
        await p.transceiver.sender.setParameters(params);
      });
      break;
    }
    case "error":
      if(!wantCall&&!joined)break; // Late signaling acknowledgments after Leave.
      report(data.message ?? "The server rejected a media operation.");
      pendingAnswer?.reject(new Error(data.message ?? 'Media operation failed.'));
      pendingAnswer = null;
      // Refused from (or removed from) a call this app cannot encrypt.
      if (data.code === "unsupported_feature" && /end-to-end/.test(data.message ?? "") && !listenOnly) stop(true);
      break;
    case "e2ee":
      if (data?.type === "welcome" && wantCall && !listenOnly)
        e2eeLoading ??= import("./e2ee.js").then((m) => { m.bind(e2eeHost); return (e2ee = m); });
      if (e2eeLoading) {
        const run = (e2eeQueue = e2eeQueue.then(() => e2eeLoading).then((m) => m.event(event)).catch(report));
        if (data?.type === "welcome") e2eeReady = run;
      }
      break;
    case "control_requested":
      controlRequested(data);
      break;
    case "control_denied":
      if (controlling?.state === "asking" && key(data.sharer) === key(controlling.sharer)) {
        endControlling();
        report(new Error("They declined remote control."));
      }
      break;
    case "control_granted":
      if (controlling && key(data.sharer) === key(controlling.sharer) && data.controller.user_id === self()) {
        controlling.state = "active";
        controlling.grant = BigInt(data.grant);
        focusedTile = [...elements].find(([, e]) => e === controlling.tile)?.[0] ?? focusedTile;
        mount();
        controlling.tile.tile.focus({ preventScroll: true });
      } else if (controlled && !controlled.pending && data.sharer.user_id === self() &&
                 key(data.controller) === key(controlled.requester)) {
        clearTimeout(controlled.timer);
        controlled.grant = String(data.grant);
        // An agent sees through pictures taken here: have the newest at hand.
        if (controlled.agent) watchFrames();
        mount();
      } else if (data.agent && data.controller.user_id === self()) {
        // Your own AI agent was allowed: say so here, with a way to stop it.
        agentControl = { sharer: data.sharer, controller: data.controller, grant: String(data.grant) };
        mount();
      }
      break;
    case "control_revoked":
      if (controlling && key(data.sharer) === key(controlling.sharer)) endControlling();
      if (controlled && data.sharer.user_id === self()) endControlled();
      if (agentControl && key(data.sharer) === key(agentControl.sharer) && String(data.grant) === agentControl.grant) endAgentControl();
      break;
    case "guild_left":
      if (data.session.user_id === self() && joined) stop(false);
      break;
    default:
      extras.event(event);
      break;
  }
  if (event.type === "left" || event.type === "participant") extras.event(event);
}
function applyState() {
  // Your microphone only: a shared screen keeps its sound while you are muted.
  const voice = publications.get("audio");
  if (voice) voice.track.enabled = !muted && !deafened;
  extras.deafen(deafened);
  // Text-to-speech stays quiet while you are deafened (speech.js).
  globalThis.__sticktalkDeafened = deafened;
  for (const e of elements.values()) applyAudio(e);
  signal("state", { muted, deafened });
  mount();
}
async function toggleCamera() {
  if (!pc) throw Error("Join a voice channel first.");
  const old = publications.get("camera");
  if (old) {
    old.track.stop();
    extras.cameraStopped();
    old.transceiver.direction = "inactive";
    publications.delete("camera");
    elements.get("self-camera")?.tile.remove();
    elements.delete("self-camera");
    await negotiate();
    mount();
    return;
  }
  const generation = epoch;
  const raw = await navigator.mediaDevices.getUserMedia({
    audio: false,
    video: {
      ...extras.cameraConstraints(),
      ...(cameraDevice ? { deviceId: { ideal: cameraDevice } } : {}),
    },
  });
  if (generation !== epoch) {
    raw.getTracks().forEach((t) => t.stop());
    return;
  }
  // With a background effect on, the published picture is the processed one.
  const stream = await extras.camera(raw);
  if (generation !== epoch) {
    stream.getTracks().forEach((t) => t.stop());
    extras.cameraStopped();
    return;
  }
  await publish(stream.getVideoTracks()[0], "camera", stream);
  const tile = createTile(
    "self-camera",
    { session: { user_id: self(), connection_id: "self" } },
    "camera",
  );
  tile.media.srcObject = stream;
  tile.media.muted = true;
  tile.media.play().catch(() => {});
  elements.set("self-camera", tile);
  await negotiate();
  extras.tune("camera");
  mount();
}
// A new picture for the published camera (a background turned on or off).
async function replaceCamera(stream) {
  const p = publications.get("camera");
  if (!p) return false;
  const track = stream.getVideoTracks()[0];
  await p.transceiver.sender.replaceTrack(track);
  p.track = track;
  p.stream = stream;
  const tile = elements.get("self-camera");
  if (tile) {
    tile.media.srcObject = stream;
    tile.media.play().catch(() => {});
  }
  return true;
}
async function share() {
  if (!pc) throw Error("Join a voice channel first.");
  if (innerWidth < 768 || !navigator.mediaDevices.getDisplayMedia)
    throw Error(
      "Screen sharing is available in supported desktop browsers. Phones can watch streams.",
    );
  if (publications.has("screen")) await enqueue(unshare);
  else showShareOptions();
}
// A stream is its picture and, when captured, its sound: withdraw both.
async function unshare() {
  if (controlled) {
    signal("revoke_control");
    endControlled();
  }
  for (const kind of ["screen", "system_audio"]) {
    const p = publications.get(kind);
    if (p) {
      p.track.stop();
      p.transceiver.direction = "inactive";
      publications.delete(kind);
    }
  }
  elements.get("self-screen")?.tile.remove();
  elements.delete("self-screen");
  dispatchEvent(new CustomEvent('sticktalk-sharing', { detail: false }));
  await negotiate();
  extras.streamsChanged();
  mount();
}
function showShareOptions() {
  const dialog = document.createElement("dialog");
  dialog.className = "live-share-options";
  dialog.setAttribute("aria-labelledby", "live-share-title");
  // Escape and Cancel both leave nothing behind, and do not close the page under it.
  dialog.addEventListener("close", () => dialog.remove());
  dialog.addEventListener("keydown", (e) => e.stopPropagation());
  const title = document.createElement("h2");
  title.id = "live-share-title";
  title.textContent = "Share Screen";
  const note = document.createElement("p");
  note.className = "muted-copy";
  note.textContent =
    "Next, choose what to share. To share its sound too, turn on the audio option in that window.";
  const quality = document.createElement("p");
  quality.className = "muted-copy";
  quality.textContent = `Your stream runs at ${STREAM_FPS} fps, as sharp as your connection allows.`;
  const start = document.createElement("button");
  start.textContent = "Share";
  start.className = "button primary";
  start.onclick = async () => {
    // getDisplayMedia is invoked inside this user gesture, not after negotiation.
    const capture = navigator.mediaDevices.getDisplayMedia({
      video: displayVideo(),
      audio: displayAudio(navigator.mediaDevices.getSupportedConstraints?.()),
    });
    dialog.close();
    try {
      const stream = await capture;
      if (!pc) {
        stream.getTracks().forEach((t) => t.stop());
        return;
      }
      const track = stream.getVideoTracks()[0];
      await publish(track, "screen", stream);
      // What you are sharing, so you can see that you are and what others see.
      const preview = createTile(
        "self-screen",
        { session: { user_id: self(), connection_id: "self" } },
        "screen",
      );
      preview.media.srcObject = stream;
      preview.media.play().catch(() => {});
      elements.get("self-screen")?.tile.remove();
      elements.set("self-screen", preview);
      mount();
      for (const audio of stream.getAudioTracks())
        await publish(audio, "system_audio", stream);
      track.onended = () => enqueue(unshare);
      // Streamer Mode can turn on while you share (preferences.js).
      dispatchEvent(new CustomEvent('sticktalk-sharing', { detail: true }));
      // In the Windows app the shared screen can be controlled, with your consent.
      if (host()) controlChannelReady();
      enqueue(negotiate);
      extras.streamsChanged();
    } catch (e) {
      if (e.name !== "NotAllowedError") report(e);
    }
  };
  const cancel = document.createElement("button");
  cancel.className = "button secondary";
  cancel.textContent = "Cancel";
  cancel.onclick = () => dialog.close();
  dialog.append(title, note, quality, start, cancel);
  document.body.append(dialog);
  dialog.showModal();
}
// ---- remote control of a shared screen ---------------------------------------
// The viewer sends pointer and key input over one ordered data channel. The
// server checks the grant and passes it to the sharer, whose StickTalk Windows
// app (never a plain browser) asks them first and then performs it.
const CONTROL = "sticktalk-control-v1";
const host = () =>
  globalThis.__sticktalkHost?.control && globalThis.ipc?.postMessage ? globalThis.ipc : null;
const tellHost = (message) => host()?.postMessage(JSON.stringify(message));
// An app that can also be controlled by a person's AI agent (see below).
const hostTakesAgents = () => Number(globalThis.__sticktalkHost?.control) >= 2;
let controlChannel = null;
// As the viewer: { sharer, tile, state: "asking" | "active", grant, sequence, held, buttons }.
let controlling = null;
// As the sharer: { requester, name, agent, pending, grant }. `agent` means the
// requester's AI agent holds the control, through the server.
let controlled = null;
// As the person whose AI agent controls someone's screen: { sharer, controller, grant }.
let agentControl = null;

// MessagePack, only as much as a control message needs: maps with text keys,
// text, whole numbers and booleans. Field names and shapes are proto's ControlData.
export function packControl(value) {
  const out = [];
  const put = (v) => {
    if (typeof v === "boolean") out.push(v ? 0xc3 : 0xc2);
    else if (typeof v === "string") {
      const bytes = new TextEncoder().encode(v);
      if (bytes.length < 32) out.push(0xa0 | bytes.length);
      else out.push(0xd9, bytes.length);
      out.push(...bytes);
    } else if (typeof v === "bigint" || Number.isInteger(v)) {
      const n = BigInt(v);
      const big = (width, from) => {
        for (let shift = BigInt(width - 8); shift >= 0n; shift -= 8n) out.push(Number((from >> shift) & 255n));
      };
      if (n < -32768n) throw Error("number out of range");
      if (n < -32n) { out.push(0xd1); big(16, BigInt.asUintN(16, n)); }
      else if (n < 0n) out.push(Number(BigInt.asUintN(8, n)));
      else if (n < 128n) out.push(Number(n));
      else if (n < 256n) out.push(0xcc, Number(n));
      else if (n < 65536n) { out.push(0xcd); big(16, n); }
      else if (n < 4294967296n) { out.push(0xce); big(32, n); }
      else { out.push(0xcf); big(64, BigInt.asUintN(64, n)); }
    } else {
      const keys = Object.keys(v);
      out.push(0x80 | keys.length);
      for (const k of keys) { put(k); put(v[k]); }
    }
  };
  put(value);
  return new Uint8Array(out);
}
// Whole numbers too large for a JavaScript number come back as decimal text.
export function unpackControl(bytes) {
  if (bytes.length > 512) throw Error("too long");
  let at = 0;
  const take = (n) => {
    if (at + n > bytes.length) throw Error("truncated");
    at += n;
    return bytes.subarray(at - n, at);
  };
  const whole = (n, signed) => {
    let v = 0n;
    for (const b of take(n)) v = (v << 8n) | BigInt(b);
    if (signed) v = BigInt.asIntN(n * 8, v);
    return v >= BigInt(Number.MIN_SAFE_INTEGER) && v <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(v) : v.toString();
  };
  const text = (n) => new TextDecoder("utf-8", { fatal: true }).decode(take(n));
  const read = (depth) => {
    if (depth > 8) throw Error("too deep");
    const t = take(1)[0];
    if (t < 0x80) return t;
    if (t >= 0xe0) return t - 256;
    if (t >= 0xa0 && t < 0xc0) return text(t & 31);
    if ((t & 0xf0) === 0x80 || t === 0xde) {
      const count = t === 0xde ? whole(2) : t & 15;
      const map = {};
      for (let i = 0; i < count; i++) {
        const k = read(depth + 1);
        if (typeof k !== "string" || k === "__proto__") throw Error("bad key");
        map[k] = read(depth + 1);
      }
      return map;
    }
    switch (t) {
      case 0xc0: return null;
      case 0xc2: return false;
      case 0xc3: return true;
      case 0xcc: return whole(1);
      case 0xcd: return whole(2);
      case 0xce: return whole(4);
      case 0xcf: return whole(8);
      case 0xd0: return whole(1, true);
      case 0xd1: return whole(2, true);
      case 0xd2: return whole(4, true);
      case 0xd3: return whole(8, true);
      case 0xd9: return text(whole(1));
      case 0xda: return text(whole(2));
      default: throw Error("unsupported");
    }
  };
  const value = read(0);
  if (at !== bytes.length) throw Error("trailing bytes");
  return value;
}
// USB HID keyboard usages by physical key (KeyboardEvent.code). The Windows key
// is left out on purpose, and the sharer's app refuses it as well.
export const KEY_USAGE = (() => {
  const usage = {
    Enter: 40, Escape: 41, Backspace: 42, Tab: 43, Space: 44, Minus: 45, Equal: 46,
    BracketLeft: 47, BracketRight: 48, Backslash: 49, Semicolon: 51, Quote: 52, Backquote: 53,
    Comma: 54, Period: 55, Slash: 56, CapsLock: 57, Insert: 73, Home: 74, PageUp: 75,
    Delete: 76, End: 77, PageDown: 78, ArrowRight: 79, ArrowLeft: 80, ArrowDown: 81, ArrowUp: 82,
    ControlLeft: 0xe0, ShiftLeft: 0xe1, AltLeft: 0xe2, ControlRight: 0xe4, ShiftRight: 0xe5, AltRight: 0xe6,
  };
  [..."ABCDEFGHIJKLMNOPQRSTUVWXYZ"].forEach((c, i) => (usage[`Key${c}`] = 4 + i));
  [..."1234567890"].forEach((c, i) => (usage[`Digit${c}`] = 30 + i));
  for (let i = 1; i <= 12; i++) usage[`F${i}`] = 57 + i;
  return usage;
})();
// Where a pointer is inside the shared picture (not its letterbox), 0..65535.
export function controlPoint(box, width, height, x, y) {
  if (!(width > 0 && height > 0 && box.width > 0 && box.height > 0)) return null;
  const scale = Math.min(box.width / width, box.height / height);
  const w = width * scale, h = height * scale;
  const px = (x - box.left - (box.width - w) / 2) / w, py = (y - box.top - (box.height - h) / 2) / h;
  if (px < 0 || px > 1 || py < 0 || py > 1) return null;
  return { x: Math.round(px * 65535), y: Math.round(py * 65535) };
}
// True when the channel was just created and the call must renegotiate for it.
function controlChannelReady() {
  if (controlChannel && controlChannel.readyState !== "closed") return false;
  controlChannel = pc.createDataChannel(CONTROL, { ordered: true });
  controlChannel.binaryType = "arraybuffer";
  controlChannel.onmessage = (e) => receiveControl(e.data);
  return true;
}
function controlChannelOpen() {
  const channel = controlChannel;
  if (channel.readyState === "open") return Promise.resolve();
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(Error("timed out")), 8000);
    channel.addEventListener("open", () => { clearTimeout(timer); resolve(); }, { once: true });
  });
}
async function requestControl(entry) {
  const p = participants.get(entry.tile.dataset.participant);
  if (!pc || !p || controlling) return;
  controlling = { sharer: p.session, tile: entry, state: "asking", sequence: 0, held: new Set(), buttons: new Set() };
  mount();
  try {
    if (controlChannelReady()) await negotiate();
    await controlChannelOpen();
    await requestSignal("request_control", { sharer: p.session });
  } catch {
    endControlling();
    throw Error(`Couldn’t ask to control this screen. ${entry.who} needs to be sharing a screen or window from the StickTalk Windows app, and only one person can ask at a time.`);
  }
}
function sendInput(input) {
  const c = controlling;
  if (c?.state !== "active" || controlChannel?.readyState !== "open") return;
  // Never fall behind: a late key-up is a stuck key on someone else's PC. Give
  // control back without sending anything more; the server tells the other
  // side to let go of everything.
  if (controlChannel.bufferedAmount > 65536) {
    controlling = null;
    if (pc && joined) signal("revoke_control");
    mount();
    return;
  }
  controlChannel.send(packControl({
    type: "input",
    data: {
      sharer: { user_id: BigInt(c.sharer.user_id), connection_id: BigInt(c.sharer.connection_id) },
      grant: c.grant,
      sequence: ++c.sequence,
      input,
    },
  }));
}
function releaseHeld() {
  const c = controlling;
  if (c?.state !== "active") return;
  for (const usage of c.held) sendInput({ type: "key", data: { usage, down: false } });
  for (const button of c.buttons) sendInput({ type: "button", data: { button, down: false } });
  c.held.clear();
  c.buttons.clear();
}
function endControlling() {
  if (!controlling) return;
  releaseHeld();
  controlling = null;
  mount();
}
function endControlled() {
  if (!controlled) return;
  clearTimeout(controlled.timer);
  tellHost({ control: "stop" });
  controlled = null;
  unwatchFrames();
  mount();
}
function endAgentControl() {
  if (!agentControl) return;
  agentControl = null;
  mount();
}
// Either side ends it: the viewer gives control back (or takes it from their
// own AI agent), or the sharer takes it back.
function stopControl() {
  if (!controlling && !controlled && !agentControl) return;
  if (pc && joined) signal("revoke_control");
  endControlling();
  endControlled();
  endAgentControl();
}
function bindControl(entry) {
  const video = entry.media, tile = entry.tile;
  const active = () =>
    controlling?.state === "active" && controlling.tile === entry && tile.classList.contains("is-focused");
  const point = (e) => controlPoint(video.getBoundingClientRect(), video.videoWidth, video.videoHeight, e.clientX, e.clientY);
  let pending = null, frame = 0;
  video.addEventListener("pointermove", (e) => {
    if (!active() || e.pointerType === "touch") return;
    pending = point(e);
    // At most one move per frame.
    frame ||= requestAnimationFrame(() => {
      frame = 0;
      if (pending && active()) sendInput({ type: "move", data: pending });
      pending = null;
    });
  });
  const press = (e, down) => {
    if (!active() || e.pointerType === "touch") return;
    // Browser buttons are left, middle, right; the protocol's are left, right, middle.
    const button = [0, 2, 1, 3, 4][e.button], at = point(e);
    if (button === undefined || (down && !at)) return;
    e.preventDefault();
    e.stopPropagation();
    if (down) {
      tile.focus({ preventScroll: true });
      video.setPointerCapture?.(e.pointerId);
      controlling.buttons.add(button);
    } else if (!controlling.buttons.delete(button)) return;
    if (at) sendInput({ type: "move", data: at });
    sendInput({ type: "button", data: { button, down } });
  };
  video.addEventListener("pointerdown", (e) => press(e, true));
  video.addEventListener("pointerup", (e) => press(e, false));
  video.addEventListener("wheel", (e) => {
    if (!active()) return;
    e.preventDefault();
    // A mouse notch is about 100 here and 120 there; "down" is the other sign.
    const lines = e.deltaMode === 1 ? 40 : 1;
    const turn = (v) => Math.max(-4096, Math.min(4096, Math.round(v * lines * 1.2)));
    sendInput({ type: "wheel", data: { x: turn(e.deltaX), y: turn(-e.deltaY) } });
  }, { passive: false });
  video.addEventListener("contextmenu", (e) => { if (active()) e.preventDefault(); });
  // A click on the picture is for the other PC, not for un-focusing the tile.
  tile.addEventListener("click", (e) => {
    if (controlling?.tile === entry && controlling.state === "active" && e.target === video) e.stopImmediatePropagation();
  }, true);
  const type = (e, down) => {
    if (!active() || e.target !== tile) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    const usage = KEY_USAGE[e.code];
    if (!usage) return;
    if (down) controlling.held.add(usage);
    else if (!controlling.held.delete(usage)) return;
    sendInput({ type: "key", data: { usage, down } });
  };
  tile.addEventListener("keydown", (e) => type(e, true), true);
  tile.addEventListener("keyup", (e) => type(e, false), true);
  tile.addEventListener("blur", () => { if (controlling?.tile === entry) releaseHeld(); });
}
// As the sharer: only the Windows app can be controlled, and it asks you itself.
function controlRequested(data) {
  const screen = publications.get("screen"), settings = screen?.track.getSettings() ?? {};
  const agent = data.agent === true;
  // An older app would ask as if the person themselves wanted control.
  if (!host() || (agent && !hostTakesAgents()) || !screen || controlled || !["monitor", "window"].includes(settings.displaySurface)) {
    signal("decide_control", { requester: data.requester, allow: false });
    return;
  }
  controlled = { requester: data.requester, name: person(data.requester.user_id).name, agent, pending: true };
  // The server forgets the question after 30 seconds; answer "no" just before.
  controlled.timer = setTimeout(() => hostDecision(false), Math.max(1000, Math.min(Number(data.timeout_ms) || 30000, 30000) - 1500));
  tellHost({
    control: "start",
    who: controlled.name,
    label: screen.track.label,
    surface: settings.displaySurface,
    width: settings.width ?? 0,
    height: settings.height ?? 0,
    agent,
  });
}
function hostDecision(allow) {
  const asked = controlled;
  if (!asked?.pending) return;
  clearTimeout(asked.timer);
  asked.pending = false;
  // "Yes" made the app ready to take input. If the call does not confirm the
  // grant (the request had expired, the answer was lost), take that back.
  const undo = () => { if (controlled === asked && !asked.grant) endControlled(); };
  requestSignal("decide_control", { requester: asked.requester, allow }).catch(undo);
  if (!allow) endControlled();
  else asked.timer = setTimeout(undo, 5000);
}
globalThis.addEventListener?.("sticktalk-host", (e) => {
  const detail = e.detail ?? {};
  if (detail.control === "allowed") hostDecision(true);
  else if (detail.control === "denied") hostDecision(false);
  else if (detail.control === "stopped" && controlled) {
    // The app ended it: Esc on the real keyboard, or input it would not perform.
    if (controlled.pending) hostDecision(false);
    else stopControl();
    if (typeof detail.reason === "string" && detail.reason) report(new Error(detail.reason.slice(0, 200)));
  }
  // The person closed the stream's window from its title bar.
  if (detail.window === "closed" && popout) {
    popout.video.srcObject = null;
    popout = null;
    mount();
  }
});
function receiveControl(data) {
  if (!(data instanceof ArrayBuffer) || !controlled?.grant) return;
  let message;
  try { message = unpackControl(new Uint8Array(data)); } catch { return; }
  if (String(message?.data?.grant) !== controlled.grant) return;
  if (message.type === "release_all") tellHost({ control: "release" });
  else if (message.type === "capture") { if (controlled.agent) sendFrame(message.data); }
  else if (message.type === "input" && String(message.data.sharer?.user_id) === self())
    tellHost({ control: "input", input: message.data.input });
}
// ---- pictures for an AI agent ------------------------------------------------
// An agent that controls this screen sees it through pictures taken here, when
// the server asks, from the very track being shared. The newest frame is kept
// at hand while that lasts: this page may be hidden behind a game, where a
// <video> would stand still. Nothing runs once control has ended.
let frames = null; // { track, reader, latest }
function watchFrames() {
  const source = publications.get("screen")?.track;
  if (frames || !source || typeof MediaStreamTrackProcessor !== "function") return;
  const track = source.clone();
  const reader = new MediaStreamTrackProcessor({ track, maxBufferSize: 1 }).readable.getReader();
  const watch = (frames = { track, reader, latest: null });
  (async () => {
    try {
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        if (frames !== watch) { value.close(); break; }
        watch.latest?.close();
        watch.latest = value;
      }
    } catch { /* the share ended */ }
  })();
}
function unwatchFrames() {
  const watch = frames;
  if (!watch) return;
  frames = null;
  watch.reader.cancel().catch(() => {});
  watch.track.stop();
  watch.latest?.close();
  watch.latest = null;
}
// The size a picture is sent at: its longer side at most `max` pixels.
export function frameSize(width, height, max) {
  const scale = Math.min(1, max / Math.max(width, height));
  return { width: Math.max(1, Math.round(width * scale)), height: Math.max(1, Math.round(height * scale)) };
}
async function sendFrame(request) {
  const asked = controlled, id = request?.id;
  if (!asked?.grant || !Number.isSafeInteger(id) || id < 0) return;
  const max = Math.max(320, Math.min(1920, Number(request.max) || 1280));
  const quality = Math.max(0.1, Math.min(1, (Number(request.quality) || 70) / 100));
  let image = null;
  try {
    watchFrames();
    image = frames?.latest?.clone() ?? null;
    if (!image) {
      // No frame has come by yet (or this engine cannot read the track): take one.
      const track = publications.get("screen")?.track;
      if (!track || typeof ImageCapture !== "function") return;
      image = await Promise.race([
        new ImageCapture(track).grabFrame(),
        new Promise((_, reject) => setTimeout(() => reject(Error("timed out")), 2000)),
      ]);
    }
    const size = frameSize(image.displayWidth ?? image.width, image.displayHeight ?? image.height, max);
    const canvas = new OffscreenCanvas(size.width, size.height);
    canvas.getContext("2d", { alpha: false }).drawImage(image, 0, 0, size.width, size.height);
    const body = await canvas.convertToBlob({ type: "image/jpeg", quality });
    // Control ended while the picture was made: it is no longer theirs to see.
    if (controlled !== asked) return;
    const csrf = document.cookie.split(";").map((v) => v.trim())
      .find((v) => v.startsWith("__Host-sticktalk_csrf="))?.split("=").slice(1).join("=") ?? "";
    await fetch(`/api/v1/voice/control/frames/${id}`, {
      method: "POST",
      credentials: "same-origin",
      cache: "no-store",
      headers: { "Content-Type": "image/jpeg", "X-StickTalk-CSRF": csrf },
      body,
    });
  } catch {
    // The agent is told that no picture arrived; there is nothing to do here.
  } finally {
    image?.close?.();
  }
}
// ---- a stream outside the call view -------------------------------------------
// Away from the call view, the stream stays on screen in the browser's own
// picture-in-picture window, which opens by itself. "Pop Out" moves a stream
// to a separate window instead, one that can be resized and go full screen.
// `pip` is the video this code put there (never one the person chose from the
// browser's menu); `pipDismissed` is set when they close it, until they return;
// `pipFailed` is a video the browser refused, not asked for again by itself.
let pip = null, pipPending = null, pipFailed = null, pipDismissed = false, pipWaiting = false, pipTries = 0;
// The stream in its own window: { id, win, video, name }.
let popout = null, popoutBound = false;
let callButtons = false;
const streamHost = () =>
  globalThis.__sticktalkHost?.windows && globalThis.ipc?.postMessage ? globalThis.ipc : null;
// The Windows app opens the window itself; an older one cannot.
const canPopOut = () => !globalThis.__sticktalkHost || !!streamHost();
// What is shown away from the call view: the tile you focused, else a shared
// screen, else a camera. Never your own picture.
function featured() {
  const videos = [...elements].filter(([, e]) => e.media.tagName === "VIDEO" && !e.tile.dataset.self);
  return (videos.find(([id]) => id === focusedTile) ?? videos.find(([, e]) => e.kind === "screen") ?? videos[0])?.[0] ?? null;
}
function leavePip() {
  const old = pip;
  pip = pipPending = null;
  if (old && document.pictureInPictureElement === old) document.exitPictureInPicture().catch(() => {});
}
// Picture-in-picture needs a click or key press first. Without one (a stream
// that starts while you read a channel), it opens on the next.
function pipOnGesture() {
  if (pipWaiting || ++pipTries > 3) return;
  pipWaiting = true;
  const retry = () => {
    pipWaiting = false;
    removeEventListener("pointerup", retry, true);
    removeEventListener("keydown", retry, true);
    pipFailed = null;
    // After the click itself has been handled: it may be the way back to the call.
    setTimeout(mount, 0);
  };
  addEventListener("pointerup", retry, true);
  addEventListener("keydown", retry, true);
}
function syncPip(away) {
  if (!document.pictureInPictureEnabled) return;
  if (!away) {
    pipDismissed = false;
    pipFailed = null;
    pipTries = 0;
  }
  const entry = away && !pipDismissed && !popout ? elements.get(featured()) : null;
  const media = entry?.media ?? null;
  if (!media) return leavePip();
  if (media === pip || media === pipPending || media === pipFailed) return;
  // A video outside the page cannot be shown, so it waits in a hidden corner.
  if (!entry.tile.isConnected) {
    let park = $("#live-call-park");
    if (!park) {
      park = document.createElement("div");
      park.id = "live-call-park";
      park.className = "call-park";
      park.inert = true;
      document.body.append(park);
    }
    park.append(entry.tile);
  }
  if (!media.readyState) {
    // No picture has arrived yet.
    media.addEventListener("loadedmetadata", mount, { once: true });
    return;
  }
  // From here the window belongs to the new video: the one before leaving it
  // is not the person closing it.
  const previous = pip;
  pip = null;
  pipPending = media;
  media.requestPictureInPicture().then(() => {
    if (pipPending !== media) {
      // Meanwhile the call view came back, or another stream took over.
      if (pip !== media && document.pictureInPictureElement === media) document.exitPictureInPicture().catch(() => {});
      return;
    }
    pipPending = null;
    pip = media;
    media.addEventListener("leavepictureinpicture", () => {
      if (pip !== media) return;
      pip = null;
      pipDismissed = true;
    }, { once: true });
  }, (error) => {
    if (pipPending !== media) return;
    pipPending = null;
    pipFailed = media;
    if (previous && document.pictureInPictureElement === previous) pip = previous;
    if (error.name === "NotAllowedError") pipOnGesture();
  });
}
// Mute and hang-up buttons in the picture-in-picture window, where the browser has them.
function syncCallButtons() {
  const session = navigator.mediaSession;
  if (!session) return;
  if (callButtons !== joined) {
    callButtons = joined;
    for (const [name, command] of [["togglemicrophone", "mute"], ["hangup", "leave"]])
      try { session.setActionHandler(name, joined ? () => action(command) : null); } catch { /* Not offered by this browser. */ }
  }
  if (joined) session.setMicrophoneActive?.(!muted && !deafened)?.catch?.(() => {});
}
function closePopout() {
  const old = popout;
  popout = null;
  if (!old) return;
  old.video.srcObject = null;
  try { old.win.close(); } catch { /* Already gone. */ }
  streamHost()?.postMessage(JSON.stringify({ window: "close" }));
}
// The Windows app makes the window first; the page then opens into it.
function hostWindow() {
  return new Promise((resolve, reject) => {
    const failed = () => reject(new Error("The stream window couldn’t be opened."));
    const done = (e) => {
      const state = e.detail?.window;
      if (state !== "ready" && state !== "failed") return;
      clearTimeout(timer);
      removeEventListener("sticktalk-host", done);
      if (state === "ready") resolve();
      else failed();
    };
    const timer = setTimeout(() => { removeEventListener("sticktalk-host", done); failed(); }, 5000);
    addEventListener("sticktalk-host", done);
    streamHost().postMessage(JSON.stringify({ window: "stream" }));
  });
}
async function popOut(id) {
  const entry = elements.get(id);
  if (!entry || entry.media.tagName !== "VIDEO" || !canPopOut()) return;
  if (popout?.id === id && !popout.win.closed) {
    popout.win.focus();
    return;
  }
  closePopout();
  const generation = epoch;
  if (streamHost()) await hostWindow();
  if (generation !== epoch || elements.get(id) !== entry) {
    streamHost()?.postMessage(JSON.stringify({ window: "close" }));
    return;
  }
  const win = window.open("", "sticktalk-stream", "popup,width=1280,height=720");
  if (!win) {
    streamHost()?.postMessage(JSON.stringify({ window: "close" }));
    throw new Error("The stream window was blocked. Allow pop-ups for StickTalk, then try again.");
  }
  // The window is a blank page of this app's own: same styles, built from here.
  // Black from the first paint, and over the app's own page background.
  const doc = win.document;
  doc.documentElement.style.background = doc.body.style.background = "#000";
  for (const sheet of document.querySelectorAll('link[rel~="stylesheet"]')) {
    const link = doc.createElement("link");
    link.rel = "stylesheet";
    link.href = sheet.href;
    doc.head.append(link);
  }
  doc.body.className = "stream-window";
  const video = doc.createElement("video");
  video.autoplay = true;
  video.muted = true; // Its sound keeps playing from the call, at the volume set there.
  video.playsInline = true;
  const bar = doc.createElement("div");
  bar.className = "stream-window-bar";
  const name = doc.createElement("span");
  const full = doc.createElement("button");
  full.type = "button";
  full.className = "icon-button";
  // Icons live in the main page; this window gets its own copy of the drawing.
  const symbol = document.getElementById("i-maximize");
  const svg = doc.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("class", "icon");
  svg.setAttribute("aria-hidden", "true");
  for (const node of symbol?.childNodes ?? []) svg.append(doc.importNode(node, true));
  full.append(svg);
  const label = () => {
    const text = doc.fullscreenElement ? "Exit Full Screen" : "Full Screen";
    full.title = text;
    full.setAttribute("aria-label", text);
  };
  label();
  const toggle = () => {
    if (doc.fullscreenElement) doc.exitFullscreen().catch(() => {});
    else doc.documentElement.requestFullscreen().catch(() => {});
  };
  full.onclick = toggle;
  video.ondblclick = toggle;
  doc.addEventListener("fullscreenchange", label);
  doc.addEventListener("keydown", (e) => {
    if (e.key !== "f" && e.key !== "F" && e.key !== "F11") return;
    e.preventDefault();
    toggle();
  });
  // The name and button show while the pointer moves, then get out of the way.
  let idle;
  const wake = () => {
    doc.body.classList.add("is-active");
    win.clearTimeout(idle);
    idle = win.setTimeout(() => doc.body.classList.remove("is-active"), 2500);
  };
  doc.addEventListener("pointermove", wake);
  doc.addEventListener("focusin", wake);
  bar.append(name, full);
  doc.body.append(video, bar);
  win.addEventListener("pagehide", () => {
    if (popout?.win !== win) return;
    closePopout();
    mount();
  });
  if (!popoutBound) {
    // Leaving or reloading the app takes the stream's window with it.
    popoutBound = true;
    addEventListener("pagehide", closePopout);
  }
  popout = { id, win, video, name };
  wake();
  mount();
}
function syncPopout() {
  if (!popout) return;
  const entry = elements.get(popout.id);
  if (!entry || popout.win.closed) return closePopout();
  const { video, win, name } = popout, stream = entry.media.srcObject, title = entry.name.textContent;
  if (video.srcObject !== stream) {
    video.srcObject = stream;
    video.play().catch(() => {});
  }
  if (name.textContent !== title) name.textContent = title;
  if (win.document.title !== title) win.document.title = title;
}
function stop(notify, keepEncryption = false) {
  if(notify){wantCall=false;wantCamera=false;listenOnly=false;pendingListen=false;stageList.stop();}
  if (!keepEncryption) {
    e2ee?.stop();
    e2ee = e2eeLoading = e2eeReady = null;
    e2eeQueue = Promise.resolve();
  }
  ptt.detach();
  ++epoch;
  for(const request of [...requests.values()])request.reject(new Error('Call ended.'));
  clearTimeout(levelsTimer);
  levelsTimer = null;
  pendingAnswer?.reject(new Error('Call ended.'));
  pendingAnswer = null;
  if (publications.has('screen')) dispatchEvent(new CustomEvent('sticktalk-sharing', { detail: false }));
  for (const p of publications.values())
    p.stream.getTracks().forEach((t) => t.stop());
  publications.clear();
  local?.getTracks().forEach((t) => t.stop());
  local = null;
  endControlling();
  endControlled();
  extras.stop();
  endAgentControl();
  controlChannel = null;
  leavePip();
  pipDismissed = false;
  pc?.close();
  pc = null;
  senders.clear();
  idleReceivers = [];
  joined = false;
  callChannel = "";
  focusedTile = null;
  status = "Disconnected";
  participants.clear();
  subscriptions.clear();
  for (const e of elements.values()) {
    boost(e, 0);
    e.media.srcObject = null;
    e.tile.remove();
  }
  elements.clear();
  speaking.clear();
  if (notify) signal("leave");
  mount();
}
async function levels() {
  if (!pc || !joined) return;
  const current = pc;
  try {
    const stats = await current.getStats();
    if (current !== pc) return;
    for (const row of stats.values())
      if (row.type === "media-source" && row.kind === "audio") {
        for (const e of elements.values())
          if (e.tile.dataset.self && e.kind !== "screen")
            e.tile.classList.toggle(
              "is-speaking",
              !muted && !deafened && Number(row.audioLevel) > 0.02,
            );
      }
    extras.stats(stats);
  } catch {
  } finally {
    if (pc === current && joined) levelsTimer = setTimeout(levels, 200);
  }
}
export function mount() {
  bindRemote();
  ptt.mount();
  const voiceView = $('.voice-view');
  if (voiceView) {
    callChannel = voiceView.dataset.callChannel;
    if (!voiceView.dataset.gesture) {
      voiceView.dataset.gesture = 'true';
      let start;
      voiceView.addEventListener('touchstart', e => {
        start = e.touches.length === 1 && !e.target.closest('.call-controls')
          ? {x:e.touches[0].clientX,y:e.touches[0].clientY} : undefined;
      }, {passive:true});
      voiceView.addEventListener('touchcancel', () => { start = undefined; }, {passive:true});
      voiceView.addEventListener('touchend', e => {
        const end = e.changedTouches[0];
        if (start && end.clientY-start.y>96 && Math.abs(end.clientX-start.x)<64) action('minimize');
        start = undefined;
      }, {passive:true});
    }
  }
  syncPopout();
  let mini = $('#live-call-mini');
  if (joined && !voiceView && !$('.live-modal')) {
    if (!mini) {
      mini = document.createElement('aside'); mini.id='live-call-mini'; mini.className='call-mini-bar';
      const restore=document.createElement('button');restore.dataset.live=`voice:${callChannel}`;restore.textContent='Voice Connected · Return to call';
      const pop=document.createElement('button');pop.className='icon-button call-mini-popout';pop.setAttribute('aria-label','Pop Out Stream');pop.title='Pop Out Stream';
      pop.append(glyph('external-link'));pop.onclick=()=>popOut(featured()).catch(report);
      mini.append(restore,pop);
      for (const [glyph,label,command] of [['mic','Mute','mute'],['phone-off','Disconnect','leave']]) {
        const button=document.createElement('button');button.className='icon-button';button.dataset.live=`media:${command}`;button.setAttribute('aria-label',label);
        const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');svg.setAttribute('viewBox','0 0 24 24');svg.classList.add('icon');svg.setAttribute('aria-hidden','true');
        const use=document.createElementNS(svg.namespaceURI,'use');use.setAttribute('href',`#i-${glyph}`);svg.append(use);button.append(svg);mini.append(button);
      }
      document.body.append(mini);
    }
    // With a stream to watch, the bar can move it to a window of its own.
    const stream = canPopOut() && !popout && !!featured();
    mini.querySelector('.call-mini-popout').hidden = !stream;
    mini.classList.toggle('has-stream', stream);
  } else mini?.remove();
  const away = joined && !voiceView;
  syncPip(away);
  syncCallButtons();
  const stage = $("#live-media-stage");
  if (focusedTile && !elements.has(focusedTile)) focusedTile = null;
  // A focused voice tile gives way to that person's camera when they turn it on.
  const focused = elements.get(focusedTile);
  if (focused?.kind === "audio") {
    const camera = [...elements].find(([, e]) =>
      e.kind === "camera" && e.tile.dataset.participant === focused.tile.dataset.participant);
    if (camera) focusedTile = camera[0];
  }
  stage?.classList.toggle('has-focus', !!focusedTile);
  if (stage)
    for (const e of elements.values()) {
      describe(e);
      const kind = e.tile.dataset.kind;
      const has = (other) =>
        [...elements.values()].some(
          (v) =>
            v.tile.dataset.participant === e.tile.dataset.participant &&
            v.tile.dataset.kind === other,
        );
      // Stream sound plays from its own hidden tile, not as a second person.
      e.tile.hidden = kind === "system_audio" || (kind === "audio" && has("camera"));
      if (e.volume) e.volume.root.hidden = kind === "screen" && !has("system_audio");
      e.tile.classList.toggle('is-focused', e.tile === elements.get(focusedTile)?.tile);
      if (!e.tile.classList.contains('is-focused')) e.tile.resetZoom?.();
      e.tile.setAttribute('aria-pressed', String(e.tile.classList.contains('is-focused')));
      if (e.tile.parentElement !== stage) stage.append(e.tile);
    }
  if (stage) {
    // Lay tiles out as a grid that fits the screen: 2 across up to four people, then 3, then 4.
    const count = [...elements.values()].filter((e) => !e.tile.hidden).length;
    const columns = innerWidth < 768 ? (count <= 3 ? 1 : 2) : count <= 1 ? 1 : count <= 4 ? 2 : count <= 9 ? 3 : 4;
    stage.style.setProperty("--columns", String(columns));
    stage.style.setProperty("--rows", String(Math.max(1, Math.ceil(count / columns))));
  }
  if (!away) $("#live-call-park")?.remove();
  // Someone is using your mouse and keyboard: always say so, with a way to stop.
  let banner = $("#live-control-banner");
  const controlNotice = controlled?.grant
    ? `${controlled.name}${controlled.agent ? "’s AI agent" : ""} is controlling your screen. Press Esc to stop.`
    : agentControl
      ? `Your AI agent is controlling ${person(agentControl.sharer.user_id).name}’s screen.`
      : null;
  if (controlNotice) {
    if (!banner) {
      banner = document.createElement("aside");
      banner.id = "live-control-banner";
      banner.className = "control-banner";
      banner.setAttribute("role", "status");
      const text = document.createElement("span");
      const end = document.createElement("button");
      end.className = "button danger";
      end.dataset.live = "media:control-stop";
      end.textContent = "Stop";
      banner.append(text, end);
      document.body.append(banner);
    }
    if (banner.firstChild.textContent !== controlNotice) banner.firstChild.textContent = controlNotice;
  } else banner?.remove();
  if (stopCameraTest && !cameraBox?.isConnected) stopCameraTest();
  const connected = $("#live-connected");
  if (connected) connected.hidden = !joined;
  const text = $("#live-call-status");
  if (text) {
    const sharing = publications.has("screen");
    text.textContent = sharing && status === "Voice Connected" ? "Voice Connected · You’re sharing your screen" : status;
    text.classList.toggle("is-live", status === "Voice Connected");
  }
  // Off states are drawn slashed and red, so they are not mistaken for "on".
  const swap = (button, icon) => button.querySelector("use")?.setAttribute("href", `#i-${icon}`);
  for (const e of document.querySelectorAll('[data-live="media:mute"]')) {
    e.setAttribute("aria-pressed", String(muted));
    e.classList.toggle("is-muted", muted);
    swap(e, muted ? "mic-off" : "mic");
  }
  for (const e of document.querySelectorAll('[data-live="media:deafen"]')) {
    e.setAttribute("aria-pressed", String(deafened));
    e.classList.toggle("is-muted", deafened);
    swap(e, deafened ? "headphone-off" : "headphones");
  }
  for (const e of document.querySelectorAll('[data-live="media:camera"]'))
    e.setAttribute("aria-pressed", String(publications.has("camera")));
  for (const e of document.querySelectorAll('[data-live="media:share"]'))
    e.setAttribute("aria-pressed", String(publications.has("screen")));
  extras.mount();
  e2ee?.mount();
  stageList.mount();
  const settings = $("#live-device-settings");
  if (settings && !settings.dataset.initialized) {
    settings.dataset.initialized = "true";
    devices().catch(report);
  }
}
async function devices() {
  const root = $("#live-device-settings");
  if (!root) return;
  if (!navigator.mediaDevices?.enumerateDevices) {
    const note = document.createElement('p');
    note.dataset.mediaUnavailable = '';
    note.setAttribute('role', 'status');
    note.textContent = 'Voice and video are unavailable in this browser. Device settings require browser media support and a secure connection.';
    root.replaceChildren(note);
    return;
  }
  const all = await navigator.mediaDevices.enumerateDevices();
  root.replaceChildren();
  for (const [kind, label, current, change] of [
    [
      "audioinput",
      "Input Device",
      input,
      (v) => {
        input = v;
        rememberDevices();
        enqueue(async () => {
          if (local) {
            const stream = await navigator.mediaDevices.getUserMedia({
              audio: {
                ...(v ? { deviceId: { exact: v } } : {}),
                echoCancellation: true,
                noiseSuppression: true,
                autoGainControl: true,
              },
            });
            const p = publications.get("audio");
            const old = p.track;
            p.track = stream.getAudioTracks()[0];
            p.stream = stream;
            local = stream;
            // The new microphone obeys push to talk like the old one did.
            await gateAudio();
            old.stop();
            applyState();
          }
        });
      },
    ],
    [
      "videoinput",
      "Camera",
      cameraDevice,
      (v) => {
        cameraDevice = v;
        rememberDevices();
        if (publications.has("camera"))
          enqueue(async () => {
            await toggleCamera();
            await toggleCamera();
          });
      },
    ],
    ["audiooutput", "Output Device", output, (v) => setOutput(v)],
  ]) {
    const field = document.createElement("label");
    field.className = "field";
    field.textContent = label;
    const select = document.createElement("select");
    select.setAttribute("aria-label", label);
    select.add(new Option("Default", ""));
    for (const d of all.filter((d) => d.kind === kind))
      select.add(
        new Option(d.label || `${label} ${select.length}`, d.deviceId),
      );
    select.value = current;
    // A remembered device that is no longer there shows as the default.
    if (select.value !== current) select.value = "";
    select.onchange = () => change(select.value);
    if (kind === "audiooutput" && !HTMLMediaElement.prototype.setSinkId) {
      select.disabled = true;
      const note = document.createElement("small");
      note.textContent = "Use your system audio output picker in this browser.";
      field.append(note);
    }
    field.append(select);
    root.append(field);
  }
  const chime = document.createElement("label");
  chime.className = "live-checkbox";
  const chimeLabel = document.createElement("span");
  chimeLabel.textContent = "Play a sound when someone joins your call";
  const chimeSwitch = document.createElement("input");
  chimeSwitch.type = "checkbox";
  chimeSwitch.checked = joinSounds;
  chimeSwitch.onchange = () => {
    joinSounds = chimeSwitch.checked;
    rememberDevices();
    // Turning it on plays it once, so you know what to listen for.
    joinSound();
  };
  chime.append(chimeLabel, chimeSwitch);
  root.append(chime);
  const note = document.createElement("p");
  note.textContent =
    "Echo cancellation, noise suppression, and automatic gain control are enabled for voice.";
  root.append(note, micTest(root), cameraTest(),
    ptt.settingsView({ enabled: ringtoneEnabled, set: setRingtoneEnabled }), extras.settings());
  if (!deviceListener) {
    deviceListener = true;
    navigator.mediaDevices.addEventListener("devicechange", () => {
      if ($("#live-device-settings")) devices().catch(report);
    });
  }
}
async function setOutput(id) {
  output = id;
  rememberDevices();
  audioContext?.setSinkId?.(id).catch(() => {});
  extras.output(id);
  // What may be boosted depends on the speaker.
  for (const e of elements.values()) applyAudio(e);
  for (const e of elements.values())
    if (e.media.setSinkId) await e.media.setSinkId(id);
}
// "Test Microphone": a level meter for the chosen microphone. Nothing runs
// until it is started, and it stops with the button or when the page goes away.
function micTest(root) {
  const box = document.createElement("div");
  box.className = "mic-test";
  const button = document.createElement("button");
  button.type = "button";
  button.className = "button secondary";
  button.textContent = "Test Microphone";
  const meter = document.createElement("div");
  meter.className = "mic-meter";
  meter.setAttribute("role", "meter");
  meter.setAttribute("aria-label", "Microphone level");
  meter.setAttribute("aria-valuemin", "0");
  meter.setAttribute("aria-valuemax", "100");
  meter.setAttribute("aria-valuenow", "0");
  const bar = document.createElement("span");
  meter.append(bar);
  const hint = document.createElement("small");
  hint.textContent = "Speak to see the meter move. Others can’t hear this test.";
  hint.hidden = true;
  let stream, context, frame;
  const stop = () => {
    cancelAnimationFrame(frame);
    stream?.getTracks().forEach((t) => t.stop());
    context?.close().catch(() => {});
    stream = context = undefined;
    bar.style.width = "0%";
    meter.setAttribute("aria-valuenow", "0");
    button.textContent = "Test Microphone";
    hint.hidden = true;
  };
  button.onclick = async () => {
    if (stream) return stop();
    try {
      stream = await navigator.mediaDevices.getUserMedia({
        audio: input ? { deviceId: { ideal: input } } : true,
      });
      context = new AudioContext();
      const analyser = context.createAnalyser();
      analyser.fftSize = 512;
      context.createMediaStreamSource(stream).connect(analyser);
      const samples = new Uint8Array(analyser.fftSize);
      button.textContent = "Stop Test";
      hint.hidden = false;
      const tick = () => {
        if (!root.isConnected) return stop();
        analyser.getByteTimeDomainData(samples);
        let peak = 0;
        for (const v of samples) peak = Math.max(peak, Math.abs(v - 128));
        const level = Math.min(100, Math.round((peak / 128) * 140));
        bar.style.width = `${level}%`;
        meter.setAttribute("aria-valuenow", String(level));
        frame = requestAnimationFrame(tick);
      };
      tick();
    } catch (e) {
      stop();
      report(e.name === "NotAllowedError"
        ? new Error("Microphone access was denied. Allow it in your browser’s site settings.")
        : e);
    }
  };
  box.append(button, meter, hint);
  return box;
}
// "Test Camera": what the chosen camera sees, shown only to you. It stops with
// the button, and when the settings page it is on goes away.
let stopCameraTest = null;
function cameraTest() {
  stopCameraTest?.();
  const box = document.createElement("div");
  box.className = "mic-test";
  const button = document.createElement("button");
  button.type = "button";
  button.className = "button secondary";
  button.textContent = "Test Camera";
  const video = document.createElement("video");
  video.className = "camera-test";
  video.muted = true;
  video.playsInline = true;
  video.hidden = true;
  video.setAttribute("aria-label", "Camera preview");
  let stream;
  const stop = () => {
    stream?.getTracks().forEach((t) => t.stop());
    stream = undefined;
    video.srcObject = null;
    video.hidden = true;
    button.textContent = "Test Camera";
    if (stopCameraTest === stop) stopCameraTest = null;
  };
  button.onclick = async () => {
    if (stream) return stop();
    try {
      stream = await navigator.mediaDevices.getUserMedia({
        video: cameraDevice ? { deviceId: { ideal: cameraDevice } } : true,
      });
      if (!box.isConnected) return stop();
      video.srcObject = stream;
      video.hidden = false;
      video.play().catch(() => {});
      button.textContent = "Stop Camera";
      stopCameraTest = stop;
    } catch (e) {
      stop();
      report(e.name === "NotAllowedError"
        ? new Error("Camera access was denied. Allow it in your browser’s site settings.")
        : e.name === "NotFoundError" ? new Error("No camera was found.") : e);
    }
  };
  box.append(button, video);
  cameraBox = box;
  return box;
}
let cameraBox = null;
// The in-call "Audio Devices" button: the same pickers, without leaving the call.
function showDevices() {
  if ($(".live-device-dialog")) return;
  const dialog = document.createElement("dialog");
  dialog.className = "live-share-options live-device-dialog";
  dialog.setAttribute("aria-label", "Audio Devices");
  const title = document.createElement("h2");
  title.textContent = "Audio Devices";
  const body = document.createElement("div");
  body.id = "live-device-settings";
  const close = document.createElement("button");
  close.className = "button secondary";
  close.textContent = "Done";
  close.onclick = () => dialog.close();
  dialog.addEventListener("close", () => dialog.remove());
  dialog.addEventListener("keydown", (e) => e.stopPropagation());
  dialog.append(title, body, close);
  document.body.append(dialog);
  dialog.showModal();
  mount();
}
export function action(action) {
  switch (action) {
    case 'minimize': {
      const chat = $('.channel-header [aria-label="Open Chat"]');
      emit(chat?.dataset.live ?? 'home');
      break;
    }
    case "camera-on-join":
      wantCamera=true;
      break;
    case "joining":
      wantCall=true;
      listenOnly = pendingListen;
      pendingListen = false;
      status = "Connecting…";
      break;
    case "stage-listener":
      if (joined) enqueue(quiet);
      else listenOnly = pendingListen = true;
      break;
    case "stage-speaker":
    case "stage-speaker-muted":
      pendingListen = false;
      // Invited and accepted speakers start muted; they unmute when ready.
      if (action === "stage-speaker-muted") muted = true;
      if (joined) enqueue(speak);
      else listenOnly = false;
      break;
    case "mute":
      muted = !muted;
      if (joined) applyState();
      else mount();
      break;
    case "deafen":
      deafened = !deafened;
      if (joined) applyState();
      else mount();
      break;
    case "camera":
      enqueue(toggleCamera);
      break;
    case "share":
      share().catch(report);
      break;
    case "leave":
      stop(true);
      break;
    case "control-stop":
      stopControl();
      break;
    case "transport-lost":
      wantCall=false;
      wantCamera=false;
      listenOnly=false;
      pendingListen=false;
      stageList.stop();
      stop(false);
      break;
    case "reconnected":
      break;
    case "devices":
      mount();
      devices().catch(report);
      break;
    case "output":
      showDevices();
      break;
    default:
      if (action.startsWith("stage-roster:")) {
        try { stageList.roster(JSON.parse(action.slice(13))); } catch {}
        mount();
      } else extras.action(action);
      break;
  }
}
// Inspectable proof API: reads browser WebRTC counters, does not fake media data.
export async function diagnostics() {
  const rows = pc
    ? [...(await pc.getStats()).values()].map((r) => ({ ...r }))
    : [];
  return {
    joined,
    joinSounds: chimes,
    audioEngine: audioContext?.state ?? "none",
    pendingRequests:requests.size,
    trace,
    localSdp: pc?.localDescription?.sdp,
    remoteSdp: pc?.remoteDescription?.sdp,
    state: pc?.connectionState,
    publications: publications.size,
    subscriptions: subscriptions.size,
    transmitting: ptt.transmitting(),
    microphoneSending: !!publications.get("audio")?.transceiver.sender.track,
    listenOnly,
    stage: stageList.diagnostics(),
    extras: extras.diagnostics(),
    e2ee: e2ee ? await e2ee.diagnostics() : null,
    sending: [...publications.values()].map((p) => {
      const params = p.transceiver.sender.getParameters();
      return {
        kind: p.kind,
        hint: p.track.contentHint,
        degradation: params.degradationPreference,
        encodings: params.encodings,
      };
    }),
    tracks: [...elements.values()].map((e) => ({
      kind: e.tile.dataset.kind,
      own: !!e.tile.dataset.self,
      paused: e.media.paused,
      streamTracks: e.media.srcObject?.getTracks().map((t) => ({
        id: t.id,
        kind: t.kind,
        muted: t.muted,
        readyState: t.readyState,
      })),
      readyState: e.media.readyState,
      videoWidth: e.media.videoWidth ?? 0,
    })),
    stats: rows,
  };
}
// What e2ee.js may see and do in the call.
const e2eeHost = {
  signal,
  report,
  mount,
  self,
  person,
  leave: () => stop(true),
  keyframe: (mid) => signal("request_keyframe", { receiver_mid: mid, fir: false }),
};
// What callextras.js may see and do in the call.
extras.bind({
  get pc() { return pc; },
  get joined() { return joined; },
  get deafened() { return deafened; },
  get output() { return output; },
  get channel() { return callChannel; },
  get cameraDevice() { return cameraDevice; },
  self,
  person,
  report,
  signal,
  enqueue,
  mount,
  action,
  negotiate: () => negotiate(),
  publication: (kind) => publications.get(kind),
  elements: () => elements,
  participants: () => participants,
  replaceCamera,
  restartCamera: () => enqueue(async () => { await toggleCamera(); await toggleCamera(); }),
});

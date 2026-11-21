// Loaded only when a user joins a call or opens Voice & Video settings. One SFU
// peer connection; no mesh, external ICE service, or media work while disconnected.
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
  deviceListener = false;
let idleReceivers = [];
// The chosen microphone, camera and speaker, remembered on this device.
const DEVICES = 'sticktalk-call-devices';
try {
  const stored = JSON.parse(localStorage.getItem(DEVICES) ?? '{}');
  const id = (value) => (typeof value === 'string' && value.length <= 512 ? value : '');
  input = id(stored.input);
  output = id(stored.output);
  cameraDevice = id(stored.camera);
} catch { /* Storage may be disabled in a private browser. */ }
function rememberDevices() {
  try { localStorage.setItem(DEVICES, JSON.stringify({ input, output, camera: cameraDevice })); } catch {}
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
      return [id, { volume: Number.isFinite(n) ? Math.min(1, Math.max(0, n)) : 1, muted: v?.muted === true }];
    });
}
let volumes = new Map();
try {
  const raw = localStorage.getItem(VOLUMES);
  if (raw && raw.length <= 8192) volumes = new Map(normalizeVolumes(JSON.parse(raw)));
} catch { /* Storage may be disabled in a private browser. */ }
const volumeKey = (p, kind) =>
  `${p.session.user_id}:${kind === 'screen' || kind === 'system_audio' ? 'stream' : 'voice'}`;
const volumeOf = (id) => volumes.get(id) ?? { volume: 1, muted: false };
function setVolume(id, next) {
  volumes.delete(id);
  if (next.muted || next.volume < 1) volumes.set(id, next);
  while (volumes.size > 64) volumes.delete(volumes.keys().next().value);
  try { localStorage.setItem(VOLUMES, JSON.stringify([...volumes])); } catch {}
  for (const e of elements.values()) if (e.volumeKey === id) applyAudio(e);
}
// The one place that decides what a tile's media element plays.
function applyAudio(entry) {
  const v = volumeOf(entry.volumeKey);
  const video = entry.media.tagName === 'VIDEO';
  entry.media.muted = video || deafened || v.muted;
  if (!video) entry.media.volume = v.volume;
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
  slider.max = '100';
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
  const browserSdp = normalizeAnswer(sdp);
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
  publications.set(kind, { kind, track, stream, transceiver });
  return transceiver;
}
async function start(data) {
  stop(false);
  if (!globalThis.RTCPeerConnection || !navigator.mediaDevices?.getUserMedia) {
    signal('leave');
    wantCall = false;
    throw Error('Voice and video are unavailable in this browser. Use Safari, Chrome or Firefox with media support.');
  }
  joined = true;
  const generation = ++epoch;
  participants = new Map(data.participants.map((p) => [key(p.session), p]));
  pc = new RTCPeerConnection({
    iceServers: [],
    bundlePolicy: "max-bundle",
    rtcpMuxPolicy: "require",
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
  try {
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
      return;
    }
    local = stream;
    await publish(stream.getAudioTracks()[0], "audio", stream);
    elements.set(
      "self-audio",
      createTile(
        "self-audio",
        { session: { user_id: self(), connection_id: "self" } },
        "audio",
      ),
    );
    applyState();
    await negotiate();
    for (const participant of participants.values())
      await updateParticipant(participant);
    levelsTimer = setTimeout(levels, 200);
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
  const entry = { tile, label, name, state, media, user: p.session.user_id, kind, volumeKey: volumeKey(p, kind) };
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
      s.transceiver.direction = "inactive";
      idleReceivers.push(s.transceiver);
      subscriptions.delete(id);
      elements.get(id)?.tile.remove();
      elements.delete(id);
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
    transceiver.direction = "recvonly";
    if (track.codec) preferH264(transceiver);
    else preferOpus(transceiver);
    const subscription = {
      key: id,
      participant,
      kind: track.kind,
      transceiver,
      source_mid: track.mid,
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
    await negotiate(()=>Promise.all(added.map(s=>requestSignal("subscribe", {
        publisher: participant.session,
        source_mid: s.source_mid,
        receiver_mid: s.transceiver.mid,
        rid: null,
      }))));
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
      for (const [id, s] of subscriptions)
        if (key(s.participant.session) === prefix) {
          signal("unsubscribe", { receiver_mid: s.transceiver.mid });
          s.transceiver.direction = "inactive";
          idleReceivers.push(s.transceiver);
          subscriptions.delete(id);
          elements.get(id)?.tile.remove();
          elements.delete(id);
        }
      mount();
      break;
    }
    case "speaking":
      retainSpeaking(speaking, key(data.session), data.speaking);
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
        for (const e of params.encodings) e.maxBitrate = Number(data.bps);
        await p.transceiver.sender.setParameters(params);
      });
      break;
    }
    case "error":
      if(!wantCall&&!joined)break; // Late signaling acknowledgments after Leave.
      report(data.message ?? "The server rejected a media operation.");
      pendingAnswer?.reject(new Error(data.message ?? 'Media operation failed.'));
      pendingAnswer = null;
      break;
    case "guild_left":
      if (data.session.user_id === self() && joined) stop(false);
      break;
    default:
      break;
  }
}
function applyState() {
  // Your microphone only: a shared screen keeps its sound while you are muted.
  const voice = publications.get("audio");
  if (voice) voice.track.enabled = !muted && !deafened;
  for (const e of elements.values()) applyAudio(e);
  signal("state", { muted, deafened });
  mount();
}
async function toggleCamera() {
  if (!pc) throw Error("Join a voice channel first.");
  const old = publications.get("camera");
  if (old) {
    old.track.stop();
    old.transceiver.direction = "inactive";
    publications.delete("camera");
    elements.get("self-camera")?.tile.remove();
    elements.delete("self-camera");
    await negotiate();
    mount();
    return;
  }
  const generation = epoch;
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: false,
    video: {
      width: { ideal: 1280 },
      height: { ideal: 720 },
      frameRate: { ideal: 30 },
      ...(cameraDevice ? { deviceId: { ideal: cameraDevice } } : {}),
    },
  });
  if (generation !== epoch) {
    stream.getTracks().forEach((t) => t.stop());
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
  mount();
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
  await negotiate();
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
  const labelled = (text, control) => {
    const label = document.createElement("label");
    label.className = "field";
    label.textContent = text;
    label.append(control);
    return label;
  };
  const fps = document.createElement("select");
  for (const n of [15, 30, 60]) {
    const o = new Option(`${n} fps`, String(n));
    fps.add(o);
  }
  fps.value = "30";
  const size = document.createElement("select");
  for (const [h, label] of [
    [720, "720p"],
    [1080, "1080p"],
    [1440, "1440p"],
  ])
    size.add(new Option(label, String(h)));
  const note = document.createElement("p");
  note.className = "muted-copy";
  note.textContent =
    "Next, choose what to share. To share its sound too, turn on the audio option in that window.";
  const start = document.createElement("button");
  start.textContent = "Share";
  start.className = "button primary";
  start.onclick = async () => {
    // getDisplayMedia is invoked inside this user gesture, not after negotiation.
    const capture = navigator.mediaDevices.getDisplayMedia({
      video: {
        frameRate: { ideal: Number(fps.value), max: Number(fps.value) },
        height: { ideal: Number(size.value) },
        width: { ideal: Math.round((Number(size.value) * 16) / 9) },
      },
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
      enqueue(negotiate);
    } catch (e) {
      if (e.name !== "NotAllowedError") report(e);
    }
  };
  const cancel = document.createElement("button");
  cancel.className = "button secondary";
  cancel.textContent = "Cancel";
  cancel.onclick = () => dialog.close();
  dialog.append(title, labelled("Resolution", size), labelled("Frame Rate", fps), note, start, cancel);
  document.body.append(dialog);
  dialog.showModal();
}
function stop(notify) {
  if(notify)wantCall=false;
  ++epoch;
  for(const request of [...requests.values()])request.reject(new Error('Call ended.'));
  clearTimeout(levelsTimer);
  levelsTimer = null;
  pendingAnswer?.reject(new Error('Call ended.'));
  pendingAnswer = null;
  for (const p of publications.values())
    p.stream.getTracks().forEach((t) => t.stop());
  publications.clear();
  local?.getTracks().forEach((t) => t.stop());
  local = null;
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
  } catch {
  } finally {
    if (pc === current && joined) levelsTimer = setTimeout(levels, 200);
  }
}
export function mount() {
  bindRemote();
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
  let mini = $('#live-call-mini');
  if (joined && !voiceView && !$('.live-modal')) {
    if (!mini) {
      mini = document.createElement('aside'); mini.id='live-call-mini'; mini.className='call-mini-bar';
      const restore=document.createElement('button');restore.dataset.live=`voice:${callChannel}`;restore.textContent='Voice Connected · Return to call';
      mini.append(restore);
      for (const [glyph,label,command] of [['mic','Mute','mute'],['phone-off','Disconnect','leave']]) {
        const button=document.createElement('button');button.className='icon-button';button.dataset.live=`media:${command}`;button.setAttribute('aria-label',label);
        const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');svg.setAttribute('viewBox','0 0 24 24');svg.classList.add('icon');svg.setAttribute('aria-hidden','true');
        const use=document.createElementNS(svg.namespaceURI,'use');use.setAttribute('href',`#i-${glyph}`);svg.append(use);button.append(svg);mini.append(button);
      }
      document.body.append(mini);
    }
  } else mini?.remove();
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
            await p.transceiver.sender.replaceTrack(stream.getAudioTracks()[0]);
            p.track.stop();
            p.track = stream.getAudioTracks()[0];
            p.stream = stream;
            local = stream;
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
  const note = document.createElement("p");
  note.textContent =
    "Echo cancellation, noise suppression, and automatic gain control are enabled for voice.";
  root.append(note, micTest(root));
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
    case "joining":
      wantCall=true;
      status = "Connecting…";
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
    case "transport-lost":
      wantCall=false;
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
    pendingRequests:requests.size,
    trace,
    localSdp: pc?.localDescription?.sdp,
    remoteSdp: pc?.remoteDescription?.sdp,
    state: pc?.connectionState,
    publications: publications.size,
    subscriptions: subscriptions.size,
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

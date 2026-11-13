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
const speaking = new Set();
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
function preferH264(transceiver) {
  const codecs = RTCRtpReceiver.getCapabilities("video")?.codecs ?? [];
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
  transceiver.setCodecPreferences(h264);
}
function preferOpus(transceiver) {
  // Match the SFU's Opus-only audio contract; do not offer RED/G.722/DTMF
  // codecs whose extra fmtp mappings differ across browser versions.
  const codecs=RTCRtpReceiver.getCapabilities('audio')?.codecs.filter(c=>c.mimeType.toLowerCase()==='audio/opus')??[];
  if(!codecs.length)throw Error('This browser does not support Opus audio.');
  transceiver.setCodecPreferences(codecs);
}
async function negotiate() {
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
  // The SFU allocates a fresh forwarding SSRC *after* answering Subscribe.
  // Do not pin Chromium's video receiver to the answer's discarded default SSRC.
  // Its generated send-stream MSIDs also need the wildcard semantic declaration.
  const browserSdp = normalizeAnswer(sdp);
  await current.setRemoteDescription({ type: "answer", sdp: browserSdp });
  bindRemote();
}
export function normalizeOffer(sdp) {
  // Chromium also gathers TCP host candidates. StickTalk's SFU is UDP-only
  // and correctly rejects those at ingress. Advertise only usable candidates;
  // leave the browser's local description and its DTLS/codec fields unchanged.
  return sdp.split(/\r?\n/).filter(line=>!line.startsWith('a=candidate:')||line.split(/\s+/)[2]?.toLowerCase()==='udp').join('\r\n');
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
    entry.media.muted = track.kind === "video" || deafened;
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
    status =
      current.connectionState === "connected"
        ? "Voice Connected"
        : current.connectionState;
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
    entry.media.muted = e.track.kind === "video" || deafened;
    entry.media.play().catch(() => {
      entry.label.textContent += " · Tap to play";
      entry.tile.onclick = () => entry.media.play().catch(report);
    });
    if (output && entry.media.setSinkId)
      entry.media.setSinkId(output).catch(report);
    mount();
  };
  try {
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: {
        echoCancellation: true,
        noiseSuppression: true,
        autoGainControl: true,
        ...(input ? { deviceId: { exact: input } } : {}),
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
    mount();
  } catch (e) {
    stop(true);
    if(e.name==='NotAllowedError')throw new Error('Microphone access was denied. Allow it in your browser’s site settings, then rejoin.');
    throw e;
  }
}
function createTile(id, p, kind) {
  const tile = document.createElement("div");
  tile.className = "participant-tile";
  tile.dataset.participant = key(p.session);
  tile.classList.toggle('is-speaking', speaking.has(key(p.session)));
  tile.dataset.kind = kind;
  if (p.session.user_id === self()) tile.dataset.self = "true";
  const label = document.createElement("div");
  label.className = "participant-label";
  const user =
    $(`[data-member-name="${p.session.user_id}"]`)?.textContent ??
    (p.session.user_id === self() ? "You" : "Participant");
  label.textContent = `${user}${kind === "screen" ? "'s screen" : ""}`;
  const media = document.createElement(
    kind === "audio" || kind === "system_audio" ? "audio" : "video",
  );
  media.autoplay = true;
  media.playsInline = true;
  if (media.tagName === "VIDEO") media.className = "call-video";
  else {
    const placeholder = document.createElement("span");
    placeholder.className = "call-avatar";
    placeholder.textContent = user.slice(0, 2);
    tile.append(placeholder);
  }
  tile.append(media, label);
  return { tile, label, media };
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
    if (!elements.has(id)) {
      const entry = createTile(id, participant, track.kind);
      entry.media.srcObject = new MediaStream([transceiver.receiver.track]);
      entry.media.muted = kind === "video" || deafened;
      entry.media.play().catch(() => {});
      elements.set(id, entry);
    }
  }
  if (added.length) {
    await negotiate();
    for (const s of added) {
      signal("subscribe", {
        publisher: participant.session,
        source_mid: s.source_mid,
        receiver_mid: s.transceiver.mid,
        rid: null,
      });
    }
  }
  mount();
}
export function event(event) {
  const data = event.data;
  record("event", event.type === "answer" ? { type: "answer" } : event);
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
        if (entry.tile.dataset.participant === key(data.session))
          entry.tile.classList.toggle("is-speaking", data.speaking);
      break;
    case "bitrate": {
      const p = [...publications.values()].find(
        (p) => p.transceiver.mid === data.mid,
      );
      if (p) {
        const params = p.transceiver.sender.getParameters();
        params.encodings ??= [{}];
        for (const e of params.encodings) e.maxBitrate = Number(data.bps);
        p.transceiver.sender.setParameters(params).catch(report);
      }
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
  for (const p of publications.values())
    if (p.track.kind === "audio") p.track.enabled = !muted && !deafened;
  for (const e of elements.values())
    e.media.muted = e.media.tagName === "VIDEO" || deafened;
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
      ...(cameraDevice ? { deviceId: { exact: cameraDevice } } : {}),
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
  const old = publications.get("screen");
  if (old) {
    old.stream.getTracks().forEach((t) => t.stop());
    old.transceiver.direction = "inactive";
    publications.delete("screen");
    await negotiate();
    return;
  }
  showShareOptions();
}
function showShareOptions() {
  const dialog = document.createElement("dialog");
  dialog.className = "live-share-options";
  const title = document.createElement("h2");
  title.textContent = "Share Screen";
  const fps = document.createElement("select");
  fps.setAttribute("aria-label", "Frame Rate");
  for (const n of [15, 30, 60]) {
    const o = new Option(`${n} fps`, String(n));
    fps.add(o);
  }
  fps.value = "30";
  const size = document.createElement("select");
  size.setAttribute("aria-label", "Resolution");
  for (const [h, label] of [
    [720, "720p"],
    [1080, "1080p"],
    [1440, "1440p"],
  ])
    size.add(new Option(label, String(h)));
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
      audio: {
        echoCancellation: false,
        noiseSuppression: false,
        autoGainControl: false,
      },
    });
    dialog.close();
    dialog.remove();
    try {
      const stream = await capture;
      if (!pc) {
        stream.getTracks().forEach((t) => t.stop());
        return;
      }
      const track = stream.getVideoTracks()[0];
      await publish(track, "screen", stream);
      for (const audio of stream.getAudioTracks())
        await publish(audio, "system_audio", stream);
      track.onended = () =>
        enqueue(async () => {
          for (const kind of ["screen", "system_audio"]) {
            const p = publications.get(kind);
            if (p) {
              p.track.stop();
              p.transceiver.direction = "inactive";
              publications.delete(kind);
            }
          }
          await negotiate();
          mount();
        });
      enqueue(negotiate);
    } catch (e) {
      if (e.name !== "NotAllowedError") report(e);
    }
  };
  const cancel = document.createElement("button");
  cancel.className = "button secondary";
  cancel.textContent = "Cancel";
  cancel.onclick = () => {
    dialog.close();
    dialog.remove();
  };
  dialog.append(title, size, fps, start, cancel);
  document.body.append(dialog);
  dialog.showModal();
}
function stop(notify) {
  if(notify)wantCall=false;
  ++epoch;
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
          if (e.tile.dataset.self)
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
  const stage = $("#live-media-stage");
  if (stage)
    for (const e of elements.values()) {
      const audio = e.tile.dataset.kind === "audio";
      const hasCamera = [...elements.values()].some(
        (v) =>
          v.tile.dataset.participant === e.tile.dataset.participant &&
          v.tile.dataset.kind === "camera",
      );
      e.tile.hidden = audio && hasCamera;
      if (e.tile.parentElement !== stage) stage.append(e.tile);
    }
  const connected = $("#live-connected");
  if (connected) connected.hidden = !joined;
  const text = $("#live-call-status");
  if (text) text.textContent = status;
  for (const e of document.querySelectorAll('[data-live="media:mute"]')) {
    e.setAttribute("aria-pressed", String(muted));
    e.classList.toggle("is-muted", muted);
  }
  for (const e of document.querySelectorAll('[data-live="media:deafen"]'))
    e.setAttribute("aria-pressed", String(deafened));
  for (const e of document.querySelectorAll('[data-live="media:camera"]'))
    e.setAttribute("aria-pressed", String(publications.has("camera")));
  const settings = $("#live-device-settings");
  if (settings && !settings.dataset.initialized) {
    settings.dataset.initialized = "true";
    devices().catch(report);
  }
}
async function devices() {
  const root = $("#live-device-settings");
  if (!root) return;
  const all = await navigator.mediaDevices.enumerateDevices();
  root.replaceChildren();
  for (const [kind, label, current, change] of [
    [
      "audioinput",
      "Input Device",
      input,
      (v) => {
        input = v;
        enqueue(async () => {
          if (local) {
            const stream = await navigator.mediaDevices.getUserMedia({
              audio: {
                deviceId: { exact: v },
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
  root.append(note);
  if (!deviceListener) {
    deviceListener = true;
    navigator.mediaDevices.addEventListener("devicechange", () => {
      if ($("#live-device-settings")) devices().catch(report);
    });
  }
}
async function setOutput(id) {
  output = id;
  for (const e of elements.values())
    if (e.media.setSinkId) await e.media.setSinkId(id);
}
export function action(action) {
  switch (action) {
    case "joining":
      wantCall=true;
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
      emit("dialog:media-settings");
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

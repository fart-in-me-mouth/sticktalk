// The call's encryption worker (docs/E2EE.md). It holds this call's ephemeral
// keys and every sender key, runs the pairwise key protocol, and encrypts and
// decrypts each audio and video frame, so the page's main thread never waits
// on cryptography and never sees a media key. Created when an encrypted call
// starts and terminated when it ends.
import * as e from './e2ee-core.js';

let keys = null;
let call = null; // { channel, session }
let encrypted = true;
const senders = new Map(); // id -> { kind, sig } (the frame counter is the key's: keys.sendTrack)
const receivers = new Map(); // id -> { kind, member, windows, checker, needKey, forged }
const counters = {
  sent: 0, received: 0, passedPlain: 0, droppedPlain: 0, droppedNoKey: 0, droppedReplay: 0,
  droppedAuth: 0, droppedForged: 0, retries: 0, keyframeRequests: 0,
};
// Time spent in the transform per frame (ms), by side and kind: what encryption adds.
const timing = { send: {}, receive: {} };
const post = (m) => self.postMessage(m);
const VIDEO = new Set(['camera', 'screen']);

function time(side, kind, start) {
  const list = (timing[side][kind] ??= []);
  list.push(performance.now() - start);
  if (list.length > 2048) list.splice(0, list.length - 2048);
}
let stateQueued = false;
function stateSoon() {
  if (stateQueued) return;
  stateQueued = true;
  queueMicrotask(async () => {
    stateQueued = false;
    if (!keys) return;
    const mode = keys.mode();
    encrypted = mode.encrypted;
    const members = [...keys.members.values()].map((m) => ({
      key: m.key, user: m.session.user_id, deviceId: m.deviceId, deviceName: m.deviceName,
      state: m.state, fp: m.fp ? e.hex(m.fp) : null, hasKey: !!m.keys,
    }));
    let code = null;
    try { code = await keys.code(); } catch { /* the call ended */ }
    if (!keys) return;
    post({
      type: 'state', encrypted: mode.encrypted, unsupported: mode.unsupported, members, code,
      self: { key: keys.self, fp: e.hex(keys.fp) }, rekeying: !!keys.pending,
      selfAltered: !!keys.selfAltered, events: keys.takeEvents(),
    });
  });
}

// Messages are handled strictly in order: a roster must not overtake the call's keys.
let inbox = Promise.resolve();
self.onmessage = ({ data: m }) => { inbox = inbox.then(() => handle(m)); };
async function handle(m) {
  try {
    switch (m.type) {
      case 'init': {
        keys?.dispose();
        call = { channel: String(m.channel), session: { user_id: String(m.session.user_id), connection_id: String(m.session.connection_id) } };
        keys = new e.CallKeys({
          channel: call.channel, session: call.session, deviceId: m.deviceId, identity: m.identity,
          send: (to, data) => post({ type: 'relay', to, data }), changed: stateSoon,
        });
        const announce = await keys.init();
        // A new call lifetime: new keys (with their own counters), new signature batches.
        for (const s of senders.values()) s.sig = null;
        for (const r of receivers.values()) { r.windows.clear(); r.checker = null; r.forged = false; }
        post({ type: 'announce', announce, fp: e.hex(keys.fp) });
        stateSoon();
        break;
      }
      case 'roster':
        if (keys) { await keys.roster(m.roster); onKeyEvents(); }
        break;
      case 'relay':
        if (keys) { await keys.message(m.from, m.data); onKeyEvents(); }
        break;
      case 'stream':
        // Older Chromium: the page created the encoded streams and moved them here.
        pipe({ side: m.side, id: m.id, kind: m.kind, member: m.member }, m.readable, m.writable);
        break;
      case 'rebind': {
        const r = receivers.get(m.id);
        if (r) Object.assign(r, { member: m.member, kind: m.kind, windows: new Map(), checker: null, forged: false, needKey: false, count: { ok: 0, noKey: 0, auth: 0, replay: 0, plain: 0 } });
        else receivers.set(m.id, receiver(m));
        break;
      }
      case 'forget':
        senders.delete(m.id);
        receivers.delete(m.id);
        break;
      case 'diagnostics': {
        const summary = (list) => {
          const s = [...list].sort((a, b) => a - b);
          const q = (p) => (s.length ? s[Math.min(s.length - 1, Math.floor(p * s.length))] : null);
          return { n: s.length, p50: q(0.5), p95: q(0.95), max: s.at(-1) ?? null };
        };
        post({
          type: 'diagnostics', id: m.id, counters: { ...counters }, encrypted,
          send: Object.fromEntries(Object.entries(timing.send).map(([k, v]) => [k, summary(v)])),
          receive: Object.fromEntries(Object.entries(timing.receive).map(([k, v]) => [k, summary(v)])),
          receivers: [...receivers.entries()].map(([id, r]) => ({ id, member: r.member, kind: r.kind, verdict: r.verdict ?? null, forged: r.forged, count: { ...r.count } })),
          kid: keys?.cur ? e.kidOf(keys.cur) : null, pending: !!keys?.pending,
        });
        break;
      }
      case 'reset-timing':
        timing.send = {}; timing.receive = {};
        for (const k of Object.keys(counters)) counters[k] = 0;
        break;
      case 'stop':
        keys?.dispose();
        keys = null;
        self.close();
        break;
    }
  } catch (error) {
    post({ type: 'error', message: String(error?.message ?? error) });
  }
}

// A key arrived late for a video stream: ask its sender for a fresh key frame.
function onKeyEvents() {
  for (const ev of keys?.events ?? []) {
    if (ev.type !== 'key') continue;
    for (const [id, r] of receivers) if (r.member === ev.key && r.needKey && VIDEO.has(r.kind)) {
      r.needKey = false;
      counters.keyframeRequests++;
      if (r.transformer?.sendKeyFrameRequest) r.transformer.sendKeyFrameRequest().catch(() => post({ type: 'keyframe', id }));
      else post({ type: 'keyframe', id });
    }
  }
  stateSoon();
}

function receiver(o) {
  return { kind: o.kind, member: o.member, windows: new Map(), checker: null, needKey: false, forged: false, verdict: null,
    count: { ok: 0, noKey: 0, auth: 0, replay: 0, plain: 0 } };
}

self.onrtctransform = (event) => {
  const t = event.transformer;
  pipe(t.options, t.readable, t.writable, t);
};

function pipe(o, readable, writable, transformer) {
  if (o.side === 'send') {
    const s = senders.get(o.id) ?? { kind: o.kind, sig: null };
    s.kind = o.kind;
    senders.set(o.id, s);
    readable.pipeThrough(new TransformStream({ transform: (frame, c) => send(s, frame, c) })).pipeTo(writable).catch(() => {});
  } else {
    const r = receivers.get(o.id) ?? receiver(o);
    if (o.member && !receivers.has(o.id)) Object.assign(r, { member: o.member, kind: o.kind });
    r.transformer = transformer;
    receivers.set(o.id, r);
    readable.pipeThrough(new TransformStream({ transform: (frame, c) => receive(r, frame, c) })).pipeTo(writable).catch(() => {});
  }
}

async function send(s, frame, c) {
  const start = performance.now();
  if (!frame.data.byteLength) return c.enqueue(frame);
  if (!keys?.cur) return; // an encrypted call never sends plaintext, even while starting
  if (!encrypted) { counters.passedPlain++; return c.enqueue(frame); }
  const video = VIDEO.has(s.kind);
  // The key and its counter, shared by every sender of this kind: a rebuilt
  // sender or peer connection carries on from it (no nonce is used twice).
  const track = await keys.sendTrack(s.kind);
  if (!keys || !track) return;
  s.sig ??= new e.SigSender({ channel: call.channel, user: call.session.user_id, connection: call.session.connection_id, kind: s.kind },
    keys.sig.privateKey, video ? 0 : e.AUDIO_CHUNK);
  const sealed = await e.sealFrame(track, new Uint8Array(frame.data), video, s.sig.take());
  if (!sealed) return;
  counters.retries += sealed.retries;
  s.sig.record(track.ks.kid, sealed.ctr, sealed.tag);
  frame.data = sealed.data.buffer;
  counters.sent++;
  c.enqueue(frame);
  time('send', s.kind, start);
}

async function receive(r, frame, c) {
  const start = performance.now();
  const data = new Uint8Array(frame.data);
  if (!data.length) return c.enqueue(frame);
  const parsed = e.parseFrame(data);
  const plain = () => {
    // Only a call every member's client was told is unencrypted accepts plaintext.
    if (!encrypted) { counters.passedPlain++; c.enqueue(frame); } else { counters.droppedPlain++; r.count.plain++; }
  };
  if (!parsed || !keys || !r.member) return plain();
  if (r.forged) { counters.droppedForged++; return; }
  const ks = await keys.recvKey(r.member, r.kind, parsed.kid);
  if (!ks) {
    if (!encrypted) return plain();
    r.needKey = true;
    counters.droppedNoKey++;
    r.count.noKey++;
    return;
  }
  let w = r.windows.get(parsed.kid);
  if (!w) {
    w = new e.ReplayWindow();
    r.windows.set(parsed.kid, w);
    while (r.windows.size > 4) r.windows.delete(r.windows.keys().next().value);
  }
  if (!w.check(parsed.ctr)) { counters.droppedReplay++; r.count.replay++; return; }
  const out = await e.openFrame(ks, data, parsed, VIDEO.has(r.kind));
  if (!out) {
    if (!encrypted) return plain();
    counters.droppedAuth++;
    r.count.auth++;
    return;
  }
  w.accept(parsed.ctr);
  const m = keys.member(r.member);
  if (m?.verifyKey) {
    r.checker ??= new e.SigChecker({ channel: call.channel, user: m.session.user_id, connection: m.session.connection_id, kind: r.kind }, m.verifyKey,
      (verdict) => verdictChanged(r, verdict));
    r.checker.frame(parsed.kid, parsed.ctr, parsed.tag);
    if (parsed.ext.length) r.checker.ext(parsed.ext.slice());
  }
  frame.data = out.byteOffset === 0 && out.byteLength === out.buffer.byteLength ? out.buffer : out.slice().buffer;
  counters.received++;
  r.count.ok++;
  c.enqueue(frame);
  time('receive', r.kind, start);
}

function verdictChanged(r, v) {
  const was = r.verdict;
  r.verdict = v;
  if (v.forged && !r.forged) r.forged = true;
  // Report only what the page shows: a forgery, or a sustained failure to verify.
  const unverified = v.unverified > 60 && v.unverified > v.verified;
  if (!was || was.forged !== v.forged || (was.unverified > 60 && was.unverified > was.verified) !== unverified)
    post({ type: 'verdict', member: r.member, kind: r.kind, forged: v.forged, unverified });
}

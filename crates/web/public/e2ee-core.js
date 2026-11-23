// End-to-end encrypted calls: the protocol core (docs/E2EE.md). Pure WebCrypto,
// no DOM: the call worker (e2ee-worker.js) runs it, and the Node tests
// (tests/e2ee-protocol.mjs) drive it directly. Nothing here runs outside a call.
const subtle = globalThis.crypto.subtle;
const enc = new TextEncoder();
export const LABEL = 'StickTalk-E2EE-v1 ';
export const MAGIC = 0xe2;
/** RFC 9605 cipher suite AES_128_GCM_SHA256_128. */
export const SUITE = 0x0004;
export const KINDS = ['audio', 'camera', 'screen', 'system_audio'];
/** Steps a receiver may ratchet a sender's key forward on its own. */
export const MAX_RATCHET_AHEAD = 8;
/** The ratchet gives way to a fresh key after this many steps in one epoch. */
export const MAX_STEP = 63;
/** A rekeying sender waits at most this long for every member's acknowledgment. */
export const REKEY_WAIT_MS = 1000;
/** A superseded receive key is erased this long after its successor is in use. */
export const PREVIOUS_KEY_MS = 2000;
export const REPLAY_WINDOW = 256;
export const BATCH_MS = 400;
export const BATCH_FRAMES = 32;
export const AUDIO_CHUNK = 96;
export const CODE_ITERATIONS = 10000;

// ---- bytes ---------------------------------------------------------------------
export function concat(...parts) {
  let n = 0;
  for (const p of parts) n += p.length;
  const out = new Uint8Array(n);
  let at = 0;
  for (const p of parts) { out.set(p, at); at += p.length; }
  return out;
}
export const text = (s) => enc.encode(s);
export const label = (s) => enc.encode(LABEL + s);
export function u64(v) {
  const n = BigInt(v);
  if (n < 0n || n > 0xffffffffffffffffn) throw Error('u64 out of range');
  const out = new Uint8Array(8);
  new DataView(out.buffer).setBigUint64(0, n);
  return out;
}
export function u32(v) {
  const out = new Uint8Array(4);
  new DataView(out.buffer).setUint32(0, v >>> 0);
  return out;
}
export const hex = (b) => Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');
export function fromHex(s) {
  if (!/^(?:[0-9a-f]{2})*$/i.test(s)) throw Error('bad hex');
  const out = new Uint8Array(s.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(s.slice(i * 2, i * 2 + 2), 16);
  return out;
}
export function b64u(bytes) {
  let s = '';
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}
export function unb64u(s) {
  if (typeof s !== 'string' || !/^[A-Za-z0-9_-]*$/.test(s) || s.length % 4 === 1) throw Error('bad base64');
  const raw = atob(s.replace(/-/g, '+').replace(/_/g, '/') + '==='.slice((s.length + 3) % 4));
  const out = new Uint8Array(raw.length);
  for (let i = 0; i < raw.length; i++) out[i] = raw.charCodeAt(i);
  return out;
}
export const equal = (a, b) => a.length === b.length && a.every((x, i) => x === b[i]);
export const random = (n) => globalThis.crypto.getRandomValues(new Uint8Array(n));
export const sessionKey = (s) => `${s.user_id}:${s.connection_id}`;
function compareSessions(a, b) {
  const ua = BigInt(a.user_id), ub = BigInt(b.user_id);
  if (ua !== ub) return ua < ub ? -1 : 1;
  const ca = BigInt(a.connection_id), cb = BigInt(b.connection_id);
  return ca === cb ? 0 : ca < cb ? -1 : 1;
}

// ---- HKDF (RFC 5869) through WebCrypto -----------------------------------------
async function hkdf(ikm, salt, info, bytes) {
  const key = await subtle.importKey('raw', ikm, 'HKDF', false, ['deriveBits']);
  return new Uint8Array(await subtle.deriveBits({ name: 'HKDF', hash: 'SHA-256', salt, info }, key, bytes * 8));
}
const sha256 = async (b) => new Uint8Array(await subtle.digest('SHA-256', b));

// ---- SFrame, RFC 9605 ----------------------------------------------------------
function minimal(v) {
  const out = [];
  let n = BigInt(v);
  do { out.unshift(Number(n & 0xffn)); n >>= 8n; } while (n > 0n);
  return out;
}
/** Section 4.3: config byte X K K K Y C C C, then KID and CTR when they do not fit. */
export function encodeHeader(kid, ctr) {
  let config = 0;
  const tail = [];
  if (kid < 8) config |= kid << 4;
  else { const b = minimal(kid); config |= 0x80 | ((b.length - 1) << 4); tail.push(...b); }
  if (ctr < 8) config |= ctr;
  else { const b = minimal(ctr); config |= 0x08 | (b.length - 1); tail.push(...b); }
  return Uint8Array.from([config, ...tail]);
}
export function decodeHeader(bytes, at = 0) {
  if (at >= bytes.length) throw Error('no header');
  const config = bytes[at];
  let p = at + 1;
  const read = (n) => {
    if (p + n > bytes.length) throw Error('short header');
    let v = 0;
    for (let i = 0; i < n; i++) v = v * 256 + bytes[p + i];
    p += n;
    return v;
  };
  const kid = config & 0x80 ? read(((config >> 4) & 7) + 1) : (config >> 4) & 7;
  const ctr = config & 0x08 ? read((config & 7) + 1) : config & 7;
  if (!Number.isSafeInteger(kid) || !Number.isSafeInteger(ctr)) throw Error('header value too large');
  return { kid, ctr, length: p - at };
}
/** Section 4.4.2: the frame key and salt for one KID. */
export async function deriveKeySalt(baseKey, kid) {
  const suite = Uint8Array.of(SUITE >> 8, SUITE & 255);
  const keyLabel = concat(text('SFrame 1.0 Secret key '), u64(kid), suite);
  const saltLabel = concat(text('SFrame 1.0 Secret salt '), u64(kid), suite);
  const keyBytes = await hkdf(baseKey, new Uint8Array(0), keyLabel, 16);
  const salt = await hkdf(baseKey, new Uint8Array(0), saltLabel, 12);
  const key = await subtle.importKey('raw', keyBytes, 'AES-GCM', false, ['encrypt', 'decrypt']);
  return { kid, key, salt, keyBytes };
}
export function nonceFor(salt, ctr) {
  const nonce = salt.slice();
  let n = BigInt(ctr);
  for (let i = 11; i >= 4 && n > 0n; i--) { nonce[i] ^= Number(n & 0xffn); n >>= 8n; }
  return nonce;
}
/** RFC 9605 encrypt: header || AEAD(ciphertext || tag). Used by the vectors. */
export async function sframeSeal(ks, ctr, metadata, plaintext) {
  const header = encodeHeader(ks.kid, ctr);
  const ct = new Uint8Array(await subtle.encrypt(
    { name: 'AES-GCM', iv: nonceFor(ks.salt, ctr), additionalData: concat(header, metadata), tagLength: 128 },
    ks.key, plaintext));
  return concat(header, ct);
}

// ---- H.264 layout --------------------------------------------------------------
/**
 * NAL units of an Annex B access unit as libwebrtc's packetizer finds them:
 * a 4-byte start code absorbs one zero before 00 00 01.
 */
export function nalUnits(b, end = b.length) {
  const out = [];
  for (let i = 0; i + 2 < end;) {
    if (b[i + 2] > 1) i += 3;
    else if (b[i + 2] === 1) {
      if (b[i] === 0 && b[i + 1] === 0) {
        let start = i;
        if (start > 0 && b[start - 1] === 0) start--;
        if (out.length) out[out.length - 1].end = start;
        out.push({ start, payload: i + 3, end });
      }
      i += 3;
    } else i++;
  }
  return out;
}
/** Bytes of a slice NAL unit (after its header) that hold the first three ue(v) fields. */
export function slicePrefix(b, from, end) {
  let pos = from, bit = 0, zeros = 0, used = 0;
  const limit = Math.min(end, from + 16);
  const next = () => {
    if (bit === 0) {
      if (pos >= limit) throw Error('slice header');
      // An emulation prevention byte (00 00 03) is skipped while reading.
      if (zeros >= 2 && b[pos] === 3) { pos++; zeros = 0; if (pos >= limit) throw Error('slice header'); }
      zeros = b[pos] === 0 ? zeros + 1 : 0;
    }
    const v = (b[pos] >> (7 - bit)) & 1;
    bit++;
    if (bit === 8) { bit = 0; pos++; }
    used = pos + (bit ? 1 : 0);
    return v;
  };
  const ue = () => {
    let leading = 0;
    while (next() === 0) if (++leading > 31) throw Error('slice header');
    for (let i = 0; i < leading; i++) next();
  };
  ue(); ue(); ue();
  return used - from;
}
/** Readable [payload, clearEnd) and encrypted [clearEnd, end) parts of each NAL unit. */
export function h264Ranges(b, end) {
  return nalUnits(b, end).map((n) => {
    const type = n.payload < n.end ? b[n.payload] & 31 : 0;
    let clear = Math.min(n.end, n.payload + 1);
    if (type >= 7 && type <= 12) clear = n.end;
    else if (type >= 1 && type <= 5) {
      try { clear = Math.min(n.end, n.payload + 1 + slicePrefix(b, n.payload + 1, n.end)); } catch { /* keep the header only */ }
    }
    return { ...n, clear };
  });
}
function h264Metadata(b, ranges) {
  const parts = [];
  for (const r of ranges) parts.push(u32(r.end - r.payload), b.subarray(r.payload, r.clear));
  return concat(...parts);
}
/** True when encrypted bytes could be read as a start code (or lose a trailing zero). */
export function emulates(b, ranges, total) {
  for (let k = 0; k < ranges.length; k++) {
    const r = ranges[k], last = k === ranges.length - 1;
    const end = last ? total : r.end;
    if (!last && r.clear >= r.end) continue;
    const from = Math.max(r.payload, (last ? Math.min(r.clear, r.end) : r.clear) - 2);
    for (let i = from; i + 2 < end; i++)
      if (b[i] === 0 && b[i + 1] === 0 && b[i + 2] <= 1) return true;
    if (!last && b[end - 1] === 0) return true;
  }
  return false;
}

// ---- frames --------------------------------------------------------------------
/**
 * Encrypts one encoded frame. `track` holds { ks: {kid,key,salt}, ctr } and is
 * advanced. Returns { data, ctr, tag } (tag: the 16-byte AEAD tag) or null.
 */
export async function sealFrame(track, frame, video, ext = new Uint8Array(0)) {
  for (let attempt = 0; attempt < 16; attempt++) {
    const ctr = track.ctr++;
    const header = encodeHeader(track.ks.kid, ctr);
    let ranges = null, metadata = new Uint8Array(0), plain = frame;
    if (video) {
      ranges = h264Ranges(frame, frame.length);
      if (!ranges.length) ranges = null;
      else {
        metadata = h264Metadata(frame, ranges);
        plain = concat(...ranges.map((r) => frame.subarray(r.clear, r.end)));
      }
    }
    const sealed = new Uint8Array(await subtle.encrypt(
      { name: 'AES-GCM', iv: nonceFor(track.ks.salt, ctr), additionalData: concat(header, metadata), tagLength: 128 },
      track.ks.key, plain));
    const tag = sealed.subarray(sealed.length - 16);
    const trailer = concat(header, ext);
    const tail = concat(tag, trailer, Uint8Array.of(trailer.length >> 8, trailer.length & 255, MAGIC));
    let out;
    if (ranges) {
      out = new Uint8Array(frame.length + tail.length);
      out.set(frame);
      let at = 0;
      for (const r of ranges) { out.set(sealed.subarray(at, at + r.end - r.clear), r.clear); at += r.end - r.clear; }
      out.set(tail, frame.length);
      if (emulates(out, ranges, out.length)) continue;
    } else out = concat(sealed.subarray(0, sealed.length - 16), tail);
    return { data: out, ctr, tag: tag.slice() };
  }
  return null;
}
/** Splits an encrypted frame. Returns null for a frame without the trailer. */
export function parseFrame(data) {
  const n = data.length;
  if (n < 20 || data[n - 1] !== MAGIC) return null;
  const len = (data[n - 3] << 8) | data[n - 2];
  const trailerStart = n - 3 - len;
  if (len < 1 || trailerStart < 16) return null;
  let header;
  try { header = decodeHeader(data, trailerStart); } catch { return null; }
  if (header.length > len) return null;
  return {
    kid: header.kid, ctr: header.ctr,
    header: data.subarray(trailerStart, trailerStart + header.length),
    ext: data.subarray(trailerStart + header.length, n - 3),
    bodyEnd: trailerStart - 16,
    tag: data.subarray(trailerStart - 16, trailerStart),
  };
}
/** Decrypts a parsed frame with { key, salt }. Returns the original frame or null. */
export async function openFrame(ks, data, parsed, video) {
  const { bodyEnd, header, tag, ctr } = parsed;
  let ranges = null, metadata = new Uint8Array(0), cipher = data.subarray(0, bodyEnd);
  if (video) {
    ranges = h264Ranges(data, bodyEnd);
    if (!ranges.length) ranges = null;
    else {
      metadata = h264Metadata(data, ranges);
      cipher = concat(...ranges.map((r) => data.subarray(r.clear, r.end)));
    }
  }
  let plain;
  try {
    plain = new Uint8Array(await subtle.decrypt(
      { name: 'AES-GCM', iv: nonceFor(ks.salt, ctr), additionalData: concat(header, metadata), tagLength: 128 },
      ks.key, concat(cipher, tag)));
  } catch { return null; }
  if (!ranges) return plain;
  const out = data.slice(0, bodyEnd);
  let at = 0;
  for (const r of ranges) { out.set(plain.subarray(at, at + r.end - r.clear), r.clear); at += r.end - r.clear; }
  return out;
}

/** Rejects repeated counters and counters older than the window, per KID. */
export class ReplayWindow {
  constructor(size = REPLAY_WINDOW) { this.size = size; this.max = -1; this.seen = new Set(); }
  check(ctr) {
    if (ctr > this.max) return true;
    return ctr > this.max - this.size && !this.seen.has(ctr);
  }
  accept(ctr) {
    if (ctr > this.max) {
      this.max = ctr;
      for (const c of this.seen) if (c <= ctr - this.size) this.seen.delete(c);
    }
    this.seen.add(ctr);
  }
}

// ---- sender signatures (docs/E2EE.md 5.6) --------------------------------------
function batchContext(ctx) {
  return concat(label('batch'), u64(ctx.channel), u64(ctx.user), u64(ctx.connection), Uint8Array.of(KINDS.indexOf(ctx.kind) + 1));
}
function batchBody(kid, c0, slots) {
  return concat(u32(kid), u64(c0), Uint8Array.of(slots.length / 8), slots);
}
/** One sending track's batches: record frames, sign, hand out chunks. */
export class SigSender {
  constructor(ctx, signKey, chunk, now = () => Date.now()) {
    this.ctx = ctx; this.signKey = signKey; this.chunk = chunk; this.now = now;
    this.batch = null; this.queue = []; this.seq = 0; this.signing = Promise.resolve();
  }
  record(kid, ctr, tag) {
    let b = this.batch;
    if (b && (b.kid !== kid || ctr < b.c0 || ctr - b.c0 >= BATCH_FRAMES || this.now() - b.at >= BATCH_MS)) { this.close(); b = null; }
    if (!b) b = this.batch = { kid, c0: ctr, n: 0, slots: new Uint8Array(BATCH_FRAMES * 8), at: this.now() };
    b.slots.set(tag.subarray(0, 8), (ctr - b.c0) * 8);
    b.n = Math.max(b.n, ctr - b.c0 + 1);
  }
  close() {
    const b = this.batch;
    this.batch = null;
    if (!b) return;
    const body = batchBody(b.kid, b.c0, b.slots.slice(0, b.n * 8));
    const message = concat(batchContext(this.ctx), body);
    this.signing = this.signing.then(async () => {
      const sig = new Uint8Array(await subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, this.signKey, message));
      const blob = concat(body, sig);
      const seq = this.seq = (this.seq + 1) & 0xffff;
      const size = this.chunk || blob.length;
      const count = Math.ceil(blob.length / size);
      // Never let a stalled track grow this: newest batches win.
      while (this.queue.length > 3 * count) this.queue.shift();
      for (let i = 0; i < count; i++)
        this.queue.push(concat(Uint8Array.of(1, seq >> 8, seq & 255, i, count), blob.subarray(i * size, (i + 1) * size)));
    }).catch(() => {});
  }
  /** The next chunk to carry in a frame's ext (or empty). */
  take() { return this.queue.shift() ?? new Uint8Array(0); }
}
/** One receiving stream's checks. Reports through onVerdict({forged, verified, unverified}). */
export class SigChecker {
  constructor(ctx, verifyKey, onVerdict = () => {}) {
    this.ctx = ctx; this.verifyKey = verifyKey; this.onVerdict = onVerdict;
    this.frames = new Map(); // `${kid}:${ctr}` -> { prefix, verified, overdue }
    this.order = [];
    this.parts = new Map();
    this.batches = [];
    this.waiting = [];
    this.arrivals = 0;
    this.verified = 0; this.unverified = 0; this.forged = false; this.badSignatures = 0;
  }
  frame(kid, ctr, tag) {
    const key = `${kid}:${ctr}`;
    if (this.frames.has(key)) return;
    const rec = { kid, ctr, prefix: tag.slice(0, 8), verified: false, overdue: false, at: this.arrivals++ };
    this.frames.set(key, rec);
    this.order.push(key);
    while (this.order.length > 512) this.frames.delete(this.order.shift());
    for (const b of this.batches) this.check(b, rec);
    this.waiting.push(rec);
    // A frame that arrived two full batches ago should have been covered by now.
    while (this.waiting.length && (this.waiting[0].verified || this.arrivals - this.waiting[0].at > 3 * BATCH_FRAMES)) {
      const r = this.waiting.shift();
      if (!r.verified) { r.overdue = true; this.unverified++; this.report(); }
    }
  }
  check(b, rec) {
    if (rec.kid !== b.kid || rec.ctr < b.c0 || rec.ctr >= b.c0 + b.n) return;
    const slot = b.slots.subarray((rec.ctr - b.c0) * 8, (rec.ctr - b.c0) * 8 + 8);
    if (!equal(slot, rec.prefix)) {
      if (!this.forged) { this.forged = true; this.report(); }
      return;
    }
    if (!rec.verified) {
      rec.verified = true;
      if (rec.overdue) { rec.overdue = false; this.unverified--; }
      this.verified++;
    }
  }
  async ext(bytes) {
    if (bytes.length < 6 || bytes[0] !== 1) return;
    const seq = (bytes[1] << 8) | bytes[2], index = bytes[3], count = bytes[4];
    if (!count || index >= count || count > 8) return;
    let p = this.parts.get(seq);
    if (!p) {
      p = { count, chunks: Array.from({ length: count }, () => null) };
      this.parts.set(seq, p);
      while (this.parts.size > 4) this.parts.delete(this.parts.keys().next().value);
    }
    if (p.count !== count) return;
    p.chunks[index] = bytes.slice(5);
    if (p.chunks.some((c) => !c)) return;
    this.parts.delete(seq);
    const blob = concat(...p.chunks);
    if (blob.length < 13 + 64) return;
    const v = new DataView(blob.buffer, blob.byteOffset, blob.byteLength);
    const kid = v.getUint32(0), c0 = Number(v.getBigUint64(4)), n = blob[12];
    if (blob.length !== 13 + n * 8 + 64 || !n || n > BATCH_FRAMES) return;
    const body = blob.subarray(0, 13 + n * 8);
    const ok = await subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, this.verifyKey,
      blob.subarray(13 + n * 8), concat(batchContext(this.ctx), body)).catch(() => false);
    if (!ok) { this.badSignatures++; return; }
    const batch = { kid, c0, n, slots: body.slice(13) };
    this.batches.push(batch);
    while (this.batches.length > 8) this.batches.shift();
    for (const k of this.order) this.check(batch, this.frames.get(k));
    this.report();
  }
  report() {
    this.onVerdict({ forged: this.forged, verified: this.verified, unverified: this.unverified, badSignatures: this.badSignatures });
  }
}

// ---- identity, announcements, codes --------------------------------------------
export async function newIdentity() {
  const pair = await subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, false, ['sign', 'verify']);
  const publicRaw = new Uint8Array(await subtle.exportKey('raw', pair.publicKey));
  return { privateKey: pair.privateKey, publicRaw };
}
const importVerify = (raw) => subtle.importKey('raw', raw, { name: 'ECDSA', namedCurve: 'P-256' }, false, ['verify']);
const importDh = (raw) => subtle.importKey('raw', raw, { name: 'ECDH', namedCurve: 'P-256' }, false, []);
export function announceBytes(channel, session, deviceId, identity, dh, sig) {
  if (!/^[0-9a-f]{32}$/.test(deviceId)) throw Error('bad device id');
  return concat(label('announce'), u64(channel), u64(session.user_id), u64(session.connection_id), text(deviceId), identity, dh, sig);
}
export async function fingerprint(user, identity) {
  return (await sha256(concat(label('fingerprint'), u64(user), identity))).subarray(0, 16);
}
export const formatFingerprint = (fp) => hex(fp).match(/.{4}/g).join(' ');
async function digits(password, salt) {
  const key = await subtle.importKey('raw', password, 'PBKDF2', false, ['deriveBits']);
  const bits = new Uint8Array(await subtle.deriveBits({ name: 'PBKDF2', hash: 'SHA-256', salt, iterations: CODE_ITERATIONS }, key, 240));
  const groups = [];
  for (let i = 0; i < 6; i++) {
    let n = 0n;
    for (let j = 0; j < 5; j++) n = (n << 8n) | BigInt(bits[i * 5 + j]);
    groups.push((n % 100000n).toString().padStart(5, '0'));
  }
  return groups.join(' ');
}
/** The call verification code over every device holding keys, self included. */
export async function callCode(channel, members) {
  const sorted = [...members].sort((a, b) => compareSessions(a.session, b.session));
  const parts = [label('call'), u64(channel)];
  for (const m of sorted) parts.push(u64(m.session.user_id), u64(m.session.connection_id), m.identity, m.dh, m.sig);
  return digits(await sha256(concat(...parts)), label('call code'));
}
/** Two devices' safety number: the same on both sides. */
export async function safetyNumber(a, b) {
  const [x, y] = [a, b].sort((p, q) => {
    const d = BigInt(p.user) - BigInt(q.user);
    return d !== 0n ? (d < 0n ? -1 : 1) : hex(p.fp) < hex(q.fp) ? -1 : 1;
  });
  return digits(await sha256(concat(label('safety'), u64(x.user), x.fp, u64(y.user), y.fp)), label('safety number'));
}
export const verifyPayload = (me, peer) => `sticktalk-verify:1:${me.user}:${hex(me.fp)}:${peer.user}:${hex(peer.fp)}`;
export function parseVerifyPayload(s) {
  const m = /^sticktalk-verify:1:(\d{1,20}):([0-9a-f]{32}):(\d{1,20}):([0-9a-f]{32})$/.exec(String(s).trim());
  return m ? { user: m[1], fp: m[2], peer: m[3], peerFp: m[4] } : null;
}

// ---- the call's key state machine ----------------------------------------------
/**
 * One device's keys in one call. Transport-agnostic: `send(toSessionKey, base64)`
 * relays a pairwise message; `changed()` is called when the visible state moves.
 * Timers come from `timers` so tests can run without real time.
 */
export class CallKeys {
  constructor({ channel, session, deviceId, identity, send, changed = () => {}, now = () => Date.now(), timers = globalThis }) {
    this.channel = String(channel);
    this.session = { user_id: String(session.user_id), connection_id: String(session.connection_id) };
    this.self = sessionKey(this.session);
    this.deviceId = deviceId;
    this.identity = identity;
    this.sendRelay = send;
    this.changed = changed;
    this.now = now;
    this.timers = timers;
    this.members = new Map();
    this.allow = false;
    this.started = false;
    this.cur = null;
    this.pending = null;
    this.sendKeys = new Map(); // `${kind}:${kid}` -> Promise<ks>
    this.queue = Promise.resolve();
    this.events = [];
  }
  /** Creates the call's ephemeral keys and the signed announcement. */
  async init() {
    const dh = await subtle.generateKey({ name: 'ECDH', namedCurve: 'P-256' }, false, ['deriveBits']);
    const sig = await subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, false, ['sign', 'verify']);
    this.dh = dh;
    this.sig = sig;
    this.dhRaw = new Uint8Array(await subtle.exportKey('raw', dh.publicKey));
    this.sigRaw = new Uint8Array(await subtle.exportKey('raw', sig.publicKey));
    this.announceRaw = announceBytes(this.channel, this.session, this.deviceId, this.identity.publicRaw, this.dhRaw, this.sigRaw);
    this.signature = new Uint8Array(await subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, this.identity.privateKey, this.announceRaw));
    this.fp = await fingerprint(this.session.user_id, this.identity.publicRaw);
    this.cur = { epoch: 0, step: 0, secret: random(32) };
    return { identity: b64u(this.identity.publicRaw), dh: b64u(this.dhRaw), sig: b64u(this.sigRaw), signature: b64u(this.signature) };
  }
  serial(task) {
    const run = this.queue.then(task);
    this.queue = run.catch(() => {});
    return run;
  }
  roster(r) { return this.serial(() => this.applyRoster(r)); }
  message(from, data) { return this.serial(() => this.applyMessage(from, data)); }

  async applyRoster(r) {
    this.allow = r.allow_unencrypted === true;
    const next = new Map();
    let selfSeen = false;
    for (const m of Array.isArray(r.members) ? r.members.slice(0, 64) : []) {
      if (!m?.session) continue;
      const session = { user_id: String(m.session.user_id), connection_id: String(m.session.connection_id) };
      const key = sessionKey(session);
      if (key === this.self) {
        selfSeen = true;
        // The server cannot sign for us; an altered copy of our own announcement is reported.
        this.selfAltered = !!m.announce && (m.announce.dh !== b64u(this.dhRaw) || m.announce.identity !== b64u(this.identity.publicRaw));
        continue;
      }
      const base = { session, key, deviceId: String(m.device_id ?? ''), deviceName: String(m.device_name ?? '').slice(0, 64) };
      if (!m.supported || !m.announce) { next.set(key, { ...base, state: 'unsupported' }); continue; }
      const old = this.members.get(key);
      const wire = `${m.announce.identity}.${m.announce.dh}.${m.announce.sig}.${m.announce.signature}.${base.deviceId}`;
      if (old && old.wire === wire) { next.set(key, old); continue; }
      next.set(key, await this.admit(base, m.announce, wire));
    }
    const valid = (map) => new Set([...map.values()].filter((m) => m.state === 'ok').map((m) => m.key));
    const before = valid(this.members), after = valid(next);
    const added = [...after].filter((k) => !before.has(k) || this.members.get(k) !== next.get(k));
    const removed = [...before].filter((k) => !after.has(k) || this.members.get(k) !== next.get(k));
    this.members = next;
    this.selfMissing = !selfSeen;
    if (!this.started) {
      this.started = true;
      await this.distribute(after);
    } else if (removed.length || (added.length && this.cur.step >= MAX_STEP)) {
      for (const k of removed) this.events.push({ type: 'left', key: k });
      await this.rekey();
    } else if (added.length) {
      await this.ratchetSelf();
      await this.distribute(added);
      if (this.pending) {
        for (const k of added) this.pending.waiting.add(k);
        await this.distributePending(added);
      }
    }
    for (const k of added) this.events.push({ type: 'joined', key: k });
    this.changed();
  }
  async admit(base, a, wire) {
    try {
      const identity = unb64u(a.identity), dh = unb64u(a.dh), sig = unb64u(a.sig), signature = unb64u(a.signature);
      if (identity.length !== 65 || dh.length !== 65 || sig.length !== 65 || signature.length !== 64) throw Error('size');
      const raw = announceBytes(this.channel, base.session, base.deviceId, identity, dh, sig);
      const ok = await subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, await importVerify(identity), signature, raw);
      if (!ok) throw Error('signature');
      const [lo, hi] = compareSessions(this.session, base.session) < 0 ? [this.announceRaw, raw] : [raw, this.announceRaw];
      const z = new Uint8Array(await subtle.deriveBits({ name: 'ECDH', public: await importDh(dh) }, this.dh.privateKey, 256));
      const pairwise = await subtle.importKey('raw', await hkdf(z, await sha256(concat(lo, hi)), label('pairwise'), 32), 'AES-GCM', false, ['encrypt', 'decrypt']);
      return {
        ...base, state: 'ok', wire, identity, dh, sig, pairwise,
        verifyKey: await importVerify(sig),
        fp: await fingerprint(base.session.user_id, identity),
        keys: null, prev: null, derived: new Map(),
      };
    } catch {
      return { ...base, state: 'invalid', wire };
    }
  }
  aad(from, to) {
    return concat(label('msg'), u64(from.user_id), u64(from.connection_id), u64(to.user_id), u64(to.connection_id), u64(this.channel));
  }
  async seal(m, body) {
    const iv = random(12);
    const ct = new Uint8Array(await subtle.encrypt({ name: 'AES-GCM', iv, additionalData: this.aad(this.session, m.session) }, m.pairwise, body));
    this.sendRelay(m.key, b64u(concat(iv, ct)));
  }
  keyBody(k) { return concat(Uint8Array.of(1), u32(k.epoch), Uint8Array.of(k.step), k.secret); }
  async distribute(keys) {
    for (const k of keys) { const m = this.members.get(k); if (m?.state === 'ok') await this.seal(m, this.keyBody(this.cur)); }
  }
  async distributePending(keys) {
    for (const k of keys) { const m = this.members.get(k); if (m?.state === 'ok') await this.seal(m, this.keyBody(this.pending)); }
  }
  async ratchetSelf() {
    // HKDF is not invertible: whoever gets the next step cannot go back.
    const old = this.cur;
    this.cur = { epoch: old.epoch, step: old.step + 1, secret: await ratchet(old.secret) };
  }
  async rekey() {
    if (this.pending) this.timers.clearTimeout(this.pending.timer);
    const base = this.pending ?? this.cur;
    const pending = { epoch: base.epoch + 1, step: 0, secret: random(32), waiting: new Set([...this.members.values()].filter((m) => m.state === 'ok').map((m) => m.key)) };
    this.pending = pending;
    await this.distributePending([...pending.waiting]);
    if (!pending.waiting.size) this.switchKey();
    else pending.timer = this.timers.setTimeout(() => this.serial(() => this.switchKey(pending)), REKEY_WAIT_MS);
  }
  switchKey(expected = this.pending) {
    if (!this.pending || this.pending !== expected) return;
    this.timers.clearTimeout(this.pending.timer);
    this.cur = { epoch: this.pending.epoch, step: 0, secret: this.pending.secret };
    this.pending = null;
    this.changed();
  }
  async applyMessage(fromKey, data) {
    const m = this.members.get(fromKey);
    if (m?.state !== 'ok') return false;
    let body;
    try {
      const raw = unb64u(data);
      if (raw.length < 12 + 16 + 6 || raw.length > 256) return false;
      body = new Uint8Array(await subtle.decrypt({ name: 'AES-GCM', iv: raw.subarray(0, 12), additionalData: this.aad(m.session, this.session) }, m.pairwise, raw.subarray(12)));
    } catch { return false; }
    const v = new DataView(body.buffer, body.byteOffset, body.byteLength);
    const type = body[0], epoch = v.getUint32(1), step = body[5];
    if (type === 1 && body.length === 38) {
      const k = { epoch, step, secret: body.slice(6) };
      const cur = m.keys;
      if (cur && (epoch < cur.epoch || (epoch === cur.epoch && step < cur.step))) return false; // replayed or stale
      if (!cur || epoch !== cur.epoch || step !== cur.step) {
        if (cur) m.prev = { ...cur, expires: this.now() + PREVIOUS_KEY_MS + REKEY_WAIT_MS };
        m.keys = k;
        m.derived = new Map([...m.derived].filter(([kid]) => m.prev && Number(kid.split(':')[1]) === kidOf(m.prev)));
        this.events.push({ type: 'key', key: fromKey });
      }
      await this.seal(m, concat(Uint8Array.of(2), u32(epoch), Uint8Array.of(step)));
      this.changed();
      return true;
    }
    if (type === 2 && body.length === 6) {
      const p = this.pending;
      if (p && p.epoch === epoch && p.step === step) {
        p.waiting.delete(fromKey);
        if (!p.waiting.size) this.switchKey(p);
      }
      return true;
    }
    return false;
  }
  /** The current frame key for one of our tracks. */
  async sendKey(kind) {
    const cur = this.cur;
    const kid = kidOf(cur);
    const id = `${kind}:${kid}`;
    let ks = this.sendKeys.get(id);
    if (!ks) {
      ks = trackKey(cur.secret, kind, kid);
      for (const old of [...this.sendKeys.keys()]) if (old.startsWith(`${kind}:`)) this.sendKeys.delete(old);
      this.sendKeys.set(id, ks);
    }
    return ks;
  }
  /** A member's frame key for a KID, ratcheting forward when allowed. Null when we cannot have it. */
  async recvKey(memberKey, kind, kid) {
    const m = this.members.get(memberKey);
    if (m?.state !== 'ok' || !m.keys) return null;
    if (m.prev && m.prev.expires < this.now()) {
      for (const id of [...m.derived.keys()]) if (Number(id.split(':')[1]) === kidOf(m.prev)) m.derived.delete(id);
      m.prev = null;
    }
    const id = `${kind}:${kid}`;
    if (m.derived.has(id)) return m.derived.get(id);
    const epoch = Math.floor(kid / 64), step = kid % 64;
    let secret = null;
    // The newest key, or the one it replaced (still in use until its sender
    // switches), ratcheted forward when the frame names a later step.
    for (const slot of ['keys', 'prev']) {
      const c = m[slot];
      if (!c || c.epoch !== epoch || step < c.step || step - c.step > MAX_RATCHET_AHEAD) continue;
      let s = c.secret;
      for (let i = c.step; i < step; i++) s = await ratchet(s);
      if (m[slot] !== c) return this.recvKey(memberKey, kind, kid);
      if (step > c.step) {
        if (slot === 'keys') {
          m.prev = { ...c, expires: this.now() + PREVIOUS_KEY_MS };
          m.keys = { epoch, step, secret: s };
        } else m.prev = { epoch, step, secret: s, expires: c.expires };
      }
      secret = s;
      break;
    }
    if (!secret) return null;
    const ks = trackKey(secret, kind, kid);
    m.derived.set(id, ks);
    while (m.derived.size > 16) m.derived.delete(m.derived.keys().next().value);
    return ks;
  }
  member(key) { return this.members.get(key); }
  /** Encrypted, or plaintext only when a visible member cannot encrypt and the call allows it. */
  mode() {
    const unsupported = [...this.members.values()].filter((m) => m.state === 'unsupported');
    if (unsupported.length && this.allow) return { encrypted: false, unsupported: unsupported.map((m) => m.key) };
    return { encrypted: true, unsupported: unsupported.map((m) => m.key) };
  }
  holders() {
    const list = [{ session: this.session, identity: this.identity.publicRaw, dh: this.dhRaw, sig: this.sigRaw }];
    for (const m of this.members.values()) if (m.state === 'ok') list.push(m);
    return list;
  }
  code() { return callCode(this.channel, this.holders()); }
  takeEvents() { const e = this.events; this.events = []; return e; }
  dispose() {
    if (this.pending) this.timers.clearTimeout(this.pending.timer);
    this.pending = null;
    this.members.clear();
    this.sendKeys.clear();
    this.cur = null;
  }
}
export const kidOf = (k) => k.epoch * 64 + k.step;
export function ratchet(secret) { return hkdf(secret, new Uint8Array(0), label('ratchet'), 32); }
/** The SFrame key and salt for one of a sender's tracks and KID. */
export async function trackKey(secret, kind, kid) {
  if (!KINDS.includes(kind)) throw Error('bad kind');
  return deriveKeySalt(await hkdf(secret, new Uint8Array(0), label(`track ${kind}`), 16), kid);
}

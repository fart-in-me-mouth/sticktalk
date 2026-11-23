// End-to-end encrypted calls: the protocol core without a browser (docs/E2EE.md).
// Known-answer vectors (RFC 9605 and the frame format), the key state machine
// with up to 32 members, and an adversarial relay/SFU that tampers, replays,
// reorders, drops, injects and strips. Run: node crates/web/tests/e2ee-protocol.mjs
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import * as e from '../public/e2ee-core.js';

const subtle = globalThis.crypto.subtle;
const results = [];
async function test(name, fn) {
  const t = performance.now();
  await fn();
  results.push({ name, ms: Math.round(performance.now() - t) });
  console.log(`ok - ${name}`);
}
const h = (s) => e.fromHex(s.replace(/\s+/g, ''));

// ---- vectors -------------------------------------------------------------------
await test('RFC 9605 C.1 header encoding', () => {
  for (const [kid, ctr, header] of [
    [0, 0, '00'], [0, 1, '01'], [0, 0xff, '08ff'], [0, 0x100, '090100'], [0, 0xffff, '09ffff'],
    [0, 0xffffff, '0affffff'], [0, 0x1000000, '0b01000000'], [1, 0, '10'], [0xff, 0, '80ff'],
    [0x100, 0, '900100'], [0xff, 0xffff, '89ffffff'], [0x100, 0xffff, '990100ffff'], [0xffff, 0, '90ffff'],
    [0, 0xffffffff, '0bffffffff'], [0, 0xffffffffffff, '0dffffffffffff'], [0x123, 0x4567, '9901234567'],
  ]) {
    assert.equal(e.hex(e.encodeHeader(kid, ctr)), header, `encode ${kid},${ctr}`);
    const d = e.decodeHeader(e.fromHex(header));
    assert.deepEqual([d.kid, d.ctr, d.length], [kid, ctr, header.length / 2]);
  }
});
await test('RFC 9605 C.3 AES_128_GCM_SHA256_128 encryption vector', async () => {
  const ks = await e.deriveKeySalt(h('000102030405060708090a0b0c0d0e0f'), 0x123);
  assert.equal(e.hex(ks.keyBytes), 'd34f547f4ca4f9a7447006fe7fcbf768');
  assert.equal(e.hex(ks.salt), '75234edefe07819026751816');
  assert.equal(e.hex(e.nonceFor(ks.salt, 0x4567)), '75234edefe07819026755d71');
  const ct = await e.sframeSeal(ks, 0x4567, h('4945544620534672616d65205747'), h('64726166742d696574662d736672616d652d656e63'));
  assert.equal(e.hex(ct), '9901234567b7412c2513a1b66dbb4884 1bbaf17f598751176ad847681a69c6d0 b091c07018ce4adb34eb'.replace(/ /g, ''));
});

// Deterministic frame-format vectors, checked against the Rust implementation
// in crates/server/src/e2ee/tests.rs (same fixture file).
const fixturePath = fileURLToPath(new URL('./fixtures/e2ee-vectors.json', import.meta.url));
function sps() { return [0, 0, 0, 1, 0x67, 0x42, 0xc0, 0x1f, 0xda, 0x01, 0x40, 0x16, 0xe8, 0x06, 0xd0, 0xa1, 0x35]; }
function pps() { return [0, 0, 0, 1, 0x68, 0xce, 0x06, 0xe2]; }
function slice(type, mbBits, seed, n) {
  // first_mb_in_slice, slice_type, pps_id as given bits, then picture data with no zero bytes.
  const out = [0, 0, 0, 1, type === 5 ? 0x65 : 0x41, ...mbBits];
  let x = seed;
  for (let i = 0; i < n; i++) { x = (x * 1103515245 + 12345) >>> 0; out.push(1 + ((x >>> 16) % 255)); }
  return out;
}
function keyFrame(seed = 7, n = 900) { return Uint8Array.from([...sps(), ...pps(), ...slice(5, [0x88, 0x84], seed, n)]); }
function deltaFrame(seed = 9, n = 400) { return Uint8Array.from(slice(1, [0x9a, 0x40], seed, n)); }
/** What the receiving depacketizer hands the transform: every start code 4 bytes, or 3. */
function restart(frame, len) {
  const units = e.nalUnits(frame);
  const code = len === 4 ? [0, 0, 0, 1] : [0, 0, 1];
  return Uint8Array.from(units.flatMap((u) => [...code, ...frame.subarray(u.payload, u.end)]));
}
const fixedTrack = async (secret, kind, kid) => ({ ks: await e.trackKey(secret, kind, kid), ctr: 0 });

await test('frame vectors (audio, H.264) are stable and match the fixture', async () => {
  const secret = h('202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f');
  const vectors = [];
  for (const [kind, kid, frame, video] of [
    ['audio', 0, Uint8Array.from({ length: 160 }, (_, i) => (i * 7 + 3) & 255), false],
    ['system_audio', 65, Uint8Array.from({ length: 40 }, (_, i) => i), false],
    ['camera', 1, keyFrame(), true],
    ['screen', 130, deltaFrame(), true],
  ]) {
    const track = await fixedTrack(secret, kind, kid);
    track.ctr = 5;
    const sealed = await e.sealFrame(track, frame, video);
    vectors.push({ kind, kid, ctr: sealed.ctr, frame: e.hex(frame), sealed: e.hex(sealed.data), key: e.hex(track.ks.keyBytes), salt: e.hex(track.ks.salt) });
  }
  const doc = { secret: e.hex(secret), note: 'StickTalk E2EE v1 frame vectors; see docs/E2EE.md 5.4', vectors };
  if (process.env.STICKTALK_E2EE_WRITE_VECTORS) writeFileSync(fixturePath, JSON.stringify(doc, null, 1) + '\n');
  const fixture = JSON.parse(readFileSync(fixturePath, 'utf8'));
  assert.deepEqual(fixture, doc);
});

// ---- frames --------------------------------------------------------------------
await test('audio frames: round trip, tamper, wrong key, header placement', async () => {
  const secret = e.random(32);
  const track = await fixedTrack(secret, 'audio', 3);
  const frame = e.random(120);
  const sealed = await e.sealFrame(track, frame, false);
  assert.equal(sealed.data.length, 120 + 16 + 1 + 3);
  const p = e.parseFrame(sealed.data);
  assert.deepEqual([p.kid, p.ctr], [3, 0]);
  assert.deepEqual(await e.openFrame(track.ks, sealed.data, p, false), frame);
  for (const at of [0, 60, 119, 120, 135, sealed.data.length - 4]) {
    const bad = sealed.data.slice();
    bad[at] ^= 1;
    const q = e.parseFrame(bad);
    assert.equal(q && await e.openFrame(track.ks, bad, q, false), null, `flip ${at}`);
  }
  const other = await fixedTrack(secret, 'camera', 3);
  assert.equal(await e.openFrame(other.ks, sealed.data, p, false), null, 'per-kind keys');
  assert.equal(e.parseFrame(frame.slice()), frame[119] === e.MAGIC ? e.parseFrame(frame) : null);
});
await test('H.264 frames: codec bytes readable, picture encrypted, start codes intact', async () => {
  const track = await fixedTrack(e.random(32), 'camera', 0);
  let retries = 0;
  for (let i = 0; i < 300; i++) {
    const frame = i % 10 ? deltaFrame(i, 200 + i * 37) : keyFrame(i, 3000 + i * 11);
    const before = track.ctr;
    const sealed = await e.sealFrame(track, frame, true);
    retries += sealed.ctr - before;
    const ranges = e.h264Ranges(sealed.data, e.parseFrame(sealed.data).bodyEnd);
    const plainRanges = e.h264Ranges(frame);
    assert.equal(ranges.length, plainRanges.length, 'same NAL units');
    for (let k = 0; k < ranges.length; k++) {
      const a = ranges[k], b = plainRanges[k];
      assert.deepEqual(sealed.data.subarray(a.payload, a.clear), frame.subarray(b.payload, b.clear), 'readable bytes');
      if (b.end > b.clear + 32) assert.notDeepEqual(sealed.data.subarray(a.clear, a.clear + 32), frame.subarray(b.clear, b.clear + 32), 'picture encrypted');
    }
    if (frame[4] === 0x67) assert.equal(ranges[0].clear, ranges[0].end, 'SPS stays readable');
    for (const len of [4, 3]) {
      const received = restart(sealed.data, len);
      const p = e.parseFrame(received);
      const out = await e.openFrame(track.ks, received, p, true);
      assert.deepEqual(out && restart(out, 4), restart(frame, 4), `start code ${len}`);
    }
  }
  assert.ok(retries < 10, `re-encryptions for start code emulation: ${retries}`);
});
await test('slice header prefix covers first_mb, slice_type and pps_id, with emulation prevention', () => {
  assert.equal(e.slicePrefix(Uint8Array.of(0x88, 0x84, 0xff), 0, 3), 2);
  assert.equal(e.slicePrefix(Uint8Array.of(0xb8, 0xff), 0, 2), 1); // 1 011 1 ...: mb 0, type 2, pps 0
  // first_mb_in_slice = 4095: 000000000001 + 12 bits; then 00 00 03 inside is skipped.
  assert.ok(e.slicePrefix(Uint8Array.of(0x00, 0x1f, 0xff, 0x88, 0x80), 0, 5) >= 3);
  assert.throws(() => e.slicePrefix(Uint8Array.of(0, 0, 0, 0), 0, 4));
});
await test('replay window: repeats and stale counters rejected, reordering accepted', () => {
  const w = new e.ReplayWindow(256);
  for (const c of [0, 1, 3, 2, 10, 9]) { assert.ok(w.check(c)); w.accept(c); }
  assert.ok(!w.check(3), 'repeat');
  w.accept(1000);
  assert.ok(!w.check(744), 'older than the window');
  assert.ok(w.check(800) && !w.check(1000));
});

// ---- the protocol --------------------------------------------------------------
const CHANNEL = '612345678901234567';
let nextConnection = 1000n;
async function device(user, opts = {}) {
  const identity = await e.newIdentity();
  const session = { user_id: String(user), connection_id: String(nextConnection++) };
  const deviceId = e.hex(e.random(16));
  const node = { user: String(user), session, key: e.sessionKey(session), deviceId, identity, inbox: [], supported: opts.supported !== false };
  node.keys = new e.CallKeys({
    channel: CHANNEL, session, deviceId, identity, now: () => clock.now,
    timers: clock, send: (to, data) => net.relay(node, to, data),
  });
  if (node.supported) node.announce = await node.keys.init();
  return node;
}
const clock = {
  now: 0, timers: [],
  setTimeout(fn, ms) { const t = { at: this.now + ms, fn }; this.timers.push(t); return t; },
  clearTimeout(t) { this.timers = this.timers.filter((x) => x !== t); },
  async advance(ms) {
    this.now += ms;
    for (;;) {
      const due = this.timers.filter((t) => t.at <= this.now);
      if (!due.length) break;
      this.timers = this.timers.filter((t) => t.at > this.now);
      for (const t of due) await t.fn();
      await net.flush();
    }
  },
};
const net = {
  nodes: new Map(), queue: [], sent: 0, bytes: 0,
  // The adversary: return a list of [to, from, data] to deliver instead.
  tamper: null,
  relay(from, to, data) { this.sent++; this.bytes += data.length; this.queue.push([to, from.key, data]); },
  async flush() {
    while (this.queue.length) {
      let batch = this.queue.splice(0);
      if (this.tamper) batch = this.tamper(batch);
      for (const [to, from, data] of batch) await this.nodes.get(to)?.keys.message(from, data);
    }
  },
  roster(opts = {}) {
    return {
      channel_id: CHANNEL, allow_unencrypted: !!opts.allow,
      members: [...this.nodes.values()].map((n) => ({
        session: n.session, device_id: n.deviceId, device_name: 'Test', supported: n.supported, announce: n.announce ?? null,
      })).concat(opts.extra ?? []),
    };
  },
  async broadcast(opts) {
    const r = this.roster(opts);
    for (const n of this.nodes.values()) if (n.supported) await n.keys.roster(r);
    await this.flush();
  },
  async join(node, opts) { this.nodes.set(node.key, node); await this.broadcast(opts); },
  async leave(node, opts) { this.nodes.delete(node.key); await this.broadcast(opts); },
};
async function reset() { net.nodes.clear(); net.queue = []; net.tamper = null; net.sent = 0; net.bytes = 0; clock.timers = []; }
/** Can `rx` decrypt a fresh frame of `tx`'s audio track right now? */
async function hears(tx, rx, kind = 'audio') {
  const ks = await tx.keys.sendKey(kind);
  const track = { ks, ctr: Math.floor(Math.random() * 1e6) };
  const frame = e.random(64);
  const sealed = await e.sealFrame(track, frame, false);
  const p = e.parseFrame(sealed.data);
  const rks = await rx.keys.recvKey(tx.key, kind, p.kid);
  if (!rks) return false;
  const out = await e.openFrame(rks, sealed.data, p, false);
  return !!out && e.equal(out, frame);
}
async function everyoneHears(list) {
  for (const a of list) for (const b of list) if (a !== b) assert.ok(await hears(a, b), `${a.key} -> ${b.key}`);
}

await test('join: newcomer gets only the ratcheted step; nobody else needs a message', async () => {
  await reset();
  const [a, b] = [await device(1), await device(2)];
  await net.join(a); await net.join(b);
  await everyoneHears([a, b]);
  const oldKey = await a.keys.sendKey('audio');
  const oldTrack = { ks: oldKey, ctr: 0 };
  const recorded = await e.sealFrame(oldTrack, e.random(50), false);
  const c = await device(3);
  const before = net.sent;
  await net.join(c);
  assert.equal(net.sent - before, 8, 'a,b send c their step and c sends a,b its key, each acknowledged');
  await everyoneHears([a, b, c]);
  const p = e.parseFrame(recorded.data);
  assert.equal(await c.keys.recvKey(a.key, 'audio', p.kid), null, 'the newcomer cannot reach the earlier step');
  assert.equal(e.kidOf(a.keys.cur), p.kid + 1);
});
await test('leave: fresh keys, switch after every acknowledgment, the departed cannot decrypt', async () => {
  await reset();
  const nodes = [];
  for (let i = 1; i <= 4; i++) { nodes.push(await device(i)); await net.join(nodes.at(-1)); }
  const [a, b, c, d] = nodes;
  // The departed member keeps its state (a recording adversary).
  await net.leave(d);
  for (const n of [a, b, c]) assert.equal(n.keys.pending, null, 'switched after acks');
  await everyoneHears([a, b, c]);
  const ks = await a.keys.sendKey('audio');
  const sealed = await e.sealFrame({ ks, ctr: 1 }, e.random(30), false);
  assert.equal(await d.keys.recvKey(a.key, 'audio', e.parseFrame(sealed.data).kid), null, 'post-removal');
});
await test('leave with lost acknowledgments: sender waits at most 1 s, then switches', async () => {
  await reset();
  const [a, b, c] = [await device(1), await device(2), await device(3)];
  await net.join(a); await net.join(b); await net.join(c);
  // Drop everything c sends (its acks) after b leaves.
  net.tamper = (batch) => batch.filter(([, from]) => from !== c.key);
  await net.leave(b);
  assert.ok(a.keys.pending, 'a still waits for c');
  const kidBefore = e.kidOf(a.keys.cur);
  await clock.advance(999);
  assert.ok(a.keys.pending);
  await clock.advance(1);
  assert.equal(a.keys.pending, null);
  assert.equal(e.kidOf(a.keys.cur), kidBefore - (kidBefore % 64) + 64);
  assert.ok(await hears(a, c), 'c already had the key, only its ack was lost');
});
await test('a join, then a leave before any frame: the old sender key still ratchets forward', async () => {
  await reset();
  const [a, b, c] = [await device(1), await device(2), await device(3)];
  await net.join(a); await net.join(b);
  await everyoneHears([a, b]);
  // c joins (a ratchets to the next step), then leaves; b's acks are held back,
  // so a keeps sending with the ratcheted step while b already has a's new key.
  await net.join(c);
  net.tamper = (batch) => batch.filter(([, from, ]) => from !== b.key);
  await net.leave(c);
  assert.ok(a.keys.pending, 'a waits for b');
  assert.ok(await hears(a, b), 'b decrypts the ratcheted step through the key it replaced');
  net.tamper = null;
  await clock.advance(1000);
  assert.ok(await hears(a, b), 'and the fresh key after the switch');
});
await test('16 members with churn, then 32 with a leave: everyone decrypts everyone; cost', async () => {
  await reset();
  const live = [];
  for (let i = 1; i <= 16; i++) { live.push(await device(i)); await net.join(live.at(-1)); }
  for (let round = 0; round < 6; round++) {
    const out = live.splice(round % live.length, 1)[0];
    await net.leave(out);
    live.push(await device(100 + round));
    await net.join(live.at(-1));
  }
  await everyoneHears(live.slice(0, 6));
  for (const n of live) assert.ok(await hears(live[0], n) || n === live[0]);
  await reset();
  const big = [];
  for (let i = 1; i <= 32; i++) { big.push(await device(i)); await net.join(big.at(-1)); }
  net.sent = 0; net.bytes = 0;
  const t = performance.now();
  await net.leave(big.pop());
  const ms = performance.now() - t;
  assert.equal(net.sent, 31 * 30 * 2, 'a leave: (n-1)(n-2) keys plus as many acks');
  results.push({ name: 'leave in a 32-member call', messages: net.sent, bytes: net.bytes, ms: Math.round(ms) });
  await everyoneHears([big[0], big[15], big[30]]);
});

// ---- adversarial server/SFU ----------------------------------------------------
await test('adversary: tampered, replayed, reordered and reflected key messages', async () => {
  await reset();
  const [a, b] = [await device(1), await device(2)];
  await net.join(a);
  const captured = [];
  net.tamper = (batch) => { captured.push(...batch); return batch.map(([to, from, d]) => [to, from, d.slice(0, 20) + (d[20] === 'A' ? 'B' : 'A') + d.slice(21)]); };
  await net.join(b);
  assert.ok(!(await hears(a, b)) && !(await hears(b, a)), 'tampered keys are refused');
  // Deliver the originals reversed (reordering), twice (replay), and reflected back.
  net.tamper = null;
  for (const [to, from, d] of [...captured].reverse()) await net.nodes.get(to).keys.message(from, d);
  for (const [to, from, d] of captured) {
    assert.equal(await net.nodes.get(from).keys.message(to, d), false, 'reflected to its sender');
    await net.nodes.get(to).keys.message(from, d);
  }
  await net.flush();
  await everyoneHears([a, b]);
  // An old key replayed after a rekey does not roll the receiver back.
  const oldMessages = captured.filter(([to]) => to === b.key);
  const c = await device(3);
  await net.join(c); await net.leave(c);
  const now = e.kidOf(b.keys.member(a.key).keys);
  for (const [to, from, d] of oldMessages) await b.keys.message(from, d);
  assert.equal(e.kidOf(b.keys.member(a.key).keys), now, 'stale key ignored');
  await everyoneHears([a, b]);
});
await test('adversary: dropped key messages never fall back to plaintext', async () => {
  await reset();
  const [a, b] = [await device(1), await device(2)];
  await net.join(a);
  net.tamper = () => [];
  await net.join(b);
  assert.ok(!(await hears(a, b)) && !(await hears(b, a)));
  assert.equal(a.keys.mode().encrypted, true);
});
await test('adversary: injected participants are refused or visible, and change the call code', async () => {
  await reset();
  const [a, b] = [await device(1), await device(2)];
  await net.join(a); await net.join(b);
  const codeBefore = await a.keys.code();
  assert.equal(codeBefore, await b.keys.code(), 'same view, same code');
  // 1) A forged announcement for someone else's device: invalid signature.
  const forger = await device(3);
  const stolen = { ...b.announce, dh: forger.announce.dh };
  const fake = { session: { user_id: '2', connection_id: '999' }, device_id: b.deviceId, device_name: 'x', supported: true, announce: stolen };
  net.sent = 0;
  await net.broadcast({ extra: [fake] });
  assert.equal(a.keys.member('2:999').state, 'invalid');
  assert.equal(net.sent, 0, 'no key is ever sent to an invalid device');
  // 2) The server's own device, shown to a only (a ghost): visible to a, codes differ.
  const ghost = await device(4);
  net.nodes.set(ghost.key, ghost);
  const r = net.roster();
  net.nodes.delete(ghost.key);
  await a.keys.roster(r); await ghost.keys.roster(r); await net.flush();
  assert.equal(a.keys.member(ghost.key).state, 'ok', 'a lists the extra device');
  assert.notEqual(await a.keys.code(), await b.keys.code(), 'a and b compare codes and see the difference');
});
await test('adversary: stripping encryption and a lying policy do not downgrade', async () => {
  await reset();
  const [a, b] = [await device(1), await device(2)];
  await net.join(a); await net.join(b);
  await net.broadcast({ allow: true });
  assert.equal(a.keys.mode().encrypted, true, 'allow_unencrypted alone changes nothing');
  assert.equal(e.parseFrame(e.random(100).map((x) => (x === e.MAGIC ? 0 : x))), null, 'a plaintext frame has no trailer');
  // A real unsupported member with fallback allowed: visible unencrypted mode.
  const old = await device(5, { supported: false });
  await net.join(old, { allow: true });
  assert.deepEqual(a.keys.mode(), { encrypted: false, unsupported: [old.key] });
  await net.broadcast({ allow: false });
  assert.equal(a.keys.mode().encrypted, true, 'without the setting the call stays encrypted');
});
await test('sender signatures: honest streams verify; a member forging another is caught', async () => {
  const ctx = { channel: CHANNEL, user: '1', connection: '7', kind: 'audio' };
  const signer = await subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, false, ['sign', 'verify']);
  let t = 0;
  const now = () => t;
  const ks = await e.trackKey(e.random(32), 'audio', 0);
  const track = { ks, ctr: 0 };
  const send = new e.SigSender(ctx, signer.privateKey, e.AUDIO_CHUNK, now);
  const verdicts = [];
  const check = new e.SigChecker(ctx, signer.publicKey, (v) => verdicts.push(v));
  const frames = [];
  for (let i = 0; i < 200; i++) {
    t += 20;
    const s = await e.sealFrame(track, e.random(100), false, send.take());
    send.record(ks.kid, s.ctr, s.tag);
    await send.signing;
    frames.push(s.data);
  }
  for (const f of frames) {
    const p = e.parseFrame(f);
    assert.ok(p.ext.length === 0 || p.ext.length <= e.AUDIO_CHUNK + 5, 'audio chunks stay small');
    check.frame(p.kid, p.ctr, p.tag);
    if (p.ext.length) await check.ext(p.ext);
  }
  assert.equal(check.forged, false);
  assert.ok(check.verified > 150 && check.unverified === 0, JSON.stringify(verdicts.at(-1)));
  // A colluding member holds the key: it can make frames that decrypt, but not sign them.
  const forged = { ks, ctr: track.ctr - 30 };
  const f = await e.sealFrame(forged, e.random(100), false);
  const p = e.parseFrame(f.data);
  const check2 = new e.SigChecker(ctx, signer.publicKey);
  check2.frame(p.kid, p.ctr, p.tag);
  for (const g of frames) { const q = e.parseFrame(g); if (q.ext.length) await check2.ext(q.ext); }
  assert.equal(check2.forged, true, 'forgery detected');
  // A server that drops every signature: frames become unverified, not forged.
  const check3 = new e.SigChecker(ctx, signer.publicKey);
  for (const g of frames) { const q = e.parseFrame(g); check3.frame(q.kid, q.ctr, q.tag); }
  assert.ok(check3.unverified > 100 && !check3.forged);
  // A signature by some other key counts as bad, not as proof.
  const other = await subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, false, ['sign', 'verify']);
  const check4 = new e.SigChecker(ctx, other.publicKey);
  for (const g of frames) { const q = e.parseFrame(g); check4.frame(q.kid, q.ctr, q.tag); if (q.ext.length) await check4.ext(q.ext); }
  assert.ok(check4.badSignatures > 0 && check4.verified === 0);
});
await test('verification codes: shared call code, symmetric safety number, QR payload', async () => {
  await reset();
  const [a, b, c] = [await device(11), await device(12), await device(13)];
  await net.join(a); await net.join(b); await net.join(c);
  const codes = await Promise.all([a, b, c].map((n) => n.keys.code()));
  assert.ok(codes.every((x) => x === codes[0]) && /^(\d{5} ){5}\d{5}$/.test(codes[0]));
  const seenByA = await e.safetyNumber({ user: a.user, fp: a.keys.fp }, { user: b.user, fp: a.keys.member(b.key).fp });
  const seenByB = await e.safetyNumber({ user: b.user, fp: b.keys.fp }, { user: a.user, fp: b.keys.member(a.key).fp });
  assert.equal(seenByA, seenByB);
  assert.notEqual(seenByA, await e.safetyNumber({ user: a.user, fp: a.keys.fp }, { user: c.user, fp: a.keys.member(c.key).fp }));
  const qr = e.verifyPayload({ user: a.user, fp: a.keys.fp }, { user: b.user, fp: a.keys.member(b.key).fp });
  assert.deepEqual(e.parseVerifyPayload(qr), { user: a.user, fp: e.hex(a.keys.fp), peer: b.user, peerFp: e.hex(b.keys.fp) });
  assert.equal(e.parseVerifyPayload('sticktalk-verify:1:1:zz:2:00'), null);
});

console.log(JSON.stringify({ suite: 'e2ee-protocol', passed: results.length, results }, null, 1));

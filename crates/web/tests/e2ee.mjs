// End-to-end encrypted calls in the real web app against a real local server
// (docs/E2EE.md). Headless Chromium, fake media, OS audio muted, loopback only.
// The server must be built with the test-only SFU tap (`--features media-tap`):
// it writes what the SFU forwards, and this suite tries to decode it.
//
//   STICKTALK_SERVER_EXE=<server built with --features media-tap> node tests/e2ee.mjs
//
// Parts: two people in an encrypted call (audio, camera, screen); what
// the SFU can decode; a third person joining and leaving (rekeying); pairwise
// verification with the QR scanner; reconnecting; a key change; a client that
// cannot encrypt (refused, then allowed with everyone told); "only join
// encrypted calls"; measurements of encrypted against unencrypted calls.
import assert from 'node:assert/strict';
import { mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { harness, register, login, act, until, wait, output, target } from './qa-harness.mjs';

const shots = process.env.STICKTALK_E2EE_SHOTS ?? join(output, 'e2ee');
await mkdir(shots, { recursive: true });
const checks = [], errors = [], numbers = {};
const diag = (p) => p.evaluate(async () => await (await import('/public/media.js')).diagnostics());
const enc = async (p) => (await diag(p)).e2ee;
const me = (p) => p.evaluate(() => document.querySelector('#sticktalk')?.dataset.selfId);
async function shot(p, name, sizes = [[1440, 900], [390, 844]]) {
  const before = p.viewportSize();
  for (const [width, height] of sizes) {
    await p.setViewportSize({ width, height });
    await p.evaluate(() => document.fonts.ready);
    await wait(250);
    await p.screenshot({ path: join(shots, `${name}-${width}x${height}.png`) });
  }
  await p.setViewportSize(before);
}

/** A page whose microphone is a pure tone, whose screen is a moving 1080p canvas. */
async function page(h, tone, { unsupported = false } = {}) {
  const p = await h.phone(1440, 900);
  await p.addInitScript(({ tone, unsupported }) => {
    if (unsupported) {
      // An engine without WebRTC encoded transforms (an old browser).
      delete globalThis.RTCRtpScriptTransform;
      delete RTCRtpSender.prototype.createEncodedStreams;
      delete RTCRtpReceiver.prototype.createEncodedStreams;
    }
    const original = navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);
    navigator.mediaDevices.getUserMedia = async (c) => {
      if (c?.video && window.__scanImage && !c.audio) {
        // The QR scanner's camera: the other person's code, as a camera would see it.
        const { d, size } = window.__scanImage;
        const canvas = document.createElement('canvas');
        canvas.width = 640; canvas.height = 640;
        const g = canvas.getContext('2d');
        const draw = () => {
          g.setTransform(1, 0, 0, 1, 0, 0);
          g.fillStyle = '#808080'; g.fillRect(0, 0, 640, 640);
          g.fillStyle = '#fff'; g.fillRect(120, 120, 400, 400);
          g.setTransform(400 / size, 0, 0, 400 / size, 120, 120);
          g.fillStyle = '#000'; g.fill(new Path2D(d));
        };
        draw();
        setInterval(draw, 100);
        return canvas.captureStream(10);
      }
      if (!c?.audio || c.video) return original(c);
      const ctx = new AudioContext({ sampleRate: 48000 }), out = ctx.createMediaStreamDestination();
      out.channelCount = 1;
      const o = ctx.createOscillator(), g = ctx.createGain();
      o.frequency.value = tone; g.gain.value = 0.25;
      o.connect(g).connect(out); o.start();
      await ctx.resume();
      return out.stream;
    };
    navigator.mediaDevices.getDisplayMedia = async () => {
      const canvas = document.createElement('canvas');
      canvas.width = 1920; canvas.height = 1080;
      const g = canvas.getContext('2d');
      let n = 0;
      const loop = () => {
        n++;
        g.fillStyle = `hsl(${(n * 3) % 360} 60% 40%)`; g.fillRect(0, 0, 1920, 1080);
        for (let i = 0; i < 40; i++) { g.fillStyle = `hsl(${(i * 37 + n * 5) % 360} 80% 55%)`; g.fillRect((i * 97 + n * 11) % 1920, (i * 53 + n * 7) % 1080, 160, 120); }
        g.fillStyle = '#fff'; g.font = '48px sans-serif'; g.fillText(`frame ${n}`, 80, 120);
        requestAnimationFrame(loop);
      };
      requestAnimationFrame(loop);
      const [picture] = canvas.captureStream(60).getVideoTracks();
      const settings = picture.getSettings.bind(picture);
      picture.getSettings = () => ({ ...settings(), displaySurface: 'monitor' });
      return new MediaStream([picture]);
    };
  }, { tone, unsupported });
  p.on('pageerror', (e) => errors.push(e.stack ?? String(e)));
  return p;
}

/** The loudest frequency in what this page plays for `userId` (decrypted audio, not silence). */
async function heard(p, userId) {
  return p.evaluate(async (userId) => {
    const tile = [...document.querySelectorAll('.participant-tile[data-kind="audio"]')].find((t) => t.dataset.participant?.startsWith(`${userId}:`));
    const track = tile?.querySelector('audio')?.srcObject?.getAudioTracks()[0];
    if (!track) return null;
    const ctx = new AudioContext();
    const an = ctx.createAnalyser();
    an.fftSize = 8192;
    ctx.createMediaStreamSource(new MediaStream([track])).connect(an);
    await ctx.resume().catch(() => {});
    await new Promise((r) => setTimeout(r, 800));
    const bins = new Float32Array(an.frequencyBinCount);
    an.getFloatFrequencyData(bins);
    let best = 1;
    for (let i = 1; i < bins.length; i++) if (bins[i] > bins[best]) best = i;
    const out = { hz: Math.round((best * ctx.sampleRate) / an.fftSize), db: Math.round(bins[best]) };
    ctx.close();
    return out;
  }, userId);
}
const receiverOf = (d, member, kind) => d.receivers.find((r) => r.member === member && r.kind === kind);
/** Inbound audio counters: everyone's, or only the stream of `userId`. */
async function inboundAudio(p, userId) {
  const track = userId && await p.evaluate((userId) => [...document.querySelectorAll('.participant-tile[data-kind="audio"]')]
    .find((t) => t.dataset.participant?.startsWith(`${userId}:`))?.querySelector('audio')?.srcObject?.getAudioTracks()[0]?.id, userId);
  if (userId && !track) throw Error(`no audio tile for ${userId}`);
  const rows = (await diag(p)).stats.filter((r) => r.type === 'inbound-rtp' && r.kind === 'audio' && (!track || r.trackIdentifier === track));
  return rows.reduce((a, r) => ({ samples: a.samples + (r.totalSamplesReceived ?? 0), concealed: a.concealed + (r.concealedSamples ?? 0) - (r.silentConcealedSamples ?? 0), events: a.events + (r.concealmentEvents ?? 0) }), { samples: 0, concealed: 0, events: 0 });
}
async function videoIn(p) {
  const rows = (await diag(p)).stats.filter((r) => r.type === 'inbound-rtp' && r.kind === 'video');
  return rows.reduce((a, r) => ({ frames: a.frames + (r.framesDecoded ?? 0), freezes: a.freezes + (r.freezeCount ?? 0), frozen: a.frozen + (r.totalFreezesDuration ?? 0) }), { frames: 0, freezes: 0, frozen: 0 });
}
/**
 * Samples a stream's concealment every 200 ms until stopped. Chrome may start a
 * receive stream's counters over after renegotiation, so only forward steps count.
 */
function sampler(p, userId) {
  let stop = false, last = null;
  const total = { samples: 0, concealed: 0, events: 0, resets: 0 };
  const done = (async () => {
    while (!stop) {
      const now = await inboundAudio(p, userId).catch(() => null);
      if (now && last) {
        if (now.samples >= last.samples) {
          total.samples += now.samples - last.samples;
          total.concealed += Math.max(0, now.concealed - last.concealed);
          total.events += Math.max(0, now.events - last.events);
        } else total.resets++;
      }
      if (now) last = now;
      await wait(200);
    }
  })();
  return async () => { stop = true; await done; return total; };
}
async function joinVoice(p, voice) {
  // After a reload the app opens Home: open the server first.
  for (let i = 0; i < 5 && !(await p.locator(`[data-live="${voice}"]`).count()); i++) {
    await p.locator('[data-live^="guild:"]').first().click();
    await p.locator(`[data-live="${voice}"]`).first().waitFor({ timeout: 3000 }).catch(() => {});
  }
  await p.locator(`[data-live="${voice}"]`).first().click();
}
async function connected(...pages) {
  await until(async () => (await Promise.all(pages.map(diag))).every((d) => d.state === 'connected'), 45000);
}

// ---- the SFU tap: what the server forwarded, and decoding it --------------------
function records(buf) {
  const out = [];
  for (let at = 0; at + 9 <= buf.length;) {
    const len = buf.readUInt16BE(at + 7);
    out.push({ seq: buf.readUInt16BE(at), ts: buf.readUInt32BE(at + 2), marker: buf[at + 6] === 1, payload: buf.subarray(at + 9, at + 9 + len) });
    at += 9 + len;
  }
  return out;
}
async function tapped(dir, user, kind) {
  const names = (await readdir(dir).catch(() => [])).filter((n) => n.startsWith(`${user}-`) && n.endsWith(`-${kind}.rtp`));
  const all = [];
  for (const n of names) all.push({ name: n, packets: records(await readFile(join(dir, n))) });
  return all;
}
/** Annex B access units from H.264 RTP payloads (RFC 6184: single, STAP-A, FU-A). */
function accessUnits(packets) {
  const units = [];
  let cur = [], fu = null, ts = null;
  const sc = Buffer.from([0, 0, 0, 1]);
  for (const p of packets) {
    if (ts !== null && p.ts !== ts && cur.length) { units.push({ ts, data: Buffer.concat(cur) }); cur = []; }
    ts = p.ts;
    const b = p.payload, type = b[0] & 31;
    if (type >= 1 && type <= 23) cur.push(sc, b);
    else if (type === 24) {
      for (let i = 1; i + 2 <= b.length;) { const n = b.readUInt16BE(i); cur.push(sc, b.subarray(i + 2, i + 2 + n)); i += 2 + n; }
    } else if (type === 28) {
      const start = b[1] & 0x80, end = b[1] & 0x40;
      if (start) fu = [Buffer.from([(b[0] & 0xe0) | (b[1] & 31)])];
      if (fu) fu.push(b.subarray(2));
      if (end && fu) { cur.push(sc, Buffer.concat(fu)); fu = null; }
    }
    if (p.marker && cur.length) { units.push({ ts, data: Buffer.concat(cur) }); cur = []; ts = null; }
  }
  return units;
}
/** Decode what the SFU forwarded, in a page (WebCodecs), as the server could. */
async function decodeAudio(p, packets, tone) {
  return p.evaluate(async ({ packets, tone }) => {
    const pcm = [];
    let errors = 0;
    const dec = new AudioDecoder({
      output: (d) => { const f = new Float32Array(d.numberOfFrames); d.copyTo(f, { planeIndex: 0, format: 'f32-planar' }); pcm.push(f); d.close(); },
      error: () => { errors++; },
    });
    dec.configure({ codec: 'opus', sampleRate: 48000, numberOfChannels: 1 });
    let t = 0;
    for (const b64 of packets) {
      if (dec.state !== 'configured') break;
      try { dec.decode(new EncodedAudioChunk({ type: 'key', timestamp: t, data: Uint8Array.from(atob(b64), (c) => c.charCodeAt(0)) })); } catch { errors++; }
      t += 20000;
    }
    await dec.flush().catch(() => { errors++; });
    const all = new Float32Array(pcm.reduce((n, f) => n + f.length, 0));
    let at = 0;
    for (const f of pcm) { all.set(f, at); at += f.length; }
    // Goertzel: the share of the signal's energy at the speaker's tone.
    const k = (2 * Math.PI * tone) / 48000, coeff = 2 * Math.cos(k);
    let s1 = 0, s2 = 0, total = 0;
    for (const x of all) { const s = x + coeff * s1 - s2; s2 = s1; s1 = s; total += x * x; }
    const power = s1 * s1 + s2 * s2 - coeff * s1 * s2;
    return { samples: all.length, errors, toneShare: total ? (2 * power) / (all.length * total) : 0 };
  }, { packets: packets.map((x) => x.payload.toString('base64')), tone });
}
async function decodeVideo(p, units) {
  return p.evaluate(async (units) => {
    const bytes = (b64) => Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
    let frames = 0, errors = 0, closed = false;
    const first = units.findIndex((u) => bytes(u).some((_, i, a) => i > 3 && a[i - 1] === 1 && a[i - 2] === 0 && a[i - 3] === 0 && (a[i] & 31) === 5));
    if (first < 0) return { frames: 0, errors: 0, keyframes: 0 };
    const sps = (() => { const a = bytes(units[first]); for (let i = 4; i < a.length; i++) if (a[i - 1] === 1 && a[i - 2] === 0 && (a[i] & 31) === 7) return a.subarray(i, i + 4); return null; })();
    const codec = sps ? `avc1.${[...sps.subarray(1, 4)].map((x) => x.toString(16).padStart(2, '0')).join('')}` : 'avc1.42e01f';
    const dec = new VideoDecoder({ output: (f) => { frames++; f.close(); }, error: () => { errors++; closed = true; } });
    dec.configure({ codec, optimizeForLatency: true });
    let t = 0;
    for (let i = first; i < units.length && !closed; i++) {
      const a = bytes(units[i]);
      const key = a.some((_, j) => j > 3 && a[j - 1] === 1 && a[j - 2] === 0 && (a[j] & 31) === 5);
      try { dec.decode(new EncodedVideoChunk({ type: i === first || key ? 'key' : 'delta', timestamp: t, data: a })); } catch { errors++; break; }
      t += 16666;
    }
    await dec.flush().catch(() => { errors++; });
    return { frames, errors, codec, units: units.length - first };
  }, units.map((u) => u.data.toString('base64')));
}
// Packets without payload (padding, probes) carry no media and are left out.
const trailerShare = (packets) => { const media = packets.filter((x) => x.payload.length); return media.filter((x) => x.payload[x.payload.length - 1] === 0xe2).length / Math.max(1, media.length); };

// =====================================================================================
const tap = join(target, `e2ee-tap-${Date.now()}`);
await mkdir(tap, { recursive: true });
process.env.STICKTALK_MEDIA_TAP = tap;
const h = await harness();
let alice, bob, carol, dave;
try {
  alice = await page(h, 1000);
  bob = await page(h, 1500);
  carol = await page(h, 700);
  dave = await page(h, 2000, { unsupported: true });
  await register(alice, h.origin, 'qa_e2ee_alice');
  await register(bob, h.origin, 'qa_e2ee_bob');
  await register(carol, h.origin, 'qa_e2ee_carol');
  await register(dave, h.origin, 'qa_e2ee_dave');
  await act(alice, 'dialog:create');
  await alice.waitForSelector('[data-live-form="create"]');
  await alice.locator('#f-name').fill('Private Lab');
  await alice.locator('[data-live-form] [type=submit]').click();
  await alice.waitForSelector('#live-input');
  await alice.waitForFunction(() => !document.querySelector('.live-modal'));
  await act(alice, 'dialog:invite');
  await alice.waitForSelector('[data-live-form="invite"]');
  await alice.locator('[data-live-form="invite"] [type=submit]').click();
  await alice.waitForSelector('.live-modal code');
  const code = await alice.locator('.live-modal code').first().textContent();
  await act(alice, 'close');
  for (const p of [bob, carol, dave]) {
    await act(p, 'dialog:join');
    await p.waitForSelector('[data-live-form="join"]');
    await p.locator('#f-name').fill(code);
    await p.locator('[data-live-form] [type=submit]').click();
    await p.waitForSelector('#live-input');
  }
  const voice = await alice.locator('[data-live^="voice:"]').first().getAttribute('data-live');
  const [A, B, C, D] = await Promise.all([alice, bob, carol, dave].map(me));

  // ---- 1. Two people: encrypted by default -----------------------------------------
  // Nothing about encryption loads before a call.
  assert.equal(await alice.evaluate(() => performance.getEntriesByType('resource').some((r) => /e2ee/.test(r.name))), false, 'no encryption code before a call');
  await joinVoice(alice, voice);
  await joinVoice(bob, voice);
  await connected(alice, bob);
  await until(async () => {
    const [a, b] = await Promise.all([enc(alice), enc(bob)]);
    return a?.state?.encrypted && b?.state?.encrypted && a.counters.received > 100 && b.counters.received > 100;
  }, 30000);
  const [ea, eb] = await Promise.all([enc(alice), enc(bob)]);
  assert.equal(ea.mode, 'script', 'RTCRtpScriptTransform in a worker');
  assert.equal(ea.state.code, eb.state.code, 'both see the same call code');
  assert.match(ea.state.code, /^(\d{5} ){5}\d{5}$/);
  assert.equal(ea.counters.droppedAuth + eb.counters.droppedAuth, 0);
  assert.equal(ea.persistent, true, 'identity key kept in IndexedDB');
  const bobHeard = await heard(alice, B), aliceHeard = await heard(bob, A);
  assert.ok(Math.abs(bobHeard.hz - 1500) < 20 && bobHeard.db > -60, `alice hears bob's 1500 Hz tone: ${JSON.stringify(bobHeard)}`);
  assert.ok(Math.abs(aliceHeard.hz - 1000) < 20, `bob hears alice: ${JSON.stringify(aliceHeard)}`);
  const pill = await alice.locator('.e2ee-pill').textContent();
  assert.equal(pill.trim(), 'End-to-end encrypted');
  checks.push(`two people: encrypted by default (${ea.mode}), both hear each other's tone through decryption (${bobHeard.hz} Hz, ${aliceHeard.hz} Hz), same call code`);
  await shot(alice, 'call-encrypted');
  await alice.locator('.e2ee-pill').click();
  await alice.waitForSelector('.e2ee-panel .e2ee-code');
  await alice.waitForSelector('.e2ee-panel .e2ee-device[data-member]');
  await shot(alice, 'panel');
  await alice.locator('.e2ee-panel .button.primary:has-text("Done")').click();

  // Camera and a screen share, encrypted, decoded by the other side.
  await act(bob, 'media:camera');
  await until(async () => (await diag(alice)).tracks.some((t) => t.kind === 'camera' && t.videoWidth > 0), 30000);
  await act(bob, 'media:share');
  await bob.locator('.live-share-options .button.primary').click();
  await until(async () => (await diag(alice)).tracks.some((t) => t.kind === 'screen' && t.videoWidth > 0), 30000);
  await wait(4000);
  const v0 = await videoIn(alice);
  await wait(3000);
  const v1 = await videoIn(alice);
  const ra = await enc(alice);
  assert.ok(receiverOf(ra, ra.state.members[0].key, 'screen')?.count.ok > 100, 'screen frames decrypted');
  assert.ok(receiverOf(ra, ra.state.members[0].key, 'camera')?.count.ok > 30, 'camera frames decrypted');
  numbers.encryptedVideoFps = Math.round((v1.frames - v0.frames) / 3);
  checks.push(`camera and screen share: encrypted, decrypted and decoded by the viewer (${numbers.encryptedVideoFps} decoded frames/s over both)`);
  await shot(alice, 'call-encrypted-video');

  // ---- 2. The SFU forwarded only ciphertext ----------------------------------------
  const sfuAudio = (await tapped(tap, A, 'audio')).flatMap((f) => f.packets);
  const sfuScreen = (await tapped(tap, B, 'screen')).flatMap((f) => f.packets);
  assert.ok(sfuAudio.length > 200 && sfuScreen.length > 200, 'the tap saw the call');
  numbers.sfuAudioTrailer = trailerShare(sfuAudio);
  assert.ok(numbers.sfuAudioTrailer > 0.99, 'every forwarded audio frame is an encrypted frame');
  numbers.sfuDecodeEncryptedAudio = await decodeAudio(alice, sfuAudio.filter((x) => x.payload.length).slice(-400), 1000);
  numbers.sfuDecodeEncryptedScreen = await decodeVideo(alice, accessUnits(sfuScreen).slice(0, 240));
  assert.ok(numbers.sfuDecodeEncryptedAudio.toneShare < 0.05, `the SFU's copy of alice's audio is not her tone: ${JSON.stringify(numbers.sfuDecodeEncryptedAudio)}`);
  checks.push(`the SFU's copy: ${Math.round(numbers.sfuAudioTrailer * 100)}% of audio packets are encrypted frames; decoding it gives ${(numbers.sfuDecodeEncryptedAudio.toneShare * 100).toFixed(2)}% of the speaker's tone; its screen share decodes to ${numbers.sfuDecodeEncryptedScreen.frames} frames with ${numbers.sfuDecodeEncryptedScreen.errors} errors`);
  await act(bob, 'media:share');
  await act(bob, 'media:camera');
  await until(async () => !(await diag(alice)).tracks.some((t) => t.kind === 'screen' || t.kind === 'camera'), 20000);

  // ---- 3. A third person joins, then leaves: ratchet, rekey, no gap ------------------
  const before = await enc(alice);
  const bobKey = before.state.members.find((m) => m.user === B).key;
  const kid0 = before.kid;
  const bobRx0 = receiverOf(before, bobKey, 'audio').count;
  const stopSampling = sampler(alice, B);
  const t0 = Date.now();
  await joinVoice(carol, voice);
  await connected(carol);
  await until(async () => {
    const c = await enc(carol);
    return c?.receivers.filter((r) => r.kind === 'audio' && r.count.ok > 50).length >= 2;
  }, 30000);
  numbers.joinToDecryptMs = Date.now() - t0;
  const afterJoin = await enc(alice);
  assert.equal(afterJoin.kid, kid0 + 1, 'alice ratcheted one step for the newcomer');
  const cHeard = await heard(carol, A);
  assert.ok(Math.abs(cHeard.hz - 1000) < 20, `carol hears alice: ${JSON.stringify(cHeard)}`);
  await wait(1500);
  await act(carol, 'media:leave');
  await until(async () => (await enc(alice)).kid >= Math.floor(kid0 / 64) * 64 + 64, 10000);
  await until(async () => !(await enc(alice)).pending, 5000);
  await wait(2000);
  const afterLeave = await enc(alice);
  const bobRx1 = receiverOf(afterLeave, bobKey, 'audio').count;
  const gap = await stopSampling();
  assert.equal(bobRx1.noKey - bobRx0.noKey, 0, 'no frame of bob\'s was undecryptable at alice during join and leave');
  assert.equal(bobRx1.auth - bobRx0.auth, 0);
  numbers.rekeyConcealment = gap;
  numbers.rekeyConcealedShare = numbers.rekeyConcealment.concealed / Math.max(1, numbers.rekeyConcealment.samples);
  assert.ok(numbers.rekeyConcealedShare < 0.01, `no audible gap: ${JSON.stringify(numbers.rekeyConcealment)}`);
  checks.push(`a join ratchets (KID ${kid0}→${afterJoin.kid}), a leave rekeys (→${afterLeave.kid}); the newcomer decrypts both others within ${numbers.joinToDecryptMs} ms of clicking Join; 0 of bob's frames lost at alice; ${(numbers.rekeyConcealedShare * 100).toFixed(2)}% of audio concealed across both`);

  // ---- 4. Pairwise verification: digits, QR code, the scanner -----------------------
  await bob.locator('.e2ee-pill').click();
  await bob.locator(`.e2ee-panel .e2ee-device:has-text("qa_e2ee_alice") [data-e2ee="verify"]`).click();
  await bob.waitForSelector('.e2ee-verify-dialog svg.e2ee-qr');
  const qr = await bob.evaluate(() => {
    const svg = document.querySelector('.e2ee-verify-dialog svg.e2ee-qr');
    return { d: svg.querySelector('.e2ee-qr-dark').getAttribute('d'), size: Number(svg.getAttribute('viewBox').split(' ')[2]) };
  });
  const bobNumber = await bob.locator('.e2ee-verify-dialog .e2ee-code').textContent();
  await shot(bob, 'verify');
  await alice.evaluate((src) => { window.__scanImage = src; }, qr);
  await alice.locator('.e2ee-pill').click();
  await alice.locator(`.e2ee-panel .e2ee-device:has-text("qa_e2ee_bob") [data-e2ee="verify"]`).click();
  await alice.waitForSelector('.e2ee-verify-dialog .e2ee-code');
  assert.equal(await alice.locator('.e2ee-verify-dialog .e2ee-code').textContent(), bobNumber, 'the safety number is the same on both sides');
  await alice.locator('.e2ee-verify-dialog [data-e2ee="scan"]').click();
  await alice.waitForSelector('.e2ee-verify-dialog [role="status"]:has-text("The keys match")', { timeout: 20000 });
  await alice.evaluate(() => { delete window.__scanImage; });
  await shot(alice, 'verify-scanned');
  await alice.locator('.e2ee-verify-dialog button:has-text("Close")').click();
  await bob.locator('.e2ee-verify-dialog [data-e2ee="mark"]').click();
  await bob.locator('.e2ee-verify-dialog button:has-text("Close")').click();
  await bob.keyboard.press('Escape').catch(() => {});
  await alice.keyboard.press('Escape').catch(() => {});
  await until(async () => Object.values((await enc(alice)).flags).some((f) => f.verified), 10000);
  await alice.waitForSelector('.participant-tile .tile-e2ee.is-verified');
  checks.push('pairwise verification: same 30-digit number on both sides; alice scanned bob\'s QR code with the in-app scanner and the keys matched; bob confirmed the digits; verified marks on tiles');

  // ---- 5. Reconnect: same device, same key, still verified ---------------------------
  await bob.reload();
  await bob.waitForSelector('.live-home, #live-input');
  await joinVoice(bob, voice);
  await connected(bob);
  await until(async () => (await enc(alice)).state.members.some((m) => m.user === B && m.hasKey), 20000);
  await until(async () => Object.values((await enc(alice)).flags).filter((f) => f.verified).length >= 1, 10000);
  const flagsAfter = Object.entries((await enc(alice)).flags).filter(([k]) => k.startsWith(`${B}:`));
  assert.ok(flagsAfter.some(([, f]) => f.verified && !f.changed), 'bob reconnected with the same key: still verified');
  checks.push('reconnect: a new voice session, the same device key, still verified; the others rekeyed for the old session and ratcheted for the new');

  // ---- 6. Key change: a verified person's key is different -----------------------------
  await act(bob, 'media:leave');
  await until(async () => !(await diag(bob)).joined);
  await bob.evaluate(() => new Promise((resolve) => { const r = indexedDB.deleteDatabase('sticktalk-e2ee'); r.onsuccess = r.onerror = r.onblocked = resolve; }));
  await bob.reload();
  await bob.waitForSelector('.live-home, #live-input');
  await joinVoice(bob, voice);
  await connected(bob);
  await until(async () => Object.entries((await enc(alice)).flags).some(([k, f]) => k.startsWith(`${B}:`) && f.changed), 20000);
  await alice.waitForSelector('.e2ee-banner.is-warn');
  const warning = await alice.locator('.e2ee-banner.is-warn').first().textContent();
  assert.match(warning, /key changed/);
  assert.equal((await alice.locator('.e2ee-pill').textContent()).trim(), 'Check encryption');
  await shot(alice, 'key-changed');
  checks.push(`key change: "${warning.trim()}" until verified again; never accepted silently`);

  // ---- 7. A client that cannot encrypt: refused, then allowed with everyone told -------
  await joinVoice(dave, voice);
  await dave.waitForSelector('.live-error:has-text("end-to-end encrypted")', { timeout: 20000 });
  await until(async () => !(await diag(dave)).joined);
  await shot(dave, 'refused');
  checks.push('a client without encoded transforms is refused from an encrypted call with a clear message; it never gets media');
  await alice.locator('.e2ee-pill').click();
  await alice.waitForSelector('#e2ee-allow');
  await shot(alice, 'panel-manager');
  await alice.locator('#e2ee-allow').check();
  await until(async () => alice.evaluate(() => { const i = document.querySelector('#e2ee-allow'); return i?.checked && !i.disabled; }), 10000);
  await until(async () => (await enc(alice)).state && true, 1000);
  await alice.keyboard.press('Escape');
  await joinVoice(dave, voice);
  await connected(dave);
  await until(async () => (await enc(alice)).state?.encrypted === false, 20000);
  await alice.waitForSelector('.e2ee-banner.is-off');
  const off = await alice.locator('.e2ee-banner.is-off').textContent();
  assert.match(off, /qa_e2ee_dave’s app can’t encrypt calls/);
  await dave.waitForSelector('.e2ee-banner.is-off');
  await shot(alice, 'unencrypted-fallback');
  await shot(dave, 'unencrypted-unsupported-client');
  const daveHeard = await heard(alice, D);
  assert.ok(Math.abs(daveHeard.hz - 2000) < 20, `in the fallback call alice hears dave: ${JSON.stringify(daveHeard)}`);
  // Positive control: what the SFU forwards in an unencrypted call does decode.
  await wait(3000);
  const daveAudio = (await tapped(tap, D, 'audio')).flatMap((f) => f.packets);
  numbers.sfuDecodePlainAudio = await decodeAudio(alice, daveAudio.filter((x) => x.payload.length).slice(-200), 2000);
  assert.ok(numbers.sfuDecodePlainAudio.toneShare > 0.5, `control: the SFU's copy of unencrypted audio decodes: ${JSON.stringify(numbers.sfuDecodePlainAudio)}`);
  await act(dave, 'media:share');
  await dave.waitForSelector('.live-share-options .button.primary');
  await dave.locator('.live-share-options .button.primary').click();
  await until(async () => (await diag(alice)).tracks.some((t) => t.kind === 'screen' && t.videoWidth > 0), 30000);
  await wait(4000);
  numbers.sfuDecodePlainScreen = await decodeVideo(alice, accessUnits((await tapped(tap, D, 'screen')).flatMap((f) => f.packets)).slice(0, 240));
  assert.ok(numbers.sfuDecodePlainScreen.frames > 30, `control: the SFU's copy of an unencrypted share decodes: ${JSON.stringify(numbers.sfuDecodePlainScreen)}`);
  assert.ok(numbers.sfuDecodeEncryptedScreen.frames < numbers.sfuDecodePlainScreen.frames / 4 || numbers.sfuDecodeEncryptedScreen.errors > 0,
    `the encrypted share does not decode like the plain one: ${JSON.stringify(numbers.sfuDecodeEncryptedScreen)}`);
  checks.push(`with "Allow unencrypted calls" on, the client joins and everyone sees "${off.trim()}"; control: the SFU's copy of that unencrypted audio decodes to ${(numbers.sfuDecodePlainAudio.toneShare * 100).toFixed(1)}% tone and its share to ${numbers.sfuDecodePlainScreen.frames} frames`);

  // ---- 8. "Only join end-to-end encrypted calls" ------------------------------------
  await act(dave, 'media:leave');
  await until(async () => (await enc(alice)).state?.encrypted === true, 20000);
  await bob.evaluate(() => localStorage.setItem('sticktalk-e2ee', JSON.stringify({ require: true })));
  await joinVoice(dave, voice);
  await bob.waitForSelector('.live-error:has-text("you chose to join only encrypted calls")', { timeout: 20000 });
  await until(async () => !(await diag(bob)).joined, 10000);
  checks.push('with "Only join end-to-end encrypted calls" on, a call that becomes unencrypted is left at once, with the reason');
  await act(dave, 'media:leave');
  await bob.evaluate(() => localStorage.removeItem('sticktalk-e2ee'));

  // ---- 9. Leaving releases everything ----------------------------------------------
  await act(alice, 'media:leave');
  await until(async () => !(await diag(alice)).joined);
  assert.equal((await diag(alice)).e2ee, null, 'the worker and keys are gone with the call');
  checks.push('leaving the call terminates the encryption worker and drops every call key');
  assert.deepEqual(errors, [], 'no page errors');
  await writeFile(join(shots, 'e2ee-results.json'), JSON.stringify({ revision: h.revision, checks, numbers }, null, 2));
  console.log(JSON.stringify({ checks, numbers }, null, 2));
} catch (e) {
  for (const [name, p] of [['alice', alice], ['bob', bob], ['carol', carol], ['dave', dave]])
    if (p) await p.screenshot({ path: join(shots, `failure-${name}.png`) }).catch(() => {});
  console.error(JSON.stringify({ checks, numbers, errors }, null, 2));
  throw e;
} finally {
  await h.close();
  await rm(tap, { recursive: true, force: true });
}

// =====================================================================================
// Measurements: the same two-person call, encrypted and not (both clients without
// encoded transforms, in a server that allows that: no transform runs at all),
// with and without 5% packet loss injected by the SFU tap.
delete process.env.STICKTALK_MEDIA_TAP;
async function cpu(browser) {
  const cdp = await browser.newBrowserCDPSession();
  const { processInfo } = await cdp.send('SystemInfo.getProcessInfo');
  await cdp.detach();
  return processInfo.filter((p) => p.type !== 'browser').reduce((n, p) => n + p.cpuTime, 0);
}
async function rate(p, kind, seconds) {
  const sum = (d) => d.stats.filter((r) => r.type === 'outbound-rtp' && r.kind === kind).reduce((n, r) => n + (r.bytesSent ?? 0), 0);
  const a = sum(await diag(p)), t = Date.now();
  await wait(seconds * 1000);
  return Math.round(((sum(await diag(p)) - a) * 8) / ((Date.now() - t) / 1000));
}
async function measure(encrypted, loss) {
  process.env.STICKTALK_MEDIA_LOSS_PERMILLE = String(loss);
  const m = await harness();
  const run = `${encrypted ? 'e' : 'p'}${loss}`;
  const out = { encrypted, loss: loss / 10 };
  try {
    const p1 = await page(m, 1000, { unsupported: !encrypted });
    const p2 = await page(m, 1500, { unsupported: !encrypted });
    await register(p1, m.origin, `qa_m_${run}_1`);
    await register(p2, m.origin, `qa_m_${run}_2`);
    await act(p1, 'dialog:create');
    await p1.waitForSelector('[data-live-form="create"]');
    await p1.locator('#f-name').fill('Bench');
    await p1.locator('[data-live-form] [type=submit]').click();
    await p1.waitForSelector('#live-input');
    await p1.waitForFunction(() => !document.querySelector('.live-modal'));
    await act(p1, 'dialog:invite');
    await p1.waitForSelector('[data-live-form="invite"]');
    await p1.locator('[data-live-form="invite"] [type=submit]').click();
    await p1.waitForSelector('.live-modal code');
    const invite = await p1.locator('.live-modal code').first().textContent();
    await act(p1, 'close');
    await act(p2, 'dialog:join');
    await p2.waitForSelector('[data-live-form="join"]');
    await p2.locator('#f-name').fill(invite);
    await p2.locator('[data-live-form] [type=submit]').click();
    await p2.waitForSelector('#live-input');
    const voice = await p1.locator('[data-live^="voice:"]').first().getAttribute('data-live');
    if (!encrypted) {
      const guild = await p1.evaluate(async () => {
        const csrf = document.cookie.split(';').map((v) => v.trim()).find((v) => v.startsWith('__Host-sticktalk_csrf='))?.slice(22) ?? '';
        const g = document.querySelector('[data-live^="guild:"][aria-current], [data-live^="guild:"].is-active')?.dataset.live?.slice(6)
          ?? [...document.querySelectorAll('[data-live^="guild:"]')].map((e) => e.dataset.live.slice(6)).at(-1);
        const r = await fetch(`/api/v1/guilds/${g}/e2ee`, { method: 'PUT', headers: { 'Content-Type': 'application/json', 'X-StickTalk-CSRF': csrf }, body: '{"allow_unencrypted":true}' });
        return r.status;
      });
      assert.equal(guild, 200, 'the bench server allows unencrypted calls');
    }
    await joinVoice(p1, voice);
    await connected(p1);
    // Time to first audio: from clicking Join to hearing the other person (energy at the receiver).
    const t0 = Date.now();
    await joinVoice(p2, voice);
    await until(async () => (await diag(p2)).stats.some((r) => r.type === 'inbound-rtp' && r.kind === 'audio' && (r.totalAudioEnergy ?? 0) > 1e-4), 30000);
    out.firstAudioMs = Date.now() - t0;
    await wait(3000);
    if (encrypted) for (const p of [p1, p2]) await p.evaluate(async () => (await import('/public/e2ee.js')).resetCounters());
    const a0 = await inboundAudio(p2), c0 = await cpu(m.browser), w0 = Date.now();
    out.audioBps = await rate(p1, 'audio', 8);
    const c1 = await cpu(m.browser), a1 = await inboundAudio(p2);
    out.cpuAudioPercentPerParticipant = +(((c1 - c0) / ((Date.now() - w0) / 1000)) * 100 / 2).toFixed(2);
    out.audioConcealedShare = +((a1.concealed - a0.concealed) / Math.max(1, a1.samples - a0.samples)).toFixed(4);
    if (encrypted) out.audioTransformMs = (await enc(p1)).send.audio ?? null, out.audioReceiveMs = (await enc(p2)).receive.audio ?? null;
    // A screen share.
    await act(p1, 'media:share');
    await p1.locator('.live-share-options .button.primary').click();
    await until(async () => (await diag(p2)).tracks.some((t) => t.kind === 'screen' && t.videoWidth > 0), 30000);
    await wait(5000);
    if (encrypted) for (const p of [p1, p2]) await p.evaluate(async () => (await import('/public/e2ee.js')).resetCounters());
    const v0 = await videoIn(p2), c2 = await cpu(m.browser), w1 = Date.now();
    out.videoBps = await rate(p1, 'video', 10);
    const c3 = await cpu(m.browser), v1 = await videoIn(p2), seconds = (Date.now() - w1) / 1000;
    out.cpuScreenPercentPerParticipant = +(((c3 - c2) / seconds) * 100 / 2).toFixed(2);
    out.viewerFps = +((v1.frames - v0.frames) / seconds).toFixed(1);
    out.viewerFreezes = v1.freezes - v0.freezes;
    out.viewerFrozenSeconds = +(v1.frozen - v0.frozen).toFixed(2);
    if (encrypted) {
      const s1 = await enc(p1), s2 = await enc(p2);
      out.screenTransformMs = s1.send.screen ?? null;
      out.screenReceiveMs = s2.receive.screen ?? null;
      out.retries = s1.counters.retries;
      out.drops = { auth: s2.counters.droppedAuth, noKey: s2.counters.droppedNoKey, replay: s2.counters.droppedReplay };
    }
    assert.deepEqual(errors, [], 'no page errors');
  } finally {
    await m.close();
  }
  return out;
}
if (!process.env.STICKTALK_E2EE_SKIP_MEASURE) {
  numbers.measure = [];
  for (const [encrypted, loss] of [[false, 0], [true, 0], [false, 50], [true, 50]]) numbers.measure.push(await measure(encrypted, loss));
  await writeFile(join(shots, 'e2ee-results.json'), JSON.stringify({ revision: h.revision, checks, numbers }, null, 2));
  console.log(JSON.stringify(numbers.measure, null, 2));
}

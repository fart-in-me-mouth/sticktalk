// End-to-end encrypted calls across engines: Chromium and Firefox in one
// encrypted voice call (audio). Headless, fake media, loopback only; Firefox
// reaches nothing but 127.0.0.1 (a rejecting proxy). Needs a Playwright Firefox:
//   $env:PLAYWRIGHT_BROWSERS_PATH=<task dir>\browsers; npx playwright install firefox
import assert from 'node:assert/strict';
import { createServer } from 'node:net';
import { once } from 'node:events';
import { firefox } from 'playwright';
import { harness, register, act, until, wait } from './qa-harness.mjs';

const tone = (freq) => {
  navigator.mediaDevices.getUserMedia = async () => {
    const ctx = new AudioContext({ sampleRate: 48000 }), out = ctx.createMediaStreamDestination();
    const o = ctx.createOscillator(), g = ctx.createGain();
    o.frequency.value = freq; g.gain.value = 0.25;
    o.connect(g).connect(out); o.start();
    await ctx.resume();
    return out.stream;
  };
};
const diag = (p) => p.evaluate(async () => await (await import('/public/media.js')).diagnostics());
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
    await new Promise((r) => setTimeout(r, 1000));
    const bins = new Float32Array(an.frequencyBinCount);
    an.getFloatFrequencyData(bins);
    let best = 1;
    for (let i = 1; i < bins.length; i++) if (bins[i] > bins[best]) best = i;
    const out = { hz: Math.round((best * ctx.sampleRate) / an.fftSize), db: Math.round(bins[best]) };
    ctx.close();
    return out;
  }, userId);
}

const h = await harness();
const deny = createServer((s) => s.destroy());
deny.listen(0, '127.0.0.1');
await once(deny, 'listening');
const fx = await firefox.launch({
  headless: true,
  executablePath: process.env.STICKTALK_FIREFOX_EXE,
  proxy: { server: `http://127.0.0.1:${deny.address().port}`, bypass: '127.0.0.1,localhost,[::1]' },
  firefoxUserPrefs: {
    'media.navigator.streams.fake': true, 'media.navigator.permission.disabled': true,
    'permissions.default.microphone': 1, 'permissions.default.camera': 1,
    'media.peerconnection.ice.loopback': true, 'media.peerconnection.ice.default_address_only': false,
    'media.peerconnection.ice.obfuscate_host_addresses': false,
    'media.volume_scale': '0.0', 'dom.webnotifications.enabled': false,
  },
});
const results = {};
const pages = {};
try {
  const alice = await h.phone(1440, 900);
  await alice.addInitScript(tone, 1000);
  const bob = await (await fx.newContext({ viewport: { width: 1440, height: 900 } })).newPage();
  await bob.addInitScript(tone, 1500);
  Object.assign(pages, { alice, bob });
  bob.on('pageerror', (e) => (results.bobPageErrors ??= []).push(String(e)));
  bob.on('console', (m) => { if (m.type() === 'error') (results.bobConsole ??= []).push(m.text().slice(0, 300)); });
  results.firefoxVersion = fx.version();
  results.firefoxHasScriptTransform = await bob.evaluate(() => 'RTCRtpScriptTransform' in globalThis);
  results.firefoxHasEncodedStreams = await bob.evaluate(() => 'createEncodedStreams' in RTCRtpSender.prototype);
  await register(alice, h.origin, 'qa_fx_alice');
  await register(bob, h.origin, 'qa_fx_bob');
  await act(alice, 'dialog:create');
  await alice.waitForSelector('[data-live-form="create"]');
  await alice.locator('#f-name').fill('Engines');
  await alice.locator('[data-live-form] [type=submit]').click();
  await alice.waitForSelector('#live-input');
  await alice.waitForFunction(() => !document.querySelector('.live-modal'));
  await act(alice, 'dialog:invite');
  await alice.waitForSelector('[data-live-form="invite"]');
  await alice.locator('[data-live-form="invite"] [type=submit]').click();
  await alice.waitForSelector('.live-modal code');
  const code = await alice.locator('.live-modal code').first().textContent();
  await act(alice, 'close');
  await act(bob, 'dialog:join');
  await bob.waitForSelector('[data-live-form="join"]');
  await bob.locator('#f-name').fill(code);
  await bob.locator('[data-live-form] [type=submit]').click();
  await bob.waitForSelector('#live-input');
  const voice = await alice.locator('[data-live^="voice:"]').first().getAttribute('data-live');
  await alice.locator(`[data-live="${voice}"]`).first().click();
  await bob.locator(`[data-live="${voice}"]`).first().click();
  await until(async () => (await Promise.all([diag(alice), diag(bob)])).every((d) => d.state === 'connected'), 60000);
  await until(async () => {
    const [a, b] = await Promise.all([diag(alice), diag(bob)]);
    return a.e2ee?.state?.encrypted && b.e2ee?.state?.encrypted && a.e2ee.counters.received > 100 && b.e2ee.counters.received > 100;
  }, 45000);
  const [a, b] = await Promise.all([diag(alice), diag(bob)]);
  results.firefoxMode = b.e2ee.mode;
  results.sameCode = a.e2ee.state.code === b.e2ee.state.code;
  const [A, B] = await Promise.all([alice, bob].map((p) => p.evaluate(() => document.querySelector('#sticktalk')?.dataset.selfId)));
  results.chromiumHearsFirefox = await heard(alice, B);
  results.firefoxHearsChromium = await heard(bob, A);
  results.drops = { chromium: a.e2ee.counters.droppedAuth, firefox: b.e2ee.counters.droppedAuth };
  assert.ok(results.sameCode);
  assert.ok(Math.abs(results.chromiumHearsFirefox.hz - 1500) < 20, JSON.stringify(results));
  assert.ok(Math.abs(results.firefoxHearsChromium.hz - 1000) < 20, JSON.stringify(results));
  await act(bob, 'media:leave');
  await act(alice, 'media:leave');
  await wait(500);
  console.log(JSON.stringify(results, null, 2));
} catch (e) {
  for (const [name, p] of Object.entries(pages)) {
    const d = await diag(p).catch((x) => ({ failed: String(x) }));
    results[name] = { state: d.state, joined: d.joined, trace: d.trace?.slice(-12), e2ee: d.e2ee && { mode: d.e2ee.mode, encrypted: d.e2ee.state?.encrypted, counters: d.e2ee.counters }, errors: await p.evaluate(() => [document.querySelector('.live-error')?.textContent, ...(window.__qaErrors ?? [])]).catch(() => null) };
  }
  console.error(JSON.stringify(results, null, 2));
  throw e;
} finally {
  await fx.close();
  deny.close();
  await h.close();
}

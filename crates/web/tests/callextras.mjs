// Call extras in the real web app against a real local server: soundboard,
// entrance sounds, voice bitrate, stream quality, camera backgrounds and clips.
// Headless Chromium with fake media devices, OS audio muted, loopback only.
// Soundboard playback is recorded by a silent hook (globalThis.__sticktalkSoundboard).
import assert from 'node:assert/strict';
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { harness, register, act, until, wait, output } from './qa-harness.mjs';

const shots = process.env.STICKTALK_CALLEXTRAS_SHOTS ?? join(output, 'callextras');
await mkdir(shots, { recursive: true });
const h = await harness();
const checks = [], errors = [], numbers = {};
const diagnostic = (p) => p.evaluate(async () => await (await import('/public/media.js')).diagnostics());
const extras = async (p) => (await diagnostic(p)).extras;
async function shot(page, name, sizes = [[1440, 900], [390, 844]]) {
  const before = page.viewportSize();
  for (const [width, height] of sizes) {
    await page.setViewportSize({ width, height });
    await page.evaluate(() => document.fonts.ready);
    await wait(200);
    await page.screenshot({ path: join(shots, `${name}-${width}x${height}.png`) });
  }
  await page.setViewportSize(before);
}
/** A one-second 16 kHz WAV tone. */
function wav(ms = 1000) {
  const rate = 16000, n = (rate * ms) / 1000, b = Buffer.alloc(44 + n * 2);
  b.write('RIFF', 0); b.writeUInt32LE(36 + n * 2, 4); b.write('WAVEfmt ', 8);
  b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22);
  b.writeUInt32LE(rate, 24); b.writeUInt32LE(rate * 2, 28); b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34);
  b.write('data', 36); b.writeUInt32LE(n * 2, 40);
  for (let i = 0; i < n; i++) b.writeInt16LE(Math.round(Math.sin(i / 8) * 6000), 44 + i * 2);
  return b;
}
async function page(width = 1440, height = 900) {
  const p = await h.phone(width, height);
  await p.addInitScript(() => {
    window.__sounds = [];
    globalThis.__sticktalkSoundboard = (s) => window.__sounds.push(s);
    globalThis.__sticktalkConfirm = () => true;
    // A rich microphone (noise and tones), so Opus has something to spend bits on.
    const original = navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);
    navigator.mediaDevices.getUserMedia = async (c) => {
      if (!c?.audio || c.video) return original(c);
      const ctx = new AudioContext({ sampleRate: 48000 }), out = ctx.createMediaStreamDestination();
      // A microphone is mono.
      out.channelCount = 1;
      const noise = ctx.createBuffer(1, 48000 * 2, 48000), data = noise.getChannelData(0);
      for (let i = 0; i < data.length; i++) data[i] = (Math.random() * 2 - 1) * 0.3;
      const src = ctx.createBufferSource();
      src.buffer = noise; src.loop = true; src.connect(out); src.start();
      for (const f of [220, 440, 880, 1760]) { const o = ctx.createOscillator(); o.frequency.value = f; const g = ctx.createGain(); g.gain.value = 0.05; o.connect(g).connect(out); o.start(); }
      await ctx.resume();
      return out.stream;
    };
    // Headless Chromium has no screen picker: a drawn 1080p picture that moves.
    navigator.mediaDevices.getDisplayMedia = async (constraints) => {
      window.__displayConstraints = constraints;
      const canvas = document.createElement('canvas');
      canvas.width = 1920; canvas.height = 1080;
      const g = canvas.getContext('2d');
      let n = 0;
      const draw = () => {
        n++;
        g.fillStyle = `hsl(${(n * 3) % 360} 60% 40%)`; g.fillRect(0, 0, 1920, 1080);
        for (let i = 0; i < 40; i++) { g.fillStyle = `hsl(${(i * 37 + n * 5) % 360} 80% 55%)`; g.fillRect((i * 97 + n * 11) % 1920, (i * 53 + n * 7) % 1080, 160, 120); }
        g.fillStyle = '#fff'; g.font = '48px sans-serif'; g.fillText(`frame ${n} — sharper text check`, 80, 120);
      };
      const loop = () => { draw(); requestAnimationFrame(loop); };
      requestAnimationFrame(loop);
      const [picture] = canvas.captureStream(60).getVideoTracks();
      const settings = picture.getSettings.bind(picture);
      picture.getSettings = () => ({ ...settings(), displaySurface: 'monitor' });
      return new MediaStream([picture]);
    };
  });
  p.on('pageerror', (e) => errors.push(e.stack ?? String(e)));
  return p;
}
async function connected(...pages) {
  await until(async () => (await Promise.all(pages.map(diagnostic))).every((d) => d.state === 'connected'), 45000);
}
/** Outbound bits per second over a window, from the browser's own RTP stats. */
async function sendRate(p, kind, seconds = 4) {
  const bytes = (d) => d.stats.filter((r) => r.type === 'outbound-rtp' && r.kind === kind).reduce((n, r) => n + (r.bytesSent ?? 0), 0);
  const a = bytes(await diagnostic(p)), t0 = Date.now();
  await wait(seconds * 1000);
  const b = bytes(await diagnostic(p));
  return Math.round(((b - a) * 8) / ((Date.now() - t0) / 1000));
}
async function inbound(p, seconds = 4) {
  const row = (d) => d.stats.filter((r) => r.type === 'inbound-rtp' && r.kind === 'video').sort((x, y) => (y.bytesReceived ?? 0) - (x.bytesReceived ?? 0))[0];
  const a = row(await diagnostic(p)), t0 = Date.now();
  await wait(seconds * 1000);
  const b = row(await diagnostic(p));
  return { width: b?.frameWidth, height: b?.frameHeight, fps: Math.round(b?.framesPerSecond ?? 0), bps: Math.round((((b?.bytesReceived ?? 0) - (a?.bytesReceived ?? 0)) * 8) / ((Date.now() - t0) / 1000)) };
}

let alice, bob;
try {
  alice = await page();
  bob = await page();
  await register(alice, h.origin, 'qa_sb_alice');
  await register(bob, h.origin, 'qa_sb_bob');
  // A server, and Bob in it.
  await act(alice, 'dialog:create');
  await alice.waitForSelector('[data-live-form="create"]');
  await alice.locator('#f-name').fill('Sound Lab');
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
  const voiceId = voice.slice('voice:'.length);

  // ---- Voice bitrate: before (the browser's own Opus default), then the channel's ---
  await bob.evaluate(() => { globalThis.__sticktalkLegacyQuality = true; });
  await alice.locator(`[data-live="${voice}"]`).click();
  await bob.locator(`[data-live="${voice}"]`).click();
  await connected(alice, bob);
  await wait(2000);
  numbers.audioBefore = await sendRate(bob, 'audio');
  await bob.evaluate(() => { globalThis.__sticktalkLegacyQuality = false; });
  await until(async () => (await extras(alice)).audioBitrate === 64000);
  await until(async () => (await extras(alice)).answeredBitrate === 64000, 15000);
  await wait(2500);
  numbers.audioChannelDefault = await sendRate(alice, 'audio');
  checks.push(`voice bitrate: the call tells every encoder its channel's bitrate (default 64 kbps; measured ${numbers.audioChannelDefault} b/s vs ${numbers.audioBefore} b/s with the browser's default)`);

  // ---- The soundboard: add a sound from the panel ---------------------------------
  await alice.locator('.call-controls [aria-label="Soundboard"]').click();
  await alice.waitForSelector('.soundboard-panel .soundboard-add');
  await alice.locator('.soundboard-panel .soundboard-add').first().click();
  await alice.waitForSelector('.soundboard-editor input[type="file"]');
  await alice.locator('.soundboard-editor input[type="file"]').setInputFiles({ name: 'airhorn.wav', mimeType: 'audio/wav', buffer: wav(1000) });
  await alice.locator('.soundboard-editor input[type="text"]').nth(1).fill('🎺');
  await shot(alice, 'soundboard-add-sound');
  await alice.locator('.soundboard-editor [type=submit]').click();
  await alice.waitForSelector('.soundboard-panel .soundboard-sound:has-text("airhorn")');
  // A clip that is too long is refused with the reason.
  await alice.locator('.soundboard-panel .soundboard-add').first().click();
  await alice.locator('.soundboard-editor input[type="file"]').setInputFiles({ name: 'long.wav', mimeType: 'audio/wav', buffer: wav(6000) });
  await alice.locator('.soundboard-editor [type=submit]').click();
  await alice.waitForSelector('.soundboard-editor [role=status]:has-text("5.2 seconds")');
  await alice.locator('.soundboard-editor button:has-text("Cancel")').click();
  checks.push('add a sound from the soundboard panel; a too-long clip is refused with its reason');

  // ---- Play it: everyone in the call hears it, and sees who played it --------------
  await bob.locator('.call-controls [aria-label="Soundboard"]').click();
  await bob.waitForSelector('.soundboard-panel .soundboard-sound:has-text("airhorn")');
  assert.equal(await bob.locator('.soundboard-panel .soundboard-edit').count(), 0, 'members cannot edit sounds');
  await shot(bob, 'soundboard-panel');
  await alice.locator('.soundboard-panel .soundboard-sound:has-text("airhorn")').click();
  await until(async () => (await bob.evaluate(() => window.__sounds.length)) === 1);
  await until(async () => (await alice.evaluate(() => window.__sounds.length)) === 1);
  const heard = await bob.evaluate(() => window.__sounds[0]);
  assert.equal(heard.entrance, false);
  assert(Math.abs(heard.gain - 1) < 0.01, `default volume: ${heard.gain}`);
  await bob.locator('.soundboard-panel button[aria-label="Close"], .soundboard-panel button:has-text("Close")').last().click();
  await bob.waitForSelector('.call-toast:has-text("qa_sb_alice played airhorn")');
  await shot(bob, 'soundboard-played-toast');
  // A favorite, then search.
  await bob.locator('.call-controls [aria-label="Soundboard"]').click();
  await bob.locator('.soundboard-panel .soundboard-star').first().click();
  await bob.waitForSelector('.soundboard-panel h3:has-text("Favorites")');
  await bob.locator('.soundboard-panel input[type=search]').fill('zzz');
  await bob.waitForSelector('.soundboard-panel .soundboard-status:has-text("No sounds match")');
  await bob.locator('.soundboard-panel input[type=search]').fill('air');
  // His own volume and mute: half volume, then nothing; deafened, nothing.
  await bob.locator('.soundboard-settings summary').click();
  await bob.locator('.soundboard-settings input[type=range]').evaluate((e) => { e.value = '50'; e.dispatchEvent(new Event('input')); });
  await alice.locator('.soundboard-panel .soundboard-sound:has-text("airhorn")').click();
  await until(async () => (await bob.evaluate(() => window.__sounds.length)) === 2);
  assert(Math.abs((await bob.evaluate(() => window.__sounds[1].gain)) - 0.5) < 0.01);
  await bob.locator('.soundboard-settings .live-checkbox:has-text("Mute Soundboard") input').check();
  await wait(1000);
  await alice.locator('.soundboard-panel .soundboard-sound:has-text("airhorn")').click();
  await wait(1500);
  assert.equal(await bob.evaluate(() => window.__sounds.length), 2, 'muted soundboard plays nothing');
  await bob.locator('.soundboard-settings .live-checkbox:has-text("Mute Soundboard") input').uncheck();
  await bob.locator('.soundboard-panel button:has-text("Close")').last().click();
  await act(bob, 'media:deafen');
  await wait(1000);
  await alice.locator('.soundboard-panel .soundboard-sound:has-text("airhorn")').click();
  await wait(1500);
  assert.equal(await bob.evaluate(() => window.__sounds.length), 2, 'deafened hears nothing');
  await act(bob, 'media:deafen');
  checks.push('a sound is heard by everyone in the call with who played it; listener volume, mute and deafen are respected; favorites and search');

  // The real speaker path once (OS audio is muted): decode, play, then suspend.
  await alice.evaluate(() => { delete globalThis.__sticktalkSoundboard; });
  await wait(1200);
  await alice.locator('.soundboard-panel .soundboard-sound:has-text("airhorn")').click();
  await until(async () => (await extras(alice)).cachedSounds === 1);
  await until(async () => (await extras(alice)).soundContext === 'suspended', 10000);
  await alice.evaluate(() => { globalThis.__sticktalkSoundboard = (s) => window.__sounds.push(s); });
  checks.push('the clip is decoded and played locally through the call output; the audio context suspends when it ends');

  // Managers rename and change a sound.
  await alice.locator('.soundboard-panel .soundboard-edit').first().click();
  await alice.waitForSelector('.soundboard-editor');
  await alice.locator('.soundboard-editor input[type="text"]').first().fill('air horn');
  await alice.locator('.soundboard-editor input[type="range"]').evaluate((e) => { e.value = '40'; e.dispatchEvent(new Event('input')); });
  await shot(alice, 'soundboard-edit-sound');
  await alice.locator('.soundboard-editor [type=submit]').click();
  await alice.waitForSelector('.soundboard-panel .soundboard-sound:has-text("air horn")');
  await alice.locator('.soundboard-panel button:has-text("Close")').last().click();
  checks.push('rename and change the volume of a sound (Manage Soundboard)');

  // ---- Entrance sound: Bob picks one, rejoins, and Alice hears it ------------------
  await act(bob, 'dialog:media-settings');
  await bob.waitForSelector('.entrance-settings select');
  const soundId = await bob.locator('.entrance-settings select').first().evaluate((s) => [...s.options].find((o) => o.textContent.includes('air horn')).value);
  await bob.locator('.entrance-settings select').first().selectOption(soundId);
  await bob.waitForSelector('.entrance-settings [role=status]:has-text("Saved")');
  await bob.locator('#live-call-extras-settings').scrollIntoViewIfNeeded();
  await shot(bob, 'voice-video-call-extras-settings');
  await act(bob, 'close');
  const before = await alice.evaluate(() => window.__sounds.length);
  await act(bob, 'media:leave');
  await wait(500);
  await bob.locator(`[data-live="${voice}"]`).click();
  await connected(bob);
  await until(async () => (await alice.evaluate(() => window.__sounds.length)) === before + 1, 15000);
  assert.equal(await alice.evaluate(() => window.__sounds.at(-1).entrance), true);
  // Rejoining at once does not play it again.
  await act(bob, 'media:leave');
  await wait(500);
  await bob.locator(`[data-live="${voice}"]`).click();
  await connected(bob);
  await wait(2500);
  assert.equal(await alice.evaluate(() => window.__sounds.length), before + 1, 'one entrance sound a minute');
  checks.push('entrance sound plays for others on join, at most once a minute');

  // ---- Channel bitrate in Edit Channel ------------------------------------------------
  await act(alice, `dialog:channel-edit:${voiceId}`);
  await alice.waitForSelector('[data-voice-bitrate] input[type=range]:not([disabled])');
  assert.equal(await alice.locator('[data-voice-bitrate] output').textContent(), '64 kbps');
  await alice.locator('[data-voice-bitrate] input[type=range]').evaluate((e) => { e.value = '256'; e.dispatchEvent(new Event('input')); });
  await alice.locator('[data-voice-bitrate] button:has-text("Save Bitrate")').click();
  await alice.waitForSelector('[data-voice-bitrate] [role=status]:has-text("Saved")');
  await shot(alice, 'channel-voice-bitrate');
  await act(alice, 'close');
  await until(async () => (await extras(bob)).audioBitrate === 256000, 15000);
  await until(async () => (await extras(bob)).answeredBitrate === 256000, 15000);
  await wait(2500);
  numbers.audioChannel256 = await sendRate(bob, 'audio');
  checks.push(`a voice channel's bitrate is changed in Edit Channel and reaches live calls at once (256 kbps set; measured ${numbers.audioChannel256} b/s)`);

  // ---- Stream quality: before (the old 720p30 path) and 1080p60 -----------------------
  await bob.evaluate(() => { globalThis.__sticktalkLegacyQuality = true; });
  await act(bob, 'media:share');
  await bob.locator('.live-share-options .button.primary').click();
  await until(async () => (await diagnostic(alice)).tracks.some((t) => t.kind === 'screen' && t.videoWidth > 0), 30000);
  await wait(3000);
  numbers.screenBefore = { ...(await inbound(alice)), sent: await sendRate(bob, 'video', 3) };
  await act(bob, 'media:share');
  await until(async () => !(await diagnostic(alice)).tracks.some((t) => t.kind === 'screen'), 20000);
  await bob.evaluate(() => { globalThis.__sticktalkLegacyQuality = false; });
  await act(bob, 'media:share');
  await bob.waitForSelector('.live-share-options select[aria-label="Resolution"]');
  await bob.locator('.live-share-options select[aria-label="Resolution"]').selectOption('1080');
  await bob.locator('.live-share-options select[aria-label="Frame Rate"]').selectOption('60');
  await bob.locator('.live-share-options select[aria-label="Optimize For"]').selectOption('motion');
  await shot(bob, 'share-quality-picker');
  await bob.locator('.live-share-options .button.primary').click();
  assert.equal(await bob.evaluate(() => window.__displayConstraints.video.frameRate.ideal), 60);
  await until(async () => (await diagnostic(alice)).tracks.some((t) => t.kind === 'screen' && t.videoWidth > 0), 30000);
  await wait(5000);
  numbers.screen1080p60 = { ...(await inbound(alice)), sent: await sendRate(bob, 'video', 3) };
  await alice.waitForSelector('.participant-tile[data-kind="screen"] .tile-quality');
  const badge = await alice.locator('.participant-tile[data-kind="screen"] .tile-quality').textContent();
  assert.match(badge, /^\d+×\d+ · \d+ fps$/);
  await bob.waitForSelector('.participant-tile[data-kind="screen"][data-self] .tile-quality:has-text("Sending")');
  const encoder = await bob.evaluate(async () => {
    const d = await (await import('/public/media.js')).diagnostics();
    return d.stats.filter((r) => r.type === 'outbound-rtp' && r.kind === 'video').map((r) => ({ w: r.frameWidth, h: r.frameHeight, fps: r.framesPerSecond, limit: r.qualityLimitationReason, encoder: r.encoderImplementation }));
  });
  numbers.screenEncoder = encoder;
  await shot(alice, 'stream-delivered-quality');
  checks.push(`screen share quality picker; viewers see what is delivered ("${badge}"); 1080p60: ${JSON.stringify(numbers.screen1080p60)} vs old path ${JSON.stringify(numbers.screenBefore)}`);

  // ---- Clips: on, the call is told, save one, trim, keep, share ----------------------
  await act(bob, 'media:more');
  await bob.waitForSelector('.call-more-menu');
  await shot(bob, 'call-more-menu');
  await bob.locator('.call-more-menu button:has-text("Turn On Clips")').click();
  await bob.waitForSelector('.call-extras-dialog:has-text("Keep Clips of Streams")');
  await until(async () => (await extras(bob)).clips?.active === true, 20000);
  await alice.waitForSelector('#live-clipping-notice:has-text("qa_sb_bob")');
  await alice.waitForSelector('.participant-tile.is-clipping');
  await shot(bob, 'clips-settings');
  await bob.locator('.call-extras-dialog button:has-text("Done")').click();
  await shot(alice, 'clips-on-notice');
  await wait(4000);
  const buffered = (await extras(bob)).clips;
  assert(buffered.videoChunks > 30 && buffered.audioChunks > 50, JSON.stringify(buffered));
  numbers.clipBuffer = buffered;
  await bob.locator('.voice-view').click({ position: { x: 5, y: 5 } }).catch(() => {});
  await bob.keyboard.press('Alt+KeyC');
  await bob.waitForSelector('.clip-editor .clip-readout:has-text("MB")');
  await alice.waitForSelector('.call-toast:has-text("qa_sb_bob saved a clip")');
  // The MP4 written here plays: the browser reads its duration.
  await until(async () => (await bob.locator('.clip-editor video').evaluate((v) => v.duration)) > 1, 15000);
  numbers.clipSeconds = await bob.locator('.clip-editor video').evaluate((v) => v.duration);
  await bob.locator('.clip-editor input[aria-label="Clip start"]').evaluate((e) => { e.value = '10'; e.dispatchEvent(new Event('input')); });
  await wait(600);
  await shot(bob, 'clip-editor');
  await bob.locator('.clip-editor button:has-text("Save to Library")').click();
  await bob.waitForSelector('.clip-editor [role=status]:has-text("Saved to your clip library")');
  await bob.locator('.clip-editor button:has-text("Share to Chat")').click();
  await bob.waitForSelector('.call-chat .composer-attachment, .call-chat [data-upload], .call-chat .upload', { timeout: 20000 }).catch(async () => {
    await bob.waitForFunction(() => document.querySelector('.call-chat')?.textContent.includes('.mp4'), null, { timeout: 20000 });
  });
  checks.push(`clips: off by default; turning on tells the call; Alt+C saves a clip (${numbers.clipSeconds.toFixed(1)} s MP4 that plays), trim, keep in the library, share to the call's chat`);
  await act(bob, 'media:library');
  await bob.waitForSelector('.clip-library .clip-row');
  await shot(bob, 'clip-library');
  await bob.locator('.clip-library button:has-text("Close")').click();

  // ---- Camera backgrounds -------------------------------------------------------------
  await act(bob, 'media:share');
  await until(async () => !(await diagnostic(alice)).tracks.some((t) => t.kind === 'screen'), 20000);
  assert.equal((await extras(bob)).clips?.active ?? false, false, 'no stream: no clip buffer');
  await act(bob, 'media:backgrounds');
  await bob.waitForSelector('.backgrounds-dialog .backgrounds-option[data-background="blur-light"]');
  await bob.locator('.backgrounds-option[data-background="scene:aurora"]').click();
  await bob.waitForSelector('.backgrounds-option[data-background="scene:aurora"][aria-checked="true"]', { timeout: 30000 });
  await until(async () => (await bob.locator('.backgrounds-preview').evaluate((v) => v.videoWidth)) > 0, 20000);
  await wait(1000);
  await shot(bob, 'video-effects');
  // A picture of your own, stored on your server.
  await bob.evaluate(async () => {
    const c = document.createElement('canvas'); c.width = 640; c.height = 360;
    const g = c.getContext('2d'); g.fillStyle = '#3d7'; g.fillRect(0, 0, 640, 360);
    const blob = await new Promise((r) => c.toBlob(r, 'image/png'));
    const input = document.querySelector('.backgrounds-dialog input[type=file]');
    const files = new DataTransfer(); files.items.add(new File([blob], 'green.png', { type: 'image/png' }));
    input.files = files.files;
    input.dispatchEvent(new Event('change'));
  });
  await bob.waitForSelector('.backgrounds-custom .backgrounds-option[aria-checked="true"]', { timeout: 30000 });
  await bob.locator('.backgrounds-option[data-background="blur-strong"]').click();
  await bob.waitForSelector('.backgrounds-option[data-background="blur-strong"][aria-checked="true"]');
  await bob.locator('.backgrounds-dialog button:has-text("Done")').click();
  // The camera in the call carries the effect; the model is loaded only now.
  await act(bob, 'media:camera');
  await until(async () => (await extras(bob)).backgrounds?.active === true, 30000);
  await until(async () => (await diagnostic(alice)).tracks.some((t) => t.kind === 'camera' && t.videoWidth > 0), 30000);
  await wait(3000);
  numbers.background = (await extras(bob)).backgrounds;
  const camera = await bob.evaluate(async () => (await (await import('/public/media.js')).diagnostics()).stats
    .filter((r) => r.type === 'outbound-rtp' && r.kind === 'video').map((r) => ({ w: r.frameWidth, h: r.frameHeight, fps: r.framesPerSecond })));
  numbers.backgroundCamera = camera;
  await shot(alice, 'camera-background-received');
  // Off again: the model and worker are released, the camera stays on.
  await act(bob, 'media:backgrounds');
  await bob.locator('.backgrounds-option[data-background="none"]').click();
  await until(async () => (await extras(bob)).backgrounds?.active === false, 10000);
  await bob.locator('.backgrounds-dialog button:has-text("Done")').click();
  assert((await diagnostic(bob)).publications >= 2, 'camera still published');
  checks.push(`camera backgrounds: scenes, a custom picture, blur; segmented locally (${numbers.background?.delegate ?? '?'} delegate) only while on, released when off`);

  // ---- Leaving lets go of everything ---------------------------------------------------
  await act(bob, 'media:leave');
  await act(alice, 'media:leave');
  await until(async () => !(await diagnostic(alice)).joined && !(await diagnostic(bob)).joined);
  for (const p of [alice, bob]) {
    const e = await extras(p);
    assert.equal(e.soundContext, null);
    assert.equal(e.cachedSounds, 0);
    assert.equal(e.backgrounds?.active ?? false, false);
    assert.equal(e.clips?.active ?? false, false);
  }
  checks.push('leaving the call closes the soundboard audio, clears its cache, stops clips and effects');
  for (const p of [alice, bob]) {
    const rejected = (await diagnostic(p)).trace.filter((t) => t.type === 'error' || (t.type === 'event' && t.data?.type === 'error'));
    assert.deepEqual(rejected, [], 'no media operation was rejected (TrackQuality, PlaySound, SetClipping, ClipTaken)');
  }
  assert.deepEqual(errors, [], 'no page errors');
  await writeFile(join(shots, 'callextras-results.json'), JSON.stringify({ revision: h.revision, checks, numbers }, null, 2));
  console.log(JSON.stringify({ checks, numbers }, null, 2));
} catch (e) {
  for (const [name, p] of [['alice', alice], ['bob', bob]])
    if (p) await p.screenshot({ path: join(shots, `failure-${name}.png`) }).catch(() => {});
  console.error(JSON.stringify({ checks, numbers, errors }, null, 2));
  throw e;
} finally {
  await h.close();
}

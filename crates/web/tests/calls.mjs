// Direct and group calls, voice channel chat and push to talk, in the real web
// app against a real local server. Headless Chromium with fake media devices,
// loopback only. Ring sounds are replaced by a silent recorder.
import assert from 'node:assert/strict';
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { harness, register, act, until, wait, output } from './qa-harness.mjs';

const shots = process.env.STICKTALK_CALLS_SHOTS ?? join(output, 'calls');
await mkdir(shots, { recursive: true });
const h = await harness();
const checks = [];
const errors = [];
const diagnostic = (p) => p.evaluate(async () => await (await import('/public/media.js')).diagnostics());
const ringtone = (p) => p.evaluate(() => window.__ringtone.slice());
const audioSent = (d) => d.stats.filter((r) => r.type === 'outbound-rtp' && r.kind === 'audio').reduce((n, r) => n + (r.packetsSent ?? 0), 0);
// Each new surface, at desktop and phone size.
async function shot(page, name, sizes = [[1440, 900], [390, 844]]) {
  const before = page.viewportSize();
  for (const [width, height] of sizes) {
    await page.setViewportSize({ width, height });
    await page.evaluate(() => document.fonts.ready);
    await wait(150);
    await page.screenshot({ path: join(shots, `${name}-${width}x${height}.png`) });
  }
  await page.setViewportSize(before);
}
async function page(width = 1440, height = 900) {
  const p = await h.phone(width, height);
  await p.addInitScript(() => {
    window.__ringtone = [];
    globalThis.__sticktalkRingtone = (kind) => window.__ringtone.push(kind);
  });
  p.on('pageerror', (e) => errors.push(e.stack ?? String(e)));
  return p;
}
async function friends(a, b, username) {
  await act(a, 'dialog:friend');
  await a.waitForSelector('[data-live-form="friend"]');
  await a.locator('#f-name').fill(username);
  await a.locator('[data-live-form] [type=submit]').click();
  await b.locator('[data-live^="friend:accept,"]').click();
  await a.waitForSelector(`.friend-row:has-text("${username}") [data-live^="friend:dm,"]`);
  await act(a, 'close');
}
async function message(p, text) {
  await p.locator('#live-input').fill(text);
  await act(p, 'complete-close');
  await p.locator('#live-input').press('Enter');
  await p.waitForFunction((t) => [...document.querySelectorAll('.markdown')].some((e) => e.textContent.includes(t)), text);
}
async function connected(...pages) {
  await until(async () => (await Promise.all(pages.map(diagnostic))).every((d) => d.state === 'connected'), 45000);
}
async function left(...pages) {
  await until(async () => (await Promise.all(pages.map(diagnostic))).every((d) => !d.joined && d.publications === 0));
}

let alice, bob, cara;
try {
  alice = await page();
  bob = await page();
  cara = await page(390, 844);
  await register(alice, h.origin, 'qa_call_alice');
  await register(bob, h.origin, 'qa_call_bob');
  await register(cara, h.origin, 'qa_call_cara');
  await friends(alice, bob, 'qa_call_bob');
  await friends(alice, cara, 'qa_call_cara');

  // ---- Push to talk is set up in Voice & Video before the call ---------------
  await act(bob, 'dialog:media-settings');
  await bob.waitForSelector('.voice-input-settings');
  await bob.locator('input[name="voice-input-mode"][value="ptt"]').check();
  await bob.waitForSelector('.voice-shortcut:has-text("Push to Talk Shortcut")');
  await bob.locator('.voice-shortcut:has-text("Push to Talk Shortcut") button:has-text("Record Keybind")').click();
  await bob.waitForSelector('button:has-text("Press a key or mouse button")');
  await bob.keyboard.press('KeyF');
  await bob.waitForSelector('.voice-shortcut:has-text("Push to Talk Shortcut") kbd:has-text("F")');
  await bob.locator('.voice-release-delay input').evaluate((e) => {
    e.value = '0';
    e.dispatchEvent(new Event('input'));
    e.dispatchEvent(new Event('change'));
  });
  await bob.waitForSelector('.voice-release-delay output:has-text("0 ms")');
  assert.equal(await bob.evaluate(() => JSON.parse(localStorage.getItem('sticktalk-voice-input')).talk.code), 'KeyF');
  await bob.locator('.settings-main').evaluate((e) => e.querySelector('.voice-input-settings').scrollIntoView());
  await shot(bob, 'push-to-talk-settings');
  await act(bob, 'close');
  checks.push('push-to-talk mode, recorded keybind and release delay in Voice & Video');

  // ---- A direct call: start, ring, answer --------------------------------------
  await alice.locator('.friend-row:has-text("qa_call_bob") [data-live^="friend:dm,"]').click();
  await alice.waitForSelector('#live-input');
  const dm = await alice.locator('#live-messages').getAttribute('data-channel');
  await message(alice, 'Call me?');
  await alice.waitForSelector('.channel-header [aria-label="Start Voice Call"]');
  await alice.waitForSelector('.channel-header [aria-label="Start Video Call"]');
  await shot(alice, 'dm-header-call-buttons');
  await act(bob, `channel:${dm}`);
  await bob.waitForSelector('.markdown:has-text("Call me?")');
  await alice.locator('.channel-header [aria-label="Start Voice Call"]').click();
  await alice.waitForSelector('.voice-view');
  await bob.waitForSelector('.incoming-call[role="alertdialog"]');
  assert.match(await bob.locator('#incoming-call-title').textContent(), /qa_call_alice/);
  await until(async () => (await ringtone(bob)).at(-1) === 'incoming');
  await until(async () => (await ringtone(alice)).at(-1) === 'outgoing');
  await alice.waitForSelector('.call-ring-list .ringing:has-text("Ringing")');
  await shot(bob, 'incoming-call');
  await shot(alice, 'outgoing-call-ringing');
  // The conversation shows the call in progress to the person being called.
  await bob.waitForSelector('.call-banner:has-text("Call in progress")');
  checks.push('start a call from the DM header; incoming card, incoming and outgoing ring sounds');

  await bob.locator('.incoming-answer').click();
  await bob.waitForSelector('.voice-view');
  await connected(alice, bob);
  await until(async () => (await ringtone(bob)).at(-1) === '' && (await ringtone(alice)).at(-1) === '');
  assert.equal(await bob.locator('.incoming-call').count(), 0);
  checks.push('answer: both connected over the SFU, ringing stops on both sides');

  // ---- Push to talk: nothing at all is sent until F is held ------------------
  await bob.waitForSelector('#live-ptt:not([hidden])');
  let d = await diagnostic(bob);
  assert.equal(d.microphoneSending, false);
  const quiet = audioSent(d);
  await wait(1500);
  assert.equal(audioSent(await diagnostic(bob)), quiet, 'no audio packets while the key is up');
  await bob.keyboard.down('KeyF');
  await until(async () => (await diagnostic(bob)).microphoneSending);
  await bob.waitForSelector('#live-ptt.is-live:has-text("Transmitting")');
  await until(async () => audioSent(await diagnostic(bob)) > quiet + 20, 10000);
  await shot(bob, 'push-to-talk-transmitting');
  await bob.keyboard.up('KeyF');
  await until(async () => !(await diagnostic(bob)).microphoneSending);
  const after = audioSent(await diagnostic(bob));
  await wait(1200);
  assert(audioSent(await diagnostic(bob)) <= after + 2, 'sending stops when the key is released');
  // On a phone the same control is a hold-to-talk button.
  await bob.setViewportSize({ width: 390, height: 844 });
  await wait(300);
  await bob.locator('#live-ptt').scrollIntoViewIfNeeded();
  const box = await bob.locator('#live-ptt').boundingBox();
  assert(box.height >= 48 && box.width >= 300, 'a large hold-to-talk button on phones');
  await bob.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await bob.mouse.down();
  await until(async () => (await diagnostic(bob)).microphoneSending);
  await bob.mouse.up();
  await until(async () => !(await diagnostic(bob)).microphoneSending);
  await shot(bob, 'push-to-talk-call', [[390, 844], [1440, 900]]);
  await bob.setViewportSize({ width: 1440, height: 900 });
  checks.push('push to talk: zero audio packets until held, keyboard and phone hold button, transmit indicator');

  // ---- Chat beside the call -----------------------------------------------------
  await alice.locator('.call-controls [aria-label="Show Chat"]').click();
  await alice.waitForSelector('.call-chat #live-input');
  await alice.waitForSelector('.call-chat .markdown:has-text("Call me?")');
  await message(alice, 'Chatting during the call');
  await shot(alice, 'call-chat-panel');
  assert.equal((await diagnostic(alice)).state, 'connected', 'the call keeps going with the chat open');
  await act(alice, 'call:chat-close');
  await alice.waitForFunction(() => !document.querySelector('.call-chat'));
  checks.push('the conversation chat opens beside the call (sheet on phones)');

  // ---- Ending the call leaves a record ---------------------------------------
  await act(bob, 'media:leave');
  await act(alice, 'media:leave');
  await left(alice, bob);
  await act(alice, `channel:${dm}`);
  await alice.waitForSelector('.call-record:has-text("You started a call that lasted")');
  await act(bob, `channel:${dm}`);
  await bob.waitForSelector('.call-record:has-text("qa_call_alice started a call that lasted")');
  await shot(bob, 'call-record');
  checks.push('the last one leaving ends the call; both see the call record');

  // ---- Declined, then missed ---------------------------------------------------
  await alice.locator('.channel-header [aria-label="Start Video Call"]').click();
  await bob.waitForSelector('.incoming-call');
  await bob.locator('.incoming-decline').click();
  await bob.waitForFunction(() => !document.querySelector('.incoming-call'));
  await until(async () => (await ringtone(bob)).at(-1) === '');
  await alice.waitForSelector('.call-ring-list [aria-label^="Ring "]');
  await connected(alice);
  await until(async () => (await diagnostic(alice)).publications >= 2, 15000);
  checks.push('decline stops the ring; a video call starts the camera');
  await act(alice, 'media:leave');
  await left(alice);
  await bob.waitForSelector('.call-record:has-text("You missed a call from qa_call_alice")');
  checks.push('a call nobody answered is recorded as missed');

  // ---- A group call: ring again, join later, answer with video ---------------
  await act(alice, 'dialog:group');
  await alice.waitForSelector('[data-live-form="group"]');
  await alice.locator('#f-name').fill('Call Crew');
  await alice.locator('.live-checkbox:has-text("qa_call_bob") input').check();
  await alice.locator('.live-checkbox:has-text("qa_call_cara") input').check();
  await alice.locator('[data-live-form] [type=submit]').click();
  await alice.waitForFunction(() => document.querySelector('.channel-header h1')?.textContent === 'Call Crew');
  const group = await alice.locator('#live-messages').getAttribute('data-channel');
  await alice.locator('.channel-header [aria-label="Start Voice Call"]').click();
  await cara.waitForSelector('.incoming-call:has-text("Call Crew")');
  await shot(cara, 'incoming-group-call', [[390, 844], [1440, 900]]);
  await bob.waitForSelector('.incoming-call:has-text("Call Crew")');
  await bob.locator('.incoming-decline').click();
  await alice.waitForSelector('.call-ring-list li:has-text("qa_call_bob") [aria-label="Ring qa_call_bob"]');
  await shot(alice, 'group-call-ring-again');
  await alice.locator('[aria-label="Ring qa_call_bob"]').click();
  await bob.waitForSelector('.incoming-call:has-text("Call Crew")');
  await bob.locator('.incoming-decline').click();
  checks.push('group call rings everyone; ring a member again after they decline');

  await cara.locator('.incoming-video').click();
  await cara.waitForSelector('.voice-view');
  await connected(alice, cara);
  await until(async () => (await diagnostic(cara)).publications >= 2, 15000);
  await until(async () => (await diagnostic(alice)).tracks.some((t) => t.kind === 'camera' && !t.own && t.videoWidth > 0), 20000);
  await shot(cara, 'group-call-video', [[390, 844], [1440, 900]]);
  // Bob joins later from the conversation.
  await act(bob, `channel:${group}`);
  await bob.waitForSelector('.call-banner:has-text("Call in progress")');
  await bob.waitForSelector('.call-banner:has-text("qa_call_alice")');
  await shot(bob, 'group-call-in-progress');
  await bob.locator('.call-banner [data-live^="call:answer"]:has-text("Join Call")').click();
  await connected(bob);
  await until(async () => (await diagnostic(alice)).subscriptions >= 3, 20000);
  checks.push('answer with video; join an ongoing group call later from the conversation');
  await act(cara, 'media:leave');
  await act(bob, 'media:leave');
  await act(alice, 'media:leave');
  await left(alice, bob, cara);
  await alice.waitForSelector('.call-record:has-text("You started a call that lasted")');

  // ---- Text chat in a voice channel ---------------------------------------------
  await act(alice, 'dialog:create');
  await alice.waitForSelector('[data-live-form="create"]');
  await alice.locator('#f-name').fill('Call Lab');
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
  // Every new server has a voice channel; its chat opens without joining it.
  const voice = await bob.locator('[data-live^="voice:"]').first().getAttribute('data-live');
  const voiceId = voice.slice('voice:'.length);
  assert.equal(await bob.locator(`.channel-row [data-live="call:chat,${voiceId}"]`).count(), 1, 'a chat button on the voice row');
  await act(bob, `call:chat,${voiceId}`);
  await bob.waitForSelector('.voice-chat-banner:has-text("Join Voice")');
  assert.equal((await diagnostic(bob)).joined, false, 'opening the chat does not join the call');
  await message(bob, 'Anyone up for voice?');
  await shot(bob, 'voice-channel-chat');
  await alice.waitForSelector(`.channel-row:has([data-live="${voice}"]) .live-unread`);
  checks.push('voice channel chat without joining, with unread tracking');
  await act(alice, voice);
  await connected(alice);
  await alice.locator('.call-controls [aria-label="Show Chat"]').click();
  await alice.waitForSelector('.call-chat .markdown:has-text("Anyone up for voice?")');
  await shot(alice, 'voice-channel-chat-beside-call');
  await act(alice, 'media:leave');
  await left(alice);
  checks.push('voice channel chat beside the call');

  // Nothing is left running: no call media, no ring sound.
  for (const p of [alice, bob, cara]) {
    d = await diagnostic(p);
    assert(!d.joined && d.publications === 0 && d.subscriptions === 0 && d.pendingRequests === 0);
    assert.equal((await ringtone(p)).at(-1) ?? '', '');
  }
  checks.push('cleanup: no media and no ring sound after the calls');
  assert.deepEqual(errors, [], 'no page errors');
  await writeFile(join(shots, 'calls-results.json'), JSON.stringify({ revision: h.revision, checks }, null, 2));
  console.log(JSON.stringify({ checks }, null, 2));
} catch (e) {
  for (const [name, p] of [['alice', alice], ['bob', bob], ['cara', cara]])
    if (p) await p.screenshot({ path: join(shots, `failure-${name}.png`) }).catch(() => {});
  throw e;
} finally {
  await h.close();
}

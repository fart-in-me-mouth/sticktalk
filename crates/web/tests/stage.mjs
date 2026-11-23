// Stage channels and scheduled events in the real web app against a real local
// server: start a Stage, listen without a microphone, request to speak,
// invitations, moving between audience and speakers, the end record, and
// events (create, recur, interest, share, calendar, invite). Headless Chromium,
// fake media devices, loopback only.
import assert from 'node:assert/strict';
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { harness, register, act, until, wait, output } from './qa-harness.mjs';

const shots = process.env.STICKTALK_STAGE_SHOTS ?? join(output, 'stage');
await mkdir(shots, { recursive: true });
const h = await harness();
const checks = [];
const errors = [];
const diagnostic = (p) => p.evaluate(async () => await (await import('/public/media.js')).diagnostics());
const captures = (p) => p.evaluate(() => window.__captures);
const received = (d) => d.stats.filter((r) => r.type === 'inbound-rtp' && r.kind === 'audio').reduce((n, r) => n + (r.packetsReceived ?? 0), 0);
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
async function page(width = 1440, height = 900) {
  const p = await h.phone(width, height);
  // Count microphone captures: the audience must never open one.
  await p.addInitScript(() => {
    window.__captures = 0;
    const md = navigator.mediaDevices;
    if (md?.getUserMedia) {
      const real = md.getUserMedia.bind(md);
      md.getUserMedia = (c) => { if (c?.audio) window.__captures++; return real(c); };
    }
  });
  p.on('pageerror', (e) => errors.push(e.stack ?? String(e)));
  return p;
}
async function connected(...pages) {
  await until(async () => (await Promise.all(pages.map(diagnostic))).every((d) => d.state === 'connected'), 45000);
}
async function joinServer(p, code) {
  await act(p, 'dialog:join');
  await p.waitForSelector('[data-live-form="join"]');
  await p.locator('#f-name').fill(code);
  await p.locator('[data-live-form] [type=submit]').click();
  await p.waitForSelector('#live-input');
}
async function invite(p) {
  await act(p, 'dialog:invite');
  await p.waitForSelector('[data-live-form="invite"]');
  await p.locator('[data-live-form="invite"] [type=submit]').click();
  await p.waitForSelector('.live-modal code');
  const code = await p.locator('.live-modal code').first().textContent();
  await act(p, 'close');
  return code.trim();
}
const userId = (p) => p.evaluate(() => document.querySelector('#sticktalk')?.dataset.selfId);

let alice, bob, cara, dan;
try {
  alice = await page();
  bob = await page();
  cara = await page(390, 844);
  dan = await page();
  await register(alice, h.origin, 'qa_stage_alice');
  await register(bob, h.origin, 'qa_stage_bob');
  await register(cara, h.origin, 'qa_stage_cara');
  await register(dan, h.origin, 'qa_stage_dan');
  await act(alice, 'dialog:create');
  await alice.waitForSelector('[data-live-form="create"]');
  await alice.locator('#f-name').fill('Town');
  await alice.locator('[data-live-form] [type=submit]').click();
  await alice.waitForSelector('#live-input');
  await alice.waitForFunction(() => !document.querySelector('.live-modal'));
  const code = await invite(alice);
  await joinServer(bob, code);
  await joinServer(cara, code);

  // ---- A Stage channel --------------------------------------------------------------
  await act(alice, 'dialog:channel');
  await alice.waitForSelector('[data-live-form="channel"]');
  await alice.locator('#f-name').fill('town-hall');
  await alice.locator('#f-extra').selectOption('stage');
  await alice.locator('[data-live-form="channel"] [type=submit]').click();
  await alice.waitForSelector('[data-live^="stage:open,"]');
  const open = await alice.locator('[data-live^="stage:open,"]').first().getAttribute('data-live');
  const stageId = open.split(',')[1];
  await bob.waitForSelector(`.channel-main[data-live="${open}"]`);
  // Not live: members are told so; nothing joins.
  await bob.locator(`.channel-main[data-live="${open}"]`).click();
  await bob.waitForSelector('[data-toast]:has-text("hasn’t started yet")');
  assert.equal((await diagnostic(bob)).joined, false);
  checks.push('a Stage channel; before it starts members cannot join');

  // ---- A moderator starts it --------------------------------------------------------
  await alice.locator(`.channel-main[data-live="${open}"]`).click();
  await alice.waitForSelector('[data-live-form="stage-start"]');
  await alice.locator('#stage-topic').fill('Launch Q&A');
  await alice.locator('#stage-note').fill('Members only, please do not share.');
  await shot(alice, 'stage-start-dialog');
  await alice.locator('[data-live-form="stage-start"] [type=submit]').click();
  await alice.waitForSelector('.stage-view .stage-topic:has-text("Launch Q&A")');
  await connected(alice);
  await until(async () => (await diagnostic(alice)).microphoneSending || (await diagnostic(alice)).publications > 0);
  assert.equal((await diagnostic(alice)).listenOnly, false);
  checks.push('a moderator starts the Stage with a topic and a note, and is its first speaker');

  // The channel list shows it live, with counts.
  await bob.waitForSelector(`.stage-row-live:has-text("Launch Q&A")`);
  await shot(bob, 'stage-live-in-channel-list');
  checks.push('the channel list shows the live Stage with its topic and counts');

  // ---- The audience listens with no microphone at all ---------------------------------
  await bob.locator(`.channel-main[data-live="${open}"]`).click();
  await bob.waitForSelector('.stage-view[data-stage-role="audience"]');
  await connected(bob);
  await until(async () => received(await diagnostic(bob)) > 20, 30000);
  let d = await diagnostic(bob);
  assert.equal(d.listenOnly, true);
  assert.equal(d.publications, 0);
  assert.equal(d.microphoneSending, false);
  assert.equal(await captures(bob), 0, 'no microphone capture for the audience');
  assert.equal(d.stats.filter((r) => r.type === 'outbound-rtp').length, 0, 'nothing sent upstream');
  await act(cara, open);
  await cara.waitForSelector('.stage-view[data-stage-role="audience"]');
  await connected(cara);
  await until(async () => received(await diagnostic(cara)) > 20, 30000);
  assert.equal(await captures(cara), 0);
  await alice.waitForSelector('.stage-view .stage-header:has-text("2 listening")');
  await until(async () => (await diagnostic(alice)).stage.audience === 2);
  await shot(bob, 'stage-audience-view');
  await shot(alice, 'stage-speaker-view');
  checks.push('the audience hears the speaker with no microphone capture and nothing sent upstream');

  // ---- Request to Speak ---------------------------------------------------------------
  await bob.locator('.call-controls [aria-label="Request to Speak"]').click();
  await alice.waitForSelector('.stage-requests .stage-request:has-text("qa_stage_bob")');
  await shot(alice, 'stage-request-queue');
  await alice.locator('.stage-requests [data-live^="stage:accept,"]').click();
  await bob.waitForSelector('.stage-view[data-stage-role="speaker"]');
  await until(async () => (await diagnostic(bob)).publications > 0, 20000);
  assert.equal(await captures(bob), 1, 'the microphone opens once, after acceptance');
  await alice.waitForSelector('.stage-view .stage-header:has-text("2 speaking")');
  checks.push('request to speak, the queue, accept: the microphone opens only then (muted)');

  // ---- Invite to speak, which must be accepted ------------------------------------------
  const caraId = await userId(cara);
  await alice.locator(`.stage-audience [data-live="stage:invite,${caraId}"]`).click();
  await cara.waitForSelector('.stage-invite[role="alertdialog"]');
  assert.equal(await captures(cara), 0, 'an invitation opens nothing until accepted');
  await shot(cara, 'stage-invitation', [[390, 844], [1440, 900]]);
  await cara.locator('.stage-invite [data-live="stage:answer,yes"]').click();
  await cara.waitForSelector('.stage-view[data-stage-role="speaker"]');
  await until(async () => (await diagnostic(cara)).publications > 0, 20000);
  // Back to the audience by themselves: the capture stops, nothing is sent.
  await cara.locator(`.call-controls [data-live="stage:audience,${caraId}"]`).click();
  await cara.waitForSelector('.stage-view[data-stage-role="audience"]');
  await until(async () => { const x = await diagnostic(cara); return x.publications === 0 && x.listenOnly; }, 20000);
  // A moderator moves a speaker back.
  const bobId = await userId(bob);
  await act(alice, `stage:audience,${bobId}`);
  await bob.waitForSelector('.stage-view[data-stage-role="audience"]');
  await until(async () => (await diagnostic(bob)).publications === 0, 20000);
  checks.push('invite to speak, accept, move yourself and others back to the audience');

  // ---- Text chat on the Stage ---------------------------------------------------------------
  await bob.locator('.call-controls [aria-label="Show Chat"]').click();
  await bob.waitForSelector('.call-chat:has-text("town-hall")');
  await shot(bob, 'stage-chat-beside', [[1440, 900]]);
  await bob.locator('.call-controls [aria-label="Hide Chat"]').click();
  await bob.waitForSelector('.call-chat', { state: 'detached' });
  checks.push('text chat beside the Stage');

  // ---- Scheduled events -------------------------------------------------------------------------
  await act(dan, 'home');
  await act(alice, 'media:minimize');
  await alice.waitForSelector('.stage-events-entry');
  await alice.locator('.stage-events-entry').click();
  await alice.waitForSelector('[data-dialog="events"]');
  await alice.locator('[data-live="stage:event-new"]').click();
  await alice.waitForSelector('[data-live-form="event-form"]');
  const start = await alice.evaluate(() => { const d = new Date(Date.now() + 2 * 3600e3); d.setMinutes(0, 0, 0); const p = (n) => String(n).padStart(2, '0'); return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`; });
  await alice.locator('#event-name').fill('Weekly town hall');
  await alice.locator('#event-location-kind').selectOption('stage');
  await alice.locator('#event-channel').selectOption(stageId);
  await alice.locator('#event-start').fill(start);
  await alice.locator('#event-repeat').selectOption('weekly');
  await alice.locator('#event-description').fill('Questions welcome.');
  await shot(alice, 'event-form');
  await alice.locator('[data-live-form="event-form"] [type=submit]').click();
  await alice.waitForSelector('.stage-event-detail h2:has-text("Weekly town hall")');
  await alice.waitForSelector('.stage-event-detail:has-text("Repeats weekly")');
  await shot(alice, 'event-detail');
  const eventId = (await alice.locator('.live-modal').getAttribute('data-dialog')).split(':')[1];
  // The calendar file carries the rule.
  const ics = await alice.evaluate(async (id) => (await fetch(`/api/v1/events/${id}/calendar.ics`)).text(), eventId);
  assert.match(ics, /BEGIN:VCALENDAR/);
  assert.match(ics, /RRULE:FREQ=WEEKLY/);
  assert.match(ics, /DTSTART;TZID=/);
  checks.push('create a weekly Stage event; its detail; the .ics with the rule and time zone');
  // Share it to #general: a card in chat.
  const general = await alice.locator('#event-share-channel option', { hasText: 'general' }).first().getAttribute('value');
  await alice.locator('#event-share-channel').selectOption(general);
  await alice.locator(`[data-live="stage:event-share,${eventId}"]`).click();
  await alice.waitForSelector('[data-toast]:has-text("Event shared")');
  // An invite that leads to the event.
  await alice.locator(`[data-live="stage:event-invite,${eventId}"]`).click();
  await alice.waitForSelector('.stage-event-detail code:has-text("/invite/")');
  const eventInvite = (await alice.locator('.stage-event-detail code').textContent()).split('/invite/')[1].trim();
  await act(alice, 'close');
  // Members see the count at the top of the channel list, and mark Interested.
  await bob.waitForSelector('.stage-events-entry:has-text("1")');
  await act(bob, 'media:minimize');
  await bob.locator('.stage-events-entry').click();
  await bob.waitForSelector('.stage-event-item:has-text("Weekly town hall")');
  await bob.locator('.stage-event-item [data-live^="stage:interest,"]').click();
  await bob.waitForSelector('.stage-event-item [aria-pressed="true"]');
  await shot(bob, 'events-list');
  await act(bob, 'close');
  await act(bob, `channel:${general}`);
  await bob.waitForSelector('.stage-event-card:has-text("Weekly town hall")');
  await shot(bob, 'event-card-in-chat');
  checks.push('the Events entry with a count, the list, Interested, a shared card in chat');
  // Joining through the event's invite shows it, and marks the new member interested.
  await act(dan, 'dialog:join');
  await dan.waitForSelector('[data-live-form="join"]');
  await dan.locator('#f-name').fill(eventInvite);
  await dan.waitForSelector('.invite-event:has-text("Weekly town hall")', { timeout: 15000 });
  await shot(dan, 'invite-with-event');
  await dan.locator('[data-live-form] [type=submit]').click();
  await dan.waitForSelector('#live-input');
  await dan.locator('.stage-events-entry').click();
  await dan.waitForSelector('.stage-event-item [aria-pressed="true"]');
  await act(dan, 'close');
  checks.push('an event invite shows the event and marks the new member interested');

  // ---- Ending the Stage -------------------------------------------------------------------------
  await act(alice, `voice:${stageId}`);
  await alice.waitForSelector('.stage-view [data-live="stage:end"]');
  await alice.locator('.stage-view [data-live="stage:end"]').click();
  await until(async () => !(await diagnostic(bob)).joined && !(await diagnostic(cara)).joined && !(await diagnostic(alice)).joined, 20000);
  await act(bob, `call:chat,${stageId}`);
  await bob.waitForSelector('.stage-record:has-text("Stage ended: Launch Q&A")');
  await shot(bob, 'stage-ended-record');
  await bob.waitForSelector(`.channel-row:has([data-live="${open}"])`);
  assert.equal(await bob.locator('.stage-row-live').count(), 0, 'no longer live in the channel list');
  checks.push('ending removes everyone and leaves the record in the Stage chat');

  for (const p of [alice, bob, cara, dan]) {
    d = await diagnostic(p);
    assert(!d.joined && d.publications === 0 && d.subscriptions === 0, 'nothing left running');
  }
  assert.deepEqual(errors, [], 'no page errors');
  await writeFile(join(shots, 'stage-results.json'), JSON.stringify({ revision: h.revision, checks }, null, 2));
  console.log(JSON.stringify({ checks }, null, 2));
} catch (e) {
  for (const [name, p] of [['alice', alice], ['bob', bob], ['cara', cara]])
    if (p) console.error(name, JSON.stringify(((await diagnostic(p).catch(() => ({}))).trace ?? [])
      .filter((t) => !['stats', 'speaking', 'bitrate'].includes(t.data?.type))).slice(-3000),
      await p.evaluate(() => document.querySelector('[data-toast]')?.textContent).catch(() => ''));
  console.error('server log', h.log().slice(-4000));
  for (const [name, p] of [['alice', alice], ['bob', bob], ['cara', cara], ['dan', dan]])
    if (p) await p.screenshot({ path: join(shots, `failure-${name}.png`) }).catch(() => {});
  throw e;
} finally {
  await h.close();
}

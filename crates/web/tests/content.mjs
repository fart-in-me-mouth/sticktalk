// Rich content against a real local server, in two headless browsers: polls,
// forwarding, attachment descriptions and spoilers, ten-image grids, voice
// messages (fake microphone, muted), link previews (local mock site), GIF
// categories and favorites (local mock provider) and scheduled messages.
// Everything binds to 127.0.0.1; every non-loopback request is refused.
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createServer } from 'node:net';
import { createServer as http } from 'node:http';
import { once } from 'node:events';
import { mkdtemp, rm, mkdir, readFile } from 'node:fs/promises';
import { resolve, join } from 'node:path';

const target = resolve(process.env.CARGO_TARGET_DIR ?? fileURLToPath(new URL('../../../target/', import.meta.url)));
const shots = resolve(process.env.STICKTALK_CONTENT_SHOTS ?? join(target, 'content-shots'));
await mkdir(shots, { recursive: true });
const data = await mkdtemp(join(target, 'content-test-'));
const free = async () => { const s = createServer(); s.listen(0, '127.0.0.1'); await once(s, 'listening'); const p = s.address().port; await new Promise((r) => s.close(r)); return p; };
const port = await free();
const origin = `http://127.0.0.1:${port}`;
const photo = await readFile(new URL('../public/mock-workspace.jpg', import.meta.url));
const GIF = Buffer.from('R0lGODlhAQABAIAAAP///wAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw==', 'base64');

// A local website for link previews (the server resolves only this host, to loopback).
const site = http((req, res) => {
  if (req.url.startsWith('/article')) {
    res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
    res.end(`<html><head><title>Kites</title><meta property="og:site_name" content="Sky News"><meta property="og:title" content="Kite festival fills the sky"><meta property="og:description" content="Hundreds of kites over the bay this weekend."><meta property="og:image" content="http://preview.example.com:${site.address().port}/cover.jpg"></head><body>hi</body></html>`);
  } else if (req.url === '/cover.jpg') {
    res.writeHead(200, { 'content-type': 'image/jpeg' }); res.end(photo);
  } else { res.writeHead(404); res.end(); }
});
site.listen(0, '127.0.0.1'); await once(site, 'listening');
// A local GIF provider in KLIPY's format. Media hosts are answered by the browser routes below.
const item = (n) => ({ id: `g${n}`, title: `Mock GIF ${n}`, content_description: `Mock GIF ${n}`, media_formats: {
  nanowebm: { url: `https://static.klipy.com/mock/${n}.webm`, dims: [160, 120] },
  mediumgif: { url: `https://static.klipy.com/mock/${n}.gif`, dims: [320, 240] } } });
const provider = http((req, res) => {
  const url = new URL(req.url, 'http://x');
  res.writeHead(200, { 'content-type': 'application/json' });
  if (url.pathname.endsWith('/categories')) res.end(JSON.stringify({ tags: ['Happy', 'Dance', 'Thank You', 'Wow'].map((name) => ({ name, searchterm: name.toLowerCase() })) }));
  else if (url.pathname.endsWith('/posts')) res.end(JSON.stringify({ results: (url.searchParams.get('ids') ?? '').split(',').filter(Boolean).map((id) => item(Number(id.slice(1)))), next: '' }));
  else res.end(JSON.stringify({ results: [1, 2, 3, 4, 5, 6].map(item), next: '' }));
});
provider.listen(0, '127.0.0.1'); await once(provider, 'listening');

const server = spawn(process.env.STICKTALK_SERVER_EXE ?? join(target, `debug/sticktalk-server${process.platform === 'win32' ? '.exe' : ''}`), [
  '--bind', `127.0.0.1:${port}`, '--public-url', origin, '--data-dir', data,
  '--web-root', process.env.STICKTALK_WEB_DIST ?? join(target, 'web-dist'),
  '--min-free-disk-gb', '0', '--voice-bind-ip', '127.0.0.1', '--voice-advertised-ip', '127.0.0.1',
  '--gif-provider', 'klipy', '--gif-mode', 'proxy', '--gif-api-key', 'local-test-key', '--gif-proxy-approved',
  '--mock-gif-base', `http://127.0.0.1:${provider.address().port}`,
  '--gif-media-origins', 'https://static.klipy.com',
  '--mock-link-preview-host', 'preview.example.com',
], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
let log = '';
const errors = [], checks = [], browsers = [];
const act = (page, action) => page.evaluate((a) => window.dispatchEvent(new CustomEvent('sticktalk-live', { detail: a })), action);
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const until = (page, fn, arg) => page.waitForFunction(fn, arg, { timeout: 30000 });
async function shot(page, name) {
  for (const [width, height] of [[1440, 900], [390, 844]]) {
    await page.setViewportSize({ width, height });
    await page.evaluate(() => document.fonts.ready);
    await wait(150);
    await page.evaluate(() => { const l = document.querySelector('#live-messages'); if (l) l.scrollTop = l.scrollHeight; });
    await page.screenshot({ path: join(shots, `${name}-${width}x${height}.png`) });
  }
  await page.setViewportSize({ width: 1440, height: 900 });
}
async function register(page, user) {
  await page.goto(origin);
  await page.waitForSelector('#auth-username');
  await page.waitForFunction(() => !document.querySelector('[type=submit]').disabled);
  await page.locator('[data-live=register]').click();
  await page.locator('#auth-username').fill(user);
  await page.locator('#auth-password').fill('testing-password-123456');
  await page.locator('[type=submit]').click();
  await page.waitForSelector('.live-home');
  await page.waitForFunction(() => !document.querySelector('.connection-banner'));
}
async function form(page, kind, name) {
  await act(page, `dialog:${kind}`);
  await page.waitForSelector('.live-modal [data-live-form]');
  if (name !== undefined) await page.locator('#f-name').fill(name);
  await page.locator('.live-modal [data-live-form] [type=submit]').click();
}
async function say(page, text) {
  await page.locator('#live-input').fill(text);
  await act(page, 'complete-close');
  await page.locator('#live-input').press('Enter');
  await until(page, (t) => [...document.querySelectorAll('.markdown')].some((e) => e.textContent.includes(t)), text.replaceAll('**', '').replace(/<(https?:[^>]+)>/g, '$1'));
}
const lastId = (page) => page.locator('[data-live-message]').last().getAttribute('data-live-message');

try {
  await new Promise((done, fail) => {
    const timer = setTimeout(() => fail(Error(`server timeout ${log}`)), 30000);
    const collect = (c) => { log = (log + c).slice(-20000); if (log.includes('StickTalk listening')) { clearTimeout(timer); done(); } };
    server.stdout.on('data', collect); server.stderr.on('data', collect);
    server.once('exit', (code) => fail(Error(`server exited ${code}: ${log}`)));
  });
  const launch = () => chromium.launch({ headless: true, args: ['--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream', '--mute-audio', '--disable-notifications', '--autoplay-policy=no-user-gesture-required'] });
  browsers.push(await launch(), await launch());
  const contexts = await Promise.all(browsers.map((b) => b.newContext({ viewport: { width: 1440, height: 900 }, permissions: ['microphone'] })));
  for (const context of contexts) {
    // Nothing leaves this machine: GIF media is answered locally, the rest refused.
    await context.route((url) => !['127.0.0.1', 'localhost'].includes(url.hostname), (route) => route.abort());
    await context.route(/^https:\/\/static[12]?\.klipy\.com\//, (route) => route.fulfill({ status: 200, contentType: route.request().url().endsWith('.gif') ? 'image/gif' : 'video/webm', body: route.request().url().endsWith('.gif') ? GIF : Buffer.alloc(0) }));
  }
  const [alice, bob] = await Promise.all(contexts.map((c) => c.newPage()));
  for (const page of [alice, bob]) { page.on('pageerror', (e) => errors.push(e.stack ?? String(e))); page.on('console', (m) => { if (m.type() === 'error') errors.push(`console: ${m.text()}`); }); page.on('response', (r) => { if (r.status() >= 400) errors.push(`${r.status()} ${r.request().method()} ${new URL(r.url()).pathname}`); }); }
  await register(alice, 'alice_c');
  await register(bob, 'bob_c');
  await form(alice, 'friend', 'bob_c');
  await bob.locator('[data-live^="friend:accept,"]').click();
  await alice.waitForSelector('[data-live^="friend:dm,"]');
  await alice.locator('[data-live^="friend:dm,"]').click();
  await alice.waitForSelector('#live-input');
  await say(alice, 'Hi Bob, testing content');
  const dm = await alice.locator('#live-messages').getAttribute('data-channel');
  await form(alice, 'create', 'Archive');
  await alice.waitForFunction(() => !document.querySelector('.live-modal'));
  await form(alice, 'create', 'Content Lab');
  await alice.waitForFunction(() => !document.querySelector('.live-modal'));
  await alice.waitForSelector('#live-input');
  await form(alice, 'invite');
  await alice.waitForSelector('.live-modal code');
  const invite = await alice.locator('.live-modal code').first().textContent();
  await act(alice, 'close');
  await form(bob, 'join', invite);
  await bob.waitForFunction(() => !document.querySelector('.live-modal'));
  const general = await alice.locator('#live-messages').getAttribute('data-channel');
  await act(bob, `channel:${general}`);
  await bob.waitForSelector('#live-input');

  // ---- Polls ---------------------------------------------------------------
  await act(alice, 'dialog:poll');
  await alice.waitForSelector('.live-modal [data-live-form="poll"]');
  await alice.locator('#poll-question').fill('Where should the team lunch be?');
  await alice.locator('#poll-emoji-0').fill('🍕');
  await alice.locator('#poll-answer-0').fill('Pizza place');
  await alice.locator('#poll-answer-1').fill('Noodle bar');
  await alice.locator('[data-live="poll-answer-add"]').click();
  await alice.waitForSelector('#poll-answer-2:visible');
  await alice.locator('#poll-answer-2').fill('Taco truck');
  assert.equal(await alice.locator('#poll-answer-0').inputValue(), 'Pizza place', 'typed answers survive adding a row');
  await alice.locator('#poll-duration').selectOption('1');
  await shot(alice, 'poll-create');
  await alice.locator('.live-modal [type=submit]').click();
  await until(bob, () => document.querySelector('.poll .poll-question')?.textContent.includes('team lunch'));
  checks.push('a poll created from the composer reaches the other member');
  await bob.locator('[data-live^="poll-vote:"]').nth(1).click();
  await until(alice, () => document.querySelector('.poll-footer')?.textContent.includes('1 vote'));
  await bob.locator('[data-live^="poll-vote:"]').nth(0).click();
  await until(bob, () => [...document.querySelectorAll('.poll-answer')][0]?.textContent.includes('100%'));
  await alice.locator('[data-live^="poll-results:"]').click();
  await until(alice, () => [...document.querySelectorAll('.poll-answer')][0]?.textContent.includes('100%'));
  checks.push('votes and changed votes update live for everyone');
  await shot(bob, 'poll-voted');
  await alice.locator('[data-live^="poll-voters:"]').first().click();
  await until(alice, () => document.querySelector('.live-modal')?.textContent.includes('bob_c'));
  await shot(alice, 'poll-voters');
  await act(alice, 'close');
  checks.push('voters per answer are listed');
  await alice.locator('[data-live^="poll-end:"]').click();
  await until(bob, () => document.querySelector('.poll')?.textContent.includes('Poll closed') && document.querySelector('.poll-winner'));
  assert(await bob.locator('[data-live^="poll-vote:"]').first().isDisabled(), 'a closed poll takes no votes');
  checks.push('the author ends the poll; everyone sees final results with the winner');
  await shot(bob, 'poll-closed');

  // ---- Forwarding ----------------------------------------------------------
  await say(alice, 'Forward this **plan** please');
  const plan = await lastId(alice);
  await act(alice, `forward:${plan}`);
  await alice.waitForSelector('.live-modal .forward-form');
  await alice.locator('#forward-search').fill('bob');
  await until(alice, () => document.querySelectorAll('.forward-row').length >= 1 && [...document.querySelectorAll('.forward-row')].every((r) => r.textContent.toLowerCase().includes('bob')));
  await alice.locator('.forward-row').first().click();
  await alice.locator('#forward-search').fill('');
  await until(alice, () => document.querySelectorAll('.forward-row').length > 2);
  await alice.locator('.forward-row', { hasText: 'Archive' }).first().click();
  await until(alice, () => document.querySelector('.live-modal [role=status]')?.textContent.startsWith('2 of 5'));
  await alice.locator('#forward-comment').fill('FYI');
  await shot(alice, 'forward-picker');
  await alice.locator('.live-modal [type=submit]').click();
  await alice.waitForFunction(() => !document.querySelector('.live-modal'));
  await act(bob, `channel:${dm}`);
  await until(bob, () => document.querySelector('.forwarded')?.textContent.includes('Forward this plan please'));
  assert.equal(await bob.locator('.forwarded').last().locator('xpath=..').locator('.markdown').first().textContent(), 'FYI');
  checks.push('a forwarded snapshot with a comment arrives in a DM');
  await shot(bob, 'forwarded');
  await bob.locator('.forward-source').last().click();
  await until(bob, (c) => document.querySelector('#live-messages')?.dataset.channel === c, general);
  checks.push('the link back works for someone who can read the source');

  // ---- Attachment description, spoiler, ten-image grid ---------------------
  await act(alice, `channel:${general}`);
  await alice.waitForSelector('#live-input');
  await alice.locator('#live-file').setInputFiles({ name: 'kite.jpg', mimeType: 'image/jpeg', buffer: photo });
  await alice.waitForSelector('.live-upload[data-pending]');
  await alice.locator('[data-live^="dialog:attachment:"]').click();
  await alice.locator('#attachment-description').fill('A red kite over a green hill');
  await alice.locator('#attachment-spoiler').check();
  await shot(alice, 'attachment-details');
  await alice.locator('.live-modal [type=submit]').click();
  await until(alice, () => document.querySelector('.live-upload[data-pending]')?.textContent.includes('Spoiler · A red kite'));
  await shot(alice, 'attachment-chip');
  await alice.getByRole('button', { name: 'Send Message', exact: true }).click();
  await bob.waitForSelector('.attachment-spoiler .spoiler-cover');
  await shot(bob, 'attachment-spoiler');
  await bob.locator('.spoiler-cover').last().click();
  await until(bob, () => [...document.querySelectorAll('.attachment-item img')].some((i) => i.alt === 'A red kite over a green hill'));
  await bob.locator('[data-live^="alt-toggle:"]').last().click();
  await until(bob, () => document.querySelector('.attachment-alt p')?.textContent === 'A red kite over a green hill');
  checks.push('spoiler cover until clicked; alt text on the image and on demand');
  await shot(bob, 'attachment-alt');
  await alice.locator('#live-file').setInputFiles(Array.from({ length: 10 }, (_, i) => ({ name: `photo-${i + 1}.jpg`, mimeType: 'image/jpeg', buffer: photo })));
  await until(alice, () => document.querySelectorAll('.live-upload[data-pending]').length === 10);
  await alice.getByRole('button', { name: 'Send Message', exact: true }).click();
  await until(bob, () => document.querySelector('.attachment-grid[data-count="10"]'));
  checks.push('ten attachments in one message, in a grid');
  await shot(bob, 'attachment-grid');

  // ---- Voice messages ------------------------------------------------------
  await alice.locator('.voice-record').click();
  await alice.waitForSelector('#voice-recorder[data-mode="locked"]');
  await wait(1600);
  await shot(alice, 'voice-recording');
  await alice.locator('#voice-recorder .rec-send').click();
  await bob.waitForSelector('.voice-message');
  assert(Number(await bob.locator('.voice-message').last().getAttribute('data-duration')) >= 1000);
  assert.equal(await bob.locator('.voice-message').last().locator('.voice-bar').count(), 40);
  await bob.locator('.voice-message .voice-play').last().click();
  await bob.waitForSelector('.voice-message.is-playing');
  await bob.locator('.voice-message .voice-speed').last().click();
  await until(bob, () => document.querySelector('.voice-message.is-playing .voice-speed')?.textContent === '1.5×');
  checks.push('click to record and send; play with speed control (muted)');
  await shot(bob, 'voice-message');
  // A phone: hold, slide up to lock, send. Then hold and slide left to cancel.
  await alice.setViewportSize({ width: 390, height: 844 });
  await wait(200);
  const mic = await alice.locator('.voice-record').boundingBox();
  const press = (id) => alice.locator('.voice-record').dispatchEvent('pointerdown', { pointerType: 'touch', pointerId: id, isPrimary: true, clientX: mic.x + 10, clientY: mic.y + 10 });
  const move = (id, dx, dy) => alice.evaluate(([id, x, y]) => window.dispatchEvent(new PointerEvent('pointermove', { pointerId: id, pointerType: 'touch', clientX: x, clientY: y })), [id, mic.x + 10 + dx, mic.y + 10 + dy]);
  const voices = await bob.locator('.voice-message').count();
  await press(7);
  await alice.waitForSelector('#voice-recorder[data-mode="hold"]');
  await alice.screenshot({ path: join(shots, 'voice-hold-390x844.png') });
  await move(7, 0, -100);
  await alice.waitForSelector('#voice-recorder[data-mode="locked"]');
  await wait(1300);
  await alice.locator('#voice-recorder .rec-send').click();
  await until(bob, (n) => document.querySelectorAll('.voice-message').length === n + 1, voices);
  await press(8);
  await alice.waitForSelector('#voice-recorder[data-mode="hold"]');
  await move(8, -150, 0);
  await alice.waitForFunction(() => !document.querySelector('#voice-recorder'));
  await wait(800);
  assert.equal(await bob.locator('.voice-message').count(), voices + 1, 'slide to cancel sends nothing');
  checks.push('hold to record, lock, send; slide to cancel');
  await alice.setViewportSize({ width: 1440, height: 900 });

  // ---- Link previews -------------------------------------------------------
  const sitePort = site.address().port;
  await say(alice, `Read http://preview.example.com:${sitePort}/article and <http://preview.example.com:${sitePort}/quiet>`);
  await until(bob, () => document.querySelector('.link-embed')?.textContent.includes('Kite festival fills the sky'));
  assert.equal(await bob.locator('.link-embed').count(), 1, 'the <...> link has no preview');
  await until(bob, () => { const i = document.querySelector('.link-embed img'); return i && i.complete && i.naturalWidth > 0 && new URL(i.src).origin === location.origin; });
  checks.push('a preview card with a server-served image (no third-party request)');
  await shot(bob, 'link-preview');
  await alice.locator('.link-embed-remove').last().click();
  await until(bob, () => !document.querySelector('.link-embed'));
  checks.push('the author removes the preview');
  await say(alice, `Again http://preview.example.com:${sitePort}/article?b`);
  await bob.waitForSelector('.link-embed');
  await act(bob, 'dialog:appearance');
  await bob.locator('[data-preference="linkPreviews"]').uncheck();
  await act(bob, 'close');
  await until(bob, () => getComputedStyle(document.querySelector('.link-embeds')).display === 'none');
  checks.push('the hide link previews setting');

  // ---- GIF categories and favorites ----------------------------------------
  await act(alice, 'dialog:gif');
  await alice.waitForSelector('.gif-category');
  await shot(alice, 'gif-categories');
  await alice.locator('.gif-category').first().click();
  await alice.waitForSelector('.gif-result');
  await alice.locator('.gif-result .gif-star').first().click();
  await until(alice, () => document.querySelector('.gif-result .gif-star.is-starred'));
  await alice.locator('[data-live="gif-tab:favorites"]').click();
  await until(alice, () => document.querySelectorAll('.gif-result').length === 1);
  await shot(alice, 'gif-favorites');
  await alice.locator('.gif-result [data-live^="gif-send:"]').first().click();
  await bob.waitForSelector('.chat-gif img');
  await bob.evaluate(() => { const l = document.querySelector('#live-messages'); l.scrollTop = l.scrollHeight; });
  await bob.locator('.chat-gif').last().hover();
  await bob.locator('.chat-gif .gif-star').last().click();
  await until(bob, () => document.querySelector('.chat-gif .gif-star.is-starred'));
  checks.push('categories landing, star in the picker and on a GIF in chat, favorites tab');
  await shot(bob, 'gif-in-chat');

  // ---- Scheduled messages --------------------------------------------------
  await alice.locator('#live-input').fill('Hello from the future');
  await alice.locator('.composer-more').click();
  await alice.locator('ui-menu-item[value="dialog:schedule"]').click();
  await alice.waitForSelector('.live-modal [data-live-form="schedule"]');
  await shot(alice, 'schedule');
  await alice.locator('[data-live^="schedule-at:"]').first().click();
  await alice.waitForSelector('.scheduled-bar');
  assert.equal(await alice.locator('#live-input').inputValue(), '', 'the draft moved into the schedule');
  await alice.locator('.scheduled-bar').click();
  await alice.waitForSelector('.scheduled-item');
  await shot(alice, 'scheduled-list');
  await alice.locator('.scheduled-item [data-live^="dialog:scheduled-edit:"]').click();
  await alice.locator('#scheduled-text').fill('Hello from the edited future');
  await alice.locator('.live-modal [type=submit]').click();
  await until(alice, () => document.querySelector('.scheduled-item')?.textContent.includes('edited future'));
  await alice.locator('.scheduled-item [data-live^="scheduled-send:"]').click();
  await until(bob, () => [...document.querySelectorAll('.markdown')].some((e) => e.textContent === 'Hello from the edited future'));
  await act(alice, 'close');
  await alice.waitForFunction(() => !document.querySelector('.scheduled-bar'));
  checks.push('schedule, list, edit, send now');
  await alice.locator('#live-input').fill('Never mind');
  await act(alice, 'dialog:schedule');
  await alice.locator('[data-live^="schedule-at:"]').nth(1).click();
  await alice.waitForSelector('.scheduled-bar');
  await alice.locator('.scheduled-bar').click();
  await alice.locator('.scheduled-item [data-live^="scheduled-cancel:"]').click();
  await alice.waitForFunction(() => !document.querySelector('.scheduled-item'));
  checks.push('cancel a scheduled message');

  assert.deepEqual(errors, [], 'no page errors');
  console.log(JSON.stringify({ ok: true, checks, shots }, null, 2));
} catch (e) {
  console.error(log.slice(-4000));
  console.error('page errors:', errors);
  for (const [i, b] of browsers.entries()) for (const c of b.contexts()) for (const p of c.pages()) {
    await p.screenshot({ path: join(shots, `failure-${i}.png`) }).catch(() => {});
    console.error(`browser ${i}:`, await p.evaluate(() => ({ dialog: document.querySelector('.live-modal')?.textContent?.slice(0, 300), toast: document.querySelector('[data-toast]')?.textContent, last: [...document.querySelectorAll('[data-live-message]')].slice(-2).map((m) => m.textContent.slice(0, 200)) })).catch(() => null));
  }
  throw e;
} finally {
  for (const b of browsers) await b.close().catch(() => {});
  server.kill();
  site.close(); provider.close();
  await once(server, 'exit').catch(() => {});
  await rm(data, { recursive: true, force: true }).catch(() => {});
}

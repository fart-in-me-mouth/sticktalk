// Threads, forum channels and announcement channels against a real local
// server, in two headless browsers: a thread from a message and its preview,
// joining by posting, the sidebar row, the Threads browser, archiving, a
// private thread and its members, a forum with tags, guidelines, posts and the
// gallery, publishing and following an announcement channel into another
// server, and search inside threads. Everything binds to 127.0.0.1; every
// non-loopback request is refused.
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createServer } from 'node:net';
import { once } from 'node:events';
import { mkdtemp, rm, mkdir, readFile } from 'node:fs/promises';
import { resolve, join } from 'node:path';

const target = resolve(process.env.CARGO_TARGET_DIR ?? fileURLToPath(new URL('../../../target/', import.meta.url)));
const shots = resolve(process.env.STICKTALK_THREADS_SHOTS ?? join(target, 'threads-shots'));
await mkdir(shots, { recursive: true });
const data = await mkdtemp(join(target, 'threads-test-'));
const free = async () => { const s = createServer(); s.listen(0, '127.0.0.1'); await once(s, 'listening'); const p = s.address().port; await new Promise((r) => s.close(r)); return p; };
const port = await free();
const origin = `http://127.0.0.1:${port}`;
const photo = await readFile(new URL('../public/mock-workspace.jpg', import.meta.url));

const server = spawn(process.env.STICKTALK_SERVER_EXE ?? join(target, `debug/sticktalk-server${process.platform === 'win32' ? '.exe' : ''}`), [
  '--bind', `127.0.0.1:${port}`, '--public-url', origin, '--data-dir', data,
  '--web-root', process.env.STICKTALK_WEB_DIST ?? join(target, 'web-dist'),
  '--min-free-disk-gb', '0', '--voice-bind-ip', '127.0.0.1', '--voice-advertised-ip', '127.0.0.1',
  '--link-previews', 'false',
], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
let log = '';
const errors = [], checks = [], browsers = [];
// Set while the test asks for something it must not be given (expected 404s).
let probing = false;
const act = (page, action) => page.evaluate((a) => window.dispatchEvent(new CustomEvent('sticktalk-live', { detail: a })), action);
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const until = (page, fn, arg) => page.waitForFunction(fn, arg, { timeout: 30000 });
async function shot(page, name, prepare) {
  for (const [width, height] of [[1440, 900], [390, 844]]) {
    await page.setViewportSize({ width, height });
    if (prepare) await prepare(width);
    await page.evaluate(() => document.fonts.ready);
    await wait(200);
    await page.screenshot({ path: join(shots, `${name}-${width}x${height}.png`) });
  }
  await page.setViewportSize({ width: 1440, height: 900 });
}
const api = (page, method, path, body) => page.evaluate(async ([method, path, body]) => {
  const csrf = document.cookie.split(';').map((v) => v.trim()).find((v) => v.startsWith('__Host-sticktalk_csrf='))?.slice(22) ?? '';
  const r = await fetch(`/api/v1${path}`, { method, headers: { 'content-type': 'application/json', 'x-sticktalk-csrf': csrf }, body: body ?? undefined });
  return { status: r.status, text: await r.text() };
}, [method, path, body]);
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
async function form(page, kind, fields = {}) {
  await act(page, `dialog:${kind}`);
  await page.waitForSelector('.live-modal [data-live-form]');
  for (const [id, value] of Object.entries(fields)) {
    const field = page.locator(`#${id}`);
    if ((await field.evaluate((e) => e.tagName)) === 'SELECT') await field.selectOption(value);
    else await field.fill(value);
  }
  await page.locator('.live-modal [data-live-form] [type=submit]').click();
}
const closed = (page) => page.waitForFunction(() => !document.querySelector('.live-modal'));
async function say(page, text) {
  await page.locator('#live-input').fill(text);
  await act(page, 'complete-close');
  await page.locator('#live-input').press('Enter');
  await until(page, (t) => [...document.querySelectorAll('.message .markdown')].some((e) => e.textContent.includes(t)), text);
}
const channelOf = (page) => page.locator('#live-messages').getAttribute('data-channel');
const messageWith = (page, text) => page.evaluate((t) => [...document.querySelectorAll('[data-live-message]')].find((m) => m.textContent.includes(t))?.dataset.liveMessage, text);
const rowFor = (page, name) => page.locator(`.channel-row .channel-main[title="${name}"]`).first();

try {
  await new Promise((done, fail) => {
    const timer = setTimeout(() => fail(Error(`server timeout ${log}`)), 30000);
    const collect = (c) => { log = (log + c).slice(-20000); if (log.includes('StickTalk listening')) { clearTimeout(timer); done(); } };
    server.stdout.on('data', collect); server.stderr.on('data', collect);
    server.once('exit', (code) => fail(Error(`server exited ${code}: ${log}`)));
  });
  const launch = () => chromium.launch({ headless: true, args: ['--mute-audio', '--disable-notifications'] });
  browsers.push(await launch(), await launch());
  const contexts = await Promise.all(browsers.map((b) => b.newContext({ viewport: { width: 1440, height: 900 } })));
  for (const context of contexts) await context.route((url) => !['127.0.0.1', 'localhost'].includes(url.hostname), (route) => route.abort());
  const [alice, bob] = await Promise.all(contexts.map((c) => c.newPage()));
  for (const page of [alice, bob]) {
    page.setDefaultTimeout(30000);
    page.on('pageerror', (e) => errors.push(e.stack ?? String(e)));
    page.on('console', (m) => { if (m.type() === 'error' && !probing) errors.push(`console: ${m.text()}`); });
    page.on('response', (r) => { if (r.status() >= 400 && !probing) errors.push(`${r.status()} ${r.request().method()} ${new URL(r.url()).pathname}`); });
  }
  await register(alice, 'alice_t');
  await register(bob, 'bob_t');
  await form(alice, 'create', { 'f-name': 'Thread Lab' });
  await closed(alice);
  await alice.waitForSelector('#live-input');
  const lab = (await alice.locator('.server-icon.selected').getAttribute('data-live')).slice(6);
  const general = await channelOf(alice);
  await form(alice, 'invite');
  await alice.waitForSelector('.live-modal code');
  const invite = await alice.locator('.live-modal code').first().textContent();
  await act(alice, 'close');
  await form(bob, 'join', { 'f-name': invite });
  await closed(bob);
  await act(bob, `channel:${general}`);
  await bob.waitForSelector('#live-input');

  // ---- A thread from a message --------------------------------------------
  await say(alice, 'Plan the launch party');
  const starter = await messageWith(alice, 'Plan the launch party');
  await alice.locator(`[data-live-message="${starter}"]`).hover();
  await alice.locator(`[data-live-message="${starter}"] [data-live="thread-new:${general},${starter}"]`).click();
  await alice.waitForSelector('.live-modal [data-live-form="thread-new"]');
  assert.equal(await alice.locator('#thread-name').inputValue(), 'Plan the launch party', 'the name is suggested from the message');
  await shot(alice, 'thread-create');
  await alice.locator('#thread-name').fill('Launch party');
  await alice.locator('.live-modal [type=submit]').click();
  await closed(alice);
  await until(alice, () => document.querySelector('.thread-title h1')?.textContent === 'Launch party');
  const thread = await channelOf(alice);
  assert.notEqual(thread, general);
  await say(alice, 'Snacks or a full dinner?');
  checks.push('start a thread from a message; it opens with the suggested name');
  // Bob sees the preview under the message, with the count.
  await until(bob, (s) => document.querySelector(`[data-live-message="${s}"] .thread-preview`)?.textContent.includes('1 message'), starter);
  await shot(bob, 'thread-preview');
  await bob.locator(`[data-live-message="${starter}"] .thread-preview`).click();
  await until(bob, () => document.querySelector('.thread-title h1')?.textContent === 'Launch party');
  await bob.waitForSelector('.thread-note');
  await until(bob, () => [...document.querySelectorAll('.message .markdown')].some((e) => e.textContent.includes('Snacks or a full dinner?')));
  // Posting joins the thread: it appears under #general in Bob's sidebar.
  await say(bob, 'Dinner, definitely');
  await bob.waitForSelector(`.thread-row [data-live="thread-open:${thread}"]`);
  await bob.waitForFunction(() => !document.querySelector('.thread-note'));
  await shot(bob, 'thread-view', async (w) => { if (w < 768) await act(bob, 'panel:'); });
  await shot(bob, 'thread-sidebar', async (w) => { if (w < 768) await act(bob, 'panel:left'); });
  await act(bob, 'panel:');
  // Unreads for members: Alice's sidebar row is bold once Bob posts while she is away.
  await act(alice, `channel:${general}`);
  await alice.waitForSelector(`.thread-row [data-live="thread-open:${thread}"]`);
  await say(bob, 'Who brings the cake?');
  await until(alice, (t) => document.querySelector(`.thread-row [data-live="thread-open:${t}"]`)?.classList.contains('font-semibold'), thread);
  checks.push('preview with reply count; opening and posting joins; sidebar row with unread state');

  // ---- Threads browser, archive ------------------------------------------
  await act(alice, `threads-browser:${general}`);
  await alice.waitForSelector('.threads-browser .thread-entry');
  await shot(alice, 'threads-browser');
  await alice.locator('.threads-browser .thread-entry').first().click();
  await until(alice, () => document.querySelector('.thread-title h1')?.textContent === 'Launch party');
  await alice.locator('[aria-label="Thread Options"]:visible').click();
  await alice.locator(`ui-menu-content:not([hidden]) ui-menu-item[value="thread-archive:${thread}"]`).click();
  await until(alice, () => document.querySelector('.thread-title')?.textContent.includes('Archived'));
  await until(bob, () => document.querySelector('.thread-title')?.textContent.includes('Archived'));
  await bob.waitForSelector('.thread-note');
  await act(alice, `threads-browser:${general}`);
  await act(alice, 'threads-tab:archived');
  await until(alice, () => document.querySelector('.threads-browser')?.textContent.includes('Launch party'));
  await act(alice, 'close');
  // Sending reopens it.
  await say(bob, 'Reopening this');
  await until(alice, () => !document.querySelector('.thread-title')?.textContent.includes('Archived'));
  checks.push('Threads browser (active and archived), archive, and a message reopens it');

  // ---- A private thread and its members -----------------------------------
  await act(alice, `channel:${general}`);
  await alice.waitForSelector('#live-input');
  await act(alice, `thread-new:${general}`);
  await alice.waitForSelector('#thread-private');
  await alice.locator('#thread-name').fill('Surprise planning');
  await alice.locator('#thread-private').check();
  await alice.locator('#thread-invitable').uncheck();
  await alice.locator('.live-modal [type=submit]').click();
  await closed(alice);
  await until(alice, () => document.querySelector('.thread-title h1')?.textContent === 'Surprise planning');
  const secret = await channelOf(alice);
  await say(alice, 'Do not tell Bob');
  // Bob cannot open it, find it in the browser or search it.
  probing = true;
  const hidden = await api(bob, 'GET', `/threads/${secret}`);
  assert.equal(hidden.status, 404, 'a private thread is invisible to non-members');
  const listed = await api(bob, 'GET', `/channels/${general}/threads`);
  assert(!listed.text.includes('Surprise planning'), 'not in the Threads browser for Bob');
  const searched = await api(bob, 'GET', `/guilds/${lab}/messages/search?q=tell`);
  assert(!searched.text.includes('Do not tell Bob'), 'not in search for Bob');
  await wait(100);
  probing = false;
  await act(alice, `thread-members:${secret}`);
  await alice.waitForSelector('.thread-members #thread-member');
  await shot(alice, 'thread-members');
  await alice.locator('.live-modal [type=submit]').click();
  await until(alice, () => document.querySelector('.thread-members')?.textContent.includes('bob_t'));
  await act(alice, 'close');
  const visible = await api(bob, 'GET', `/threads/${secret}`);
  assert.equal(visible.status, 200, 'an added member sees it');
  await act(bob, `thread-open:${secret}`);
  await until(bob, () => document.querySelector('.thread-title h1')?.textContent === 'Surprise planning');
  assert(await bob.locator('.thread-title [aria-label="Private thread"]').count() === 1);
  await shot(bob, 'private-thread');
  checks.push('private thread: hidden from non-members (detail, browser, search) until added');

  // ---- Search in threads -------------------------------------------------
  const found = await api(alice, 'GET', `/guilds/${lab}/messages/search?q=${encodeURIComponent('in:"Launch party" cake')}`);
  assert(found.text.includes('Who brings the cake?'), 'search with in: finds a thread message');
  checks.push('search covers thread messages, with in: a thread');

  // ---- Forum ---------------------------------------------------------------
  await form(alice, 'channel', { 'f-name': 'ideas', 'f-extra': 'forum' });
  await closed(alice);
  await rowFor(alice, 'ideas').click();
  await alice.waitForSelector('.forum-view');
  await act(alice, 'dialog:forum-settings');
  await alice.waitForSelector('.live-modal [data-live-form="forum-settings"]');
  await alice.locator('#forum-guidelines').fill('Search before posting. One idea per post.');
  await alice.locator('#tag-new').fill('feature');
  await alice.locator('#tag-new-emoji').fill('✨');
  await alice.locator('#forum-reaction').fill('👍');
  await alice.locator('.live-modal [type=submit]').click();
  await closed(alice);
  await act(alice, 'dialog:forum-settings');
  await alice.waitForSelector('#tag-name-0');
  await alice.locator('#tag-new').fill('bug');
  await shot(alice, 'forum-settings');
  await alice.locator('.live-modal [type=submit]').click();
  await closed(alice);
  await until(alice, () => document.querySelectorAll('.forum-tags .forum-tag').length === 2);
  // A post with a picture: the file goes up to the forum, then into the post.
  await act(alice, 'forum-new');
  await alice.waitForSelector('#post-title');
  await alice.locator('#post-title').fill('Dark mode for the calendar');
  await alice.locator('#post-content').fill('The calendar is too bright at night.');
  await alice.locator('label:has(#post-tag-1)').click();
  await alice.locator('#live-file').setInputFiles({ name: 'calendar.jpg', mimeType: 'image/jpeg', buffer: photo });
  await alice.waitForSelector('.live-modal .live-upload[data-pending]');
  await shot(alice, 'forum-new-post');
  await alice.locator('.live-modal [type=submit]').click();
  await closed(alice);
  await until(alice, () => document.querySelector('.thread-title h1')?.textContent === 'Dark mode for the calendar');
  await alice.waitForSelector('.message img');
  checks.push('forum settings (guidelines, tags, default reaction); a post with a tag and a picture');
  await act(bob, 'home');
  await act(bob, `guild:${lab}`);
  await rowFor(bob, 'ideas').click();
  await bob.waitForSelector('.forum-post');
  assert((await bob.locator('.forum-post').first().textContent()).includes('feature'));
  await bob.locator('.forum-post [aria-label="React with 👍"]').first().click();
  await until(bob, () => document.querySelector('.forum-post')?.textContent.includes('1 reaction'));
  await shot(bob, 'forum-list');
  await act(bob, 'forum-layout:gallery');
  await bob.waitForSelector('.forum-post .forum-cover');
  await shot(bob, 'forum-gallery');
  await act(bob, 'forum-layout:list');
  await act(bob, 'forum-new');
  await bob.waitForSelector('#post-title');
  await bob.locator('#post-title').fill('Keyboard shortcuts list');
  await bob.locator('#post-content').fill('Where can I see every shortcut?');
  await bob.locator('.live-modal [type=submit]').click();
  await closed(bob);
  await until(bob, () => document.querySelector('.thread-title h1')?.textContent === 'Keyboard shortcuts list');
  await rowFor(bob, 'ideas').click();
  await until(bob, () => document.querySelectorAll('.forum-post').length === 2);
  // Searches at a person's pace (the server rate-limits reads).
  const search = async (q) => { await wait(1500); await bob.locator('#forum-query').fill(q); await bob.locator('#forum-query').press('Enter'); };
  await search('bright');
  await until(bob, () => document.querySelectorAll('.forum-post').length === 1);
  await wait(1500);
  await act(bob, 'forum-tag:2');
  await until(bob, () => document.querySelectorAll('.forum-post').length === 0);
  await wait(1500);
  await act(bob, 'forum-tag:clear');
  await search('nothing matches this');
  await until(bob, () => document.querySelector('.forum-view')?.textContent.includes('No posts match.'));
  await search('');
  await until(bob, () => document.querySelectorAll('.forum-post').length === 2);
  await bob.locator('.forum-post .forum-post-title', { hasText: 'Dark mode' }).click();
  await until(bob, () => document.querySelector('.thread-title h1')?.textContent === 'Dark mode for the calendar');
  await say(bob, 'Yes please, and for the inbox too');
  await shot(bob, 'forum-post-view');
  checks.push('forum list with tags, reactions, gallery cover, search; replying in a post');

  // ---- Announcements and following ----------------------------------------
  await form(alice, 'channel', { 'f-name': 'news', 'f-extra': 'announcement' });
  await closed(alice);
  await rowFor(alice, 'news').click();
  await alice.waitForSelector('#live-input');
  const news = await channelOf(alice);
  // Bob owns another server and follows #news into it.
  await form(bob, 'create', { 'f-name': 'Bob HQ' });
  await closed(bob);
  await bob.waitForSelector('#live-input');
  const hq = (await bob.locator('.server-icon.selected').getAttribute('data-live')).slice(6);
  const bobGeneral = await channelOf(bob);
  await act(bob, `guild:${lab}`);
  await rowFor(bob, 'news').click();
  await bob.waitForSelector('#live-input');
  await act(bob, `follow:${news}`);
  await bob.waitForSelector('.live-modal #follow-target');
  await bob.locator('#follow-target').selectOption(bobGeneral);
  await shot(bob, 'follow-dialog');
  await bob.locator('.live-modal [type=submit]').click();
  await closed(bob);
  await say(alice, 'Version 2 ships on Friday');
  const announcement = await messageWith(alice, 'Version 2 ships on Friday');
  await alice.locator(`[data-live-message="${announcement}"]`).hover();
  await alice.locator(`[data-live-message="${announcement}"] [aria-label="Publish"]`).click();
  await alice.waitForSelector(`[data-live-message="${announcement}"] .published`);
  await shot(alice, 'announcement-published');
  await act(bob, `guild:${hq}`);
  await act(bob, `channel:${bobGeneral}`);
  await until(bob, () => document.querySelector('.crosspost')?.textContent.includes('Version 2 ships on Friday'));
  assert((await bob.locator('.crosspost').textContent()).includes('Thread Lab'), 'the copy names its source server');
  await shot(bob, 'crosspost');
  // An edit to the original updates the copy.
  await alice.locator(`[data-live-message="${announcement}"]`).hover();
  await alice.locator(`[data-live-message="${announcement}"] [data-live="edit:${announcement}"]`).click();
  await alice.locator('#f-name').fill('Version 2 ships on Saturday');
  await alice.locator('#f-name').press('Enter');
  await until(bob, () => document.querySelector('.crosspost')?.textContent.includes('Saturday'));
  const follows = await api(bob, 'GET', `/channels/${bobGeneral}/follows`);
  assert(follows.text.includes('"source_channel_name":"news"'), 'the following is listed on the target side');
  checks.push('publish, follow into another server, crosspost copy with its source, edits follow');

  assert.deepEqual(errors, [], 'no page errors');
  console.log(JSON.stringify({ ok: true, checks, shots }, null, 2));
} catch (e) {
  console.error(log.slice(-4000));
  console.error('page errors:', errors);
  for (const [i, b] of browsers.entries()) for (const c of b.contexts()) for (const p of c.pages()) {
    await p.screenshot({ path: join(shots, `failure-${i}.png`) }).catch(() => {});
    console.error(`browser ${i}:`, await p.evaluate(() => ({ dialog: document.querySelector('.live-modal')?.textContent?.slice(0, 300), toast: document.querySelector('[data-toast]')?.textContent, title: document.querySelector('.channel-header')?.textContent?.slice(0, 200), last: [...document.querySelectorAll('[data-live-message]')].slice(-2).map((m) => m.textContent.slice(0, 200)) })).catch(() => null));
  }
  throw e;
} finally {
  for (const b of browsers) await b.close().catch(() => {});
  server.kill();
  await once(server, 'exit').catch(() => {});
  await rm(data, { recursive: true, force: true }).catch(() => {});
}

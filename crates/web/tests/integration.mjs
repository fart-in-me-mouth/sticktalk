// Cross-feature UI checks on the combined app (docs/features/integration.md):
// the chat header, composer and message actions in every kind of conversation
// (text, announcement, forum, voice-channel chat, thread, DM) at 1440x900 and
// 390x844: nothing overflows or is clipped, no action appears twice, and the
// header keeps the order of UI_PARITY 1.3. Also: the channel permission editor
// offers each feature's permissions only where they apply.
// Real local server, headless Chromium, loopback only.
// STICKTALK_INTEGRATION_ASSERT=0 only measures and screenshots (for a "before" run).
// STICKTALK_INTEGRATION_THEME=light runs it all in the light theme (task qa2:
// every flow in the default dark theme and a light one). Also checked (qa2):
// the composer beside a call has room to type, settings pages fit, and nothing
// throws, logs a console error or breaks the CSP.
import assert from 'node:assert/strict';
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { harness, register, act, form, wait, output } from './qa-harness.mjs';

const shots = process.env.STICKTALK_INTEGRATION_SHOTS ?? join(output, 'integration');
const strict = process.env.STICKTALK_INTEGRATION_ASSERT !== '0';
const theme = process.env.STICKTALK_INTEGRATION_THEME ?? 'dark';
const suffix = theme === 'dark' ? '' : `-${theme}`;
const consoleErrors = [];
await mkdir(shots, { recursive: true });
const h = await harness();
const checks = [], errors = [], report = {};
const SIZES = [[1440, 900], [390, 844]];
const channelOf = (p) => p.locator('#live-messages').getAttribute('data-channel');
const closed = (p) => p.waitForFunction(() => !document.querySelector('.live-modal'));
async function say(p, text) {
  await p.locator('#live-input').fill(text);
  await act(p, 'complete-close');
  await p.locator('#live-input').press('Enter');
  await p.waitForFunction((t) => [...document.querySelectorAll('.markdown')].some((e) => e.textContent.includes(t)), text);
}
// Everything in the header, composer and hover actions that is actually shown.
const measure = (p) => p.evaluate(() => {
  const shown = (e) => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0 && getComputedStyle(e).visibility !== 'hidden'; };
  const labels = (root) => root ? [...root.querySelectorAll('button,[role=button],input[type=search],input[type=text]')].filter(shown)
    .map((e) => e.getAttribute('aria-label') || e.getAttribute('placeholder') || e.textContent.trim()).filter(Boolean) : [];
  const outside = (root) => root ? [...root.querySelectorAll('button')].filter(shown).filter((e) => {
    const r = e.getBoundingClientRect(), box = root.getBoundingClientRect();
    return r.right > Math.min(innerWidth, box.right) + 1 || r.left < Math.max(0, box.left) - 1;
  }).map((e) => e.getAttribute('aria-label') || e.textContent.trim()) : [];
  const header = document.querySelector('.channel-header');
  const composer = document.querySelector('.composer');
  const actions = [...document.querySelectorAll('.message-actions')].find(shown);
  const title = header?.querySelector('h1');
  return {
    header: labels(header?.querySelector('.header-actions')),
    headerClipped: outside(header),
    headerOverflow: header ? header.scrollWidth > header.clientWidth + 1 : false,
    titleWidth: title ? Math.round(title.getBoundingClientRect().width) : null,
    titleTruncated: title ? title.scrollWidth > title.clientWidth + 1 : false,
    composer: labels(composer),
    composerClipped: outside(composer),
    actions: labels(actions),
    actionsClipped: actions ? outside(actions.closest('.message') ?? document.body).length > 0 || actions.getBoundingClientRect().left < 0 : false,
    pageOverflow: document.documentElement.scrollWidth > innerWidth + 1,
  };
});
const dupes = (list) => list.filter((v, i) => list.indexOf(v) !== i);
async function view(p, name, open) {
  report[name] = {};
  for (const [width, height] of SIZES) {
    await p.setViewportSize({ width, height });
    await open?.();
    await p.evaluate(() => document.fonts.ready);
    await wait(250);
    const last = p.locator('[data-live-message]').last();
    if (width > 600 && await last.count()) await last.hover();
    const m = await measure(p);
    report[name][`${width}x${height}`] = m;
    await p.screenshot({ path: join(shots, `${name}${suffix}-${width}x${height}.png`) });
    if (!strict) continue;
    const where = `${name} at ${width}px`;
    assert.equal(m.pageOverflow, false, `${where}: the page scrolls sideways`);
    assert.equal(m.headerOverflow, false, `${where}: the header overflows`);
    assert.deepEqual(m.headerClipped, [], `${where}: header buttons clipped`);
    assert.deepEqual(m.composerClipped, [], `${where}: composer buttons clipped`);
    assert.equal(m.actionsClipped, false, `${where}: message actions clipped`);
    assert.deepEqual(dupes(m.header), [], `${where}: an action appears twice in the header`);
    assert.deepEqual(dupes(m.composer), [], `${where}: an action appears twice in the composer`);
    assert.deepEqual(dupes(m.actions), [], `${where}: an action appears twice on a message`);
    // Desktop: never cut off. Phones: an ellipsis is fine, but it stays readable.
    if (width > 600) assert.equal(m.titleTruncated, false, `${where}: the title is cut off (${m.titleWidth}px)`);
    else if (m.titleTruncated) assert.ok(m.titleWidth >= 60, `${where}: the title is squeezed to ${m.titleWidth}px`);
    // UI_PARITY 1.3: Threads, Notification Settings, Pinned, Member List, Search, Inbox.
    const order = ['Threads', 'Notification Settings', 'Pinned Messages', 'Member List', 'Inbox']
      .map((l) => m.header.indexOf(l)).filter((i) => i >= 0);
    assert.deepEqual(order, [...order].sort((a, b) => a - b), `${where}: header order ${m.header.join(', ')}`);
  }
  await p.setViewportSize({ width: 1440, height: 900 });
}

let alice, bob;
try {
  alice = await h.phone(1440, 900);
  bob = await h.phone(1440, 900);
  for (const p of [alice, bob]) {
    p.on('pageerror', (e) => errors.push(e.stack ?? String(e)));
    p.on('console', (m) => { if (m.type() === 'error') consoleErrors.push(`${m.text()} ${m.location()?.url ?? ''}`); });
  }
  await register(alice, h.origin, 'integ_alice');
  await register(bob, h.origin, 'integ_bob');
  if (theme !== 'dark') {
    for (const p of [alice, bob]) {
      await act(p, `appearance:theme,${theme}`);
      await p.waitForFunction((t) => document.documentElement.dataset.themeBase === t, theme);
    }
  }
  await form(alice, 'create', { 'f-name': 'Integration Lab' });
  await closed(alice);
  await alice.waitForSelector('#live-input');
  const general = await channelOf(alice);
  const voice = (await alice.locator('[data-live^="voice:"]').first().getAttribute('data-live')).slice('voice:'.length);

  // Text channel, with a message to hover.
  await say(alice, 'Plan the launch party');
  await view(alice, 'text-channel');

  // A thread from that message.
  const starter = await alice.evaluate(() => [...document.querySelectorAll('[data-live-message]')].find((m) => m.textContent.includes('Plan the launch party'))?.dataset.liveMessage);
  await act(alice, `thread-new:${general},${starter}`);
  await alice.waitForSelector('.live-modal [data-live-form="thread-new"]');
  await alice.locator('#thread-name').fill('Launch party');
  await alice.locator('.live-modal [type=submit]').click();
  await closed(alice);
  await alice.waitForFunction(() => document.querySelector('.thread-title h1')?.textContent === 'Launch party');
  await say(alice, 'Snacks or dinner?');
  await view(alice, 'thread');

  // Announcement and forum channels.
  await act(alice, `channel:${general}`);
  await alice.waitForSelector('#live-input');
  await form(alice, 'channel', { 'f-name': 'news', 'f-extra': 'announcement' });
  await closed(alice);
  await alice.waitForFunction((g) => document.querySelector('#live-messages')?.dataset.channel !== g, general);
  await say(alice, 'Version 2 is out');
  await view(alice, 'announcement');
  await form(alice, 'channel', { 'f-name': 'ideas', 'f-extra': 'forum' });
  await closed(alice);
  await alice.waitForSelector('.forum-view, [data-forum]', { timeout: 15000 }).catch(() => {});
  await view(alice, 'forum');

  // Voice-channel chat, opened without joining.
  await act(alice, `call:chat,${voice}`);
  await alice.waitForSelector('.voice-chat-banner');
  await say(alice, 'Anyone around?');
  await view(alice, 'voice-chat');
  if (strict) {
    const m = report['voice-chat']['1440x900'];
    assert.ok(!m.header.includes('Threads') && !m.actions.includes('Create Thread'), 'voice-channel chat offers no threads');
    checks.push('voice-channel chat offers no thread actions; text and announcement channels do');
  }

  // Joined to the voice channel, with its chat beside the call: room to type.
  await act(alice, `voice:${voice}`);
  await alice.waitForSelector('.voice-view, .call-stage, [data-call-view]', { timeout: 15000 }).catch(() => {});
  // `call:chat` toggles the panel: open it if it is not open yet.
  for (let i = 0; i < 2 && !(await alice.locator('.call-chat').count()); i++) {
    await act(alice, `call:chat,${voice}`);
    await alice.waitForSelector('.call-chat #live-input', { timeout: 5000 }).catch(() => {});
  }
  report.callChat = {};
  for (const [width, height] of SIZES) {
    await alice.setViewportSize({ width, height });
    await wait(300);
    const m = await alice.evaluate(() => {
      const input = document.querySelector('.call-chat #live-input');
      const r = input?.getBoundingClientRect();
      // The chat stays inside its pane: not under the member list or off screen.
      const chat = document.querySelector('.call-chat')?.getBoundingClientRect();
      const pane = document.querySelector('.call-chat')?.closest('main')?.getBoundingClientRect();
      const outside = !!(chat && pane && (chat.right > Math.min(pane.right, innerWidth) + 1));
      return { input: r ? Math.round(r.width) : 0, outside, pageOverflow: document.documentElement.scrollWidth > innerWidth + 1 };
    });
    report.callChat[`${width}x${height}`] = m;
    await alice.screenshot({ path: join(shots, `call-chat${suffix}-${width}x${height}.png`) });
    if (strict) {
      assert.ok(m.input >= 80, `call chat at ${width}px: the message box is ${m.input}px wide`);
      assert.equal(m.pageOverflow, false, `call chat at ${width}px: the page scrolls sideways`);
      assert.equal(m.outside, false, `call chat at ${width}px: it runs past its pane (under the member list)`);
    }
  }
  await alice.setViewportSize({ width: 1440, height: 900 });
  await act(alice, 'media:leave');
  await act(alice, 'call:chat-close');
  if (strict) checks.push('the message box beside a call has room to type at 1440 and 390 px');

  // A DM with a friend.
  await act(alice, 'home');
  await alice.waitForSelector('.live-home');
  await act(alice, 'dialog:friend');
  await alice.waitForSelector('[data-live-form="friend"]');
  await alice.locator('#f-name').fill('integ_bob');
  await alice.locator('[data-live-form] [type=submit]').click();
  await bob.locator('[data-live^="friend:accept,"]').click();
  await alice.waitForSelector('.friend-row:has-text("integ_bob") [data-live^="friend:dm,"]');
  await act(alice, 'close');
  await alice.locator('.friend-row:has-text("integ_bob") [data-live^="friend:dm,"]').click();
  await alice.waitForSelector('#live-input');
  await say(alice, 'Hi Bob');
  await view(alice, 'direct-message');
  checks.push('header, composer and message actions fit, without duplicates and in order, in text, thread, announcement, forum, voice-chat and DM views at 1440 and 390 px');

  // The channel permission editor offers each permission where it applies.
  await act(alice, `channel:${general}`);
  await alice.waitForSelector('#live-input');
  const editor = async (ch) => {
    await act(alice, `dialog:channel-edit:${ch}`);
    await alice.locator(`[data-live="dialog:channel-perms:${ch}"]`).click();
    await alice.waitForSelector('#ow-add-role');
    if (!await alice.locator('[data-overwrite]').count()) {
      await alice.locator('#ow-add-role').selectOption({ index: 0 });
      await alice.locator('[data-live^="mod:ow-add,role:"]').click();
      await alice.waitForSelector('[data-overwrite]');
    }
    const labels = await alice.locator('[data-overwrite] .tri-state > span').allTextContents();
    await alice.screenshot({ path: join(shots, `permissions-${ch === voice ? 'voice' : 'text'}-1440x900.png`) });
    await act(alice, 'close');
    return labels;
  };
  const text = await editor(general);
  const voiceLabels = await editor(voice);
  report.permissions = { text, voice: voiceLabels };
  if (strict) {
    for (const l of ['Create Public Threads', 'Create Private Threads', 'Send Messages in Threads', 'Manage Threads'])
      assert.ok(text.includes(l) && !voiceLabels.includes(l), `${l}: text channels only`);
    assert.ok(voiceLabels.includes('Use Soundboard') && !text.includes('Use Soundboard'), 'Use Soundboard: voice channels only');
    assert.ok(!text.includes('Connect'), 'no voice permissions on a text channel');
    assert.ok(!text.includes('Publish Messages'), 'Publish Messages only on announcement channels');
    checks.push('channel permission editor: thread permissions on text channels, Use Soundboard and voice permissions on voice channels');
  }
  // Settings pages fit at both sizes: nothing scrolls sideways or is cut off.
  report.settings = {};
  for (const page of ['settings', 'account', 'profile-edit', 'privacy', 'notifications', 'appearance', 'accessibility', 'expression-settings', 'media-settings', 'server', 'roles']) {
    for (const [width, height] of SIZES) {
      await alice.setViewportSize({ width, height });
      await act(alice, `dialog:${page}`);
      await alice.waitForFunction((n) => document.querySelector('.live-modal')?.dataset.dialog === n, page);
      await alice.evaluate(() => document.fonts.ready);
      await wait(300);
      const m = await alice.evaluate(() => {
        const modal = document.querySelector('.live-modal');
        const shown = (e) => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0; };
        const clipped = [...modal.querySelectorAll('button,label,h1,h2,h3')].filter(shown).filter((e) => {
          const r = e.getBoundingClientRect();
          return r.right > innerWidth + 1 || r.left < -1;
        }).map((e) => (e.getAttribute('aria-label') || e.textContent.trim()).slice(0, 40));
        return { pageOverflow: document.documentElement.scrollWidth > innerWidth + 1, clipped };
      });
      report.settings[`${page}-${width}`] = m;
      await alice.screenshot({ path: join(shots, `settings-${page}${suffix}-${width}x${height}.png`) });
      if (strict) {
        assert.equal(m.pageOverflow, false, `${page} at ${width}px: the page scrolls sideways`);
        assert.deepEqual(m.clipped, [], `${page} at ${width}px: cut off`);
      }
    }
    await act(alice, 'close');
  }
  await alice.setViewportSize({ width: 1440, height: 900 });
  if (strict) checks.push('settings pages fit at 1440 and 390 px');
  // Nothing threw, logged an error or broke the content security policy.
  const csp = [];
  for (const p of [alice, bob]) csp.push(...await p.evaluate(() => window.__qaCsp ?? []));
  report.consoleErrors = consoleErrors;
  report.csp = csp;
  assert.deepEqual(errors, []);
  if (strict) {
    assert.deepEqual(consoleErrors, [], 'console errors');
    assert.deepEqual(csp, [], 'CSP violations');
    checks.push(`no page errors, console errors or CSP violations (${theme} theme)`);
  }
} catch (e) {
  for (const [p, n] of [[alice, 'alice'], [bob, 'bob']]) await p?.screenshot({ path: join(shots, `failure-${n}.png`) }).catch(() => {});
  console.error(h.log().slice(-3000));
  throw e;
} finally {
  await writeFile(join(shots, `integration-results${suffix}.json`), JSON.stringify({ revision: h.revision, strict, theme, checks, report }, null, 2));
  await h.close();
}
console.log(JSON.stringify({ checks, shots }, null, 2));

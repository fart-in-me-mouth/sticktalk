// Server identity in the real web app against a real local server (127.0.0.1
// only, headless): a server tag created in settings and worn from the profile
// editor, shown next to names in chat, the member list, direct messages and
// profiles, with its card; hiding other servers' tags; a gradient role style
// and a role icon; a cropped banner, an animated icon that plays only on
// hover, an invite background; a vanity link that joins through the join
// screen; invite links rendered as cards; leaving drops the tag. Saves each
// new surface at 1440x900 and 390x844 into STICKTALK_QA_DIR.
import assert from 'node:assert/strict';
import {readFile, writeFile} from 'node:fs/promises';
import {harness, register, form, act, shot, output, until, wait} from './qa-harness.mjs';

const h = await harness(), checks = [], screenshots = [], errors = [];
async function both(page, name, show) {
  const size = page.viewportSize();
  for (const [width, height] of [[1440, 900], [390, 844]]) {
    await page.setViewportSize({width, height});
    await wait(250);
    if (show) await page.locator(show).first().evaluate((e) => e.scrollIntoView({block: 'start'}));
    screenshots.push(await shot(page, name));
  }
  await page.setViewportSize(size);
}
const gif = await readFile('tests/fixtures/profile-avatar.gif');
const png = await readFile('tests/fixtures/profile-banner.png');
const api = (page, path) => page.evaluate(async (path) => {
  const r = await fetch(`/api/v1${path}`);
  return {status: r.status, body: r.ok ? await r.json() : null};
}, path);
async function say(page, text) {
  await page.locator('#live-input').fill(text);
  await page.locator('#live-input').press('Enter');
}
async function crop(page, input, file) {
  await page.locator(input).setInputFiles(file);
  await page.waitForSelector('#identity-crop[open]');
  await page.locator('#identity-crop [data-crop-zoom]').fill('130');
  await page.locator('#identity-apply, #identity-crop-apply').click();
}
const loaded = (page, part) => page.evaluate((part) => performance.getEntriesByType('resource').some((e) => e.name.includes(part)), part);

let alice, bob, carol;
try {
  alice = await h.phone(1440, 900); bob = await h.phone(1440, 900); carol = await h.phone(1440, 900);
  for (const p of [alice, bob, carol]) {
    p.on('pageerror', (e) => errors.push(String(e)));
    // Refused requests (the invalid invite on purpose) are not script errors.
    p.on('console', (m) => { if (m.type() === 'error' && !m.text().startsWith('Failed to load resource')) errors.push(`console: ${m.text()}`); });
  }
  await register(alice, h.origin, 'id_alice');
  await register(bob, h.origin, 'id_bob');
  await register(carol, h.origin, 'id_carol');
  const bobId = await bob.locator('[data-self-id]').getAttribute('data-self-id');
  // Nothing of this feature is fetched on a plain first load.
  for (const part of ['/public/identity.js', '/public/tags/']) assert.equal(await loaded(alice, part), false, part);
  checks.push('first load fetches neither identity.js nor any tag glyph');

  await form(alice, 'create', {'f-name': 'Identity QA'});
  await alice.waitForSelector('#live-input');
  const guild = (await alice.locator('.server-icon.selected').getAttribute('data-live')).slice(6);
  await form(alice, 'invite');
  await alice.waitForSelector('.live-modal code');
  const code = await alice.locator('.live-modal code').first().textContent();
  await act(alice, 'close');
  await form(bob, 'join', {'f-name': `${h.origin}/?invite=${code}`});
  await bob.waitForSelector('#live-input');

  // ---- Server tag: validated in the browser and on the server, saved.
  await act(alice, 'dialog:id-tags');
  await alice.waitForSelector('[data-live-form="id-tag"]');
  await alice.locator('#id-tag-text').fill('M0D');
  await alice.locator('[data-live-form="id-tag"] [type=submit]').click();
  await alice.waitForSelector('[data-toast="error"]');
  assert.match(await alice.locator('[data-toast="error"]').innerText(), /reserved/);
  await alice.locator('#id-tag-text').fill('GG');
  await alice.locator('[data-identity-glyph="star"]').click();
  await alice.locator('#id-tag-c1').fill('#1c1c50');
  await alice.locator('#id-tag-c2').fill('#1d1d52');
  await alice.waitForSelector('[data-identity-contrast]:not([hidden])');
  assert.equal(await alice.locator('[data-live-form="id-tag"] [type=submit]').isDisabled(), true);
  await alice.locator('#id-tag-c2').fill('#ffd37a');
  await alice.waitForSelector('[data-identity-contrast][hidden]', {state: 'attached'});
  await alice.waitForFunction(() => document.querySelector('[data-tag-preview] .tag-text')?.textContent === 'GG');
  await alice.locator('[data-live-form="id-tag"] [type=submit]').click();
  await alice.waitForSelector('[data-live-form="id-tag"] .form-saved');
  await both(alice, 'identity-tag-settings');
  await act(alice, 'close');
  checks.push('tag editor: reserved word refused, contrast warning, live preview, saved');

  // ---- Bob wears it from the profile editor.
  await act(bob, 'dialog:profile-edit');
  await bob.waitForSelector('.tag-picker');
  await bob.locator(`.tag-picker [data-live="id:wear,${guild}"]`).click();
  await bob.waitForSelector(`.tag-picker [data-live="id:wear,${guild}"][aria-pressed="true"]`);
  await both(bob, 'identity-tag-picker');
  await act(bob, 'close');
  await say(bob, 'Wearing the tag now');
  await alice.waitForFunction(() => [...document.querySelectorAll('.message')].some((m) => m.textContent.includes('Wearing the tag') && m.querySelector('.server-tag .tag-text')?.textContent === 'GG'));
  // The badge glyph is a mask on its own small file, fetched only now.
  await until(() => loaded(alice, '/public/tags/star.svg'));
  // The member list and a direct message row show it too.
  await alice.waitForFunction((id) => document.querySelector(`.member-row[data-live="profile:${id}"] .server-tag`) !== null, bobId);
  await both(alice, 'identity-chat-tag');
  // Clicking the tag opens its card.
  await alice.locator('.message .server-tag').first().click();
  await alice.waitForSelector('.tag-card h2');
  assert.equal(await alice.locator('.tag-card h2').innerText(), 'Identity QA');
  await both(alice, 'identity-tag-card');
  await act(alice, 'close');
  // A non-member sees the card too, with nothing joinable (no invite chosen).
  const card = await api(carol, `/tags/${guild}`);
  assert.equal(card.status, 200);
  assert.equal(card.body.join.kind, 'none');
  checks.push('tag worn: chat, member list, card for members and non-members, glyph fetched on demand');

  // ---- Role style and icon.
  await act(alice, 'dialog:roles');
  await alice.waitForSelector('[data-live-form=role-create]');
  await alice.locator('#f-name').fill('Sunset');
  await alice.locator('[data-live-form=role-create] [type=submit]').click();
  await alice.waitForSelector('[data-live^="role:"]:has-text("Sunset")');
  const role = (await alice.locator('[data-live^="role:"]:has-text("Sunset")').getAttribute('data-live')).slice(5);
  await act(alice, `role:${role}`);
  await alice.waitForSelector('#id-role-paint');
  await alice.locator('#f-value').fill('#ffb36b');
  await alice.locator('[data-live-form="role-save"] [type=submit]').click();
  await alice.waitForSelector('[data-live-form=role-create]');
  await act(alice, `role:${role}`);
  await alice.waitForSelector('#id-role-paint');
  await alice.locator('#id-role-paint').selectOption('gradient');
  await alice.locator('#id-role-c2').fill('#ff7ad9');
  await alice.locator(`[data-live="id:role-style,${role}"]`).click();
  await alice.waitForSelector('.live-modal .role-pill .role-gradient');
  await alice.locator('#id-role-emoji').fill('🌅');
  await alice.locator(`[data-live="id:role-icon,${role},emoji"]`).click();
  await alice.waitForSelector('.role-identity .role-icon.large.role-icon-emoji');
  await both(alice, 'identity-role-editor', '.role-identity h2');
  await act(alice, 'dialog:member-role');
  await alice.locator('[data-live-form=member-role] #f-name').selectOption(bobId);
  await alice.locator(`[data-member-role="${role}"]`).check();
  await alice.locator('[data-live-form=member-role] [type=submit]').click();
  await alice.waitForFunction(() => !document.querySelector('.live-modal'));
  await alice.waitForFunction((id) => !!document.querySelector(`.member-row[data-live="profile:${id}"] .role-paint.role-gradient`) && !!document.querySelector(`.member-row[data-live="profile:${id}"] .role-icon`), bobId);
  await alice.waitForFunction(() => [...document.querySelectorAll('.message .author .role-gradient')].length > 0);
  const fill = await alice.locator('.message .author .role-gradient > span').first().evaluate((e) => getComputedStyle(e).backgroundImage);
  assert.match(fill, /linear-gradient/);
  // "Role colors: off" (the accessibility setting) turns styles off.
  await alice.evaluate(() => document.documentElement.dataset.roleColors = 'off');
  assert.equal(await alice.locator('.message .author .role-gradient > span').first().evaluate((e) => getComputedStyle(e).backgroundImage), 'none');
  await alice.evaluate(() => delete document.documentElement.dataset.roleColors);
  await both(alice, 'identity-role-names');
  checks.push('gradient role style and emoji role icon: editor preview, chat, member list; role colors off honored');

  // ---- Appearance: banner (cropped), animated icon, invite background.
  await act(alice, 'dialog:id-appearance');
  await alice.waitForSelector('#id-pick-banner', {state: 'attached'});
  await crop(alice, '#id-pick-banner', {name: 'banner.png', mimeType: 'image/png', buffer: png});
  await alice.waitForSelector('.live-modal .guild-banner img');
  await crop(alice, '#id-pick-icon', {name: 'icon.gif', mimeType: 'image/gif', buffer: gif});
  await alice.waitForFunction(() => document.querySelector('.server-icon.selected .id-anim') !== null);
  // The page reloads its settings after each save; pick the next file after that.
  await alice.waitForSelector('.live-modal :text("Animated: it plays")');
  await crop(alice, '#id-pick-background', {name: 'bg.png', mimeType: 'image/png', buffer: png});
  await alice.waitForSelector('.live-modal .invite-hero img');
  await both(alice, 'identity-appearance', '.identity-image:last-of-type');
  await act(alice, 'close');
  await alice.waitForSelector('.channel-sidebar.has-banner .guild-banner img');
  // The animated icon is a still until hovered; then the animation is fetched.
  const anim = alice.locator('.server-icon.selected .id-anim');
  assert.equal(await anim.evaluate((e) => getComputedStyle(e).backgroundImage), 'none');
  await alice.locator('.server-icon.selected').hover();
  await alice.waitForFunction(() => getComputedStyle(document.querySelector('.server-icon.selected .id-anim')).backgroundImage.includes('/api/v1/files/'));
  await both(alice, 'identity-sidebar-banner');
  // On a phone the channel list is a drawer: show it open with the banner.
  await alice.setViewportSize({width: 390, height: 844});
  await act(alice, 'panel:left');
  await wait(400);
  screenshots.push(await shot(alice, 'identity-sidebar-banner-drawer'));
  await act(alice, 'panel:');
  await alice.setViewportSize({width: 1440, height: 900});
  await wait(300);
  // Scrolling the channel list never moves the layout: the banner scrolls with it.
  await wait(300);
  const before = await alice.locator('.channel-sidebar').boundingBox();
  await alice.locator('.channel-scroll').evaluate((e) => e.scrollTop = 200);
  assert.deepEqual(await alice.locator('.channel-sidebar').boundingBox(), before);
  // Reduced motion: nothing plays.
  await alice.emulateMedia({reducedMotion: 'reduce'});
  assert.equal(await anim.evaluate((e) => getComputedStyle(e).display), 'none');
  await alice.emulateMedia({reducedMotion: 'no-preference'});
  checks.push('banner cropped and shown over the channel list; animated icon plays only on hover, never with reduced motion');

  // ---- Vanity link.
  await act(alice, 'dialog:id-vanity');
  await alice.waitForSelector('#id-vanity-code');
  await alice.locator('#id-vanity-code').fill('admin');
  await alice.locator('[data-live-form="id-vanity"] [type=submit]').click();
  await alice.waitForSelector('[data-toast="error"]');
  await alice.locator('#id-vanity-code').fill('Identity-QA');
  await alice.locator('[data-live-form="id-vanity"] [type=submit]').click();
  await alice.waitForSelector('.live-modal .invite-code');
  assert.equal(await alice.locator('.live-modal .invite-code').innerText(), 'identity-qa');
  await both(alice, 'identity-vanity');
  await act(alice, 'close');
  checks.push('vanity link: reserved code refused, claimed (lowercased), shown through the invite code helper');

  // ---- Carol opens the vanity link: the join screen shows the background.
  await carol.goto(`${h.origin}/invite/identity-qa`);
  await carol.waitForSelector('.live-modal[data-dialog="join"] .invite-hero img');
  await carol.waitForSelector('.live-modal .invite-hero .server-tag');
  await both(carol, 'identity-join-screen');
  await carol.locator('[data-live-form=join] [type=submit]').click();
  await carol.waitForSelector('#live-input');
  checks.push('vanity link redirects to the join screen with background, banner fallback and tag; joining works');

  // ---- Invite links in chat become cards.
  await say(alice, `Come hang out ${h.origin}/invite/identity-qa and ${h.origin}/?invite=nope-not-real`);
  await carol.waitForSelector('.invite-card:not(.is-loading):not(.is-invalid) strong');
  await carol.waitForSelector('.invite-card.is-invalid');
  assert.equal(await carol.locator('.invite-card:not(.is-invalid) .button').first().innerText(), 'Joined');
  await both(carol, 'identity-invite-card');
  checks.push('invite links render as cards (valid and invalid), with Joined for members');

  // ---- Another server's tag, and hiding it.
  await form(carol, 'create', {'f-name': 'Carol Place'});
  await carol.waitForSelector('.server-icon.selected[aria-label="Carol Place"]');
  await carol.waitForSelector('#live-input');
  await act(carol, 'dialog:id-tags');
  await carol.waitForSelector('[data-live-form="id-tag"]');
  await carol.locator('#id-tag-text').fill('cc');
  await carol.locator('[data-live-form="id-tag"] [type=submit]').click();
  await carol.waitForSelector('[data-live-form="id-tag"] .form-saved');
  // Unique on the instance, ignoring case.
  await act(alice, 'dialog:id-tags');
  await alice.waitForSelector('[data-live-form="id-tag"]');
  await alice.locator('#id-tag-text').fill('CC');
  await alice.locator('[data-live-form="id-tag"] [type=submit]').click();
  await alice.waitForSelector('[data-toast="error"]');
  assert.match(await alice.locator('[data-toast="error"]').innerText(), /Another server/);
  await act(alice, 'close');
  await act(carol, 'dialog:profile-edit');
  const carolGuild = (await carol.locator('.server-icon.selected').getAttribute('data-live')).slice(6);
  await carol.locator(`.tag-picker [data-live="id:wear,${carolGuild}"]`).click();
  await carol.waitForSelector(`.tag-picker [data-live="id:wear,${carolGuild}"][aria-pressed="true"]`);
  await act(carol, 'close');
  await act(carol, `guild:${guild}`);
  await carol.waitForSelector('#live-input');
  await say(carol, 'From another server');
  await alice.waitForFunction(() => [...document.querySelectorAll('.message')].some((m) => m.textContent.includes('From another server') && m.querySelector('.server-tag .tag-text')?.textContent === 'cc'));
  await act(alice, 'dialog:id-tags');
  await alice.locator('#id-hide-tags').check();
  await alice.waitForFunction(() => document.querySelector('#id-hide-tags')?.checked);
  await act(alice, 'close');
  await alice.waitForFunction(() => { const m = [...document.querySelectorAll('.message')].find((m) => m.textContent.includes('From another server')); return m && !m.querySelector('.server-tag'); });
  // The server's own tag still shows.
  assert.ok(await alice.locator('.message .server-tag .tag-text:text-is("GG")').count() > 0);
  checks.push('tags are unique ignoring case; a server can hide other servers’ tags (its own still show)');

  // ---- Leaving drops the tag at once.
  await act(bob, 'leave-guild');
  await bob.waitForFunction(() => !document.querySelector('.server-icon.selected'));
  await until(async () => (await api(bob, '/users/@me/tags')).body?.wearing === null);
  await alice.waitForFunction((id) => !document.querySelector(`.member-row[data-live="profile:${id}"]`), bobId);
  checks.push('leaving the server drops the worn tag');

  // ---- The tag card's Join offer, for an invite configured on the tag.
  await act(alice, 'dialog:id-tags');
  await alice.locator('#id-tag-invite').selectOption('identity-qa');
  await alice.locator('[data-live-form="id-tag"] [type=submit]').click();
  await alice.waitForSelector('[data-live-form="id-tag"] .form-saved');
  const offer = await api(bob, `/tags/${guild}`);
  assert.deepEqual(offer.body.join, {kind: 'invite', code: 'identity-qa', apply: false});
  checks.push('a tag with a configured invite offers Join on its card');

  const csp = await Promise.all([alice, bob, carol].map((p) => p.evaluate(() => window.__qaCsp)));
  assert.deepEqual(csp.flat(), []);
  assert.deepEqual(errors, []);
  await writeFile(`${output}/identity-report.json`, JSON.stringify({checks, screenshots}, null, 2));
  console.log(`identity: ${checks.length} checks passed`);
} finally {
  if (errors.length) console.error(errors);
  await h.close();
}

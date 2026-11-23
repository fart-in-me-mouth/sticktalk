// Profiles and cosmetics in the real web app against a real local server
// (127.0.0.1 only, headless): the profile editor with cropping, animated
// avatars that play only on hover, banners, colors, name styles, decorations,
// nameplates, effects and frames, per-server profiles, moderator resets with
// the audit log, badges (including one the admin CLI awards), and the
// "Show plain names" / "Reduce cosmetics" settings. Saves each new surface at
// 1440x900 and 390x844 into STICKTALK_QA_DIR.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {readFile, writeFile} from 'node:fs/promises';
import {join} from 'node:path';
import {harness, register, form, act, shot, output, until, wait, target} from './qa-harness.mjs';

const h = await harness(), checks = [], screenshots = [], errors = [];
const server = process.env.STICKTALK_SERVER_EXE ?? join(target, 'debug/sticktalk-server.exe');
async function both(page, name) {
  const size = page.viewportSize();
  for (const [width, height] of [[1440, 900], [390, 844]]) {
    await page.setViewportSize({width, height});
    await wait(200);
    screenshots.push(await shot(page, name));
  }
  await page.setViewportSize(size);
}
const gif = await readFile('tests/fixtures/profile-avatar.gif');
const png = await readFile('tests/fixtures/profile-banner.png');
async function crop(page, input, file, zoom) {
  await page.locator(input).setInputFiles(file);
  await page.waitForSelector('#profiles-crop[open]');
  if (zoom) await page.locator('#profiles-crop [data-crop-zoom]').fill(String(zoom));
}
async function saveForm(page, kind) {
  await page.locator(`[data-live-form="${kind}"] .profile-save-bar [type=submit]`).click();
  await page.waitForSelector(`[data-live-form="${kind}"] .form-saved`);
}
const card = (page) => page.locator('.live-modal .profile-card');
let alice, bob;
try {
  alice = await h.phone(1440, 900); bob = await h.phone(1440, 900);
  for (const p of [alice, bob]) p.on('pageerror', (e) => errors.push(String(e)));
  await register(alice, h.origin, 'pf_alice');
  await register(bob, h.origin, 'pf_bob');
  const aliceId = await alice.locator('[data-self-id]').getAttribute('data-self-id');
  const bobId = await bob.locator('[data-self-id]').getAttribute('data-self-id');
  // Nothing styled is on screen yet: no name font has been fetched.
  const fonts = (page) => page.evaluate(() => performance.getEntriesByType('resource').map((e) => e.name).filter((n) => n.includes('/fonts/names/')));
  assert.deepEqual(await fonts(alice), []);

  // ---- User profile: text, colors, name style, cosmetics, live preview.
  await act(alice, 'dialog:profile-edit');
  await alice.waitForSelector('[data-live-form="profiles-user"]');
  await alice.locator('#f-name').fill('Alice Pixel');
  await alice.locator('#pf-pronouns').fill('she/her');
  await alice.locator('#pf-about').fill('I build **things** at https://example.com 🙂');
  await alice.locator('#pf-name-on').check();
  await alice.locator('#pf-name-font').selectOption('pixel');
  await alice.locator('#pf-name-effect').selectOption('gradient');
  // A dark name color cannot be saved.
  await alice.locator('#pf-name-c1').fill('#101012');
  await alice.waitForSelector('[data-pv="name-warning"]:not([hidden])');
  assert.equal(await alice.locator('.profile-save-bar [type=submit]').isDisabled(), true);
  await alice.locator('#pf-name-c1').fill('#ffd37a');
  await alice.locator('#pf-name-c2').fill('#3ddc97');
  await alice.waitForSelector('[data-pv="name-warning"][hidden]', {state: 'attached'});
  await alice.locator('#pf-colors-on').check();
  await alice.locator('#pf-primary').fill('#f4f1e8');
  await alice.locator('#pf-accent').fill('#ffd1e0');
  for (const [field, value] of [['pf-deco', 'halo'], ['pf-plate', 'forest'], ['pf-effect', 'snowfall'], ['pf-frame', 'neon']])
    await alice.locator(`[data-profiles="pick,${field},${value}"]`).click();
  // The preview follows every field: light colors turn the card text dark.
  await alice.waitForFunction(() => document.querySelector('[data-pv="card"]').classList.contains('dark-text'));
  assert.equal(await alice.locator('[data-pv="card"] [data-pv="deco"]').getAttribute('class'), 'avatar-deco cosmetic deco-halo is-self');
  assert.match(await alice.locator('[data-pv="name"] span').getAttribute('class'), /name-font-pixel name-effect-gradient/);
  assert.equal(await alice.locator('[data-pv="name"]').innerText(), 'Alice Pixel');
  await alice.waitForSelector('.profile-save-bar:not([hidden])');
  await both(alice, 'profile-editor-dirty');
  await saveForm(alice, 'profiles-user');
  await alice.waitForFunction(() => [...document.querySelectorAll('[data-pv="about"] span[style*="font-weight"]')].some((e) => e.textContent === 'things'));
  assert.equal(await alice.locator('[data-pv="about"] a').getAttribute('href'), 'https://example.com');
  // The pixel font is fetched now that a name uses it, and no other name font.
  await until(async () => (await fonts(alice)).length > 0);
  assert.ok((await fonts(alice)).every((f) => f.includes('press-start-2p')));
  checks.push('profile editor: bio with Markdown and links, pronouns, colors with readable text, name style with legibility check, four cosmetic pickers, live preview, lazy name font');

  // ---- Animated avatar, cropped; banner cropped to 5:2.
  await crop(alice, '#live-avatar', {name: 'moving.gif', mimeType: 'image/gif', buffer: gif}, 160);
  await both(alice, 'profile-crop-avatar');
  await alice.locator('#profiles-crop-apply').click();
  await alice.waitForSelector('.settings-identity .avatar-anim', {state: 'attached', timeout: 20000});
  const anim = await alice.locator('.settings-identity .avatar-anim').getAttribute('style');
  assert.match(anim, /--anim:url\(\/api\/v1\/files\/\d+\)/);
  assert.match(anim, /--crop-z:1\.6/);
  await crop(alice, '#live-banner', {name: 'banner.png', mimeType: 'image/png', buffer: png});
  await both(alice, 'profile-crop-banner');
  await alice.locator('#profiles-crop-apply').click();
  await alice.waitForSelector('[data-pv="banner"] img.banner-face', {timeout: 20000});
  const banner = await alice.locator('[data-pv="banner"] img.banner-face').evaluate((img) => new Promise((done) => img.complete ? done([img.naturalWidth, img.naturalHeight]) : img.onload = () => done([img.naturalWidth, img.naturalHeight])));
  assert.deepEqual(banner, [1500, 600]);
  // Saving and uploading leave the form as saved: nothing typed is undone.
  assert.equal(await alice.locator('#pf-pronouns').inputValue(), 'she/her');
  assert.equal(await alice.locator('#pf-colors-on').isChecked(), true);
  assert.equal(await alice.locator('.profile-save-bar').isHidden(), true);
  await both(alice, 'profile-editor');
  checks.push('animated GIF avatar kept animated with a CSS crop; still banner cropped to 1500x600 before upload');
  // The badges tab: Early Member is earned by a new account (default cutoff).
  await act(alice, 'profiles:tab,badges');
  await alice.waitForSelector('.badge-row[data-badge="early_member"]');
  await both(alice, 'profile-badges');
  await alice.locator('[data-profiles-badge="early_member"]').check();
  await alice.locator('[data-live-form="profiles-badges"] [type=submit]').click();
  await alice.waitForSelector('[data-live-form="profiles-badges"] .form-saved');
  await act(alice, 'close');

  // ---- A server where both are members.
  await form(alice, 'create', {'f-name': 'Profiles QA'}); await alice.waitForSelector('#live-input');
  const guild = (await alice.locator('.server-icon.selected').getAttribute('data-live')).slice(6);
  await form(alice, 'invite'); await alice.waitForSelector('.live-modal code');
  const code = await alice.locator('.live-modal code').first().textContent(); await act(alice, 'close');
  await form(bob, 'join', {'f-name': code}); await bob.waitForSelector('#live-input');
  for (const [p, text] of [[alice, 'Hello from Alice'], [bob, 'Hi Alice']]) {
    await p.locator('#live-input').fill(text); await p.locator('#live-input').press('Enter');
    await p.waitForSelector(`.message:has-text("${text}")`);
  }
  await bob.waitForSelector('.message:has-text("Hello from Alice")');
  const aliceMessage = bob.locator('.message.group-head:has-text("Hello from Alice")');
  await bob.waitForFunction(() => document.querySelector('.message .author .styled-name.name-font-pixel'));
  assert.equal(await aliceMessage.locator('.avatar-deco.deco-halo').count(), 1);
  // Lists show the still frame; the animation is only fetched while hovered.
  const background = () => aliceMessage.locator('.avatar-anim').evaluate((e) => getComputedStyle(e).backgroundImage);
  assert.equal(await background(), 'none');
  await aliceMessage.hover();
  await until(async () => (await background()).includes('/api/v1/files/'));
  await bob.locator('#live-input').hover();
  await until(async () => (await background()) === 'none');
  await bob.waitForSelector(`.member-row[data-live="profile:${aliceId}"] .nameplate.plate-forest`);
  await aliceMessage.hover();
  await both(bob, 'chat-styled-names');
  checks.push('messages: styled name and decoration; animated avatar decoded only while hovered; member list nameplate');

  // ---- The profile card another member sees.
  await act(bob, `profile:${aliceId}`);
  await bob.waitForSelector('.live-modal .profile-card .profile-badges img[alt="Early Member"]');
  assert.equal(await card(bob).locator('.profile-effect.effect-snowfall').count(), 1);
  assert.equal(await card(bob).locator('.profile-frame.frame-neon').count(), 1);
  assert.equal(await card(bob).locator('[data-pv="about"] span[style*="font-weight"]').first().textContent(), 'things');
  await both(bob, 'profile-card');
  await act(bob, 'close');

  // ---- Per-server profile, from the server menu: nickname, look, server avatar.
  await act(alice, `profiles:open-server,${guild}`);
  await alice.waitForSelector('[data-live-form="profiles-server"]');
  await alice.locator('#pf-nickname').fill('Ali in QA');
  await alice.locator('#pf-about').fill('Only in this server.');
  await alice.locator('[data-profiles="pick,pf-deco,crown"]').click();
  await alice.waitForFunction(() => document.querySelector('[data-live-form="profiles-server"]').closest('.profile-editor').querySelector('[data-pv="deco"]').classList.contains('deco-crown'));
  await saveForm(alice, 'profiles-server');
  await both(alice, 'server-profile-editor');
  await act(alice, 'close');
  await bob.waitForFunction(() => [...document.querySelectorAll('.message .author')].some((a) => a.textContent === 'Ali in QA'));
  await bob.waitForSelector('.message.group-head:has-text("Hello from Alice") .avatar-deco.deco-crown');
  await act(bob, `profile:${aliceId}`);
  await bob.waitForFunction(() => document.querySelector('.live-modal .profile-card [data-pv="about"]')?.textContent.includes('Only in this server.'));
  await both(bob, 'profile-card-server');
  await act(bob, 'close');
  checks.push('server profile from the server menu: nickname, bio and decoration shown to members in that server');

  // Bob gives himself a server avatar; Alice (the owner) removes it, audited.
  await act(bob, `profiles:open-server,${guild}`);
  await bob.waitForSelector('[data-live-form="profiles-server"]');
  await crop(bob, '#live-avatar', {name: 'still.png', mimeType: 'image/png', buffer: png});
  await bob.locator('#profiles-crop-apply').click();
  await bob.waitForSelector(`[data-live="profiles:remove,avatar,${guild}"]`, {timeout: 20000});
  await act(bob, 'close');
  await act(alice, `profile:${bobId}`);
  await alice.waitForSelector(`[data-live="profiles:reset,${guild},${bobId},avatar"]`);
  await both(alice, 'profile-card-moderation');
  await alice.locator(`[data-live="profiles:reset,${guild},${bobId},avatar"]`).click();
  await alice.waitForSelector('#live-confirm');
  await alice.locator('#live-confirm [data-accept]').click();
  await until(async () => (await alice.evaluate(async (g) => (await (await fetch(`/api/v1/guilds/${g}/audit-log?action=member_profile_reset`)).json()).items?.length ?? 0, guild)) === 1);
  // A member without Time Out Members cannot do the same.
  checks.push('moderator removes a member server avatar; the audit log records it');

  // ---- An instance administrator badge from the admin CLI.
  execFileSync(server, ['--data-dir', h.data, 'admin', 'badge', 'award', 'contributor', 'pf_alice'], {windowsHide: true, stdio: 'ignore'});
  await act(bob, `profile:${aliceId}`);
  await bob.waitForSelector('.live-modal .profile-card .profile-badges img[alt="Early Member"]');
  await act(alice, 'dialog:profile-edit'); await act(alice, 'profiles:tab,badges');
  await alice.waitForSelector('.badge-row[data-badge="contributor"]');
  await alice.locator('.badge-row[data-badge="contributor"] [data-profiles="badge-up"]').click();
  await alice.locator('[data-profiles-badge="contributor"]').check();
  await alice.locator('[data-live-form="profiles-badges"] [type=submit]').click();
  await alice.waitForSelector('[data-live-form="profiles-badges"] .form-saved');
  await act(alice, 'close');
  await act(bob, 'close'); await act(bob, `profile:${aliceId}`);
  await bob.waitForSelector('.live-modal .profile-card .profile-badges img[alt="Contributor"]');
  const order = await bob.locator('.live-modal .profile-badges img').evaluateAll((es) => es.map((e) => e.alt));
  assert.deepEqual(order, ['Contributor', 'Early Member']);
  await act(bob, 'close');
  checks.push('admin CLI awards a badge; the member chooses and orders shown badges');

  // ---- Viewer settings: plain names and reduced cosmetics.
  await act(bob, 'dialog:appearance');
  await bob.locator('[data-preference="plainNames"]').check();
  await bob.locator('[data-preference="reduceCosmetics"]').check();
  await both(bob, 'appearance-profiles');
  await act(bob, 'close');
  const font = await bob.locator('.message .author .styled-name').first().evaluate((e) => getComputedStyle(e).fontFamily);
  assert.ok(!font.includes('ST Pixel'), font);
  assert.equal(await aliceMessage.locator('.avatar-deco').evaluate((e) => getComputedStyle(e).display), 'none');
  checks.push('Show plain names drops other people\'s styles; Reduce cosmetics hides their decorations');
  // Reduced motion: no animation plays anywhere, even on hover.
  await bob.emulateMedia({reducedMotion: 'reduce'});
  await aliceMessage.hover();
  assert.equal(await aliceMessage.locator('.avatar-anim').evaluate((e) => getComputedStyle(e).display), 'none');
  checks.push('reduced motion disables animated avatars and effects');

  // Phone: the editor and a card at 390 px are covered by both() above.
  assert.deepEqual(errors, []);
  assert.deepEqual(await alice.evaluate(() => window.__qaCsp), []);
  await writeFile(`${output}/profiles-results.json`, JSON.stringify({revision: h.revision, checks, screenshots, errors}, null, 2));
  console.log(JSON.stringify({checks, screenshots: screenshots.length, errors}));
} catch (e) {
  await writeFile(`${output}/profiles-failure.json`, JSON.stringify({error: String(e.stack ?? e), checks, errors, log: h.log().slice(-4000),
    alice: await alice?.locator('body').innerText().catch(() => ''), bob: await bob?.locator('body').innerText().catch(() => '')}, null, 2));
  for (const [name, p] of [['alice', alice], ['bob', bob]]) await p?.screenshot({path: `${output}/profiles-failure-${name}.png`}).catch(() => {});
  throw e;
} finally { await h.close(); }

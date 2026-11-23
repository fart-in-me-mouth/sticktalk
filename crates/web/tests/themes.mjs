// Themes, appearance and accessibility in the real web app against a real
// local server (127.0.0.1 only, headless): base themes (dark, light, darker,
// sync with system, followed live), preset themes, the theme editor with
// enforced contrast, share codes and links, server themes and the per-member
// opt-out, accessibility controls, text-to-speech (speech synthesis mocked,
// nothing is ever spoken), Streamer Mode, and app icons. Measures the
// contrast of the text actually rendered in every theme. Saves each new
// surface at 1440x900 and 390x844 into STICKTALK_QA_DIR.
import assert from 'node:assert/strict';
import {writeFile} from 'node:fs/promises';
import {harness, register, form, act, shot, output, until, wait} from './qa-harness.mjs';

const h = await harness(), checks = [], screenshots = [], errors = [], contrast = {};
async function both(page, name) {
  const size = page.viewportSize();
  for (const [width, height] of [[1440, 900], [390, 844]]) {
    await page.setViewportSize({width, height});
    await wait(250);
    screenshots.push(await shot(page, name));
  }
  await page.setViewportSize(size);
  await wait(150);
}
const token = (page, name) => page.evaluate((n) => getComputedStyle(document.documentElement).getPropertyValue(n).trim().toLowerCase(), name);
const attr = (page, name) => page.evaluate((n) => document.documentElement.dataset[n], name);
const sheet = (page) => page.evaluate(() => [...(globalThis.__sticktalkThemeSheet?.cssRules ?? [])].map((r) => r.cssText).join(''));
// The WCAG contrast of every visible text element against what is painted
// behind it (the nearest opaque background). Text dimmed on purpose (disabled,
// offline members at 60 %) and text over images is left out.
async function audit(page, label) {
  const result = await page.evaluate(() => {
    const parse = (c) => { const m = c.match(/rgba?\(([^)]+)\)/); if (!m) return null; const p = m[1].split(/[ ,/]+/).filter(Boolean).map(Number); return {r: p[0], g: p[1], b: p[2], a: p.length > 3 ? p[3] : 1}; };
    const lin = (v) => { v /= 255; return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; };
    const lum = (c) => 0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b);
    const over = (top, under) => ({r: top.r * top.a + under.r * (1 - top.a), g: top.g * top.a + under.g * (1 - top.a), b: top.b * top.a + under.b * (1 - top.a), a: 1});
    const ratio = (x, y) => { const [a, b] = [lum(x), lum(y)]; return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05); };
    function background(el) {
      const layers = [];
      for (let e = el; e; e = e.parentElement) {
        const s = getComputedStyle(e);
        if (s.backgroundImage !== 'none' && /url\(/.test(s.backgroundImage)) return null;
        const c = parse(s.backgroundColor);
        if (c && c.a > 0) { layers.push(c); if (c.a >= 0.99) break; }
      }
      let bg = {r: 0, g: 0, b: 0, a: 1};
      for (const c of layers.reverse()) bg = c.a >= 0.99 ? c : over(c, bg);
      return bg;
    }
    let worst = 99, count = 0; const failures = [];
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    const seen = new Set();
    while (walker.nextNode()) {
      const el = walker.currentNode.parentElement;
      if (!el || seen.has(el) || !walker.currentNode.textContent.trim()) continue;
      seen.add(el);
      const r = el.getBoundingClientRect();
      if (r.width < 2 || r.height < 2 || r.bottom < 0 || r.top > innerHeight || r.right < 0 || r.left > innerWidth) continue;
      if (el.closest('[hidden],[inert] .live-shell,.sr-only,button:disabled,[aria-disabled="true"],svg,.styled-name,.role-color,.profile-card.tinted,.theme-swatch,.theme-preview,.app-icon-option,.attachment-spoiler,.live-spoiler:not(.revealed),.skeleton-shell,.chat-gif,.gif-category,.streamer-indicator,.mention-badge,.live-unread')) continue;
      let dim = false; for (let e = el; e; e = e.parentElement) { const s = getComputedStyle(e); if (Number(s.opacity) < 1 || s.visibility === 'hidden' || s.filter !== 'none') { dim = true; break; } }
      if (dim) continue;
      const s = getComputedStyle(el), fg = parse(s.color), bg = background(el);
      if (!fg || !bg || fg.a === 0) continue;
      const c = ratio(over(fg, bg), bg);
      const large = parseFloat(s.fontSize) >= 24 || (parseFloat(s.fontSize) >= 18.66 && Number(s.fontWeight) >= 700);
      count++; worst = Math.min(worst, c);
      if (c < (large ? 3 : 4.5)) failures.push({text: el.textContent.trim().slice(0, 40), cls: String(el.className).slice(0, 60), ratio: Math.round(c * 100) / 100});
    }
    return {count, worst: Math.round(worst * 100) / 100, failures: failures.slice(0, 12)};
  });
  contrast[label] = result;
  assert.ok(result.count > 10, `${label}: too little text measured`);
  assert.deepEqual(result.failures, [], `${label}: text below AA`);
  return result;
}
let alice, bob;
try {
  alice = await h.phone(1440, 900); bob = await h.phone(1440, 900);
  for (const p of [alice, bob]) {
    p.on('pageerror', (e) => errors.push(String(e)));
    // Speech synthesis is mocked: nothing is ever spoken aloud.
    await p.addInitScript(() => {
      window.__spoken = [];
      globalThis.__sticktalkSpeech = (text, rate) => window.__spoken.push([text, rate]);
      // What the page looks like before the app's code runs (theme-boot.js).
      document.addEventListener('DOMContentLoaded', () => {
        window.__boot = {base: document.documentElement.dataset.themeBase, gradient: document.documentElement.dataset.themeGradient,
          chat: getComputedStyle(document.documentElement).getPropertyValue('--bg-chat').trim(), icon: document.querySelector('link[rel~="icon"]')?.getAttribute('href')};
      });
    });
  }
  await register(alice, h.origin, 'th_alice');
  await register(bob, h.origin, 'th_bob');
  await form(alice, 'create', {'f-name': 'Themes QA'}); await alice.waitForSelector('#live-input');
  await form(alice, 'invite'); await alice.waitForSelector('.live-modal code');
  const code = await alice.locator('.live-modal code').first().textContent(); await act(alice, 'close');
  await form(bob, 'join', {'f-name': code}); await bob.waitForSelector('#live-input');
  for (const [p, text] of [[alice, 'Hello from Alice, see https://example.com'], [bob, `Hi @th_alice, a long message to check wrapping and spacing in every theme, with **bold** and \`code\`.`]]) {
    await p.locator('#live-input').fill(text); await p.locator('#live-input').press('Enter');
    await p.waitForSelector(`.message:has-text("${text.slice(0, 16)}")`);
  }
  await alice.waitForSelector('.message:has-text("Hi @th_alice")');

  // ---- The default dark theme is exactly UI_SPEC's: nothing derived, nothing overridden.
  assert.equal(await token(alice, '--bg-chat'), '#1c1c20');
  assert.equal(await token(alice, '--text-muted'), '#a0a0aa');
  assert.equal(await sheet(alice), '');
  assert.equal(await alice.evaluate(() => getComputedStyle(document.body).backgroundColor), 'rgb(17, 17, 19)');
  await audit(alice, 'dark');
  checks.push('default dark theme: UI_SPEC tokens, no derived sheet, text meets AA');

  // ---- Appearance page; a theme change does not re-render it.
  await act(alice, 'dialog:appearance');
  await alice.waitForSelector('input[data-theme-choice][value="light"]');
  await both(alice, 'themes-appearance');
  await alice.evaluate(() => { document.querySelector('input[data-theme-choice][value="light"]').__kept = true; });
  await alice.locator('label.theme-option:has(input[value="light"])').click();
  await until(async () => (await attr(alice, 'themeBase')) === 'light');
  assert.equal(await token(alice, '--bg-chat'), '#ffffff');
  assert.equal(await alice.evaluate(() => document.querySelector('input[data-theme-choice][value="light"]').__kept), true, 'the page was not rebuilt');
  assert.equal(await alice.evaluate(() => document.querySelector('meta[name="theme-color"]').content.toLowerCase()), '#e3e5e8');
  await both(alice, 'themes-appearance-light');
  await act(alice, 'close');
  await audit(alice, 'light');
  await both(alice, 'themes-chat-light');
  checks.push('Light theme from the picker: tokens, theme-color, no re-render of the page, text meets AA');
  // Darker (OLED).
  await act(alice, 'appearance:theme,darker');
  await until(async () => (await token(alice, '--bg-chat')) === '#000000');
  await audit(alice, 'darker');
  await both(alice, 'themes-chat-darker');
  // Sync with system follows the OS live, with no script running.
  await act(alice, 'appearance:theme,system');
  await alice.emulateMedia({colorScheme: 'light'});
  await until(async () => (await token(alice, '--bg-chat')) === '#ffffff');
  await alice.emulateMedia({colorScheme: 'dark'});
  await until(async () => (await token(alice, '--bg-chat')) === '#1c1c20');
  checks.push('Darker theme; Sync with system follows the OS setting live');

  // ---- A preset: derived tokens and a gradient, and it is there before first paint after a reload.
  await act(alice, 'appearance:theme,preset:aurora');
  await until(async () => (await attr(alice, 'themeGradient')) === 'true');
  assert.match(await sheet(alice), /--surface-chat: linear-gradient/);
  assert.match(await alice.locator('.app-shell').first().evaluate((e) => getComputedStyle(e).backgroundImage), /linear-gradient/);
  await audit(alice, 'preset aurora');
  await both(alice, 'themes-chat-aurora');
  const aurora = await token(alice, '--bg-chat');
  await alice.reload();
  await alice.waitForSelector('.server-icon:not(.brand):not(.favorites-icon)');
  await until(async () => { await act(alice, await alice.locator('.server-icon[data-live^="guild:"]').first().getAttribute('data-live')); return (await alice.locator('#live-input').count()) > 0; });
  const boot = await alice.evaluate(() => window.__boot);
  assert.equal(boot.gradient, 'true');
  assert.equal(boot.chat.toLowerCase(), aurora, 'painted before the app loaded');
  await act(alice, 'appearance:theme,preset:sky');
  await until(async () => (await attr(alice, 'themeBase')) === 'light');
  await audit(alice, 'preset sky');
  await both(alice, 'themes-chat-sky');
  checks.push('presets: derived tokens and gradient surfaces, applied before first paint on reload, text meets AA (dark and light based)');
  for (const id of ['midnight', 'ember', 'neon', 'peach', 'lemonade', 'glacier']) {
    await act(alice, `appearance:theme,preset:${id}`);
    await wait(150);
    await audit(alice, `preset ${id}`);
  }
  checks.push('rendered text meets AA in 8 presets');

  // ---- The theme editor: enforced contrast, save, share.
  await act(alice, 'dialog:appearance');
  await act(alice, 'appearance:edit,new');
  await alice.waitForSelector('[data-theme-editor] #te-accent');
  await alice.locator('#te-name').fill('Night Shift');
  await alice.locator('#te-accent').fill('#101030');
  await alice.locator('#te-accent').dispatchEvent('change');
  await alice.waitForSelector('.theme-warning:has-text("changed to")');
  await both(alice, 'themes-editor');
  await act(alice, 'appearance:editor-fix-accent');
  await alice.waitForSelector('.theme-warning', {state: 'detached'});
  await act(alice, 'appearance:editor-color-add');
  await alice.waitForSelector('#te-color-2');
  const share = await alice.locator('#te-code').inputValue();
  await act(alice, 'appearance:editor-save');
  await alice.waitForSelector('input[data-theme-choice][value^="custom:"]');
  await until(async () => (await alice.evaluate(() => JSON.parse(localStorage.getItem('sticktalk-appearance')).theme)) === 'custom');
  await act(alice, 'close');
  await audit(alice, 'custom theme');
  checks.push('theme editor: an unreadable accent is flagged and fixed, saved theme (synced) is applied, share code shown');
  // Untrusted codes are refused; a shared link opens a preview first.
  await act(bob, 'dialog:appearance');
  await bob.locator('#theme-code').fill('AQ!!garbage');
  await act(bob, 'appearance:import');
  await bob.waitForSelector('#theme-code-help:has-text("not a theme code")');
  await bob.goto(`${h.origin}/?theme=${share}`);
  await bob.waitForSelector('.live-modal h2:has-text("Night Shift")', {timeout: 30000});
  assert.equal(await bob.evaluate(() => location.search), '', 'the code is taken out of the address');
  await both(bob, 'themes-shared');
  await act(bob, 'appearance:shared-save');
  await bob.waitForSelector('input[data-theme-choice][value^="custom:"]');
  await act(bob, 'close');
  checks.push('a shared theme link opens a preview, garbage codes are refused, Save copies it to the account');

  // ---- Server themes: Manage Server only; members can keep their own.
  await act(alice, 'appearance:theme,dark');
  await act(bob, 'appearance:theme,dark');
  await until(async () => { await act(bob, await bob.locator('.server-icon[data-live^="guild:"]').first().getAttribute('data-live')); return (await bob.locator('#live-input').count()) > 0; });
  await act(bob, 'dialog:server-theme');
  await bob.waitForSelector('fieldset[disabled] input[value="ocean"]');
  await act(bob, 'close');
  await act(alice, 'dialog:server-theme');
  await alice.waitForSelector('input[name="server-theme"][value="ocean"]');
  await alice.locator('label.theme-option:has(input[value="ocean"])').click();
  await alice.waitForSelector('[data-toast="notice"]');
  await both(alice, 'themes-server-settings');
  await act(alice, 'close');
  await until(async () => /--surface-chat/.test(await sheet(bob)));
  const ocean = await token(bob, '--bg-chat');
  await audit(bob, 'server theme ocean');
  await both(bob, 'themes-chat-server-theme');
  // Away from the server it is gone; back in it, it returns.
  await act(bob, 'home');
  await until(async () => (await sheet(bob)) === '');
  await act(bob, await bob.locator('.server-icon[data-live^="guild:"]').first().getAttribute('data-live'));
  await until(async () => (await token(bob, '--bg-chat')) === ocean);
  // Turned off for this person, then back on.
  await act(bob, 'dialog:appearance');
  await bob.locator('[data-appearance="server-themes,default"]').uncheck();
  await until(async () => (await sheet(bob)) === '');
  await bob.locator('[data-appearance="server-themes,default"]').check();
  await until(async () => (await token(bob, '--bg-chat')) === ocean);
  await act(bob, 'close');
  checks.push('server theme: members only see it in that server; Manage Server needed; members can turn server themes off');

  // ---- Accessibility (on Home: the server's theme is the server's own).
  await act(alice, 'home');
  await until(async () => (await sheet(alice)) === '');
  await act(alice, 'dialog:accessibility');
  await alice.waitForSelector('[data-preference="saturation"]');
  await both(alice, 'themes-accessibility');
  await alice.locator('[data-preference="saturation"]').fill('0');
  await alice.locator('[data-preference="saturation"]').dispatchEvent('change');
  await until(async () => (await token(alice, '--accent')) !== '#3ddc97');
  const gray = await token(alice, '--accent');
  assert.match(gray, /^#([0-9a-f]{2})\1\1$/, `fully desaturated accent ${gray}`);
  assert.equal(await attr(alice, 'roleAdjust'), 'true');
  await alice.locator('[data-preference="saturation"]').fill('100');
  await alice.locator('[data-preference="saturation"]').dispatchEvent('change');
  await alice.locator('[data-preference="contrastBoost"]').check();
  await until(async () => (await token(alice, '--text-primary')) === '#ffffff');
  await alice.locator('label:has(input[name="pref-roleColors"][value="dot"])').click();
  await alice.locator('label:has(input[name="pref-motion"][value="on"])').click();
  assert.equal(await attr(alice, 'motion'), 'reduce');
  await alice.locator('[data-preference="focusAlways"]').check();
  await alice.locator('[data-preference="autoplayEmoji"]').selectOption('never');
  // The dyslexia-friendly font is fetched only once it is chosen.
  const chatFonts = () => alice.evaluate(() => performance.getEntriesByType('resource').map((e) => e.name).filter((n) => n.includes('/fonts/chat/')));
  assert.deepEqual(await chatFonts(), []);
  await act(alice, 'dialog:appearance');
  await alice.locator('[data-preference="chatFont"]').selectOption('dyslexic');
  await alice.locator('[data-preference="groupSpacing"]').fill('24');
  await alice.locator('[data-preference="groupSpacing"]').dispatchEvent('change');
  await act(alice, 'close');
  await until(async () => { await act(alice, await alice.locator('.server-icon[data-live^="guild:"]').first().getAttribute('data-live')); return (await alice.locator('.message .markdown').count()) > 0; });
  await until(async () => (await chatFonts()).some((f) => f.includes('opendyslexic')));
  assert.equal(await alice.locator('.message.group-head').first().evaluate((e) => getComputedStyle(e).marginTop), '24px');
  assert.match(await alice.locator('.message .markdown').first().evaluate((e) => getComputedStyle(e).fontFamily), /OpenDyslexic/);
  assert.equal(await alice.locator('.animated, .message').first().evaluate(() => document.documentElement.dataset.autoplayEmoji), 'never');
  await audit(alice, 'contrast boost');
  await both(alice, 'themes-chat-accessibility');
  checks.push('accessibility: saturation, contrast boost, role color dots, reduced motion on, always show focus, autoplay, lazily loaded dyslexia font, group spacing');
  await act(alice, 'dialog:accessibility');
  await alice.locator('[data-preference="contrastBoost"]').uncheck();
  await alice.locator('label:has(input[name="pref-motion"][value="system"])').click();
  await act(alice, 'close');

  // ---- Text-to-speech (mocked).
  await act(alice, 'appearance:tts-preview');
  await until(async () => (await alice.evaluate(() => window.__spoken.length)) === 1);
  await bob.locator('#live-input').fill('/tts read this aloud please');
  await bob.locator('#live-input').press('Enter');
  await bob.waitForSelector('.message:has-text("read this aloud please")');
  await until(async () => (await alice.evaluate(() => window.__spoken.map((s) => s[0]))).some((t) => /said read this aloud please/.test(t)));
  assert.ok(!(await bob.evaluate(() => window.__spoken.length)), 'the sender does not hear it');
  const id = await alice.locator('.message:has-text("read this aloud please")').getAttribute('data-live-message');
  await act(alice, `appearance:speak,${id}`);
  await until(async () => (await alice.evaluate(() => window.__spoken.length)) === 3);
  // Not in Do Not Disturb.
  await act(alice, 'privacy:status,dnd');
  await wait(400);
  await bob.locator('#live-input').fill('/tts quiet please');
  await bob.locator('#live-input').press('Enter');
  await alice.waitForSelector('.message:has-text("quiet please")');
  await wait(400);
  assert.equal(await alice.evaluate(() => window.__spoken.length), 3);
  await act(alice, 'privacy:status,online');
  checks.push('text-to-speech: preview, /tts messages read to viewers (not the sender), Speak Message, silent in Do Not Disturb');

  // ---- Streamer Mode: keybind, hidden invites revealed once, auto on screen share.
  await alice.keyboard.press('Control+Alt+Shift+KeyS');
  await alice.waitForSelector('#streamer-indicator');
  await form(alice, 'invite'); await alice.waitForSelector('.live-modal code.streamer-invite');
  const invite = alice.locator('.live-modal code.streamer-invite').first();
  assert.equal(await invite.evaluate((e) => getComputedStyle(e).color), 'rgba(0, 0, 0, 0)');
  await both(alice, 'themes-streamer-invites');
  await invite.click();
  assert.notEqual(await invite.evaluate((e) => getComputedStyle(e).color), 'rgba(0, 0, 0, 0)');
  await act(alice, 'close');
  await alice.keyboard.press('Control+Alt+Shift+KeyS');
  await alice.waitForSelector('#streamer-indicator', {state: 'detached'});
  await act(alice, 'dialog:streamer');
  await alice.locator('[data-preference="streamerAuto"]').check();
  await both(alice, 'themes-streamer');
  await act(alice, 'close');
  await alice.evaluate(() => dispatchEvent(new CustomEvent('sticktalk-sharing', {detail: true})));
  await alice.waitForSelector('#streamer-indicator');
  await alice.evaluate(() => dispatchEvent(new CustomEvent('sticktalk-sharing', {detail: false})));
  await alice.waitForSelector('#streamer-indicator', {state: 'detached'});
  checks.push('Streamer Mode: keybind and indicator, invite codes hidden with reveal once, turns on with a screen share and back off after');

  // ---- App icons.
  await act(alice, 'dialog:appearance');
  await alice.locator('label.app-icon-option:has(input[value="ocean"])').click();
  await until(async () => (await alice.evaluate(() => document.querySelector('link[rel~="icon"]').getAttribute('href'))).endsWith('/public/app-icons/ocean-192.png'));
  assert.match(await alice.evaluate(() => document.querySelector('link[rel="manifest"]').getAttribute('href')), /app-icons\/ocean\.webmanifest$/);
  await act(alice, 'close');
  await until(async () => (await alice.locator('.server-icon.brand img').getAttribute('src')).endsWith('ocean-192.png'));
  const manifest = await alice.evaluate(async () => (await fetch('/public/app-icons/ocean.webmanifest')).json());
  assert.equal(manifest.id, '/');
  assert.equal(manifest.icons[0].src, '/public/app-icons/ocean-192.png');
  await alice.reload(); await alice.waitForSelector('.server-icon.brand img');
  assert.equal((await alice.evaluate(() => window.__boot)).icon, '/public/app-icons/ocean-192.png');
  checks.push('app icon: favicon, in-app logo and manifest change at once and before first paint after a reload');

  assert.deepEqual(errors, []);
  for (const p of [alice, bob]) assert.deepEqual(await p.evaluate(() => window.__qaCsp), []);
  await writeFile(`${output}/themes-results.json`, JSON.stringify({revision: h.revision, checks, contrast, screenshots, errors}, null, 2));
  console.log(JSON.stringify({checks, contrast, screenshots: screenshots.length, errors}, null, 1));
} catch (e) {
  await writeFile(`${output}/themes-failure.json`, JSON.stringify({error: String(e.stack ?? e), checks, contrast, errors, log: h.log().slice(-4000),
    alice: await alice?.locator('body').innerText().catch(() => ''), bob: await bob?.locator('body').innerText().catch(() => '')}, null, 2));
  for (const [name, p] of [['alice', alice], ['bob', bob]]) await p?.screenshot({path: `${output}/themes-failure-${name}.png`}).catch(() => {});
  throw e;
} finally { await h.close(); }

import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import { harness, register, form, act, shot, until, output } from './qa-harness.mjs';
import { engine } from './engines.mjs';
const h = await harness(), checks = [], errors = [], screenshots = [];
let page;
try {
  page = await h.phone();
  page.on('pageerror', e => errors.push(String(e)));
  await page.addInitScript(() => {
    // Storage-denied/private browsing edge, not a claim about all private modes.
    for (const key of ['localStorage', 'sessionStorage', 'indexedDB'])
      Object.defineProperty(window, key, { configurable: true, get() { throw new DOMException('Storage disabled', 'SecurityError'); } });
  });
  await register(page, h.origin, 'qa_private');
  const capabilities = await page.evaluate(() => ({
    userAgent: navigator.userAgent, secure: isSecureContext, width: innerWidth,
    viewport: { width: visualViewport.width, height: visualViewport.height },
    touch: navigator.maxTouchPoints, rtc: typeof RTCPeerConnection,
    audioContext: typeof AudioContext, getUserMedia: typeof navigator.mediaDevices?.getUserMedia,
    h264: globalThis.RTCRtpReceiver?.getCapabilities('video')?.codecs.filter(c => /h264/i.test(c.mimeType)),
    mp4: document.createElement('video').canPlayType('video/mp4; codecs="avc1.42E01E"'),
  }));
  await form(page, 'create', { 'f-name': 'Private browsing' });
  await page.waitForSelector('#live-input');
  await act(page, 'dialog:appearance');
  await page.locator('[data-preference=density]').selectOption('compact');
  await act(page, 'close');
  assert.equal(await page.evaluate(() => document.documentElement.dataset.density), 'compact');
  await page.locator('#live-input').fill('Message without persistent browser storage');
  await page.getByRole('button', { name: 'Send Message', exact: true }).click();
  await page.waitForSelector('.markdown');
  checks.push('blocked localStorage/sessionStorage/IndexedDB: account, preferences and messaging remain usable');
  await page.locator('#live-input').focus();
  const bounds = await page.evaluate(() => {
    Object.defineProperties(visualViewport, { height: { configurable: true, value: 400 }, offsetTop: { configurable: true, value: 100 } });
    visualViewport.dispatchEvent(new Event('resize')); visualViewport.dispatchEvent(new Event('scroll'));
    const input = document.querySelector('#live-input').getBoundingClientRect();
    return { bottom: input.bottom, header: document.querySelector('.channel-header').getBoundingClientRect().top };
  });
  assert(bounds.bottom <= 500 && bounds.header >= 100, JSON.stringify(bounds));
  screenshots.push(await shot(page, 'visual-viewport-panned'));
  await page.evaluate(() => {
    delete visualViewport.height; delete visualViewport.offsetTop;
    visualViewport.dispatchEvent(new Event('resize'));
  });
  checks.push('visualViewport resize AND offsetTop pan keep composer above simulated keyboard');
  await act(page, 'dialog:media-settings');
  await page.waitForSelector('#live-device-settings select, [data-media-unavailable]');
  screenshots.push(await shot(page, 'media-capabilities'));
  await act(page, 'close');
  if (capabilities.rtc === 'undefined') {
    await form(page, 'channel', { 'f-name': 'Unsupported media', 'f-extra': 'voice' });
    await page.waitForSelector('.live-error:has-text("Voice and video are unavailable")');
    await until(() => page.evaluate(async () => !(await (await import('/public/media.js')).diagnostics()).joined));
    screenshots.push(await shot(page, 'media-unavailable'));
    checks.push('missing WebRTC: actionable error, no peer/publications retained; NOT a media playback pass');
  }
  assert.deepEqual(errors, []);
  await writeFile(`${output}/compatibility-results.json`, JSON.stringify({ revision: h.revision, engine,
    browser: h.browser.version(), capabilities, checks, screenshots, errors }, null, 2));
  console.log(JSON.stringify({ checks, capabilities, errors }));
} catch (e) {
  await writeFile(`${output}/compatibility-failure.json`, JSON.stringify({ error: String(e), checks, errors,
    body: await page?.locator('body').innerText().catch(() => '') }, null, 2)); throw e;
} finally { await h.close(); }

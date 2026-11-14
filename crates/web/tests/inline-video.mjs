import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { harness, register, form, shot, output } from './qa-harness.mjs';
import { engine } from './engines.mjs';
const h = await harness(), errors = [], checks = [];
let page;
try {
  page = await h.phone(); page.on('pageerror', e => errors.push(String(e)));
  await register(page, h.origin, 'qa_h264_file');
  await form(page, 'create', { 'f-name': 'Inline H.264' }); await page.waitForSelector('#live-input');
  await page.locator('#live-file').setInputFiles({ name: 'phone-h264.mp4', mimeType: 'video/mp4',
    buffer: await readFile('tests/fixtures/phone-h264.mp4') });
  await page.waitForSelector('.live-upload:has-text("phone-h264.mp4")');
  await page.locator('#live-input').fill('H.264 baseline phone video');
  await page.getByRole('button', { name: 'Send Message', exact: true }).click();
  const video = page.locator('.attachment-video video'); await video.waitFor();
  assert(await video.evaluate(v => v.hasAttribute('playsinline')));
  let supported = true;
  await video.evaluate(v => { v.muted = true; return v.play(); }).catch(error => {
    if (engine !== 'webkit' || process.platform !== 'win32' || !String(error).includes('NotSupportedError')) throw error;
    supported = false;
  });
  if (supported) {
  await page.waitForFunction(() => {
    const v = document.querySelector('.attachment-video video');
    return v?.videoWidth === 320 && v.currentTime > .1 && v.readyState >= 2;
  });
  const playback = await video.evaluate(v => ({ width: v.videoWidth, height: v.videoHeight,
    time: v.currentTime, frames: v.getVideoPlaybackQuality?.().totalVideoFrames ?? v.webkitDecodedFrameCount }));
  checks.push('real MP4 upload, H.264 baseline file decode and inline muted playback; NOT a WebRTC test');
  const screenshot = await shot(page, 'inline-h264'); assert.deepEqual(errors, []);
  await writeFile(`${output}/inline-video-results.json`, JSON.stringify({ revision: h.revision, checks, playback, errors, screenshots: [screenshot] }, null, 2));
  console.log(JSON.stringify({ checks, playback, errors }));
  } else {
    assert.equal(await video.evaluate(v => v.error?.code), 4);
    assert(await page.getByRole('link', { name: 'Download phone-h264.mp4', exact: true }).isVisible());
    checks.push('Windows WebKit lacks MP4 decode despite canPlayType: download fallback is available; decode BLOCKED');
    assert.deepEqual(errors, []);
    await writeFile(`${output}/inline-video-results.json`, JSON.stringify({ revision: h.revision, checks,
      playback: { supported: false, reason: 'Windows WebKit media backend unavailable' }, errors,
      screenshots: [await shot(page, 'inline-h264-fallback')] }, null, 2));
    console.log(JSON.stringify({ checks, errors }));
  }
} catch (e) {
  await writeFile(`${output}/inline-video-failure.json`, JSON.stringify({ error: String(e), errors, checks,
    body: await page?.locator('body').innerText().catch(() => '') }, null, 2)); throw e;
} finally { await h.close(); }

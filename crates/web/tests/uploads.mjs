import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import { harness, register, form, act, shot, until, output } from './qa-harness.mjs';
// Gate the real loopback proxy, not page.route: service-worker-controlled
// uploads bypass page interception in WebKit. No response/body is mocked.
let release, gated = false;
const putReleased = new Promise(r => release = r);
const h = await harness([], { tls: true, beforeRequest: async req => {
  if (req.method === 'PUT' && /\/uploads\/resumable\/[^/]+$/.test(req.url) && !gated) {
    gated = true; await putReleased;
  }
} }), checks = [], errors = [];
let page;
try {
  page = await h.phone(); page.on('pageerror', e => errors.push(String(e)));
  await register(page, h.origin, 'qa_resumable');
  await form(page, 'create', { 'f-name': 'Resumable uploads' }); await page.waitForSelector('#live-input');
  const file = { name: 'browser-proof.bin', mimeType: 'application/octet-stream', buffer: Buffer.alloc(17 * 1024 * 1024, 0x53) };
  await page.locator('#live-file').setInputFiles(file);
  await until(() => gated);
  await act(page, 'upload-cancel'); release();
  await page.waitForSelector('.live-error:has-text("Upload paused")');
  const paused = await shot(page, 'upload-paused'); await act(page, 'dismiss-error');
  const resumed = page.waitForResponse(r => r.request().method() === 'GET' && /\/uploads\/resumable\/[^/]+$/.test(r.url()));
  await page.locator('#live-file').setInputFiles(file);
  assert.equal((await (await resumed).json()).offset, 16 * 1024 * 1024);
  await page.waitForSelector('.live-upload:has-text("browser-proof.bin")');
  await page.locator('#live-input').fill('File from bounded Blob.slice reads');
  await page.getByRole('button', { name: 'Send Message', exact: true }).click(); await page.waitForSelector('a.file-card');
  const attachment = await page.locator('a.file-card').getAttribute('href');
  const downloaded = await page.context().request.get(`${h.origin}${attachment}`);
  assert(downloaded.ok()); assert.deepEqual(await downloaded.body(), file.buffer);
  checks.push('17 MiB upload pauses at 16 MiB, file reselection resumes server offset, downloaded bytes match');
  assert.deepEqual(errors, []);
  await writeFile(`${output}/uploads-results.json`, JSON.stringify({ revision: h.revision, checks, errors,
    screenshots: [paused, await shot(page, 'upload-complete')] }, null, 2));
  console.log(JSON.stringify({ checks, errors }));
} catch (e) {
  await writeFile(`${output}/uploads-failure.json`, JSON.stringify({ error: String(e), checks, errors,
    body: await page?.locator('body').innerText().catch(() => '') }, null, 2)); throw e;
} finally { release(); await h.close(); }

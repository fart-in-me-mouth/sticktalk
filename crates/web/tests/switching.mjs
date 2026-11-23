// Switching conversations must feel instant and must never trip the server's
// rate limit (docs/features/integration.md, "Channel switching"). Opens a
// server with eight channels and switches through them quickly, three times:
// - every switch shows the channel's messages,
// - no request is refused (429),
// - a first visit costs at most 3 REST requests (newest page, history extent,
//   the channel's topic once), a return to one of the four most recently
//   visited channels costs none, and slowmode status is only asked for in a
//   channel that has slowmode.
// Real local server, headless Chromium, loopback only.
import assert from 'node:assert/strict';
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { harness, register, form, act, wait, output } from './qa-harness.mjs';

const shots = process.env.STICKTALK_SWITCHING_SHOTS ?? process.env.STICKTALK_INTEGRATION_SHOTS ?? join(output, 'integration');
await mkdir(shots, { recursive: true });
// STICKTALK_SWITCHING_ASSERT=0 only measures (for a "before" run).
const strict = process.env.STICKTALK_SWITCHING_ASSERT !== '0';
const h = await harness();
const requests = [], refused = [], checks = [];
let phase = 'setup';
const report = {};
try {
  const page = await h.phone(1440, 900);
  page.on('request', (r) => {
    const u = new URL(r.url());
    if (u.pathname.startsWith('/api/v1/') && r.method() === 'GET') requests.push([phase, u.pathname.replace(/\d{6,}/g, '#')]);
  });
  page.on('response', (r) => { if (r.status() === 429) refused.push([phase, new URL(r.url()).pathname]); });
  await register(page, h.origin, 'switch_owner');
  await form(page, 'create', { 'f-name': 'Switching' });
  await page.waitForSelector('#live-input');
  const channels = [];
  const say = async (text) => {
    await page.locator('#live-input').fill(text);
    await act(page, 'complete-close');
    await page.locator('#live-input').press('Enter');
    await page.waitForFunction((t) => [...document.querySelectorAll('.markdown')].some((e) => e.textContent.includes(t)), text);
  };
  channels.push(await page.locator('#live-messages').getAttribute('data-channel'));
  await say('hello from general');
  for (let i = 1; i < 8; i++) {
    await form(page, 'channel', { 'f-name': `room-${i}` });
    await page.waitForFunction((known) => { const c = document.querySelector('#live-messages')?.dataset.channel; return c && !known.includes(c); }, channels);
    channels.push(await page.locator('#live-messages').getAttribute('data-channel'));
    await say(`hello from room ${i}`);
    // Setup is not what is measured: let the per-user budget refill.
    await wait(2500);
  }
  // A fresh tab, after the setup's own requests have been forgotten.
  await page.reload();
  await page.waitForSelector('.live-home');
  await wait(16000);
  const open = async (c, label) => {
    phase = label;
    const before = requests.length;
    await act(page, `channel:${c}`);
    const shown = await page.waitForFunction((c) => document.querySelector('#live-messages')?.dataset.channel === c
      && [...document.querySelectorAll('.markdown')].some((e) => e.textContent.startsWith('hello from')), c, { timeout: 10000 }).then(() => true, () => false);
    if (strict) assert.ok(shown, `${label}: the channel's messages are shown`);
    await wait(250);
    return requests.slice(before).filter(([p]) => p === label).map(([, path]) => path);
  };
  const guild = await page.evaluate(() => document.querySelector('[data-live^="guild:"]')?.dataset.live);
  if (guild) { phase = 'open-server'; await act(page, guild); await page.waitForSelector('#live-input'); await wait(500); }
  const rounds = [
    ['first visits', channels, 3],
    ['back to the four most recent', channels.slice(4).reverse(), 0],
    ['all again', channels, 3],
  ];
  for (const [name, list, bound] of rounds) {
    report[name] = [];
    for (const [i, c] of list.entries()) {
      const made = await open(c, `${name}:${i}`);
      report[name].push(made);
      if (strict) assert.ok(made.length <= bound, `${name}: switch ${i} made ${made.length} requests (${made.join(', ')}), at most ${bound} expected`);
      if (strict) assert.ok(!made.some((p) => p.endsWith('/slowmode')), `${name}: asked for slowmode in a channel without it`);
    }
  }
  if (strict) assert.deepEqual(refused, [], 'no request was refused while switching');
  await page.screenshot({ path: join(shots, 'switching-1440x900.png') });
  const per = Object.fromEntries(Object.entries(report).map(([k, v]) => [k, v.map((x) => x.length)]));
  checks.push(`switching through 8 channels three times: no 429; requests per switch ${JSON.stringify(per)}`);
} finally {
  await writeFile(join(shots, 'switching-results.json'), JSON.stringify({ revision: h.revision, strict, checks, report, refused }, null, 2));
  await h.close();
}
console.log(JSON.stringify({ checks }, null, 2));

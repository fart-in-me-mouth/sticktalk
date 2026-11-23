// First-open latency of the most used lazily loaded features (docs/features/perf.md).
// Each open is the first in a fresh browser context (cold HTTP and module
// caches, like a new session), on a real local server, headless Chromium.
// - "press": the control is pressed the moment the pointer reaches it.
// - "intent": the pointer rests on it for 150 ms first (a person moving to
//   click), which is when the app prefetches the module.
// Writes perf-latency.json; STICKTALK_PERF_OUT overrides the file.
import { writeFile, mkdir } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { harness, register, login, form, act, wait, until, output } from './qa-harness.mjs';

const out = process.env.STICKTALK_PERF_OUT ?? join(output, 'perf-latency.json');
const rounds = Number(process.env.STICKTALK_PERF_ROUNDS ?? 3);
const features = [
  ['User Settings', '[data-live="dialog:settings"]', () => { const m = document.querySelector('.live-modal'); return m && !m.textContent.includes('Loading…') && m.querySelector('h2,h1'); }],
  ['Emoji picker', '[data-live="dialog:emoji"]', () => document.querySelector('.emoji-grid button')],
  ['Inbox', '[data-live="nav-inbox"]', () => { const m = document.querySelector('.live-modal'); return m && /Mentions|Unreads|Saved/.test(m.textContent) && !m.textContent.includes('Loading…'); }],
  ['Profile card', '.user-panel .self-label', () => { const m = document.querySelector('.live-modal, .profile-card'); return m && !m.textContent.includes('Loading…') && /Edit Profile|Member Since|About/.test(m.textContent); }],
  ['Server menu', '.server-heading', () => document.querySelector('ui-menu-content[data-open]')],
  // Typing `/` in the composer: the command picker with the built-in commands.
  ['/ command picker', '#live-input', () => document.querySelector('.command-picker [role="option"]'), { type: '/' }],
  // In a voice call: the Activities button in the call controls.
  ['Activities launcher', '.call-controls [aria-label="Activities"]', () => document.querySelector('dialog.activity-launcher .activity-choice'), { prepare: 'call' }],
  // A server tag after a name in a message.
  ['Server tag card', '.message .server-tag', () => document.querySelector('.tag-card h2')],
  // A role in Server Settings › Roles: the editor with its style and icon controls.
  ['Role editor', '[data-live^="role:"]', () => document.querySelector('.live-modal #id-role-paint'), { prepare: 'roles' }],
];
// STICKTALK_PERF_ONLY=<name> measures one feature.
if (process.env.STICKTALK_PERF_ONLY) features.splice(0, features.length, ...features.filter(([n]) => n === process.env.STICKTALK_PERF_ONLY));
const h = await harness();
const results = {};
try {
  const first = await h.phone(1440, 900);
  await register(first, h.origin, 'perf_owner');
  await form(first, 'create', { 'f-name': 'Perf' });
  await first.waitForSelector('#live-input');
  await first.waitForFunction(() => !document.querySelector('.live-modal'));
  const guild = (await first.locator('.server-icon.selected').getAttribute('data-live')).slice(6);
  // A server tag the owner wears, on a message in the first channel; a role.
  await act(first, 'dialog:id-tags');
  await first.waitForSelector('[data-live-form="id-tag"]');
  await first.locator('#id-tag-text').fill('PRF');
  await first.locator('[data-live-form="id-tag"] [type=submit]').click();
  await first.waitForSelector('[data-live-form="id-tag"] .form-saved');
  await act(first, `id:wear,${guild}`);
  await until(() => first.evaluate(() => document.querySelector('[role="status"], [data-toast]')?.textContent?.includes('wearing') ?? false).catch(() => false), 10000).catch(() => {});
  await act(first, 'close');
  await first.locator('#live-input').fill('Hello from the perf owner');
  await first.keyboard.press('Enter');
  await first.waitForFunction(() => [...document.querySelectorAll('.message')].some((m) => m.textContent.includes('Hello from the perf owner') && m.querySelector('.server-tag')));
  await act(first, 'dialog:roles');
  await first.waitForSelector('[data-live-form=role-create]');
  await first.locator('#f-name').fill('Perf Role');
  await first.locator('[data-live-form=role-create] [type=submit]').click();
  await first.waitForSelector('[data-live^="role:"]');
  await first.context().close();
  for (const mode of ['press', 'intent']) {
    for (const [name, selector, ready, how = {}] of features) {
      const samples = [];
      for (let i = 0; i < rounds; i++) {
        const page = await h.phone(1440, 900);
        await page.goto(h.origin);
        await login(page, 'perf_owner', 'synthetic-qa-password-1234');
        // Connected (Ready applied), as a person would be before opening anything.
        await page.waitForFunction(() => !document.querySelector('.connection-banner') && document.querySelector('.server-rail [data-live^="guild:"]'));
        await act(page, await page.locator('.server-rail [data-live^="guild:"]').first().getAttribute('data-live'));
        await page.waitForSelector('.server-heading');
        if (how.prepare === 'call') {
          await act(page, await page.locator('[data-live^="voice:"]').first().getAttribute('data-live'));
          await until(async () => (await page.evaluate(async () => (await import('/public/media.js')).diagnostics())).state === 'connected', 45000);
        } else if (how.prepare === 'roles') {
          await act(page, 'dialog:roles');
          await page.waitForSelector('[data-live^="role:"]');
        }
        // Let the first screen settle (startup modules, idle).
        await wait(1500);
        if (process.env.STICKTALK_PERF_TRACE) { const t0 = Date.now(); page.on('request', (r) => { const p = new URL(r.url()).pathname; if (/\.(wasm|js)$/.test(p)) console.log('  req', Date.now() - t0, p.replace(/^.*\//, '')); }); page.on('requestfinished', (r) => { const p = new URL(r.url()).pathname; if (/\.(wasm|js)$/.test(p)) console.log('  done', Date.now() - t0, p.replace(/^.*\//, '')); }); globalThis.__t0 = t0; }
        const control = page.locator(selector).first();
        await control.scrollIntoViewIfNeeded();
        const box = await control.boundingBox();
        if (mode === 'intent') {
          await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
          await wait(150);
        }
        const start = Date.now();
        if (process.env.STICKTALK_PERF_TRACE) console.log('  click', start - globalThis.__t0);
        await control.click();
        if (how.type) await page.keyboard.type(how.type);
        await page.waitForFunction(ready, null, { timeout: 15000 });
        samples.push(Date.now() - start);
        // Sign out: the account may hold only a few sessions.
        await page.keyboard.press('Escape');
        if (how.prepare === 'call') await act(page, 'media:leave');
        await act(page, 'logout');
        await page.waitForSelector('#auth-username', { timeout: 15000 });
        await page.context().close();
      }
      samples.sort((a, b) => a - b);
      (results[name] ??= {})[mode] = { median_ms: samples[Math.floor(samples.length / 2)], samples_ms: samples };
      console.log(`${name} (${mode}): ${samples.join(', ')} ms`);
    }
  }
  await mkdir(dirname(out), { recursive: true });
  await writeFile(out, JSON.stringify({ revision: h.revision, rounds, results }, null, 2));
  console.log(`First-open latency written to ${out}`);
} finally {
  await h.close();
}

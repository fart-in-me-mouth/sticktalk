// Account security in the real web app against a real local server, headless
// and loopback-only (docs/features/authsec.md): passkeys (Chromium virtual
// authenticator over CDP, never a device prompt), QR sign-in scanned through a
// fake camera, email reset through a mock SMTP server, the account switcher
// and the personal data export. Screenshots of each new surface at 1440x900
// and 390x844.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {createServer} from 'node:net';
import {once} from 'node:events';
import {mkdir, mkdtemp, writeFile} from 'node:fs/promises';
import {join} from 'node:path';
import {chromium} from 'playwright';
import {launchOptions, contextOptions} from './engines.mjs';
import {target, freePort, register, login, act, wait, until} from './qa-harness.mjs';

const shots = process.env.STICKTALK_AUTHSEC_SHOTS ?? join(target, 'authsec-shots');
await mkdir(shots, {recursive: true});
const verbose = process.env.STICKTALK_AUTHSEC_VERBOSE;
const log = (s) => verbose && console.log(`[authsec] ${s}`);
const checks = [], errors = [], saved = [];
const PASSWORD = 'synthetic-qa-password-1234';

// ----- Mock SMTP on 127.0.0.1: every message lands in `mails` --------------------
const mails = [];
const smtp = createServer((socket) => {
  socket.setEncoding('utf8');
  let buffer = '', data = null;
  socket.write('220 localhost ESMTP test\r\n');
  socket.on('data', (chunk) => {
    buffer += chunk;
    let at;
    while ((at = buffer.indexOf('\r\n')) >= 0) {
      const line = buffer.slice(0, at); buffer = buffer.slice(at + 2);
      if (data !== null) {
        if (line === '.') { mails.push(data); data = null; socket.write('250 OK\r\n'); }
        else data += line + '\n';
        continue;
      }
      const verb = line.split(' ')[0].toUpperCase();
      if (verb === 'EHLO' || verb === 'HELO') socket.write('250-localhost\r\n250 8BITMIME\r\n');
      else if (verb === 'DATA') { data = ''; socket.write('354 go ahead\r\n'); }
      else if (verb === 'QUIT') { socket.end('221 bye\r\n'); }
      else socket.write('250 OK\r\n');
    }
  });
  socket.on('error', () => {});
});
smtp.listen(0, '127.0.0.1');
await once(smtp, 'listening');
const linkIn = (mail, kind) => {
  const flat = mail.replace(/=\n/g, '').replace(/=3D/g, '=');
  return flat.match(new RegExp(`#${kind}=([A-Za-z0-9_-]{43})`))[1];
};
async function nextMail(count) {
  await until(() => mails.length >= count, 20000);
  return mails[count - 1];
}

// ----- Server: public URL on "localhost", the only plain-HTTP WebAuthn origin ------
const port = await freePort();
const origin = `http://localhost:${port}`;
const data = await mkdtemp(join(target, 'authsec-data-'));
const server = spawn(process.env.STICKTALK_SERVER_EXE ?? join(target, 'debug/sticktalk-server.exe'), [
  '--bind', `127.0.0.1:${port}`, '--public-url', origin, '--data-dir', data,
  '--web-root', process.env.STICKTALK_WEB_DIST ?? join(target, 'web-dist'),
  '--min-free-disk-gb', '0', '--voice-bind-ip', '127.0.0.1', '--voice-advertised-ip', '127.0.0.1',
  '--smtp-host', '127.0.0.1', '--smtp-port', String(smtp.address().port), '--smtp-security', 'none',
  '--smtp-from', 'StickTalk <no-reply@localhost.test>',
], {windowsHide: true, stdio: ['ignore', 'pipe', 'pipe']});
let serverLog = '';
await new Promise((done, fail) => {
  const timer = setTimeout(() => fail(Error(serverLog)), 30000);
  const collect = (c) => { serverLog = (serverLog + c).slice(-20000); if (serverLog.includes('StickTalk listening')) { clearTimeout(timer); done(); } };
  server.stdout.on('data', collect); server.stderr.on('data', collect);
  server.once('exit', (code) => { clearTimeout(timer); fail(Error(`server exit ${code}: ${serverLog}`)); });
});
const browser = await chromium.launch(launchOptions);
const browsers = [browser];
async function page(width, height, b = browser) {
  const context = await b.newContext({...contextOptions(width, height), permissions: ['camera']});
  // Never permit an external request.
  await context.route('**/*', (route) => ['127.0.0.1', 'localhost', '[::1]'].includes(new URL(route.request().url()).hostname) ? route.continue() : route.abort('blockedbyclient'));
  const p = await context.newPage();
  p.on('pageerror', (e) => errors.push(String(e)));
  p.on('console', (m) => { if (m.type() === 'error' && !/Failed to load resource/.test(m.text())) errors.push(`console: ${m.text()}`); });
  p.setDefaultTimeout(30000);
  return p;
}
async function authenticator(p) {
  const cdp = await p.context().newCDPSession(p);
  await cdp.send('WebAuthn.enable', {enableUI: false});
  const {authenticatorId} = await cdp.send('WebAuthn.addVirtualAuthenticator', {options: {
    protocol: 'ctap2', transport: 'internal', hasResidentKey: true, hasUserVerification: true,
    isUserVerified: true, automaticPresenceSimulation: true,
  }});
  return {cdp, id: authenticatorId};
}
async function shot(p, name, prepare) {
  const viewport = p.viewportSize();
  for (const [width, height] of [[1440, 900], [390, 844]]) {
    await p.setViewportSize({width, height});
    if (prepare) await prepare(width);
    await p.evaluate(() => document.fonts.ready); await wait(200);
    const path = join(shots, `${name}-${width}x${height}.png`);
    await p.screenshot({path, caret: 'initial'}); saved.push(path);
  }
  await p.setViewportSize(viewport);
}
const settings = async (p, page) => { await act(p, `dialog:${page}`); await p.waitForSelector(`.live-modal[data-dialog="${page}"]`); };
async function reauth(p) {
  await p.waitForSelector('#authsec-reauth-password');
  await p.locator('#authsec-reauth-password').fill(PASSWORD);
  // The shared login limiter (burst 5 per IP) is real; this test is just fast.
  for (let attempt = 0; attempt < 12; attempt++) {
    await p.evaluate(() => { const a = document.querySelector('.authsec-reauth [role=alert]'); if (a) a.textContent = ''; });
    await p.locator('.authsec-reauth [type=submit]').click();
    const outcome = await Promise.race([
      p.waitForSelector('.authsec-reauth', {state: 'detached'}).then(() => 'done'),
      p.waitForFunction(() => document.querySelector('.authsec-reauth [role=alert]')?.textContent.includes('Too many')).then(() => 'limited'),
    ]);
    if (outcome === 'done') return;
    log('reauth rate limited; waiting');
    await wait(7000);
  }
  throw Error('re-authentication stayed rate limited');
}
/** Open a mailed or QR link as a new page load would. */
const open = async (p, url) => { await p.goto('about:blank'); await p.goto(url); };
const me = (p) => p.evaluate(() => fetch('/api/v1/browser/session').then(r => r.ok ? r.json() : null).then(s => s?.user?.user?.username ?? null));

try {
  // --- Passkeys: add one in settings, after re-authenticating -------------------------
  const alice = await page(1440, 900);
  const aliceKey = await authenticator(alice);
  await register(alice, origin, 'alice_sec');
  await settings(alice, 'password');
  await alice.waitForSelector('[data-authsec-root="security"] .authsec-section');
  await shot(alice, 'settings-security');
  await alice.getByRole('button', {name: 'Add a Passkey'}).click();
  await alice.locator('#authsec-passkey-name').fill('Test laptop');
  await alice.locator('#authsec-dialog [type=submit]').click();
  await reauth(alice);
  await alice.waitForSelector('[data-passkey]');
  assert.match(await alice.locator('[data-passkey]').first().textContent(), /Test laptop/);
  await shot(alice, 'settings-passkeys');
  checks.push('passkey added from settings after password re-authentication (virtual authenticator)');

  // --- Email: add, confirm through the emailed link ------------------------------------
  await alice.getByRole('button', {name: 'Add Email'}).click();
  await alice.locator('#authsec-email').fill('alice@example.test');
  await alice.locator('#authsec-dialog [type=submit]').click();
  await alice.waitForSelector('#authsec-reauth-password');
  await shot(alice, 'settings-reauth');
  await reauth(alice);
  await alice.waitForFunction(() => document.querySelector('[data-authsec-root]')?.textContent.includes('Open it within 24 hours'));
  await shot(alice, 'settings-email-pending');
  const verify = linkIn(await nextMail(1), 'verify-email');
  await open(alice, `${origin}/#verify-email=${verify}`);
  await alice.waitForSelector('#authsec-dialog');
  assert.equal(await alice.locator('#authsec-dialog h2').textContent(), 'Email Confirmed');
  assert.equal(new URL(alice.url()).hash, '', 'the secret left the address bar');
  await shot(alice, 'email-confirmed');
  await alice.locator('#authsec-dialog .button').click();
  checks.push('email address added and confirmed through the mailed link (secret stripped from the URL)');

  // --- Data export: request, pushed to ready, download ------------------------------------
  await settings(alice, 'data-export');
  await alice.getByRole('button', {name: 'Request My Data'}).click();
  await reauth(alice);
  await alice.waitForSelector('.authsec-export[data-state="ready"]', {timeout: 60000});
  await shot(alice, 'data-export-ready');
  const href = await alice.locator('.authsec-export a.button').getAttribute('href');
  const download = await alice.evaluate(async (href) => { const r = await fetch(href); const b = await r.arrayBuffer(); return [r.status, r.headers.get('content-type'), new Uint8Array(b.slice(0, 2))[0], b.byteLength]; }, href);
  assert.deepEqual(download.slice(0, 3), [200, 'application/zip', 0x50]);
  checks.push(`data export requested, became ready by server push, downloaded ${download[3]} bytes`);

  // --- Passkey sign-in on another browser (the credential copied between virtual authenticators)
  const phone = await page(390, 844);
  const phoneKey = await authenticator(phone);
  const {credentials} = await aliceKey.cdp.send('WebAuthn.getCredentials', {authenticatorId: aliceKey.id});
  assert.equal(credentials.length, 1);
  assert.ok(credentials[0].isResidentCredential, 'a discoverable credential');
  await phoneKey.cdp.send('WebAuthn.addCredential', {authenticatorId: phoneKey.id, credential: credentials[0]});
  await phone.goto(origin);
  await phone.waitForSelector('#authsec-passkey:not([hidden])');
  await shot(phone, 'login', async (w) => { if (w > 800) await phone.waitForSelector('.authsec-qr-card svg'); });
  // On a phone the code is behind a button; open it once for its picture.
  await phone.locator('.authsec-qr-toggle').click();
  await phone.waitForSelector('.authsec-qr-card svg');
  const qrPhone = join(shots, 'login-qr-phone-390x844.png');
  await phone.screenshot({path: qrPhone, fullPage: true}); saved.push(qrPhone);
  await phone.locator('.authsec-qr-toggle').click();
  await phone.locator('#authsec-passkey').click();
  await phone.waitForSelector('.live-home');
  assert.equal(await me(phone), 'alice_sec');
  checks.push('passwordless "Sign In with a Passkey" signed a second browser in');

  // --- QR sign-in: a signed-out desktop shows a code; the phone scans it with its camera ---
  const desktop = await page(1440, 900);
  await desktop.goto(origin);
  const svg = desktop.locator('.authsec-qr-card svg.authsec-qr');
  await svg.waitFor();
  const [size, modules] = await svg.evaluate((s) => [Number(s.dataset.size), s.dataset.modules]);
  // One Y4M frame (I420) of the code, black on white, for Chromium's fake camera.
  const W = 640, H = 480, n = size + 8, scale = Math.floor(440 / n), x0 = (W - n * scale) >> 1, y0 = (H - n * scale) >> 1;
  const y = Buffer.alloc(W * H, 235);
  for (let r = 0; r < size; r++) for (let c = 0; c < size; c++) if (modules[r * size + c] === '1')
    for (let dy = 0; dy < scale; dy++) y.fill(16, (y0 + (r + 4) * scale + dy) * W + x0 + (c + 4) * scale, (y0 + (r + 4) * scale + dy) * W + x0 + (c + 5) * scale);
  const camera = join(data, 'qr.y4m');
  await writeFile(camera, Buffer.concat([Buffer.from(`YUV4MPEG2 W${W} H${H} F10:1 Ip A1:1 C420jpeg\nFRAME\n`), y, Buffer.alloc(W * H / 2, 128)]));
  const scannerBrowser = await chromium.launch({...launchOptions, args: [...launchOptions.args, `--use-file-for-fake-video-capture=${camera}`]});
  browsers.push(scannerBrowser);
  const scanner = await page(390, 844, scannerBrowser);
  await scanner.goto(origin);
  await login(scanner, 'alice_sec', PASSWORD);
  await settings(scanner, 'password');
  await scanner.getByRole('button', {name: 'Scan QR Code'}).click();
  await scanner.waitForFunction(() => document.querySelector('#authsec-dialog h2')?.textContent === 'Sign in on another device?', null, {timeout: 30000});
  const request = await scanner.locator('.authsec-qr-request').textContent();
  // The harness emulates a Pixel 7, so the asking device reports Android.
  assert.match(request, /Chrome on Android/);
  assert.match(request, /127\.0\.0\.1/);
  await desktop.waitForFunction(() => document.querySelector('.authsec-qr-card')?.textContent.includes('Check your other device'));
  await shot(desktop, 'login-qr-scanned');
  await shot(scanner, 'qr-confirm');
  await scanner.getByRole('button', {name: 'Yes, Sign In'}).click();
  await desktop.waitForSelector('.live-home');
  assert.equal(await me(desktop), 'alice_sec');
  const cookies = (await desktop.context().cookies()).find(c => c.name === '__Host-sticktalk_session');
  assert.ok(cookies.httpOnly, 'the new session is an HttpOnly cookie');
  checks.push('QR sign-in: fake-camera scan, confirmation naming the device, waiting browser signed in with its own session');

  // A code opened through its link shows the same confirmation; declining issues a new code.
  const desktop2 = await page(1440, 900);
  await desktop2.goto(origin);
  await desktop2.locator('.authsec-qr-card svg.authsec-qr').waitFor();
  const token = await desktop2.evaluate(() => new Promise((done) => {
    // A second waiting socket on this page, to learn its code's link.
    const ws = new WebSocket(location.origin.replace('http', 'ws') + '/api/v1/auth/qr/socket');
    ws.onmessage = (e) => { const m = JSON.parse(e.data); if (m.type === 'code') { done(m.url.split('#qr=')[1]); } };
  }));
  await open(phone, `${origin}/#qr=${token}`);
  await phone.waitForFunction(() => document.querySelector('#authsec-dialog h2')?.textContent === 'Sign in on another device?');
  await phone.getByRole('button', {name: 'Cancel'}).click();
  checks.push('QR link opened on a signed-in phone shows the confirmation; Cancel declines it');

  // --- Account switcher ----------------------------------------------------------------
  await alice.goto(origin);
  await alice.waitForSelector('.live-home');
  await act(alice, `profile:${await alice.evaluate(() => fetch('/api/v1/browser/session').then(r => r.text()).then(t => t.match(/"id":(\d+)/)[1]))}`);
  await alice.getByRole('button', {name: 'Switch Accounts'}).click();
  await alice.waitForSelector('#authsec-dialog [data-account="alice_sec"]');
  await alice.getByRole('button', {name: 'Add an Account'}).click();
  await alice.waitForSelector('.authsec-accounts');
  assert.match(await alice.locator('.authsec-accounts').textContent(), /alice_sec/);
  await shot(alice, 'login-account-chooser');
  // Register a second account on the same browser (retries on the login limiter).
  await register(alice, origin, 'bob_sec');
  assert.equal(await me(alice), 'bob_sec');
  await act(alice, `profile:${await alice.evaluate(() => fetch('/api/v1/browser/session').then(r => r.text()).then(t => t.match(/"id":(\d+)/)[1]))}`);
  await alice.getByRole('button', {name: 'Switch Accounts'}).click();
  await alice.waitForSelector('#authsec-dialog [data-account="alice_sec"]');
  await shot(alice, 'account-switcher');
  await Promise.all([alice.waitForEvent('load'), alice.getByRole('button', {name: 'Switch to alice_sec'}).click()]);
  await alice.waitForSelector('.live-home');
  assert.equal(await me(alice), 'alice_sec');
  // Signing Bob out leaves Alice signed in.
  await act(alice, `profile:${await alice.evaluate(() => fetch('/api/v1/browser/session').then(r => r.text()).then(t => t.match(/"id":(\d+)/)[1]))}`);
  await alice.getByRole('button', {name: 'Switch Accounts'}).click();
  await alice.getByRole('button', {name: 'Sign out bob_sec'}).click();
  await alice.waitForFunction(() => !document.querySelector('#authsec-dialog [data-account="bob_sec"]'));
  assert.equal(await me(alice), 'alice_sec');
  checks.push('account switcher: add, switch without credentials, per-account sign-out');
  await alice.keyboard.press('Escape');

  // --- Email reset from the sign-in screen (signs every session out) ----------------------
  const reset = await page(1440, 900);
  await reset.goto(origin);
  await reset.locator('#boot-forgot').click();
  await reset.waitForSelector('#authsec-login');
  await shot(reset, 'reset-request');
  await reset.locator('#authsec-login').fill('alice_sec');
  for (let attempt = 0; ; attempt++) {
    await reset.getByRole('button', {name: 'Email Me a Link'}).click();
    await reset.waitForFunction(() => /on its way|Too many/.test(document.querySelector('.authsec-panel [role]')?.textContent ?? ''));
    if (/on its way/.test(await reset.locator('.authsec-panel [role]').textContent())) break;
    assert.ok(attempt < 12, 'reset request stayed rate limited');
    await wait(7000);
  }
  const resetToken = linkIn(await nextMail(2), 'reset');
  await open(reset, `${origin}/#reset=${resetToken}`);
  await reset.waitForSelector('#authsec-new');
  await shot(reset, 'reset-new-password');
  await reset.locator('#authsec-new').fill('a-brand-new-password');
  await reset.locator('#authsec-confirm').fill('a-brand-new-password');
  for (let attempt = 0; ; attempt++) {
    await reset.getByRole('button', {name: 'Set New Password'}).click();
    await reset.waitForFunction(() => document.getElementById('boot-status')?.textContent.includes('password was changed') || /Too many/.test(document.querySelector('.authsec-panel [role]')?.textContent ?? ''));
    if (!(await reset.locator('.authsec-panel').count())) break;
    assert.ok(attempt < 12, 'reset stayed rate limited');
    await wait(7000);
  }
  await until(async () => (await me(phone)) === null, 10000);
  assert.equal(await me(desktop), null, 'every session was signed out');
  await login(reset, 'alice_sec', 'a-brand-new-password');
  checks.push('email reset: request reveals nothing, link sets a new password, all sessions signed out');

  assert.deepEqual(errors.filter(e => !/WebSocket|net::ERR|401|Unauthorized/i.test(e)), []);
  console.log(JSON.stringify({checks, screenshots: saved.length, shots}, null, 2));
} catch (e) {
  console.error(e);
  for (const ctx of browsers.flatMap(b => b.contexts())) for (const p of ctx.pages()) {
    const state = await p.evaluate(async () => ({
      url: location.href, root: document.querySelector('[data-authsec-root]')?.textContent?.slice(0, 400),
      dialog: document.querySelector('#authsec-dialog')?.textContent?.slice(0, 300),
      exportStatus: await fetch('/api/v1/users/@me/data-export').then(r => r.text()).catch(() => null),
    })).catch(() => null);
    console.error(JSON.stringify(state));
  }
  console.error(errors.join('\n'));
  console.error(serverLog.slice(-4000));
  process.exitCode = 1;
} finally {
  for (const b of browsers) await b.close().catch(() => {});
  smtp.close();
  if (server.exitCode === null) { const exit = once(server, 'exit'); server.kill(); await exit; }
}

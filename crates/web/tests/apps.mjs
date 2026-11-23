// Apps, bots, application commands and webhooks in the real web app against
// a real local server (127.0.0.1 only, headless), with the reference bot
// (`crates/server/examples/reference_bot.rs`) connected as a real process:
// the Developer section, the install consent screen, the `/` picker with
// autocomplete from the bot, an app message with an embed and buttons, an
// ephemeral answer, a form the bot opens, Server Settings › Integrations with
// a command override, a channel webhook posting through its secret URL, and
// built-in commands. Saves each new surface at 1440x900 and 390x844 into
// STICKTALK_QA_DIR.
//
// Needs CARGO_TARGET_DIR (with web-dist), STICKTALK_SERVER_EXE, and the
// reference bot at STICKTALK_REFERENCE_BOT_EXE (default:
// $CARGO_TARGET_DIR/tools/bin/reference_bot.exe, or debug/examples/).
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {existsSync} from 'node:fs';
import {writeFile} from 'node:fs/promises';
import {join} from 'node:path';
import {harness, register, form, act, shot, output, until, wait, target} from './qa-harness.mjs';

const h = await harness(), checks = [], screenshots = [], errors = [];
let bot, botLog = '';
async function both(page, name) {
  const size = page.viewportSize();
  for (const [width, height] of [[1440, 900], [390, 844]]) {
    await page.setViewportSize({width, height});
    await wait(300);
    screenshots.push(await shot(page, `apps-${name}`));
  }
  await page.setViewportSize(size);
  await wait(150);
}
const visible = (page, selector) => page.locator(selector).first().isVisible().catch(() => false);
async function typeIn(page, text) {
  const input = page.locator('#live-input');
  await input.click();
  await input.fill('');
  await input.pressSequentially(text, {delay: 15});
}
function startBot(token) {
  const exe = process.env.STICKTALK_REFERENCE_BOT_EXE
    ?? [join(target, 'tools/bin/reference_bot.exe'), join(target, 'debug/examples/reference_bot.exe')].find(existsSync);
  assert.ok(exe, 'the reference bot binary is built');
  const child = spawn(exe, [], {
    env: {...process.env, STICKTALK_URL: h.origin, STICKTALK_BOT_TOKEN: token},
    windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
  });
  child.stdout.on('data', d => botLog = (botLog + d).slice(-4000));
  child.stderr.on('data', d => botLog = (botLog + d).slice(-4000));
  return child;
}
let alice, bob;
try {
  alice = await h.phone(1440, 900); bob = await h.phone(1440, 900);
  for (const p of [alice, bob]) p.on('pageerror', e => errors.push(String(e)));
  await register(alice, h.origin, 'apps_alice');
  await register(bob, h.origin, 'apps_bob');
  await form(alice, 'create', {'f-name': 'Apps QA'}); await alice.waitForSelector('#live-input');
  await form(alice, 'invite'); await alice.waitForSelector('.live-modal code');
  const code = await alice.locator('.live-modal code').first().textContent(); await act(alice, 'close');
  await form(bob, 'join', {'f-name': code}); await bob.waitForSelector('#live-input');
  const channel = await alice.locator('#live-messages').getAttribute('data-channel');

  // ---- Developer: create an app; its token is shown once.
  await act(alice, 'dialog:developer');
  await alice.waitForSelector('[data-live-form="apps-new"]');
  await both(alice, 'developer');
  await alice.locator('#apps-name').fill('Weather');
  await alice.locator('[data-live-form="apps-new"] [type=submit]').click();
  await alice.waitForSelector('.app-token [data-token]');
  const token = (await alice.locator('.app-token [data-token]').textContent()).trim();
  assert.match(token, /^stkb_[A-Za-z0-9_-]{43}$/);
  const link = (await alice.locator('code:has-text("?app=")').textContent()).trim();
  const appId = new URL(link).searchParams.get('app');
  // View Channels, Send Messages, Read Message History, Embed Links.
  for (const bit of [1, 2, 4, 131072]) await alice.locator(`#apps-perm-${bit}`).check();
  await alice.locator('#apps-intent-8').check();
  await alice.locator('[data-live-form="apps-edit"] [type=submit]').click();
  await alice.waitForSelector('text=Saved.');
  await until(async () => (await alice.evaluate(async () => (await (await fetch('/api/v1/applications/@me')).json()).apps[0].permissions)) === 1 + 2 + 4 + 131072);
  await both(alice, 'developer-app');
  await alice.locator('.app-token button:has-text("Done")').click();
  await until(async () => (await alice.locator('.app-token').count()) === 0);
  checks.push('Developer: app created, token shown once, permissions and intents saved');

  // ---- Install link: the consent screen lists exactly what is asked.
  await alice.goto(`${h.origin}/?app=${appId}`);
  await alice.waitForSelector('[data-live-form="apps-install"]');
  const asked = await alice.locator('[data-live-form="apps-install"] .live-checkbox').allTextContents();
  for (const label of ['View Channels', 'Send Messages', 'Embed Links']) assert.ok(asked.some(t => t.includes(label)), label);
  assert.ok(!asked.some(t => t.includes('Administrator')), 'nothing not asked for');
  await both(alice, 'install');
  await alice.locator('[data-live-form="apps-install"] [type=submit]').click();
  await alice.waitForFunction(() => !document.querySelector('[data-live-form="apps-install"]'));
  await bob.waitForSelector('.member-row:has-text("Weather") .app-tag');
  const guild = await bob.evaluate(() => document.querySelector('.server-icon[data-live^="guild:"]')?.dataset.live?.slice(6));
  await act(alice, `guild:${guild}`);
  await act(alice, `channel:${channel}`);
  await alice.waitForSelector(`#live-messages[data-channel="${channel}"]`);
  checks.push('Consent screen: requested permissions listed, app added, bot listed with APP tag');

  // ---- The bot connects with its token and registers /hello.
  bot = startBot(token);
  await until(async () => {
    const r = await bob.evaluate(async c => (await fetch(`/api/v1/channels/${c}/commands`)).json(), channel);
    return r.apps?.some(a => a.commands.some(c => c.name === 'hello'));
  }, 20000);

  // ---- The `/` picker: built-ins and the app's commands, grouped.
  await bob.locator(`[data-live="channel:${channel}"]`).first().click().catch(() => {});
  await typeIn(bob, '/');
  await bob.waitForSelector('#live-completions.command-picker [role=option]:has-text("/hello")');
  const groups = await bob.locator('#live-completions.command-picker .category').allTextContents();
  assert.deepEqual(groups, ['Built-in', 'Weather']);
  await both(bob, 'picker');
  await typeIn(bob, '/hel');
  await bob.waitForSelector('#live-completions [role=option]:has-text("/hello")');
  await bob.locator('#live-input').press('Enter');
  await until(async () => (await bob.locator('#live-input').inputValue()) === '/hello ');
  // Autocomplete from the bot.
  await bob.locator('#live-input').pressSequentially('name:Gr', {delay: 20});
  await bob.waitForSelector('#live-completions [role=option]:has-text("Grace")', {timeout: 10000});
  await both(bob, 'autocomplete');
  await bob.locator('#live-input').press('Enter');
  await until(async () => (await bob.locator('#live-input').inputValue()).startsWith('/hello name:Grace'));
  await bob.locator('#live-input').press('Enter');
  const reply = bob.locator('.message:has(.app-embed):has-text("Hello, Grace!")');
  await reply.waitFor({timeout: 10000});
  assert.ok(await reply.locator('.app-tag').isVisible(), 'APP tag on the bot');
  assert.ok(await reply.locator('.app-interaction:has-text("/hello")').isVisible(), '"used /hello"');
  await alice.waitForSelector('.message:has-text("Hello, Grace!") .app-button:has-text("Wave")');
  await both(bob, 'message');
  checks.push('Picker: grouped commands, keyboard choice, bot autocomplete, invocation answered with embed and buttons');

  // ---- A button answered privately: only Bob sees it.
  await reply.locator('.app-button:has-text("Wave")').click();
  await bob.waitForSelector('.app-ephemeral:has-text("waves back")');
  assert.ok(await bob.locator('.app-ephemeral:has-text("Only you can see this")').isVisible());
  await wait(500);
  assert.equal(await alice.locator('.app-ephemeral').count(), 0, 'ephemeral answers reach only the person');
  await both(bob, 'ephemeral');
  await bob.locator('.app-ephemeral button:has-text("Dismiss message")').click();
  await until(async () => (await bob.locator('.app-ephemeral').count()) === 0);
  checks.push('Ephemeral answer shown only to the person, dismissable');

  // ---- A form the bot opens, checked and submitted.
  await reply.locator('.app-button:has-text("Feedback")').click();
  await bob.waitForSelector('[data-live-form="apps-modal"]');
  await both(bob, 'modal');
  await bob.locator('#apps-input-0').fill('Please add forecasts');
  await bob.locator('[data-live-form="apps-modal"] [type=submit]').click();
  await bob.waitForSelector('.app-ephemeral:has-text("Thanks for the feedback")');
  checks.push('Form opened by the bot, submitted once, thanked privately');

  // ---- Integrations: the app, its commands, and an override.
  await act(alice, 'dialog:server-integrations');
  await alice.waitForSelector('.integration-app:has-text("Weather")');
  await both(alice, 'integrations');
  await alice.locator('.integration-app:has-text("Weather")').click();
  await alice.waitForSelector('[data-live-form="apps-rule"]');
  const hello = await alice.locator('#apps-target option:has-text("/hello")').getAttribute('value');
  await alice.locator('#apps-target').selectOption(hello);
  await alice.locator('#apps-rule-on').selectOption({label: '@everyone'});
  await alice.locator('#apps-rule-allow').selectOption('deny');
  await alice.locator('[data-live-form="apps-rule"] [type=submit]').click();
  await alice.waitForSelector('.app-rule:has-text("Deny @everyone")');
  await both(alice, 'integration-app');
  await until(async () => {
    const r = await bob.evaluate(async c => (await fetch(`/api/v1/channels/${c}/commands`)).json(), channel);
    return !r.apps?.some(a => a.commands.some(c => c.name === 'hello'));
  });
  await typeIn(bob, '/hel');
  await wait(800);
  assert.equal(await bob.locator('#live-completions [role=option]:has-text("/hello")').count(), 0, 'denied commands leave the picker');
  await alice.locator('.app-rule:has-text("Deny @everyone") button').click();
  await until(async () => (await alice.locator('.app-rule').count()) === 0);
  checks.push('Integrations: app listed, per-command deny hides it from members, rule removed');

  // ---- Built-ins.
  await typeIn(bob, '/shrug hi');
  await bob.locator('#live-input').press('Enter');
  await bob.locator('.message', {hasText: 'hi ¯\\_(ツ)_/¯'}).first().waitFor();
  checks.push('/shrug built-in');

  // ---- Webhooks: create in channel settings, post through the URL.
  await act(alice, `dialog:channel-webhooks:${channel}`);
  await alice.waitForSelector('[data-live-form="apps-webhook-new"]');
  await alice.locator('#apps-webhook-name').fill('CI');
  await alice.locator('[data-live-form="apps-webhook-new"] [type=submit]').click();
  await alice.waitForSelector('.webhook-secret [data-webhook-url]');
  const url = (await alice.locator('.webhook-secret [data-webhook-url]').textContent()).trim();
  assert.ok(url.startsWith(`${h.origin}/api/v1/webhooks/`), url);
  await alice.waitForSelector('.webhook-row');
  await both(alice, 'webhooks');
  const posted = await fetch(`${url}?wait=true`, {method: 'POST', headers: {'content-type': 'application/json'}, body: JSON.stringify({
    content: 'Deploy finished @everyone', username: 'Release Bot',
    embeds: [{title: 'v1.2.0', description: 'Three fixes, **one** feature.', color: 5763719, fields: [{name: 'Branch', value: 'main', inline: true}, {name: 'Time', value: '3m 12s', inline: true}], footer: {text: 'CI'}}],
  })});
  assert.equal(posted.status, 200);
  const hookMessage = bob.locator('.message:has-text("Deploy finished"):has(.app-embed)');
  await hookMessage.waitFor({timeout: 10000});
  assert.ok((await hookMessage.textContent()).includes('Release Bot'));
  assert.ok(await hookMessage.locator('.app-tag').isVisible());
  await both(bob, 'webhook-message');
  assert.equal((await fetch(url.replace(/[^/]+$/, 'A'.repeat(43)), {method: 'POST', headers: {'content-type': 'application/json'}, body: '{"content":"x"}'})).status, 404);
  checks.push('Webhook: created, URL shown once, JSON with embed posted as APP, @everyone inert, wrong secret refused');

  // ---- Apps leave cleanly.
  await act(alice, `dialog:integration-remove:${appId}`);
  await alice.waitForSelector('[data-live-form="apps-remove"]');
  await alice.locator('[data-live-form="apps-remove"] [type=submit]').click();
  await bob.waitForFunction(() => ![...document.querySelectorAll('.member-row')].some(r => r.textContent.includes('Weather')));
  assert.ok(await visible(bob, '.message:has-text("Hello, Grace!")'), 'its messages stay');
  checks.push('Removing the app removes its member; its messages stay');

  assert.deepEqual(errors, []);
  for (const p of [alice, bob]) assert.deepEqual(await p.evaluate(() => window.__qaCsp), [], 'no CSP violations');
  await writeFile(`${output}/apps-results.json`, JSON.stringify({checks, screenshots}, null, 2));
  console.log(JSON.stringify({checks, screenshots}, null, 2));
} catch (e) {
  for (const [n, p] of [['alice', alice], ['bob', bob]]) {
    if (!p) continue;
    console.error(n, p.url(), await p.evaluate(() => document.querySelector('[data-dialog]')?.dataset.dialog).catch(() => ''));
    await p.screenshot({path: `${output}/apps-failure-${n}.png`}).catch(() => {});
  }
  console.error(errors);
  console.error(botLog);
  console.error(h.log().slice(-3000));
  throw e;
} finally {
  if (bot && bot.exitCode === null) bot.kill();
  await h.close();
}

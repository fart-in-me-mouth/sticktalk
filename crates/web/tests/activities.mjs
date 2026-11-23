// Shared activities in the real web app against a real local server:
// the launcher, the sandboxed frame and its isolation, the whiteboard, a
// server-run game, Watch Together's synchronized player, invite cards with
// one-step Join (from a phone), a registered third-party activity in its
// shell, the server's settings page, and nothing loaded before an activity.
// Headless Chromium, fake media devices, loopback only, no sound played.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createServer } from 'node:http';
import { once } from 'node:events';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { join, normalize, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { harness, register, act, until, wait, output, target } from './qa-harness.mjs';

const shots = process.env.STICKTALK_ACTIVITIES_SHOTS ?? join(output, 'activities');
await mkdir(shots, { recursive: true });

// A "third-party" activity on another origin: the example served by itself.
const activitiesDir = resolve(fileURLToPath(new URL('../public/activities/', import.meta.url)));
const thirdParty = createServer(async (req, res) => {
  const path = normalize(decodeURIComponent(new URL(req.url, 'http://x').pathname)).replace(/^[\\/]+/, '');
  const file = join(activitiesDir, path.endsWith('/') || path === '' ? join(path, 'index.html') : path);
  if (!file.startsWith(activitiesDir)) { res.writeHead(404); return res.end(); }
  try {
    const body = await readFile(file.endsWith('\\') || file.endsWith('/') ? join(file, 'index.html') : file);
    const type = file.endsWith('.js') ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : 'text/html';
    // As ACTIVITIES.md asks: no frame-ancestors (the shell's origin is opaque),
    // and its own origin named explicitly (a sandboxed page's 'self' is opaque too).
    res.writeHead(200, { 'content-type': type, 'content-security-policy': `default-src ${thirdOrigin}; connect-src 'none'` });
    res.end(body);
  } catch {
    res.writeHead(404);
    res.end();
  }
});
thirdParty.listen(0, '127.0.0.1');
await once(thirdParty, 'listening');
const thirdOrigin = `http://127.0.0.1:${thirdParty.address().port}`;

const h = await harness();
const server = process.env.STICKTALK_SERVER_EXE ?? join(target, 'debug/sticktalk-server.exe');
const checks = [];
const errors = [];
const media = (p) => p.evaluate(async () => await (await import('/public/media.js')).diagnostics());
const activity = (p) => p.evaluate(() => globalThis.__sticktalkActivities?.() ?? null);

async function shot(page, name, sizes = [[1440, 900], [390, 844]]) {
  const before = page.viewportSize();
  for (const [width, height] of sizes) {
    await page.setViewportSize({ width, height });
    await page.evaluate(() => document.fonts.ready);
    await wait(250);
    await page.screenshot({ path: join(shots, `${name}-${width}x${height}.png`) });
  }
  await page.setViewportSize(before);
  await wait(150);
}
async function page(width = 1440, height = 900) {
  const p = await h.phone(width, height);
  p.on('pageerror', (e) => errors.push(e.stack ?? String(e)));
  return p;
}
async function connected(...pages) {
  await until(async () => (await Promise.all(pages.map(media))).every((d) => d.state === 'connected'), 45000);
}
async function joinServer(p, code) {
  await act(p, 'dialog:join');
  await p.waitForSelector('[data-live-form="join"]');
  await p.locator('#f-name').fill(code);
  await p.locator('[data-live-form] [type=submit]').click();
  await p.waitForSelector('#live-input');
}
const frameOf = (p, path) => p.frames().find((f) => f.url().includes(path));
async function frame(p, path) {
  await until(async () => {
    const f = frameOf(p, path);
    return !!f && await f.evaluate(() => document.body?.dataset.ready != null).catch(() => false);
  }, 30000);
  return frameOf(p, path);
}
async function launch(p, name) {
  await p.locator('.call-controls [aria-label="Activities"]').click();
  await p.waitForSelector('dialog.activity-launcher .activity-choice');
  await p.locator(`dialog.activity-launcher [aria-label="Start ${name}"]`).click();
  await p.waitForSelector('#live-activity-layer .activity-frame');
  await until(async () => (await activity(p))?.frame?.ready === true);
}
async function joinBanner(p) {
  await p.waitForSelector('.activity-banner [data-live^="activity:join,"]');
  await p.locator('.activity-banner [data-live^="activity:join,"]').click();
  await p.waitForSelector('#live-activity-layer .activity-frame');
  await until(async () => (await activity(p))?.frame?.ready === true);
}
async function endActivity(p) {
  await p.locator('#live-activity-layer [aria-label="Activity Options"]').click();
  await p.locator('dialog.activity-options button:has-text("End Activity for Everyone")').click();
  await p.waitForFunction(() => !document.querySelector('#live-activity-layer'));
}
// A short synthetic WebM (moving picture and a tone), recorded in the page
// with MediaRecorder: no external media. The server sniffs WebM as video.
async function clip(p, seconds = 6) {
  const b64 = await p.evaluate(async (ms) => {
    const c = Object.assign(document.createElement('canvas'), { width: 320, height: 180 });
    const g = c.getContext('2d');
    const ac = new AudioContext();
    const osc = ac.createOscillator();
    const out = ac.createMediaStreamDestination();
    osc.connect(out);
    osc.start();
    const stream = new MediaStream([...c.captureStream(15).getVideoTracks(), ...out.stream.getAudioTracks()]);
    const rec = new MediaRecorder(stream, { mimeType: 'video/webm;codecs=vp8,opus' });
    const parts = [];
    rec.ondataavailable = (e) => parts.push(e.data);
    let t = 0;
    const draw = setInterval(() => { g.fillStyle = `hsl(${(t += 12) % 360} 60% 40%)`; g.fillRect(0, 0, 320, 180); }, 66);
    rec.start();
    await new Promise((r) => setTimeout(r, ms));
    await new Promise((r) => { rec.onstop = r; rec.stop(); });
    clearInterval(draw);
    osc.stop();
    await ac.close();
    const bytes = new Uint8Array(await new Blob(parts).arrayBuffer());
    let s = '';
    for (const x of bytes) s += String.fromCharCode(x);
    return btoa(s);
  }, seconds * 1000);
  return Buffer.from(b64, 'base64');
}

let alice, bob, cara;
try {
  alice = await page();
  bob = await page();
  cara = await page(390, 844);
  await register(alice, h.origin, 'qa_act_alice');
  await register(bob, h.origin, 'qa_act_bob');
  await register(cara, h.origin, 'qa_act_cara');
  await act(alice, 'dialog:create');
  await alice.waitForSelector('[data-live-form="create"]');
  await alice.locator('#f-name').fill('Activity Lab');
  await alice.locator('[data-live-form] [type=submit]').click();
  await alice.waitForSelector('#live-input');
  await alice.waitForFunction(() => !document.querySelector('.live-modal'));
  await act(alice, 'dialog:invite');
  await alice.waitForSelector('[data-live-form="invite"]');
  await alice.locator('[data-live-form="invite"] [type=submit]').click();
  await alice.waitForSelector('.live-modal code');
  const code = (await alice.locator('.live-modal code').first().textContent()).trim();
  await act(alice, 'close');
  await joinServer(bob, code);
  await joinServer(cara, code);
  const general = await alice.locator('#live-messages').getAttribute('data-channel');
  const voice = await alice.locator('[data-live^="voice:"]').first().getAttribute('data-live');

  // ---- Nobody pays for activities before one is used ---------------------------------
  const loaded = (p) => p.evaluate(() => performance.getEntriesByType('resource').some((r) => /\/activities(\.js|\/)/.test(new URL(r.name).pathname)));
  assert.equal(await loaded(bob), false, 'activities.js is not loaded on sign-in');
  checks.push('no activity code is loaded at sign-in');

  // ---- The launcher and the Whiteboard ------------------------------------------------
  await act(alice, voice);
  await act(bob, voice);
  await connected(alice, bob);
  assert.equal(await loaded(bob), false, 'joining a call loads no activity code either');
  await alice.locator('.call-controls [aria-label="Activities"]').click();
  await alice.waitForSelector('dialog.activity-launcher .activity-choice');
  assert.equal(await alice.locator('dialog.activity-launcher .activity-choice').count(), 5);
  await shot(alice, 'launcher');
  await alice.locator('dialog.activity-launcher [aria-label="Start Whiteboard"]').click();
  await alice.waitForSelector('#live-activity-layer .activity-frame');
  await until(async () => (await activity(alice))?.frame?.ready === true);
  const d = await activity(alice);
  assert.equal(d.frame.sandbox, 'allow-scripts', 'strict sandbox');
  assert.equal(d.session.you, 1);
  checks.push('launcher lists the five built-in activities; starting one opens its sandboxed frame');

  // The frame is isolated: an opaque origin, no cookies, no storage, no app DOM, no network.
  const board = await frame(alice, '/activities/whiteboard/');
  const isolation = await board.evaluate(async () => {
    const out = { origin: self.origin };
    try { out.cookie = document.cookie; } catch { out.cookie = 'blocked'; }
    try { localStorage.getItem('x'); out.storage = 'open'; } catch { out.storage = 'blocked'; }
    try { out.parent = !!parent.document; } catch { out.parent = 'blocked'; }
    try { await fetch('/api/v1/users/@me'); out.fetch = 'open'; } catch { out.fetch = 'blocked'; }
    return out;
  });
  assert.deepEqual(isolation, { origin: 'null', cookie: 'blocked', storage: 'blocked', parent: 'blocked', fetch: 'blocked' });
  checks.push(`frame isolation: ${JSON.stringify(isolation)}`);

  // Bob sees "Alice started Whiteboard" with Join.
  await bob.waitForSelector('.activity-banner:has-text("started Whiteboard")');
  await shot(bob, 'started-banner', [[1440, 900]]);
  await joinBanner(bob);
  const bobBoard = await frame(bob, '/activities/whiteboard/');
  await until(async () => (await activity(alice)).session.participants === 2);
  checks.push('the others see who started it, and Join');

  // Alice draws: the stroke is one key; Bob sees it, and her cursor live.
  await bobBoard.evaluate(() => { window.__messages = 0; window.StickTalk.on('message', () => window.__messages++); });
  const canvas = await board.locator('canvas.over').boundingBox();
  await alice.mouse.move(canvas.x + 200, canvas.y + 150);
  await alice.mouse.down();
  for (let i = 0; i < 30; i++) await alice.mouse.move(canvas.x + 200 + i * 8, canvas.y + 150 + Math.sin(i / 4) * 40, { steps: 2 });
  await alice.mouse.up();
  await until(async () => (await bobBoard.evaluate(() => window.StickTalk.entries('i/').length)) === 1);
  assert(await bobBoard.evaluate(() => window.__messages) > 0, 'live stroke and cursor messages arrived');
  // A sticky note with text.
  await board.locator('button[aria-label="Sticky Note"]').click();
  await alice.mouse.click(canvas.x + 500, canvas.y + 300);
  await board.locator('textarea.editor').fill('Plan the trip');
  await board.locator('textarea.editor').press('Control+Enter');
  await until(async () => (await bobBoard.evaluate(() => window.StickTalk.entries('i/').some((e) => e.value.text === 'Plan the trip'))));
  // Undo removes Alice's own last item only.
  await board.locator('button[aria-label="Undo"]').click();
  await until(async () => (await bobBoard.evaluate(() => window.StickTalk.entries('i/').length)) === 1);
  await board.locator('button[aria-label="Redo"]').click();
  await until(async () => (await bobBoard.evaluate(() => window.StickTalk.entries('i/').length)) === 2);
  await shot(alice, 'whiteboard-split');
  checks.push('whiteboard: a stroke and a sticky note sync as single keys; live cursor messages; per-person undo and redo');

  // Layouts: focus, mini (picture-in-picture while browsing chat), split.
  await alice.locator('#live-activity-layer [aria-label="Focus Activity"]').click();
  await alice.waitForSelector('#live-activity-layer[data-mode="focus"]');
  await shot(alice, 'whiteboard-focus', [[1440, 900]]);
  await act(alice, `channel:${general}`);
  await alice.waitForSelector('#live-activity-layer[data-mode="mini"]');
  assert.equal(frameOf(alice, '/activities/whiteboard/'), board, 'the frame did not reload');
  assert.equal(await board.evaluate(() => window.StickTalk.entries('i/').length), 2);
  await shot(alice, 'whiteboard-mini');
  await alice.locator('#live-activity-layer [aria-label="Return to Activity"]').click();
  await alice.waitForSelector('#live-activity-layer[data-mode="split"]');
  checks.push('focus, split and a floating mini window while in chat, without reloading the frame');

  // Export asks before saving a PNG.
  await board.locator('button[aria-label="Export as PNG"]').click();
  await alice.waitForSelector('dialog.activity-dialog:has-text("Save it to this device?")');
  const [download] = await Promise.all([alice.waitForEvent('download'), alice.locator('dialog.activity-dialog button:has-text("Save")').click()]);
  assert.equal(download.suggestedFilename(), 'whiteboard.png');
  checks.push('export to PNG through the host, confirmed by the person');

  // ---- An invite card, and Join in one step from a phone ------------------------------
  await alice.locator('#live-activity-layer [aria-label="Invite"]').click();
  await alice.waitForSelector('#activity-invite-target');
  await alice.locator('#activity-invite-target').selectOption(general);
  await alice.locator('dialog.activity-dialog button:has-text("Post Invite")').click();
  await act(cara, `channel:${general}`);
  await cara.waitForSelector('.activity-card:has-text("Whiteboard")');
  await cara.waitForSelector('.activity-card:has-text("qa_act_alice")');
  await shot(cara, 'invite-card');
  await cara.locator('.activity-card [data-live^="activity:join,"]').click();
  await cara.waitForSelector('#live-activity-layer .activity-frame');
  await until(async () => (await activity(cara))?.session?.you != null && (await activity(cara)).frame?.ready === true, 45000);
  await connected(cara);
  await shot(cara, 'whiteboard-phone', [[390, 844]]);
  // The card follows: three people now.
  await alice.waitForSelector(`#live-messages .activity-card`).catch(() => {});
  await act(bob, `channel:${general}`);
  await bob.waitForSelector('.activity-card:has-text("qa_act_cara")');
  await act(bob, voice);
  checks.push('invite card in a text channel; Join from a phone joins the call and the activity at once; the card updates');

  // ---- Ending: the card shows the summary ---------------------------------------------
  await endActivity(alice);
  await act(bob, `channel:${general}`);
  await bob.waitForSelector('.activity-card:has-text("Ended")');
  await shot(bob, 'invite-card-ended', [[1440, 900]]);
  await act(bob, voice);
  await cara.waitForFunction(() => !document.querySelector('#live-activity-layer'));
  checks.push('the host ends it for everyone; the card shows the summary');

  // ---- Four in a Row: rules enforced by the server, a spectator, the result ---------------
  await launch(alice, 'Four in a Row');
  await joinBanner(bob);
  await joinBanner(cara);
  const ga = await frame(alice, '/activities/four-in-a-row/');
  const gb = await frame(bob, '/activities/four-in-a-row/');
  const gc = await frame(cara, '/activities/four-in-a-row/');
  await ga.locator('button[aria-label="Play Red"]').click();
  await gb.locator('button[aria-label="Play Yellow"]').click();
  await gb.waitForSelector('.status:has-text("turn")');
  // A tampered move out of turn is refused by the server.
  const refused = await gb.evaluate(() => window.StickTalk.action({ type: 'move', col: 0 }).then(() => 'accepted', (e) => e.message));
  assert.match(refused, /isn.t your turn/);
  const cheat = await gb.evaluate(() => window.StickTalk.set('s/game', { status: 'over', winner: 1 }).then(() => 'accepted', (e) => e.message));
  assert.notEqual(cheat, 'accepted', 'nobody writes the game state');
  assert.equal(await gc.locator('.c4-col:not(:disabled)').count(), 0, 'a spectator cannot move');
  for (let i = 0; i < 3; i++) {
    await ga.locator('button[aria-label="Drop in column 1"]').click();
    await until(async () => (await gb.locator('.c4-col:not(:disabled)').count()) > 0);
    await gb.locator('button[aria-label="Drop in column 2"]').click();
    await until(async () => (await ga.locator('.c4-col:not(:disabled)').count()) > 0);
  }
  await ga.locator('button[aria-label="Drop in column 1"]').click();
  await gc.waitForSelector('.status:has-text("qa_act_alice wins!")');
  await shot(cara, 'four-in-a-row-phone', [[390, 844]]);
  await shot(alice, 'four-in-a-row');
  await ga.locator('button:has-text("Rematch")').click();
  await gb.locator('button:has-text("Rematch")').click();
  await ga.waitForSelector('.status:has-text("qa_act_bob’s turn")');
  checks.push('four in a row: seats, server-checked turns and win, a spectator, rematch with the other player first');
  await endActivity(alice);
  // The result is kept with the conversation.
  const results = await alice.evaluate(async (c) => (await (await fetch(`/api/v1/activities?channel_id=${c}`)).json()).results, voice.slice(6));
  assert(results.some((r) => /qa_act_alice beat qa_act_bob at Four in a Row/.test(r.text)), JSON.stringify(results));
  checks.push('the game result is kept and shown in the launcher');

  // ---- Sketch & Guess: the word reaches only the person drawing ------------------------
  await launch(bob, 'Sketch & Guess');
  await joinBanner(alice);
  const sb = await frame(bob, '/activities/sketch-guess/');
  const sa = await frame(alice, '/activities/sketch-guess/');
  await sb.locator('button:has-text("Start Game")').click();
  await until(async () => (await sb.evaluate(() => window.StickTalk.get('s/game')?.status)) === 'drawing');
  const drawer = await sb.evaluate(() => window.StickTalk.get('s/game').drawer);
  const [drawFrame, guessFrame] = drawer === (await sb.evaluate(() => window.StickTalk.context().you)) ? [sb, sa] : [sa, sb];
  await until(async () => /^Draw: [a-z ]+$/.test(await drawFrame.locator('.info').textContent()));
  const word = (await drawFrame.locator('.info').textContent()).slice(6);
  assert.equal(await guessFrame.evaluate(() => window.StickTalk.get('s/game').word), '', 'the word is not in the shared state');
  assert(!(await guessFrame.locator('.info').textContent()).includes(word));
  await guessFrame.locator('.guess input').fill('definitely wrong');
  await guessFrame.locator('.guess input').press('Enter');
  await guessFrame.locator('.guess input').fill(word);
  await guessFrame.locator('.guess input').press('Enter');
  await until(async () => (await drawFrame.evaluate(() => window.StickTalk.get('s/game').status)) === 'reveal');
  await shot(drawFrame === sb ? bob : alice, 'sketch-guess', [[1440, 900]]);
  checks.push('sketch & guess: a private word for the drawer, guesses checked and scored by the server');
  await endActivity(bob);

  // ---- Watch Together: a shared file plays in sync through the call's output -----------
  await launch(alice, 'Watch Together');
  await joinBanner(bob);
  const wa = await frame(alice, '/activities/watch-together/');
  await wa.locator('button:has-text("Add a Video or Audio File")').click();
  await alice.waitForSelector('dialog.activity-media');
  await alice.locator('dialog.activity-media .activity-upload input[type=file]').setInputFiles({ name: 'clip.webm', mimeType: 'video/webm', buffer: await clip(alice) });
  await until(async () => (await activity(alice))?.player?.item != null, 30000);
  await until(async () => (await activity(bob))?.player?.item != null && !(await activity(bob)).player.paused, 30000);
  await wait(1500);
  const [pa, pb] = [await activity(alice), await activity(bob)];
  assert(Math.abs(pa.player.time - pb.player.time) < 0.5, `in sync: ${pa.player.time} vs ${pb.player.time}`);
  assert(Math.abs(pb.player.volume - 0.8) < 0.01 && !pb.player.muted, 'activity volume 80% by default');
  await shot(alice, 'watch-together');
  // Deafen silences the activity's sound too.
  await bob.locator('.call-controls [aria-label="Deafen"]').click();
  await until(async () => (await activity(bob)).player.muted === true);
  await bob.locator('.call-controls [aria-label="Undeafen"], .call-controls [aria-label="Deafen"]').first().click();
  // Pause by the host pauses everyone.
  await wa.locator('button[aria-label="Pause"]').click();
  await until(async () => (await activity(bob)).player.paused === true);
  checks.push(`watch together: an uploaded file plays for everyone within ${Math.abs(pa.player.time - pb.player.time).toFixed(2)} s; activity volume; deafen silences it; the host pauses everyone`);
  await endActivity(alice);

  // ---- A registered activity on another origin, turned on in Server Settings ---------------
  execFileSync(server, ['--data-dir', h.data, 'admin', 'activities', 'add', '--name', 'Counter', '--url', `${thirdOrigin}/example/`, '--icon', '🧮', '--description', 'A shared counter', '--min-players', '1', '--max-players', '8'], { windowsHide: true, stdio: 'ignore' });
  await act(alice, 'dialog:server-activities');
  await alice.waitForSelector('[data-activity-settings] .activity-setting');
  await alice.locator('input[aria-label="Allow Counter"]').check();
  await until(async () => (await alice.evaluate(async (g) => (await (await fetch(`/api/v1/guilds/${g}/activities`)).json()).activities.find((a) => a.activity.name === 'Counter')?.enabled, await alice.evaluate(() => document.querySelector('[data-activity-settings]').dataset.activitySettings))) === true);
  await shot(alice, 'server-settings');
  await act(alice, 'close');
  await act(alice, voice);
  await launch(alice, 'Counter');
  await joinBanner(bob);
  const counterA = await frame(alice, `${thirdOrigin}/example/`);
  const counterB = await frame(bob, `${thirdOrigin}/example/`);
  await counterA.locator('#add').click();
  await counterA.locator('#add').click();
  await until(async () => (await counterB.locator('#count').textContent()) === '2');
  const shell = alice.frames().find((f) => f.url().includes('/activities/frame/custom-'));
  assert(shell, 'the registered activity runs inside the shell');
  const shellCsp = await alice.evaluate(async (u) => (await fetch(u)).headers.get('content-security-policy'), new URL(shell.url()).pathname);
  assert(shellCsp.includes(`frame-src ${thirdOrigin}`), shellCsp);
  const appCsp = await alice.evaluate(async () => (await fetch('/')).headers.get('content-security-policy'));
  assert(!appCsp.includes(thirdOrigin), 'the app itself never allows the third-party origin');
  await shot(bob, 'registered-activity', [[1440, 900]]);
  checks.push('a registered activity on another origin, enabled per server, runs in its shell; only the shell may frame its origin');
  await endActivity(alice);

  // ---- Leaving the call ends everything here ---------------------------------------------
  await act(alice, 'media:leave');
  await act(bob, 'media:leave');
  await act(cara, 'media:leave');
  await until(async () => (await Promise.all([alice, bob, cara].map(activity))).every((d) => !d?.frame && !d?.session));
  checks.push('after the call: no frame, no session, no player');

  const csp = (await Promise.all([alice, bob, cara].map((p) => p.evaluate(() => window.__qaCsp)))).flat();
  assert.deepEqual(csp, [], 'no CSP violations in the app');
  assert.deepEqual(errors, [], 'no page errors');
  await writeFile(join(shots, 'activities-results.json'), JSON.stringify({ revision: h.revision, checks }, null, 2));
  console.log(JSON.stringify({ checks }, null, 2));
} catch (e) {
  for (const [name, p] of [['alice', alice], ['bob', bob], ['cara', cara]])
    if (p) await p.screenshot({ path: join(shots, `failure-${name}.png`) }).catch(() => {});
  console.error(h.log().slice(-4000));
  throw e;
} finally {
  await h.close();
  await new Promise((done) => thirdParty.close(done));
}

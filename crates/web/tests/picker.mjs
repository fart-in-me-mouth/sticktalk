// The expression picker in the real web app against a real local server
// (127.0.0.1 only, headless): Emoji, Stickers, GIFs and Kaomoji are the same
// four tabs, in the same order, wherever the picker is opened; switching
// between them keeps the dialog's size and place; nothing is missing while
// the lazily loaded modules are still on their way (they are delayed here);
// GIFs on a server without GIF search; arrow keys between the tabs; the emoji
// category bar (jump to a section, the marked category follows the scroll);
// Add Reaction offers emoji only; the phone layout. Then the GIFs tab on a
// server with GIF search (a local mock of the provider). Saves every picker at
// 1440x900 and 390x844 in the default and the light theme.
import assert from 'node:assert/strict';
import {mkdir, writeFile} from 'node:fs/promises';
import {createServer as http} from 'node:http';
import {once} from 'node:events';
import {deflateSync} from 'node:zlib';
import {join} from 'node:path';
import {harness,register,form,act,output,wait,until} from './qa-harness.mjs';

const shots = process.env.STICKTALK_PICKER_SHOTS ?? join(output, 'picker');
await mkdir(shots, {recursive: true});
const KINDS = ['emoji', 'stickers', 'gif', 'kaomoji'];
const LABELS = ['Emoji', 'Stickers', 'GIFs', 'Kaomoji'];
const checks = [], screenshots = [], errors = [];

// A tiny PNG, so the test needs no image files.
const crcTable=Array.from({length:256},(_,n)=>{let c=n;for(let k=0;k<8;k++)c=c&1?0xedb88320^(c>>>1):c>>>1;return c>>>0;});
const crc=b=>{let c=0xffffffff;for(const x of b)c=crcTable[(c^x)&255]^(c>>>8);return (c^0xffffffff)>>>0;};
function png(size,[r,g,b]){
  const chunk=(type,data)=>{const len=Buffer.alloc(4);len.writeUInt32BE(data.length);const td=Buffer.concat([Buffer.from(type),data]);const c=Buffer.alloc(4);c.writeUInt32BE(crc(td));return Buffer.concat([len,td,c]);};
  const ihdr=Buffer.alloc(13);ihdr.writeUInt32BE(size,0);ihdr.writeUInt32BE(size,4);ihdr[8]=8;ihdr[9]=6;
  const rows=[];
  for(let y=0;y<size;y++){rows.push(0);for(let x=0;x<size;x++){const d=Math.hypot(x-size/2,y-size/2)<size*0.45;rows.push(r,g,b,d?255:0);}}
  return Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),chunk('IHDR',ihdr),chunk('IDAT',deflateSync(Buffer.from(rows))),chunk('IEND',Buffer.alloc(0))]);
}

const dialogOf = page => page.evaluate(() => document.querySelector('[data-dialog]')?.dataset.dialog ?? '');
// What the tab row offers: labels in order, and which are selected.
const tabState = page => page.evaluate(() => {
  const tabs = [...document.querySelectorAll('.live-modal [role=tablist] [role=tab]')];
  return {labels: tabs.map(t => t.textContent.trim()), selected: tabs.filter(t => t.getAttribute('aria-selected') === 'true').map(t => t.textContent.trim())};
});
const rect = (page, selector) => page.evaluate(s => { const r = document.querySelector(s)?.getBoundingClientRect(); return r && {x: r.x, y: r.y, w: r.width, h: r.height}; }, selector);
const near = (a, b, what) => {
  assert.ok(a && b, `${what}: both measured`);
  for (const k of ['x', 'y', 'w', 'h']) assert.ok(Math.abs(a[k] - b[k]) <= 2, `${what}: ${k} ${a[k]} vs ${b[k]}`);
};
// Each tab's contents, once they are drawn.
const READY = {
  emoji: '.picker-modal #emoji-query[placeholder="Search all emoji"]',
  stickers: '.picker-modal #emoji-query[placeholder="Search stickers"]',
  gif: '.picker-modal #gif-query, .picker-modal [data-live="pick-image"]',
  kaomoji: '.picker-modal .kaomoji',
};
async function open(page, kind) {
  await act(page, `dialog:${kind}`);
  await page.waitForFunction(k => document.querySelector('[data-dialog]')?.dataset.dialog === k, kind);
  await page.waitForSelector(READY[kind]);
}
async function expectTabs(page, kind, where) {
  const {labels, selected} = await tabState(page);
  assert.deepEqual(labels, LABELS, `${where}: the four choices in order`);
  assert.deepEqual(selected, [LABELS[KINDS.indexOf(kind)]], `${where}: exactly the open one selected`);
}
async function shot(page, name) {
  const size = page.viewportSize();
  await page.evaluate(() => document.fonts.ready);
  await wait(150);
  const path = join(shots, `${name}-${size.width}x${size.height}.png`);
  await page.screenshot({path});
  screenshots.push(path);
}
async function setTheme(page, base) {
  await act(page, 'dialog:appearance');
  await page.waitForSelector(`input[data-theme-choice][value="${base}"]`);
  await page.locator(`label.theme-option:has(input[value="${base}"])`).click();
  await page.waitForFunction(b => document.documentElement.dataset.themeBase === b, base);
  await act(page, 'close');
  await page.waitForFunction(() => !document.querySelector('[data-dialog]'));
}
async function send(page, text) {
  await page.locator('#live-input').fill(text);
  await page.locator('#live-input').press('Enter');
  await page.waitForFunction(t => [...document.querySelectorAll('.message .markdown')].some(m => m.textContent.includes(t)), text);
}

// ---- A server without GIF search (the owner's live server is set up so) ----
let h = await harness();
let page, phone, slow;
try {
  page = await h.phone(1440, 900);
  page.on('pageerror', e => errors.push(String(e)));
  await register(page, h.origin, 'picker_owner');
  await form(page, 'create', {'f-name': 'Picker QA'});
  await page.waitForSelector('#live-input');
  // A custom emoji, so the server's section and its place in the category bar show.
  await act(page, 'dialog:server-emoji');
  await page.waitForSelector('[data-live-form=expr-emoji-upload]');
  await page.locator('#expr-file').setInputFiles({name: 'wave.png', mimeType: 'image/png', buffer: png(96, [46, 160, 120])});
  await page.locator('#expr-name').fill('pickwave');
  await page.locator('[data-live-form=expr-emoji-upload] [type=submit]').click();
  await page.waitForSelector('[data-emoji-row=pickwave]');
  await act(page, 'close');
  await send(page, 'something to react to');

  // The same four tabs wherever the picker is opened: by name, and from the
  // composer's own buttons.
  for (const kind of KINDS) {
    await open(page, kind);
    await expectTabs(page, kind, `dialog:${kind}`);
  }
  await act(page, 'close');
  for (const [button, kind] of [['[aria-label="Emoji Picker"]', 'emoji'], ['[aria-label="Sticker Picker"]', 'stickers'], ['.composer-gif', 'gif'], ['.composer-kaomoji', 'kaomoji']]) {
    await page.locator(`.composer ${button}`).click();
    await page.waitForSelector(READY[kind]);
    await expectTabs(page, kind, `composer ${button}`);
    await act(page, 'close');
    await page.waitForFunction(() => !document.querySelector('[data-dialog]'));
  }
  checks.push('the four pickers offer Emoji, Stickers, GIFs, Kaomoji in that order with exactly one selected, opened by name or from the composer');

  // Switching between all four by pointer keeps the dialog where it is, the
  // same size, and the tab row in place; the search field takes focus.
  await page.locator('.composer [aria-label="Emoji Picker"]').click();
  await page.waitForSelector(READY.emoji);
  await page.waitForFunction(() => document.activeElement?.id === 'emoji-query');
  const frame = await rect(page, '.live-modal'), row = await rect(page, '.picker-tabs');
  for (const kind of ['stickers', 'gif', 'kaomoji', 'emoji', 'kaomoji', 'stickers', 'emoji']) {
    await page.locator(`#picker-tab-${kind}`).click();
    await page.waitForSelector(READY[kind]);
    near(await rect(page, '.live-modal'), frame, `the dialog on ${kind}`);
    near(await rect(page, '.picker-tabs'), row, `the tabs on ${kind}`);
    await expectTabs(page, kind, `after choosing ${kind}`);
    if (kind === 'emoji' || kind === 'stickers') await page.waitForFunction(() => document.activeElement?.id === 'emoji-query');
  }
  checks.push('switching tabs keeps the dialog and the tab row within 2 px; the search field takes focus');

  // Pressing the open tab again changes nothing (what is typed stays).
  await page.locator('#emoji-query').fill('heart');
  await wait(300);
  await page.locator('#picker-tab-emoji').click();
  await wait(200);
  assert.equal(await page.locator('#emoji-query').inputValue(), 'heart');
  await page.locator('#emoji-query').fill('');
  await wait(300);
  checks.push('pressing the open tab again keeps the search');

  // Arrow keys move along the tabs (and wrap); focus stays on the tabs.
  await page.locator('#picker-tab-emoji').focus();
  for (const [key, kind] of [['ArrowRight', 'stickers'], ['ArrowRight', 'gif'], ['End', 'kaomoji'], ['ArrowRight', 'emoji'], ['ArrowLeft', 'kaomoji'], ['Home', 'emoji']]) {
    await page.keyboard.press(key);
    await page.waitForSelector(READY[kind]);
    assert.equal(await dialogOf(page), kind, `${key} opens ${kind}`);
    assert.equal(await page.evaluate(() => document.activeElement?.id), `picker-tab-${kind}`, `${key}: focus on the ${kind} tab`);
    assert.equal(await page.evaluate(() => document.activeElement?.getAttribute('tabindex')), '0', 'the open tab is the tab stop');
  }
  checks.push('arrow keys, Home and End move between the tabs and keep focus on them');

  // The tabs look like tabs: the selected one has its own background, the
  // others are not bare link text (they have padding and a height of a control).
  const looks = await page.evaluate(() => [...document.querySelectorAll('.picker-tabs [role=tab]')].map(t => {
    const s = getComputedStyle(t);
    return {on: t.getAttribute('aria-selected') === 'true', bg: s.backgroundColor, h: t.getBoundingClientRect().height, pad: parseFloat(s.paddingLeft), line: s.textDecorationLine};
  }));
  const on = looks.find(l => l.on), off = looks.filter(l => !l.on);
  assert.ok(off.every(l => l.bg !== on.bg), 'the selected tab has its own background');
  assert.ok(looks.every(l => l.h >= 28 && l.pad >= 6 && l.line === 'none'), 'every tab is a control, not link text');
  assert.ok(new Set(looks.map(l => Math.round(l.h))).size === 1, 'every tab is the same height');
  checks.push('a segmented tab row with a clear selected state');

  // GIFs without GIF search on the server: the tab stays, and says so inside.
  await page.locator('#picker-tab-gif').click();
  await page.waitForSelector(READY.gif);
  assert.ok((await page.locator('.picker-modal').textContent()).includes('GIF search is not enabled on this server'));
  assert.equal(await page.locator('.picker-modal [data-live="pick-image"]').count(), 1);
  await expectTabs(page, 'gif', 'GIFs disabled');
  checks.push('GIFs tab on a server without GIF search: same tabs, the explanation and an upload inside');

  // Emoji categories: one row of distinct choices, the first marked; a jump
  // goes to the section and draws it; the mark follows the scroll.
  await page.locator('#picker-tab-emoji').click();
  await page.waitForSelector(READY.emoji);
  const nav = await page.evaluate(() => {
    const n = document.querySelector('.picker-nav');
    const b = [...n.querySelectorAll('[data-emoji-jump]')];
    return {names: b.map(x => x.getAttribute('aria-label')), tops: new Set(b.map(x => Math.round(x.getBoundingClientRect().top))).size, current: b.filter(x => x.getAttribute('aria-current') === 'true').map(x => x.getAttribute('aria-label')), wraps: n.scrollHeight > n.clientHeight + 1};
  });
  for (const name of ['Picker QA', 'Smileys & People', 'Nature', 'Food', 'Activities', 'Travel', 'Objects', 'Symbols', 'Flags'])
    assert.ok(nav.names.includes(name), `category bar offers ${name}`);
  assert.equal(nav.tops, 1, 'the category bar is one row');
  assert.equal(nav.wraps, false, 'the category bar does not wrap');
  assert.equal(nav.current.length, 1, 'one category is marked');
  const jumpTo = async name => {
    await page.locator(`.picker-nav [aria-label="${name}"]`).click();
    await page.waitForFunction(n => document.querySelector(`.picker-nav [aria-label="${n}"]`)?.getAttribute('aria-current') === 'true', name);
    const key = await page.locator(`.picker-nav [aria-label="${name}"]`).getAttribute('data-emoji-jump');
    await page.waitForFunction(k => document.querySelectorAll(`[data-emoji-section="${k}"] .emoji-grid button, [data-emoji-section="${k}"] .emoji-grid img`).length > 0, key);
    const off = await page.evaluate(k => document.querySelector(`[data-emoji-section="${k}"]`).getBoundingClientRect().top - document.querySelector('.picker-scroll').getBoundingClientRect().top, key);
    assert.ok(Math.abs(off) <= 16, `${name} is at the top of the list (${off})`);
    const marked = await page.evaluate(() => [...document.querySelectorAll('.picker-nav [aria-current=true]')].map(b => b.getAttribute('aria-label')));
    assert.deepEqual(marked, [name], `only ${name} is marked`);
    return key;
  };
  await jumpTo('Flags');
  await shot(page, 'picker-emoji-flags');
  await jumpTo('Travel');
  await jumpTo('Picker QA');
  assert.equal(await page.locator('[data-emoji-section] [aria-label=":pickwave:"]').count(), 1, 'the server emoji is in its section');
  // Scrolling (as a person would, with the wheel) moves the mark.
  const food = await page.evaluate(() => {
    const list = document.querySelector('.picker-scroll'), s = [...list.querySelectorAll('[data-emoji-section]')].find(x => x.getAttribute('aria-label') === 'Food');
    return s.getBoundingClientRect().top - list.getBoundingClientRect().top;
  });
  const box = await rect(page, '.picker-scroll');
  await page.mouse.move(box.x + box.w / 2, box.y + box.h / 2);
  await page.mouse.wheel(0, food + 4);
  await page.waitForFunction(() => document.querySelector('.picker-nav [aria-current=true]')?.getAttribute('aria-label') === 'Food');
  // Everything between was drawn as it came near, and the list did not jump.
  const foodKey = await page.locator('.picker-nav [aria-label="Food"]').getAttribute('data-emoji-jump');
  await page.waitForFunction(k => document.querySelectorAll(`[data-emoji-section="${k}"] .emoji-grid button`).length > 0, foodKey);
  const drift = await page.evaluate(() => {
    const list = document.querySelector('.picker-scroll'), s = [...list.querySelectorAll('[data-emoji-section]')].find(x => x.getAttribute('aria-label') === 'Food');
    return s.getBoundingClientRect().top - list.getBoundingClientRect().top;
  });
  assert.ok(Math.abs(drift) <= 40, `the Food section stays where the scroll put it (${drift})`);
  checks.push('category bar: one row of icon choices, jump to a section (drawn on arrival), the mark follows the scroll');
  // Search: one list of results, no category bar; custom emoji found by name.
  await page.locator('#emoji-query').fill('pickw');
  await page.waitForSelector('[data-emoji-section] [aria-label=":pickwave:"]');
  await page.locator('#emoji-query').fill('smile');
  await page.waitForSelector('[data-emoji-section=search] .emoji-grid button');
  assert.equal(await page.locator('.picker-nav').count(), 0, 'no category bar while searching');
  near(await rect(page, '.live-modal'), frame, 'the dialog while searching');
  await page.locator('#emoji-query').fill('');
  await page.waitForSelector('.picker-nav');
  checks.push('search finds custom and bundled emoji in the same frame');

  // Add Reaction: emoji only, by design; same frame and categories.
  const mid = await page.locator('[data-live-message]').last().getAttribute('data-live-message');
  await act(page, `dialog:emoji-react:${mid}`);
  await page.waitForSelector(READY.emoji);
  assert.equal(await page.locator('.live-modal [role=tablist]').count(), 0, 'Add Reaction has no tabs');
  assert.equal(await page.locator('.picker-modal h2').textContent(), 'Add Reaction');
  assert.ok(await page.locator('.picker-nav [data-emoji-jump]').count() >= 9);
  near(await rect(page, '.live-modal'), frame, 'Add Reaction');
  await shot(page, 'picker-reaction');
  await act(page, 'close');
  checks.push('Add Reaction offers emoji only, in the same frame, with the category bar');

  // Desktop screenshots of each picker, default theme.
  for (const kind of KINDS) { await open(page, kind); await shot(page, `picker-${kind}`); }
  await act(page, 'close');

  // ---- While the modules are still on their way -------------------------
  // Every WASM module and chunk requested from now on arrives 1.5 s late.
  slow = await h.phone(1440, 900);
  slow.on('pageerror', e => errors.push(String(e)));
  await register(slow, h.origin, 'picker_slow');
  const code = await (async () => {
    await form(page, 'invite'); await page.waitForSelector('.live-modal code');
    const c = await page.locator('.live-modal code').first().textContent(); await act(page, 'close'); return c;
  })();
  await form(slow, 'join', {'f-name': code});
  await slow.waitForSelector('#live-input');
  await wait(1000);
  let delayed = 0;
  await slow.context().route(/\/(split_|chunk_)[^/]*\.wasm/, async route => { delayed++; await wait(1500); await route.continue(); });
  await slow.locator('.composer [aria-label="Emoji Picker"]').click();
  await slow.waitForSelector('.picker-modal .picker-tabs');
  assert.equal(await slow.locator(READY.emoji).count(), 0, 'measured before the contents arrived');
  await expectTabs(slow, 'emoji', 'while loading');
  const early = await rect(slow, '.live-modal');
  // A tab chosen while loading shows at once, with the same choices.
  await slow.locator('#picker-tab-stickers').click();
  await slow.waitForFunction(() => document.querySelector('[data-dialog]')?.dataset.dialog === 'stickers');
  await expectTabs(slow, 'stickers', 'stickers while loading');
  near(await rect(slow, '.live-modal'), early, 'stickers while loading');
  await slow.locator('#picker-tab-gif').click();
  await slow.waitForFunction(() => document.querySelector('[data-dialog]')?.dataset.dialog === 'gif');
  await expectTabs(slow, 'gif', 'GIFs while loading');
  await slow.waitForSelector(READY.gif, {timeout: 30000});
  assert.ok(delayed > 0, 'a module was delayed');
  near(await rect(slow, '.live-modal'), early, 'after the contents arrived');
  await expectTabs(slow, 'gif', 'after loading');
  for (const kind of ['kaomoji', 'stickers', 'emoji']) {
    await slow.locator(`#picker-tab-${kind}`).click();
    await slow.waitForSelector(READY[kind], {timeout: 30000});
    await expectTabs(slow, kind, `${kind} after loading`);
    near(await rect(slow, '.live-modal'), early, `${kind} after loading`);
  }
  await slow.context().unroute(/\/(split_|chunk_)[^/]*\.wasm/);
  checks.push(`while modules load (${delayed} delayed) the same frame and four tabs show, no other layout`);

  // ---- Phone (390x844, touch) --------------------------------------------
  phone = await h.phone(390, 844);
  phone.on('pageerror', e => errors.push(String(e)));
  await register(phone, h.origin, 'picker_phone');
  await form(phone, 'join', {'f-name': code});
  await phone.waitForSelector('#live-input');
  await open(phone, 'emoji');
  const phoneFrame = await rect(phone, '.live-modal');
  for (const kind of [...KINDS, 'emoji']) {
    if (kind !== 'emoji' || (await dialogOf(phone)) !== 'emoji') await phone.locator(`#picker-tab-${kind}`).tap();
    await phone.waitForSelector(READY[kind]);
    await expectTabs(phone, kind, `phone ${kind}`);
    near(await rect(phone, '.live-modal'), phoneFrame, `phone dialog on ${kind}`);
    const tabs = await phone.evaluate(() => [...document.querySelectorAll('.picker-tabs [role=tab]')].map(t => { const r = t.getBoundingClientRect(); return {h: r.height, top: Math.round(r.top), right: r.right}; }));
    assert.ok(tabs.every(t => t.h >= 44), 'phone tabs are at least 44 px tall');
    assert.equal(new Set(tabs.map(t => t.top)).size, 1, 'phone tabs are one row');
    assert.ok(tabs.every(t => t.right <= 390), 'phone tabs fit the screen');
  }
  // Nothing is focused for typing on a phone (the keyboard would cover it).
  assert.notEqual(await phone.evaluate(() => document.activeElement?.id), 'emoji-query');
  const phoneJumps = await phone.evaluate(() => [...document.querySelectorAll('.picker-nav [data-emoji-jump]')].map(b => b.getBoundingClientRect().height));
  assert.ok(phoneJumps.length >= 9 && phoneJumps.every(x => x >= 44), 'phone category choices are 44 px');
  await phone.locator('.picker-nav [aria-label="Nature"]').tap();
  await phone.waitForFunction(() => document.querySelector('.picker-nav [aria-label="Nature"]')?.getAttribute('aria-current') === 'true');
  // The first choice (the server's emoji) goes back to the very top.
  await phone.locator('.picker-nav [data-emoji-jump]').first().tap();
  await phone.waitForFunction(() => document.querySelector('.picker-scroll').scrollTop === 0 && document.querySelector('.picker-nav [data-emoji-jump]').getAttribute('aria-current') === 'true');
  for (const kind of KINDS) { await open(phone, kind); await shot(phone, `picker-${kind}`); }
  await act(phone, 'close');
  checks.push('phone: the same tabs in one row of 44 px targets, the same frame on every tab, category bar of 44 px choices');

  // ---- The light theme ------------------------------------------------------
  for (const p of [page, phone]) {
    await setTheme(p, 'light');
    for (const kind of KINDS) { await open(p, kind); await shot(p, `picker-${kind}-light`); }
    await act(p, 'close');
    await setTheme(p, 'dark');
  }
  checks.push('screenshots in the default and the light theme');
} catch (e) {
  for (const [name, p] of [['desktop', page], ['phone', phone], ['slow', slow]]) await p?.screenshot({path: join(shots, `failure-${name}.png`)}).catch(() => {});
  await h.close();
  throw e;
}
await h.close();

// ---- A server with GIF search (a local mock of the provider) ------------------
const GIF = Buffer.from('R0lGODlhAQABAIAAAP///wAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw==', 'base64');
const item = n => ({id: `g${n}`, title: `Mock GIF ${n}`, content_description: `Mock GIF ${n}`, media_formats: {
  nanowebm: {url: `https://static.klipy.com/mock/${n}.webm`, dims: [160, 120]},
  mediumgif: {url: `https://static.klipy.com/mock/${n}.gif`, dims: [320, 240]}}});
const provider = http((req, res) => {
  const url = new URL(req.url, 'http://x');
  res.writeHead(200, {'content-type': 'application/json'});
  if (url.pathname.endsWith('/categories')) res.end(JSON.stringify({tags: ['Happy', 'Dance', 'Thank You', 'Wow'].map(name => ({name, searchterm: name.toLowerCase()}))}));
  else if (url.pathname.endsWith('/posts')) res.end(JSON.stringify({results: (url.searchParams.get('ids') ?? '').split(',').filter(Boolean).map(id => item(Number(id.slice(1)))), next: ''}));
  else res.end(JSON.stringify({results: [1, 2, 3, 4, 5, 6].map(item), next: ''}));
});
provider.listen(0, '127.0.0.1'); await once(provider, 'listening');
h = await harness(['--gif-provider', 'klipy', '--gif-mode', 'proxy', '--gif-api-key', 'local-test-key', '--gif-proxy-approved',
  '--mock-gif-base', `http://127.0.0.1:${provider.address().port}`, '--gif-media-origins', 'https://static.klipy.com']);
let gifPage;
try {
  gifPage = await h.phone(1440, 900);
  gifPage.on('pageerror', e => errors.push(String(e)));
  await gifPage.context().route(/^https:\/\/static[12]?\.klipy\.com\//, route => route.fulfill({status: 200, contentType: route.request().url().endsWith('.gif') ? 'image/gif' : 'video/webm', body: route.request().url().endsWith('.gif') ? GIF : Buffer.alloc(0)}));
  await register(gifPage, h.origin, 'picker_gifs');
  await form(gifPage, 'create', {'f-name': 'GIF QA'});
  await gifPage.waitForSelector('#live-input');
  await gifPage.locator('.composer .composer-gif').click();
  await gifPage.waitForSelector('.gif-category');
  await expectTabs(gifPage, 'gif', 'GIFs enabled');
  const frame = await rect(gifPage, '.live-modal');
  // Browse and Favorites look like the category bar's choices.
  assert.equal(await gifPage.locator('.picker-nav [aria-pressed=true]').textContent(), 'Browse');
  await gifPage.locator('.gif-category').first().click();
  await gifPage.waitForSelector('.gif-result');
  await gifPage.locator('.gif-result .gif-star').first().click();
  await gifPage.waitForSelector('.gif-result .gif-star.is-starred');
  await gifPage.locator('[data-live="gif-tab:favorites"]').click();
  await gifPage.waitForFunction(() => document.querySelectorAll('.gif-result').length === 1);
  // A search from Favorites shows its results.
  await gifPage.locator('#gif-query').fill('wow');
  await gifPage.locator('#gif-query').press('Enter');
  await gifPage.waitForFunction(() => document.querySelectorAll('.gif-result').length === 6);
  near(await rect(gifPage, '.live-modal'), frame, 'GIFs with results');
  for (const kind of ['emoji', 'gif', 'kaomoji', 'stickers', 'gif']) {
    await gifPage.locator(`#picker-tab-${kind}`).click();
    await gifPage.waitForSelector(READY[kind]);
    await expectTabs(gifPage, kind, `GIFs enabled, ${kind}`);
    near(await rect(gifPage, '.live-modal'), frame, `GIFs enabled, ${kind}`);
  }
  await gifPage.waitForSelector('.gif-category');
  await shot(gifPage, 'picker-gif-enabled');
  await gifPage.setViewportSize({width: 390, height: 844});
  await shot(gifPage, 'picker-gif-enabled');
  checks.push('GIFs on a server with GIF search: the same tabs and frame, Browse and Favorites, search from Favorites');
  assert.deepEqual(errors, []);
  await writeFile(join(shots, 'picker-results.json'), JSON.stringify({revision: h.revision, checks, screenshots}, null, 2));
  console.log(JSON.stringify({checks, screenshots: screenshots.length, errors}, null, 2));
} catch (e) {
  await gifPage?.screenshot({path: join(shots, 'failure-gif.png')}).catch(() => {});
  throw e;
} finally {
  await h.close();
  provider.close();
}

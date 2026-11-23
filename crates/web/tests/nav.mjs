// Navigation features against a real local server, headless and loopback-only:
// search with filters, Quick Switcher, inbox, saved messages and reminders,
// pinned conversations, Favorites and server folders (drag and menu paths).
import assert from 'node:assert/strict';
import {mkdir,writeFile} from 'node:fs/promises';
import {join} from 'node:path';
import {harness,register,form,act,wait,until} from './qa-harness.mjs';

const shots=process.env.STICKTALK_NAV_SHOTS??join(process.env.CARGO_TARGET_DIR??'.','nav-shots');
await mkdir(shots,{recursive:true});
const h=await harness(),checks=[],errors=[],saved=[];
const log=s=>process.env.STICKTALK_NAV_VERBOSE&&console.log(`[nav] ${s}`);
let alice,bob;
const sizes=[[1440,900],[390,844]];
async function shot(page,name,prepare){
  log(`shot ${name}`);
  const viewport=page.viewportSize();
  for(const [width,height] of sizes){
    await page.setViewportSize({width,height});
    if(prepare)await prepare(width);
    await page.evaluate(()=>document.fonts.ready);await wait(150);
    const path=join(shots,`${name}-${width}x${height}.png`);
    await page.screenshot({path,caret:'initial'});saved.push(path);
  }
  await page.setViewportSize(viewport);
}
/** A same-origin API call from the signed-in page, with its CSRF header.
 * `body` is raw JSON text: Snowflake IDs do not fit in a JS number. */
const api=(page,method,path,body)=>page.evaluate(async([method,path,body])=>{
  const csrf=document.cookie.split(';').map(v=>v.trim()).find(v=>v.startsWith('__Host-sticktalk_csrf='))?.slice(22)??'';
  const r=await fetch(`/api/v1${path}`,{method,headers:{'content-type':'application/json','x-sticktalk-csrf':csrf},body:body??undefined});
  return {status:r.status,body:await r.json().catch(()=>null)};
},[method,path,body]);
async function send(page,text){
  await page.locator('#live-input').fill(text);
  await page.locator('#live-input').press('Enter');
  await page.waitForFunction(t=>[...document.querySelectorAll('.message .markdown')].some(m=>m.textContent.includes(t)),text.replace(/@\w+/g,'').trim().split(' ').at(-1));
}
const dialog=page=>page.evaluate(()=>document.querySelector('.live-modal')?.dataset.dialog??'');
async function closeDialog(page){await act(page,'close');await page.waitForFunction(()=>!document.querySelector('.live-modal'));}

try {
  alice=await h.phone(1440,900);bob=await h.phone(390,844);
  for(const p of[alice,bob]){p.on('pageerror',e=>errors.push(String(e)));p.setDefaultTimeout(30000);}
  await register(alice,h.origin,'alice_nav');await register(bob,h.origin,'bob_nav');
  await form(alice,'create',{'f-name':'Nav HQ'});await alice.waitForSelector('#live-input');
  const hq=(await alice.locator('.server-icon.selected').getAttribute('data-live')).slice(6);
  const general=await alice.locator('#live-messages').getAttribute('data-channel');
  await form(alice,'invite');await alice.waitForSelector('.live-modal code');
  const code=await alice.locator('.live-modal code').first().textContent();await closeDialog(alice);
  await form(bob,'join',{'f-name':code});await bob.waitForSelector('#live-input');
  await form(alice,'create',{'f-name':'Side Room'});await alice.waitForSelector('[aria-label="Side Room"].selected');
  await form(alice,'create',{'f-name':'Third Place'});await alice.waitForSelector('[aria-label="Third Place"].selected');
  await form(alice,'friend',{'f-name':'bob_nav'});
  await act(bob,'home');
  await bob.locator('[data-live^="friend:accept,"]').click();
  await act(bob,`guild:${hq}`);await bob.waitForSelector('#live-input');
  log('check');checks.push('three servers, a shared server and a friendship set up through the real UI');

  // Bob writes in the shared server while Alice is elsewhere.
  await send(bob,'launch plan https://example.test/plan');
  await send(bob,'@alice_nav please review the launch checklist');
  await send(bob,'unrelated chatter');

  // ---- Search ----
  log('Search');
  await alice.locator(`.server-rail [data-live="guild:${hq}"]`).click();
  await alice.waitForFunction(g=>document.querySelector('.server-icon.selected')?.dataset.live===`guild:${g}`,hq);
  const box=alice.locator('#nav-search-input');
  await box.click();await box.pressSequentially('fr');
  await alice.waitForSelector('#nav-suggest:not(.hidden) [data-nav-suggest]');
  assert.match(await alice.locator('#nav-suggest').innerText(),/from:/);
  await box.press('Tab');await box.pressSequentially('bob');
  await alice.waitForFunction(()=>document.querySelector('#nav-suggest')?.innerText.includes('@bob_nav'));
  await box.press('ArrowDown');await box.press('Enter');
  assert.equal(await box.inputValue(),'from:bob_nav ');
  await box.pressSequentially('launch');await box.press('Enter');
  await alice.waitForSelector('.nav-results .nav-card');
  assert.equal(await alice.locator('.nav-results .nav-card').count(),2);
  assert(await alice.locator('.nav-results mark.nav-hit').count()>=2);
  log('check');checks.push('search box suggests filters and people as you type; results highlight terms');
  await shot(alice,'search-results',async w=>{if(w<768)await act(alice,'nav-search-open');});
  log('filters');
  await box.fill('has:link launch');await box.press('Enter');
  await alice.waitForFunction(()=>document.querySelectorAll('.nav-results .nav-card').length===1&&document.querySelector('.nav-results').textContent.includes('example.test'));
  await box.fill('has:video launch');await box.press('Enter');
  await alice.waitForFunction(()=>document.querySelector('.nav-results')?.textContent.includes('No results'));
  await box.fill('during:1999-01-01 launch');await box.press('Enter');
  await alice.waitForFunction(()=>document.querySelector('.nav-results')?.textContent.includes('No results'));
  log('check');checks.push('has:link, has:video and during: filters narrow results on the real server');
  await box.fill('checklist');await box.press('Enter');
  await alice.waitForSelector('.nav-results .nav-card');
  const target=await alice.locator('.nav-results .nav-card').first().getAttribute('data-nav-message');
  // Results stay open while you go elsewhere; a result still jumps across servers.
  await alice.locator('[aria-label="Side Room"][data-rail]').click();
  await alice.waitForSelector('[aria-label="Side Room"].selected');
  await alice.locator('.nav-results .nav-card-main').first().click();
  await alice.waitForFunction(([c,m])=>document.querySelector('#live-messages')?.dataset.channel===c&&!!document.querySelector(`[data-live-message="${m}"]`),[general,target]);
  log('check');checks.push('a result jumps to the message in its channel');
  await act(alice,'nav-search-close');await alice.waitForFunction(()=>!document.querySelector('.nav-results'));
  // DM search: Alice and Bob talk directly; search from Home covers only DMs.
  await alice.locator('[data-live="home"]').first().click();
  await alice.locator('[data-live^="friend:dm,"]').click();await alice.waitForSelector('#live-input');
  const dm=await alice.locator('#live-messages').getAttribute('data-channel');
  await send(alice,'direct launch secret');
  await box.fill('launch');await box.press('Enter');
  await alice.waitForFunction(()=>document.querySelectorAll('.nav-results .nav-card').length===1&&document.querySelector('.nav-results').textContent.includes('direct launch'));
  const outsider=await api(bob,'GET',`/users/@me/messages/search?q=launch&channel_id=${general}`);
  assert.equal(outsider.status,404);
  log('check');checks.push('direct-message search finds only the open DM; a server channel is refused');
  await act(alice,'nav-search-close');

  // ---- Quick Switcher ----
  log('Quick Switcher');
  await send(bob,'@alice_nav ping about the launch');
  await alice.waitForSelector(`[data-rail="g${hq}"] .mention-badge`);
  await alice.keyboard.press('Control+k');
  await alice.waitForFunction(()=>document.querySelector('.live-modal')?.dataset.dialog==='nav-switcher');
  await alice.waitForFunction(()=>document.activeElement?.id==='nav-switcher-input');
  // Unread and mentioned conversations are ranked first before typing.
  assert.equal(await alice.locator('#nav-switcher-list [role=option]:not([hidden])').first().getAttribute('data-rank'),'2');
  await shot(alice,'quick-switcher');
  await alice.keyboard.type('*side');
  await alice.waitForFunction(()=>document.querySelector('#nav-switcher-list [aria-selected=true]')?.textContent.includes('Side Room'));
  await alice.keyboard.press('Enter');
  await alice.waitForSelector('[aria-label="Side Room"].selected');
  assert.equal(await dialog(alice),'');
  await alice.keyboard.press('Control+k');await alice.waitForSelector('#nav-switcher-input');
  await alice.keyboard.type('gnrl');
  await alice.keyboard.press('ArrowDown');await alice.keyboard.press('ArrowUp');
  await alice.waitForFunction(()=>document.querySelector('#nav-switcher-list [aria-selected=true]')?.textContent.includes('general'));
  await alice.keyboard.press('Escape');await alice.waitForFunction(()=>!document.querySelector('.live-modal'));
  log('check');checks.push('Ctrl+K opens the switcher; fuzzy matches, sigils, arrows, Enter and Escape work');

  // ---- Inbox ----
  log('Inbox');
  await alice.locator('[data-live="nav-inbox"]').first().click();
  await alice.waitForSelector('.nav-unread .nav-card');
  assert.match(await alice.locator('.nav-inbox-body').innerText(),/Nav HQ/i);
  await shot(alice,'inbox-unreads');
  await act(alice,'nav-inbox-tab:mentions');
  await alice.waitForSelector('.nav-inbox-body .nav-card');
  assert.match(await alice.locator('.nav-inbox-body').innerText(),/checklist/);
  await shot(alice,'inbox-mentions');
  await act(alice,'nav-inbox-tab:unreads');await alice.waitForSelector('[data-live="nav-ack-all"]');
  await alice.locator('[data-live="nav-ack-all"]').click();
  await alice.waitForFunction(()=>document.querySelector('.nav-inbox-body')?.textContent.includes('caught up'));
  log('check');checks.push('inbox lists unreads by server and mentions; Mark All as Read clears them');
  await closeDialog(alice);

  // ---- Saved messages and reminders ----
  log('Saved messages and reminders');
  await alice.locator(`.server-rail [data-live="guild:${hq}"]`).click();
  await alice.waitForFunction(c=>document.querySelector('#live-messages')?.dataset.channel===c,general);
  await alice.waitForSelector(`[data-live-message="${target}"]`);
  const row=alice.locator(`[data-live-message="${target}"]`);
  await row.hover();await row.locator('[data-live^="nav-save:"]').click();
  await alice.waitForSelector('[data-toast=notice]');
  await row.hover();await row.locator('[data-live^="nav-remind:"]').click();
  await alice.waitForFunction(()=>document.querySelector('.live-modal')?.dataset.dialog?.startsWith('nav-remind:'));
  await shot(alice,'remind-me');
  await alice.locator('.nav-presets button',{hasText:'In 1 hour'}).click();
  await alice.waitForFunction(()=>document.querySelector('[data-toast=notice]')?.textContent.includes('Reminder set'));
  const soon=await api(alice,'PUT',`/users/@me/saved/${target}`,`{"channel_id":${general},"remind_at":${Date.now()+1500}}`);
  assert.equal(soon.status,200,JSON.stringify(soon.body));
  await alice.waitForSelector('.nav-reminder',{timeout:15000});
  assert.match(await alice.locator('.nav-reminder').innerText(),/checklist/);
  await shot(alice,'reminder-due');
  await act(alice,'nav-inbox:saved');
  await alice.waitForSelector('.nav-inbox-body .nav-due');
  await shot(alice,'inbox-saved');
  log('check');checks.push('save, reminder presets and a timer-fired reminder reach the in-app toast and Saved view');
  // A message cannot be saved under a conversation it is not in.
  assert.equal((await api(bob,'PUT',`/users/@me/saved/${target}`,`{"channel_id":${dm}}`)).status,404);
  await closeDialog(alice);

  // ---- Pins and Favorites ----
  log('Pins and Favorites');
  await alice.locator('[data-live="home"]').first().click();
  await alice.locator(`.channel-row:has([data-live="channel:${dm}"])`).click({button:'right'});
  await alice.waitForFunction(()=>document.querySelector('.live-modal')?.dataset.dialog?.startsWith('nav-conv:'));
  await shot(alice,'conversation-options');
  await alice.locator('.nav-menu button',{hasText:'Pin to Top'}).click();
  await alice.waitForSelector('.nav-pinned');
  assert.equal(await alice.locator(`.nav-conv-row [data-live="channel:${dm}"]`).count(),1);
  await act(alice,`nav-fav:${dm}`);
  await act(alice,`nav-fav:${general}`);
  await alice.locator('[data-live="nav-favorites"]').click();
  await alice.waitForFunction(()=>document.querySelector('.sidebar-header')?.textContent.includes('Favorites'));
  await act(alice,'nav-menu:favorites-manage');
  await alice.locator('#nav-fav-cat-name').fill('Launch');await alice.locator('[data-nav-form="fav-category"] [type=submit]').click();
  await alice.waitForSelector('[data-nav-fav-category]');
  await alice.locator(`[data-nav-fav-category="${general}"]`).selectOption({label:'Launch'});
  await alice.waitForFunction(()=>document.querySelectorAll('.category-row').length>=1);
  await closeDialog(alice);
  await shot(alice,'favorites',async w=>{if(w<768)await act(alice,'panel:left');});
  await act(alice,'panel:');
  log('check');checks.push('pin a DM to the top; add favorites, create a category and file a channel in it');

  // ---- Server folders ----
  log('Server folders');
  await alice.locator(`.server-rail [data-live="guild:${hq}"]`).click();
  await alice.locator('[aria-label="Side Room"][data-rail]').dragTo(alice.locator(`[data-rail="g${hq}"]`));
  await alice.waitForSelector('.rail-folder');
  assert.equal(await alice.locator('.rail-folder [data-rail^="g"]').count(),2);
  log('check');checks.push('dragging a server onto another makes a folder');
  await alice.locator('[aria-label="Third Place"][data-rail]').click({button:'right'});
  await alice.waitForFunction(()=>document.querySelector('.live-modal')?.dataset.dialog?.startsWith('nav-server:'));
  await alice.locator('.nav-menu button',{hasText:'Add to New Folder'}).click();
  await alice.waitForFunction(()=>document.querySelectorAll('.rail-folder [data-rail^="g"]').length===3);
  const folder=await alice.locator('.rail-folder').getAttribute('data-folder');
  await act(alice,`nav-menu:folder:${folder}`);
  await alice.locator('#nav-folder-name').fill('Launch Crew');
  await alice.locator('[data-nav-color="#f2b84b"]').click();
  await shot(alice,'folder-settings');
  await alice.locator('[data-nav-form^="folder:"] [type=submit]').click();
  await alice.waitForFunction(()=>!document.querySelector('.live-modal'));
  await alice.locator(`[data-rail="f${folder}"]`).click();
  await alice.waitForFunction(()=>document.querySelector('.folder-icon')?.getAttribute('aria-expanded')==='false');
  await shot(alice,'rail-folder',async w=>{if(w<768)await act(alice,'panel:left');});
  await act(alice,'panel:');
  log('check');checks.push('menu path adds to a folder; rename, recolor and collapse persist');
  // Everything above is stored per account on the server and survives a reload.
  await alice.reload();await alice.waitForSelector('.server-rail');
  await alice.waitForFunction(()=>document.querySelector('.folder-icon')?.getAttribute('aria-label')?.startsWith('Launch Crew'));
  const nav=await api(alice,'GET','/users/@me/nav');
  assert.equal(nav.body.data.pinned.length,1);
  assert.equal(nav.body.data.favorites.length,2);
  assert.equal(nav.body.data.folders[0].name,'Launch Crew');
  // Bob cannot see or change Alice's arrangement.
  const bobNav=await api(bob,'GET','/users/@me/nav');
  assert.equal(bobNav.body.data.folders.length,0);
  assert.equal((await api(bob,'PUT','/users/@me/nav/pins',`{"channel_ids":[${dm}]}`)).status,200,'bob is in the DM too');
  assert.equal((await api(bob,'PUT','/users/@me/nav/pins',`{"channel_ids":[${general},${general}]}`)).status,400);
  log('check');checks.push('arrangement persists across reload, is private to each account, and writes are validated');

  // ---- Phone entry points ----
  log('Phone entry points');
  await act(bob,'home');
  await act(bob,'panel:left');
  await bob.locator('.nav-find').click();
  await bob.waitForFunction(()=>document.querySelector('.live-modal')?.dataset.dialog==='nav-switcher');
  await closeDialog(bob);
  await act(bob,'panel:');
  await bob.locator('.bottom-tabs [data-live="nav-inbox"]').click();
  await bob.waitForFunction(()=>document.querySelector('.live-modal')?.dataset.dialog==='nav-inbox');
  await closeDialog(bob);
  log('check');checks.push('phone: sidebar search opens the switcher; Notifications tab opens the inbox');
  assert.deepEqual(errors,[]);
  await writeFile(join(shots,'nav-results.json'),JSON.stringify({revision:h.revision,checks,screenshots:saved,errors},null,2));
  console.log(JSON.stringify({checks,errors,screenshots:saved.length}));
} catch(e) {
  await writeFile(join(shots,'nav-failure.json'),JSON.stringify({error:String(e.stack??e),checks,errors,body:await alice?.locator('body').innerText().catch(()=>'')},null,2));
  if(alice)await alice.screenshot({path:join(shots,'failure.png')}).catch(()=>{});
  throw e;
} finally {await h.close();}

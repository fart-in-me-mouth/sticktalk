import assert from 'node:assert/strict';
import {cp,mkdtemp,readFile,writeFile} from 'node:fs/promises';
import {join} from 'node:path';
import {brotliCompressSync} from 'node:zlib';
import {harness,register,form,act,shotAll as shot,output,target} from './qa-harness.mjs';
const dist=await mkdtemp(join(target,'pwa-dist-'));
await cp(join(target,'web-dist'),dist,{recursive:true});
const h=await harness([],{dist}),checks=[],screenshots=[],errors=[];
let page;
try {
  page=await h.phone();page.on('pageerror',e=>errors.push(String(e)));
  await register(page,h.origin,'qa_pwa');await form(page,'create',{'f-name':'Offline and Updates'});await page.waitForSelector('#live-input');
  const channel=await page.locator('#live-messages').getAttribute('data-channel');
  await page.locator('#live-input').fill('Private QA message must never enter the offline cache');await act(page,'complete-close');
  await page.getByRole('button',{name:'Send Message',exact:true}).click();await page.waitForSelector('.markdown');
  await page.goto(`${h.origin}/?channel=${channel}`);await page.waitForSelector('.markdown');
  await page.waitForFunction(()=>!!navigator.serviceWorker.controller,null,{timeout:45000});
  const manifest=await page.evaluate(async()=>await(await fetch('/public/manifest.webmanifest')).json());
  assert.equal(manifest.display,'standalone');assert.equal(manifest.scope,'/');assert.equal(manifest.id,'/');
  for(const icon of manifest.icons) {
    const bytes=await readFile(join(dist,'public',icon.src));
    assert.equal(`${bytes.readUInt32BE(16)}x${bytes.readUInt32BE(20)}`,icon.sizes);
  }
  assert(manifest.icons.some(i=>i.purpose==='maskable'&&i.sizes==='512x512'));
  assert(await page.locator('link[rel=apple-touch-icon]').count());
  const apple=await readFile(join(dist,'public/icons/apple-touch-icon.png'));
  assert.equal(apple.readUInt32BE(16),180);assert.equal(apple.readUInt32BE(20),180);
  checks.push('standalone manifest, 192/512 PNGs, maskable icon, apple-touch install metadata');
  // Simulate a changed stable alias while this tab has not loaded media yet.
  // A release-pinned import must bypass it and keep the tab's own module graph.
  const mediaAlias=await readFile(join(dist,'public/media.js'));
  const poisoned=Buffer.from("throw new Error('new release alias must not load in an old tab');");
  await writeFile(join(dist,'public/media.js'),poisoned);
  await writeFile(join(dist,'public/media.js.br'),brotliCompressSync(poisoned));
  await act(page,'dialog:media-settings');await page.waitForSelector('#live-device-settings select');
  await act(page,'close');
  await writeFile(join(dist,'public/media.js'),mediaAlias);
  await writeFile(join(dist,'public/media.js.br'),brotliCompressSync(mediaAlias));
  checks.push('first lazy media import remains pinned when a deployment replaces stable JS aliases');
  const keys=await page.evaluate(async()=>{
    const results=[];for(const name of await caches.keys())for(const r of await(await caches.open(name)).keys())results.push(r.url);return results;
  });
  assert(!keys.some(url=>new URL(url).pathname.startsWith('/api/')));
  assert(keys.some(url=>url.endsWith('_bg.wasm')));
  await page.context().setOffline(true);await page.reload();
  await page.waitForSelector('#offline-banner');
  await page.waitForFunction(()=>document.body.textContent.includes('Reconnecting…'));
  assert(!await page.locator('body').innerText().then(t=>t.includes('Private QA message')));
  screenshots.push(await shot(page,'offline-shell'));
  checks.push('real service-worker offline reload boots cached shell/core; private messages and API excluded');
  await page.context().setOffline(false);
  await page.waitForSelector('.markdown', {timeout:45000});
  checks.push('online recovery restores cookie session, websocket and authenticated history');

  const sw=await readFile(join(dist,'sw.js'),'utf8');
  const updated=sw.replace(/sticktalk-shell-([a-f0-9]+)/,'sticktalk-shell-qa-update-$1');
  assert.notEqual(sw,updated);
  await writeFile(join(dist,'sw.js'),updated);await writeFile(join(dist,'sw.js.br'),brotliCompressSync(Buffer.from(updated)));
  await page.evaluate(async()=>{await(await navigator.serviceWorker.getRegistration()).update();});
  await page.waitForSelector('#app-update');
  screenshots.push(await shot(page,'update-ready'));
  await page.locator('#live-input').fill('Unsent draft in the original channel');
  await form(page,'channel',{'f-name':'another-channel'});await page.waitForFunction(()=>!document.querySelector('.live-modal'));
  await page.waitForFunction(()=>document.querySelector('#live-input')?.value==='');
  await page.locator('#app-update button').click();
  await page.waitForFunction(()=>document.querySelector('#app-update')?.textContent.includes('Finish your drafts'));
  assert(await page.evaluate(async()=>!!(await navigator.serviceWorker.getRegistration()).waiting));
  checks.push('waiting update never auto-activates; draft in another channel blocks Reload');
  await act(page,`channel:${channel}`);await page.waitForFunction(()=>document.querySelector('#live-input')?.value==='Unsent draft in the original channel');
  await page.locator('#live-input').fill('');
  await Promise.all([page.waitForEvent('load'),page.locator('#app-update button').click()]);
  await page.waitForSelector('.markdown');
  const cachesAfter=await page.evaluate(()=>caches.keys());
  assert.equal(cachesAfter.filter(k=>k.startsWith('sticktalk-shell-')).length,1);
  assert(cachesAfter.some(k=>k.includes('qa-update')));
  checks.push('explicit Reload activates new worker, reloads app, removes obsolete shell cache');
  assert.deepEqual(errors,[]);
  await writeFile(`${output}/pwa-results.json`,JSON.stringify({revision:h.revision,checks,screenshots,errors,manifest,cachedAssets:keys.length},null,2));
  console.log(JSON.stringify({checks,errors}));
} catch(e) {
  await writeFile(`${output}/pwa-failure.json`,JSON.stringify({error:String(e),checks,errors,body:await page?.locator('body').innerText().catch(()=>'' )},null,2));throw e;
} finally {await h.close();}

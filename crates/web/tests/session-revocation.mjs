import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {writeFile} from 'node:fs/promises';
import {join} from 'node:path';
import {harness,register,login,form,act,until,wait,output,target} from './qa-harness.mjs';

const checks=[];
for(const mode of ['delete','password','admin-disable']) {
  const h=await harness(),errors=[];
  let owner;
  try {
    const username=`qa_session_${mode.replaceAll('-','_')}`;
    owner=await h.phone();owner.on('pageerror',e=>errors.push(String(e)));
    await register(owner,h.origin,username);
    await form(owner,'create',{'f-name':'Private session regression'});
    await owner.waitForSelector('#live-input');
    const channel=await owner.locator('#live-messages').getAttribute('data-channel');
    const online=await h.phone(),offline=await h.phone();
    // Playwright's HTTP offline switch does not reliably sever an established
    // WebSocket. Close the browser-owned transport while offline as well, so
    // this device cannot receive the server's later 4001 close notification.
    await offline.addInitScript(()=>{
      const Socket=WebSocket;
      globalThis.__qaSockets=[];
      globalThis.WebSocket=class extends Socket {
        constructor(...args){super(...args);globalThis.__qaSockets.push(this);}
      };
    });
    const counts=new Map();
    for(const page of [online,offline]) {
      counts.set(page,0);page.on('websocket',()=>counts.set(page,counts.get(page)+1));
      page.on('pageerror',e=>errors.push(String(e)));
      await page.goto(h.origin);await login(page,username,'synthetic-qa-password-1234');
      await act(page,`channel:${channel}`);await page.waitForSelector('#live-input');
      await page.locator('#live-input').fill('private draft must be erased');
      await page.evaluate(async()=>{
        await navigator.serviceWorker.ready;
        const privateCache=await caches.open('sticktalk-private-regression');
        await privateCache.put('/api/v1/private-fixture',new Response('private fixture'));
        const shell=(await caches.keys()).find(k=>k.startsWith('sticktalk-shell-'));
        if(shell)await(await caches.open(shell)).put('/api/v1/private-fixture',new Response('private fixture'));
      });
    }
    await offline.context().setOffline(true);await offline.waitForSelector('#offline-banner');
    await offline.evaluate(()=>globalThis.__qaSockets.forEach(socket=>socket.close(1000)));
    await until(()=>offline.locator('.connection-banner').count());
    if(mode==='delete') {
      await form(owner,'delete-account',{'f-name':'synthetic-qa-password-1234','f-value':'DELETE'});
      await owner.waitForSelector('#auth-username');
      await owner.getByRole('status').filter({hasText:'Your account was deleted.'}).waitFor();
    } else if(mode==='password') {
      const oldCookie=(await owner.context().cookies()).find(c=>c.name==='__Host-sticktalk_session').value;
      const staleProbes=[],inspections=[];
      owner.on('request',request=>{
        if(new URL(request.url()).pathname==='/api/v1/browser/session') {
          inspections.push(request.allHeaders().then(headers=>{
            if(headers.cookie?.includes(oldCookie))staleProbes.push(request.url());
          }));
        }
      });
      await act(owner,'dialog:password');
      await owner.locator('#f-name').fill('synthetic-qa-password-1234');
      await owner.locator('#f-value').fill('updated-synthetic-qa-password-5678');
      await Promise.all([owner.waitForEvent('load'),owner.locator('[type=submit]').click()]);
      await owner.waitForSelector('.live-shell');
      await Promise.all(inspections);
      assert.deepEqual(staleProbes,[],'current-device rotation must not race an old-cookie expiry probe');
    } else {
      execFileSync(process.env.STICKTALK_SERVER_EXE??join(target,'debug/sticktalk-server.exe'),[
        '--data-dir',h.data,'admin','disable-user',username,
      ],{windowsHide:true,stdio:'pipe'});
      // A host ban is observed by the existing heartbeat (no test-only polling).
      await owner.waitForSelector('#auth-username',{timeout:45000});
    }
    await online.waitForSelector('#auth-username',{timeout:45000});
    await offline.context().setOffline(false);
    await offline.waitForSelector('#auth-username',{timeout:45000});
    for(const page of [online,offline]) {
      await until(async()=>!(await page.context().cookies()).some(c=>c.name==='__Host-sticktalk_session'));
      await until(()=>page.evaluate(async()=>{
        for(const name of await caches.keys()) {
          if(name==='sticktalk-private-regression')return false;
          for(const r of await(await caches.open(name)).keys())if(new URL(r.url).pathname.startsWith('/api/'))return false;
        }return true;
      }));
      assert.equal(await page.locator('.live-shell,.connection-banner,.live-modal').count(),0);
    }
    const stopped=[counts.get(online),counts.get(offline)];
    await wait(2500);
    assert.deepEqual([counts.get(online),counts.get(offline)],stopped,'revoked devices must not reconnect');
    if(mode==='password') {
      await login(online,username,'updated-synthetic-qa-password-5678');
      await act(online,`channel:${channel}`);await online.waitForSelector('#live-input');
      assert.equal(await online.locator('#live-input').inputValue(),'','old account draft cannot return');
    } else {
      await online.reload();await online.waitForSelector('#auth-username');
    }
    assert.deepEqual(errors,[]);
    assert(!h.log().includes('housekeeping failed'),h.log());
    checks.push(`${mode}: live and offline devices return to login, clear cookies/private caches/drafts and stop reconnecting`);
  } catch(error) {
    await writeFile(`${output}/session-revocation-failure.json`,JSON.stringify({mode,error:String(error),log:h.log(),body:await owner?.locator('body').innerText().catch(()=>''),errors},null,2));
    throw error;
  } finally {await h.close();}
}
await writeFile(`${output}/session-revocation-results.json`,JSON.stringify({checks},null,2));
console.log(JSON.stringify({checks}));

import {swipe} from './gestures.mjs';
import {phoneViewports, engine} from './engines.mjs';
import assert from 'node:assert/strict';
import { writeFile, readFile, readdir } from 'node:fs/promises';
import { join } from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { harness, register, login, form, act, shot, wait, until, output, target } from './qa-harness.mjs';

let h;
const checks = [], screenshots = [], errors = [], csp = [], layout = [];
let page;
try {
  for (const [width, height] of phoneViewports) {
    h=await harness();
    page = await h.phone(width, height);
    page.on('pageerror', e => errors.push(String(e)));
    await page.goto(h.origin);
    screenshots.push(await shot(page, 'login'));
    if(width<768) {
      await page.setViewportSize({width,height:440});
      await page.locator('#auth-password').focus();
      await page.locator('[type=submit]').scrollIntoViewIfNeeded();
      screenshots.push(await shot(page,'login-keyboard-resize'));
      assert(await page.locator('[type=submit]').evaluate(e=>e.getBoundingClientRect().bottom<=visualViewport.height));
      await page.setViewportSize({width,height});
    }
    await page.locator('[data-live=register]').click();
    screenshots.push(await shot(page, 'register'));
    // Start from the login view, so the helper can follow the same user gesture.
    await page.locator('[data-live=register]').click();
    await register(page, h.origin, `qa_phone_${width}`);
    screenshots.push(await shot(page, 'friends-empty'));
    await form(page, 'create', {'f-name':'After Hours — Design & Engineering'});
    await page.waitForSelector('#live-input');
    await page.waitForFunction(() => !document.querySelector('.live-modal'));
    const channel = await page.locator('#live-messages').getAttribute('data-channel');
    const guild = await page.locator('.server-icon.selected').getAttribute('data-live');
    await page.locator('#live-input').fill('Phone release checklist: **markdown**, a long link https://example.invalid/a-very-long-path-that-must-wrap and `inline code`.');
    await act(page, 'complete-close');
    await page.getByRole('button', {name:'Send Message',exact:true}).click();
    await page.waitForSelector('.markdown');
    screenshots.push(await shot(page, 'chat'));
    const bounds = await page.evaluate(() => {
      const input = document.querySelector('#live-input').getBoundingClientRect();
      const main = document.querySelector('.main-pane').getBoundingClientRect();
      return { input: {left:input.left,right:input.right,bottom:input.bottom,width:input.width},
        main:{left:main.left,right:main.right}, viewport:innerWidth, height:visualViewport.height };
    });
    assert(bounds.input.width >= 80 && bounds.input.right <= width);
    assert(bounds.input.bottom <= height);
    layout.push({width,height,composer:bounds});
    // Use actual touch gestures, not only synthetic app actions, on phones.
    if (width < 768) {
      await page.getByRole('button',{name:'Open Channels',exact:true}).tap();
      await page.waitForSelector('.panel-left');
      screenshots.push(await shot(page,'channels-drawer'));
      if(engine==='chromium') {
        const cdp=await page.context().newCDPSession(page);
        await cdp.send('Input.synthesizeScrollGesture',{x:280,y:400,xDistance:-180,yDistance:0,speed:600,gestureSourceType:'touch',preventFling:true});
      } else await swipe(page,{x:280,y:400},{x:100,y:400});
      await page.waitForFunction(()=>!document.querySelector('.panel-left,.panel-right'));
      await page.waitForFunction(()=>!history.state?.sticktalkOverlay?.panel);
      await page.getByRole('button',{name:'Member List',exact:true}).tap();
      await page.waitForSelector('.panel-right');
      screenshots.push(await shot(page,'members-drawer'));
      await page.goBack();
      await page.waitForFunction(()=>!document.querySelector('.panel-right'));
      await page.waitForSelector('#live-input');
      checks.push(`${width}: touch drawer dismissal and browser Back`);
      await page.evaluate(()=>{document.documentElement.style.setProperty('--safe-top','47px');document.documentElement.style.setProperty('--safe-bottom','34px');});
      screenshots.push(await shot(page,'safe-area'));
      assert(await page.locator('.channel-header').evaluate(e=>e.getBoundingClientRect().top>=47));
      assert(await page.locator('#live-input').evaluate(e=>e.getBoundingClientRect().bottom<=innerHeight-34));
      await page.evaluate(()=>{document.documentElement.style.removeProperty('--safe-top');document.documentElement.style.removeProperty('--safe-bottom');});
      checks.push(`${width}: injected 47px/34px safe-area layout`);
      // VisualViewport resize is represented by an actual smaller mobile viewport.
      // This is not a claim that a physical iOS keyboard was certified.
      await page.setViewportSize({width,height:440});
      await page.locator('#live-input').focus();
      await page.locator('#live-input').fill('Draft while the keyboard is open');
      screenshots.push(await shot(page,'keyboard-resize'));
      assert(await page.locator('#live-input').evaluate(e=>e.getBoundingClientRect().bottom<=visualViewport.height));
      await page.setViewportSize({width,height});
      await page.locator('#live-input').fill('');
    }
    for (const screen of ['settings','account','profile-edit','about','appearance','privacy','notifications','devices','media-settings','delete-account',
      'password','server','server-overview','server-channels','roles','member-role','invite','server-bans','server-emoji']) {
      await act(page,`dialog:${screen}`);
      await page.waitForSelector(`.live-modal[data-dialog="${screen}"]`);
      if(screen==='devices') await page.waitForSelector('.device-row');
      if(screen==='media-settings') await page.waitForSelector('#live-device-settings select, #live-device-settings [data-media-unavailable]');
      screenshots.push(await shot(page,screen));
      const overflowing = await page.evaluate(()=>[...document.querySelectorAll('.settings-main,.settings-content,.settings-nav,.live-modal')]
        .filter(e=>e.getClientRects().length && e.scrollWidth>e.clientWidth+2).map(e=>({class:e.className,client:e.clientWidth,scroll:e.scrollWidth})));
      assert.deepEqual(overflowing,[],`${width} ${screen} overflow`);
    }
    await act(page,'dialog:appearance');
    await page.locator('[data-preference=density]').selectOption('compact');
    await page.locator('[data-preference=fontScale]').selectOption('1.25');
    await act(page,'close');
    assert.equal(await page.evaluate(()=>document.documentElement.dataset.density),'compact');
    await act(page,'dialog:appearance');
    await page.locator('[data-preference=density]').selectOption('cozy');
    await page.locator('[data-preference=fontScale]').selectOption('1');
    await act(page,'close');
    checks.push(`${width}: settings/admin screens, responsive layout, local appearance preferences`);
    if (width===phoneViewports[0][0]) {
      await form(page,'about',{'f-name':'they/them','f-value':'Mobile QA profile','f-extra':'#496c63'});
      await until(()=>page.evaluate(async()=>{
        const id=document.querySelector('[data-self-id]').dataset.selfId;
        const p=await(await fetch(`/api/v1/users/${id}/profile`)).json();
        return p.data.profile.about_me==='Mobile QA profile';
      }));
      await act(page,'close');
      await act(page,'dialog:about');
      await page.waitForFunction(()=>document.querySelector('#f-value')?.value==='Mobile QA profile');
      assert.equal(await page.locator('#f-name').inputValue(),'they/them');
      await act(page,'close');
      checks.push('profile pronouns/about/banner save and hydrate from the real server');
      // Notification values must hydrate from the server and survive reopen.
      await act(page,'dialog:notifications');
      await page.locator('#f-name').selectOption('nothing');
      await page.locator('#f-muted').check();
      await page.locator('[type=submit]').click();
      await page.waitForFunction(()=>!document.querySelector('.live-modal'));
      await act(page,'dialog:notifications');
      await page.waitForFunction(()=>document.querySelector('#f-name')?.value==='nothing');
      assert(await page.locator('#f-muted').isChecked());
      await act(page,'close');
      checks.push('notification form roundtrip/hydration');

      // Server settings perform real typed WS mutations, not local-only edits.
      await form(page,'server-overview',{'f-name':'QA mobile server'});
      await page.waitForSelector('.sidebar-header:has-text("QA mobile server")');
      await act(page,'close');
      await form(page,'channel',{'f-name':'rename-me'});
      await page.waitForFunction(()=>!document.querySelector('.live-modal'));
      const renameId=await page.locator('#live-messages').getAttribute('data-channel');
      await act(page,`dialog:channel-edit:${renameId}`);
      await page.locator('#f-name').fill('release-notes');
      await page.locator('#f-value').fill('Release checklist and QA findings');
      await page.locator('[type=submit]').click();
      await page.waitForSelector('.channel-header:has-text("release-notes")');
      await act(page,`dialog:channel-delete:${renameId}`);
      await page.locator('#f-name').fill('DELETE');
      await page.locator('[type=submit]').click();
      await page.waitForFunction(id=>!document.querySelector(`[data-live="channel:${id}"]`),renameId);
      await act(page,`channel:${channel}`);
      checks.push('server rename, channel rename/topic/delete');

      // Actual picker buttons and phone-sized tap targets.
      await act(page,'dialog:emoji');
      await page.locator('#emoji-query').fill('smile');
      await page.waitForSelector('.emoji-grid button');
      screenshots.push(await shot(page,'emoji-picker'));
      assert(await page.locator('.emoji-grid button').first().evaluate(e=>e.getBoundingClientRect().height>=44));
      await page.locator('.emoji-grid button').first().tap();
      assert((await page.locator('#live-input').inputValue()).length>0);
      await page.locator('#live-input').fill('');
      await act(page,'dialog:gif');
      screenshots.push(await shot(page,'gif-picker'));
      await act(page,'close');
      checks.push('phone emoji insertion and GIF picker layout (provider disabled)');

      // Offline does not promise persisted private history. The shell is cached.
      await page.evaluate(async()=>{await navigator.serviceWorker.ready;});
      await page.context().setOffline(true);
      await page.waitForSelector('#offline-banner');
      screenshots.push(await shot(page,'offline'));
      await page.context().setOffline(false);
      await page.waitForFunction(()=>!document.querySelector('#offline-banner'));
      await page.waitForFunction(()=>!document.querySelector('.connection-banner'),null,{timeout:45000});
      checks.push('offline banner and live reconnect');

      // Fill an empty history with synthetic DB fixtures. All reads/seeks are real HTTP.
      await form(page,'channel',{'f-name':'long-history'});
      await page.waitForFunction(()=>!document.querySelector('.live-modal'));
      const historyChannel=await page.locator('#live-messages').getAttribute('data-channel');
      const files=await readdir(h.data), dbfile=files.find(f=>f.endsWith('.sqlite3')||f.endsWith('.sqlite')||f.endsWith('.db'));
      assert(dbfile,'temporary server SQLite file');
      const db=new DatabaseSync(join(h.data,dbfile));
      db.exec('PRAGMA busy_timeout=10000; BEGIN IMMEDIATE');
      const insert=db.prepare('INSERT INTO messages(id,channel_id,author_id,content,created_at) VALUES(?,?,?,?,?)');
      const user=await page.locator('[data-self-id]').getAttribute('data-self-id');
      const now=Date.now(), base=(BigInt(now)-1735689600000n-100000n)<<22n;
      for(let i=0;i<10000;i++)insert.run(base+BigInt(i)*4096n,BigInt(historyChannel),BigInt(user),`History message ${i}: bounded mobile scrolling and pagination.`,now-10000000+i*1000);
      db.exec('COMMIT'); db.close();
      await act(page,`channel:${historyChannel}`);
      await page.waitForFunction(()=>document.querySelector('#live-messages')?.dataset.total==='10000');
      await act(page,'seek:0');
      await page.waitForFunction(()=>document.querySelector('#live-messages')?.dataset.windowStart==='0');
      await act(page,'history-newer');
      await page.waitForFunction(()=>{const rank=Number(document.querySelector('#live-messages')?.dataset.windowStart);return rank>0&&rank<50;});
      await page.waitForFunction(()=>[...document.querySelectorAll('.markdown')].some(e=>e.textContent.includes('History message 59:')));
      for(let i=0;i<3;i++) {
        const rank=Number(await page.locator('#live-messages').getAttribute('data-window-start'));
        if(rank===0)break;
        await act(page,'history-older');
        await page.waitForFunction(rank=>Number(document.querySelector('#live-messages')?.dataset.windowStart)<rank,rank);
      }
      await page.waitForFunction(()=>document.querySelector('#live-messages')?.dataset.windowStart==='0');
      checks.push('adjacent keyset windows reach rows between sparse extent anchors');
      const frames=await page.evaluate(()=>new Promise(done=>{
        const samples=[]; let start,last,maxRows=0;
        function frame(at){
          start??=at; if(last)samples.push(at-last);last=at;
          const list=document.querySelector('#live-messages');
          maxRows=Math.max(maxRows,list.querySelectorAll('[data-live-message]').length);
          list.scrollTop+=20;
          if(at-start<3000)requestAnimationFrame(frame);
          else {samples.sort((a,b)=>a-b);done({duration:at-start,frames:samples.length,fps:samples.length*1000/(at-start),p95:samples[Math.floor(samples.length*.95)],slowFrames:samples.filter(v=>v>25).length,maxRows});}
        }requestAnimationFrame(frame);
      }));
      assert(frames.maxRows<=50);
      await act(page,'seek:9950');
      await page.waitForFunction(()=>document.querySelector('#live-messages')?.dataset.windowEnd==='10000');
      screenshots.push(await shot(page,'history-10000'));
      await writeFile(`${output}/scroll-performance.json`,JSON.stringify({fixture:'10000 synthetic DB rows; real server reads',viewport:{width,height},...frames},null,2));
      checks.push('10000-row real history extent/seek, bounded 50 DOM rows, rAF scroll measurement');
      await act(page,`channel:${channel}`);
    }
    if(width===phoneViewports[0][0]) {
      const other=await h.phone();
      await other.goto(h.origin);
      await other.locator('#auth-username').fill(`qa_phone_${width}`);
      await other.locator('#auth-password').fill('synthetic-qa-password-1234');
      await other.locator('[type=submit]').click();await other.waitForSelector('.live-home');
      await act(page,'dialog:devices');
      await page.waitForFunction(()=>document.querySelectorAll('.device-row').length===2);
      screenshots.push(await shot(page,'sessions-multiple'));
      await page.locator('.device-row').filter({hasNotText:'Current Device'}).getByRole('button',{name:'Log Out',exact:true}).click();
      await other.waitForSelector('#auth-username');
      await other.locator('#auth-username').fill(`qa_phone_${width}`);
      await other.locator('#auth-password').fill('synthetic-qa-password-1234');
      await other.locator('[type=submit]').click();await other.waitForSelector('.live-home');
      await act(page,'dialog:devices');
      await page.getByRole('button',{name:'Log Out All Other Devices',exact:true}).click();
      await other.waitForSelector('#auth-username');await other.context().close();
      checks.push('session-specific and all-other-device logout invalidate a second live browser');
    }
    let password='synthetic-qa-password-1234';
    if(width===phoneViewports[0][0]) {
      await act(page,'dialog:password');
      await page.locator('#f-name').fill(password);
      password='updated-synthetic-qa-password-5678';
      await page.locator('#f-value').fill(password);
      await Promise.all([page.waitForEvent('load'),page.locator('[type=submit]').click()]);
      await page.waitForSelector('.live-shell');
      checks.push('password change with current-password proof and cookie-session reload');
    }
    await act(page,'dialog:settings');
    await page.getByRole('button',{name:'Log Out',exact:true}).click();
    await page.waitForSelector('#auth-username');
    await wait(200);assert.equal(await page.locator('.live-error').count(),0,'logout has no late media error');
    await login(page,`qa_phone_${width}`,password);
    const deletingUser=await page.locator('[data-self-id]').getAttribute('data-self-id');
    await act(page,'dialog:delete-account');
    await page.locator('#f-name').fill(password);
    await page.locator('#f-value').fill('DELETE');
    const started=Date.now();
    const [deletion]=await Promise.all([
      page.waitForResponse(r=>new URL(r.url()).pathname==='/api/v1/auth/account/delete'),
      page.locator('[type=submit]').click(),
    ]);
    assert.equal(deletion.status(),204,`Account deletion must commit on the first attempt: ${await deletion.text()}`);
    await page.waitForSelector('#auth-username');
    await page.getByRole('status').filter({hasText:'Your account was deleted.'}).waitFor();
    screenshots.push(await shot(page,'account-deleted-login'));
    assert(Date.now()-started<15000,'erasure must finish within the API deadline');
    assert(!(await page.context().cookies()).some(c=>c.name==='__Host-sticktalk_session'));
    assert.equal(await page.locator('.connection-banner,.live-modal').count(),0);
    assert(!h.log().includes('housekeeping failed'));
    const database=(await readdir(h.data)).find(f=>/\.(sqlite3|sqlite|db)$/.test(f));
    const erased=new DatabaseSync(join(h.data,database));
    try {
      erased.exec('PRAGMA busy_timeout=10000');
      assert.equal(erased.prepare('SELECT COUNT(*) AS n FROM deleted_accounts WHERE user_id=?').get(BigInt(deletingUser)).n,1);
      assert.equal(erased.prepare("SELECT COUNT(*) AS n FROM messages WHERE author_id=? AND (content<>'' OR deleted_at IS NULL)").get(BigInt(deletingUser)).n,0);
    }finally{erased.close();}
    checks.push(`${width}: logout, password login, confirmed account erasure`);
    csp.push(...await page.evaluate(()=>window.__qaCsp??[]));
    await page.context().close();
    await h.close();h=undefined;
  }
  assert.deepEqual(errors,[]); assert.deepEqual(csp,[]);
  await writeFile(`${output}/phone-results.json`,JSON.stringify({checks,screenshots,layout,errors,csp,
    revision:JSON.parse(await readFile(join(target,'web-dist/sizes.json'),'utf8')).revision},null,2));
  console.log(JSON.stringify({checks,screenshots:screenshots.length,errors,csp}));
} catch(e) {
  await writeFile(`${output}/phone-failure.json`,JSON.stringify({error:String(e),checks,errors,log:h?.log(),body:await page?.locator('body').innerText().catch(()=>'' )},null,2));
  if(page)await page.screenshot({path:`${output}/phone-failure.png`}).catch(()=>{});
  throw e;
} finally { await h?.close(); }

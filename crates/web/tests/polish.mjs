import {phoneViewports} from './engines.mjs';
import {swipe} from './gestures.mjs';
// Phone polish contracts against real CSR/server state; no fixture API routes.
import assert from 'node:assert/strict';
import {writeFile,rm} from 'node:fs/promises';
import {harness,register,form,act,shot,wait,until,output} from './qa-harness.mjs';
const checks=[],screenshots=[],errors=[];
let h,page;
const revisions=[];
await rm(`${output}/polish-results.json`,{force:true});
try {
  for(const [width,height] of (process.env.STICKTALK_QA_ENGINE ? phoneViewports : [[390,844],[412,915],[1440,900]])) {
    h=await harness();revisions.push(h.revision);page=await h.phone(width,height);
    page.on('pageerror',e=>errors.push(String(e)));
    await page.goto(h.origin);await page.evaluate(()=>document.fonts.ready);
    assert.equal(await page.locator('h1').innerText(),'Welcome back!');
    assert(await page.locator('#auth-username').evaluate(e=>parseFloat(getComputedStyle(e).fontSize)>=16));
    assert(await page.locator('[type=submit]').evaluate(e=>e.getBoundingClientRect().height>=48));
    await page.waitForFunction(()=>{const e=document.querySelector('[type=submit]');return Math.abs(e.getBoundingClientRect().width-e.form.querySelector('input').getBoundingClientRect().width)<4;});
    screenshots.push(await shot(page,'login'));
    await register(page,h.origin,`polish_${width}`);
    await act(page,'dialog:notifications');await page.waitForSelector('.settings-page');
    assert.equal(await page.locator('[data-live-form=notifications]').isVisible(),false);
    assert.equal(await page.locator('[data-push-enable]:disabled').count(),0);
    screenshots.push(await shot(page,'notifications-empty'));
    await act(page,'dialog:settings');await page.waitForSelector('.settings-identity');
    assert.equal(await page.locator('.settings-nav [data-live="dialog:delete-account"]').count(),0);
    assert(await page.locator('.settings-identity > .avatar').evaluate(e=>e.getBoundingClientRect().width>=64));
    await page.evaluate(()=>document.documentElement.style.setProperty('--safe-bottom','34px'));
    await page.locator('.settings-nav [data-live=logout]').scrollIntoViewIfNeeded();
    const logout=await page.locator('.settings-nav [data-live=logout]').boundingBox();
    assert(logout.y+logout.height<=height-34,'Log Out clears safe-area inset');
    screenshots.push(await shot(page,'settings-bottom'));
    await page.evaluate(()=>document.documentElement.style.removeProperty('--safe-bottom'));
    await act(page,'dialog:account');await page.waitForSelector('.danger-zone');
    await page.locator('.danger-zone').scrollIntoViewIfNeeded();
    screenshots.push(await shot(page,'account-danger-zone'));
    await act(page,'dialog:profile-edit');await page.waitForSelector('[data-profile-editor]');
    await until(()=>page.evaluate(()=>document.querySelector('.profile-preview .profile-banner')?.style.background!==''));
    assert.equal(await page.locator('.profile-save-bar').isVisible(),false);
    assert.notEqual(await page.locator('.profile-banner').evaluate(e=>getComputedStyle(e).backgroundColor),'rgb(0, 0, 0)');
    screenshots.push(await shot(page,'profile-empty'));
    await form(page,'about',{'f-name':'they/them','f-value':'Building things with good people.','f-extra':'#496c63'});
    await act(page,'dialog:profile-edit');await page.waitForSelector('[data-profile-editor]');
    await page.waitForFunction(()=>document.querySelector('.profile-preview-copy').textContent.includes('Building things with good people.'));
    assert(await page.evaluate(async()=>{
      const id=document.querySelector('[data-self-id]').dataset.selfId;
      const p=await(await fetch(`/api/v1/users/${id}/profile`)).json();return p.data.profile.pronouns==='they/them'&&p.data.profile.about_me==='Building things with good people.';
    }),'first lazy save captures the original bio/pronouns even after immediate navigation');

    screenshots.push(await shot(page,'profile-edit'));
    screenshots.push(await shot(page,'profile-populated'));
    await page.locator('#f-name').fill('Alex Morgan — Design & Engineering');
    await page.locator('#f-value').fill('Reviewing the phone experience');
    await page.waitForSelector('.profile-save-bar:not([hidden])');
    assert.equal(await page.locator('[data-preview-name]').innerText(),'Alex Morgan — Design & Engineering');
    screenshots.push(await shot(page,'profile-dirty'));
    await page.getByRole('button',{name:'Reset',exact:true}).click();
    await page.waitForSelector('.profile-save-bar[hidden]',{state:'attached'});
    await page.locator('#f-name').fill('Alex Morgan');
    await page.getByRole('button',{name:'Save changes',exact:true}).click();
    await until(()=>page.evaluate(async()=>{
      const id=document.querySelector('[data-self-id]').dataset.selfId;
      const p=await(await fetch(`/api/v1/users/${id}/profile`)).json();return p.data.profile.user.display_name==='Alex Morgan';
    }));
    await page.waitForSelector('.profile-save-bar[hidden]',{state:'attached'});
    await act(page,'close');await act(page,'dialog:profile-edit');
    await page.waitForFunction(()=>document.querySelector('#f-name')?.value==='Alex Morgan');
    assert.equal(await page.locator('.profile-save-bar').isVisible(),false);
    await act(page,'close');
    checks.push(`${width}: 16px inputs, 48px primary button, safe logout, danger zone, live profile preview/reset/save/hydration`);
    await form(page,'create',{'f-name':'After Hours'});await page.waitForSelector('#live-input');
    assert.equal(await page.locator('#app-update').count(),0,'first worker activation is not an app update');
    await act(page,'dialog:roles');await page.waitForSelector('[data-live-form=role-create]');
    await page.locator('#f-name').fill('Design');await page.locator('[type=submit]').click();
    await page.waitForSelector('.role-editor-link:has-text("Design")');
    await page.locator('.role-editor-link:has-text("Design")').click();
    await page.waitForSelector('[data-live-form=role-save]');
    assert.equal(await page.locator('.permission-group').count(),5);
    assert.equal(await page.locator('[data-role-bit]').count(),27);
    screenshots.push(await shot(page,'role-editor'));
    await act(page,'dialog:roles');await page.waitForSelector('.role-color');
    screenshots.push(await shot(page,'roles'));
    await page.getByRole('button',{name:'Reorder',exact:true}).click();
    screenshots.push(await shot(page,'roles-reorder-mode'));
    await act(page,'close');
    await page.locator('#live-input').fill('A message to test touch actions.');
    await page.getByRole('button',{name:'Send Message',exact:true}).click();await page.waitForSelector('[data-live-message]');
    await page.locator('[data-live-message]').last().focus();await page.keyboard.press('Shift+F10');
    await page.waitForSelector('#live-message-menu');
    assert.equal(await page.locator('.quick-reactions button').count(),6);
    assert.equal(await page.locator('#live-message-menu button:has-text("Cancel")').count(),0);
    assert(await page.locator('.message-menu-action').first().evaluate(e=>e.getBoundingClientRect().height>=48));
    screenshots.push(await shot(page,'message-actions'));
    await page.locator('.quick-reactions button').nth(2).click();
    await page.waitForSelector('.reaction');
    assert.equal(await page.locator('#live-message-menu').count(),0);
    if(width<768) {
      await page.locator('[data-live-message]').last().focus();await page.keyboard.press('Shift+F10');
      await page.waitForSelector('.sheet-handle');
      assert.equal(await page.locator('.quick-reactions button').first().textContent(),'😂','recent reaction moves to the front');
      const handle=await page.locator('.sheet-handle').boundingBox();
      const x=handle.x+handle.width/2,y=handle.y+handle.height/2;
      await swipe(page,{x,y},{x,y:y+100});
      await page.waitForSelector('#live-message-menu',{state:'detached'});
    }

    await act(page,'dialog:notifications');await page.waitForSelector('[data-live-form=notifications]');
    assert(!await page.locator('.settings-content').innerText().then(s=>s.includes('not available')));
    screenshots.push(await shot(page,'notifications'));
    checks.push(`${width}: grouped role permissions, reorder mode, six quick reactions persist to server, no dead notification controls`);
    for(const screen of ['settings','account','about','appearance','privacy','devices','media-settings','password','server','server-overview','server-channels','member-role','invite','server-bans','create','join','friend','group','channel','pins','gif','emoji']) {
      await act(page,`dialog:${screen}`);await page.waitForSelector(`.live-modal[data-dialog="${screen}"]`);
      if(screen==='devices')await page.waitForSelector('.device-row');
      if(screen==='media-settings')await page.waitForSelector('#live-device-settings select, #live-device-settings [data-media-unavailable]');
      screenshots.push(await shot(page,screen));
      assert.deepEqual(await page.evaluate(()=>[...document.querySelectorAll('.live-modal,.settings-main,.settings-content,.settings-nav')].filter(e=>e.getClientRects().length&&e.scrollWidth>e.clientWidth+2).map(e=>e.className)),[],`${width}: ${screen} horizontal overflow`);
    }

    await h.close();h=undefined;
  }
  assert.deepEqual(errors,[]);
  await writeFile(`${output}/polish-results.json`,JSON.stringify({revision:revisions[0],revisions,checks,screenshots,errors},null,2));
  await rm(`${output}/polish-failure.json`,{force:true});
  console.log(JSON.stringify({checks,errors}));
} catch(e) {
  await writeFile(`${output}/polish-failure.json`,JSON.stringify({error:String(e),checks,errors,body:await page?.locator('body').innerText().catch(()=>'' )},null,2));throw e;
} finally {await h?.close();}

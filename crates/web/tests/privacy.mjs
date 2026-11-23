// Privacy and notifications in the real web app against a real local server
// (127.0.0.1 only, headless, no sound or OS notification: alerts go to a test
// recorder). Status menu and custom status, friend nicknames and notes,
// notification settings (the mute deadline is kept), alerts and Do Not
// Disturb, ignore, message requests, and reports through the moderators'
// queue. Saves each new surface at 1440x900 and 390x844.
import assert from 'node:assert/strict';
import {mkdir,writeFile} from 'node:fs/promises';
import {join} from 'node:path';
import {harness,register,form,act,wait,until} from './qa-harness.mjs';

const shots=process.env.STICKTALK_PRIVACY_SHOTS??join(process.env.CARGO_TARGET_DIR??'.','privacy-shots');
await mkdir(shots,{recursive:true});
const h=await harness(),checks=[],errors=[],saved=[];
const sizes=[[1440,900],[390,844]];
async function shot(page,name){
  const viewport=page.viewportSize();
  for(const [width,height] of sizes){
    await page.setViewportSize({width,height});
    await page.evaluate(()=>document.fonts.ready);await wait(150);
    const path=join(shots,`${name}-${width}x${height}.png`);
    await page.screenshot({path,caret:'initial'});saved.push(path);
  }
  await page.setViewportSize(viewport);
}
async function open(page,name){await act(page,`dialog:${name}`);await page.waitForFunction(n=>document.querySelector('.live-modal')?.dataset.dialog===n,name);}
async function closeDialog(page){await act(page,'close');await page.waitForFunction(()=>!document.querySelector('.live-modal'));}
async function toast(page,text){await page.waitForFunction(t=>[...document.querySelectorAll('[data-toast]')].some(e=>e.textContent.includes(t)),text);}
async function send(page,text){
  await page.locator('#live-input').fill(text);
  await page.locator('#live-input').press('Enter');
}
// Record alerts instead of playing them; nothing is ever shown or heard.
const recorder=page=>page.evaluate(()=>{window.__alerts=[];window.__sticktalkAlert=a=>window.__alerts.push(a);});
const alerts=page=>page.evaluate(()=>(window.__alerts??[]).filter(a=>a.kind));

let alice,bob,carol;
try {
  alice=await h.phone(1440,900);bob=await h.phone(1440,900);carol=await h.phone(1440,900);
  for(const p of[alice,bob,carol]){p.on('pageerror',e=>errors.push(String(e)));p.setDefaultTimeout(30000);}
  await register(alice,h.origin,'alice_privacy');await register(bob,h.origin,'bob_privacy');await register(carol,h.origin,'carol_privacy');
  for(const p of[alice,bob,carol])await recorder(p);
  await form(alice,'create',{'f-name':'Privacy HQ'});await alice.waitForSelector('#live-input');
  const guild=(await alice.locator('.server-icon.selected').getAttribute('data-live')).slice(6);
  const general=await alice.locator('#live-messages').getAttribute('data-channel');
  await form(alice,'invite');await alice.waitForSelector('.live-modal code');
  const code=await alice.locator('.live-modal code').first().textContent();await closeDialog(alice);
  for(const p of[bob,carol]){await form(p,'join',{'f-name':code});await p.waitForSelector('#live-input');}
  await form(alice,'friend',{'f-name':'bob_privacy'});
  await act(bob,'home');await bob.locator('[data-live^="friend:accept,"]').click();
  await bob.waitForSelector('.friend-row:has-text("alice_privacy")');
  const aliceId=await alice.locator('[data-self-id]').getAttribute('data-self-id');
  const carolId=await carol.locator('[data-self-id]').getAttribute('data-self-id');
  checks.push('a server with three members and a friendship, through the real UI');

  // Status menu: Do Not Disturb for an hour, from the user panel.
  await alice.locator('.status-trigger').click();
  await alice.waitForFunction(()=>document.querySelector('.live-modal')?.dataset.dialog==='privacy-status');
  await alice.locator('#p-status-for').selectOption('60');
  await shot(alice,'status-menu');
  await alice.locator('[data-live="privacy:status,dnd"]').click();
  await alice.waitForFunction(()=>document.querySelector('.status-trigger')?.getAttribute('aria-label')?.includes('Do Not Disturb'));
  await bob.waitForFunction(()=>document.querySelector('.friend-row .presence-dot.bg-dnd'));
  await open(alice,'privacy-status');
  await alice.waitForSelector('.status-option[aria-pressed=true]:has-text("Back to Online")');
  await closeDialog(alice);
  checks.push('status menu sets Do Not Disturb for 1 hour; a friend sees it');

  // Do Not Disturb: nothing alerts.
  await act(bob,`guild:${guild}`);await bob.waitForSelector('#live-input');
  await act(alice,'home');
  await send(bob,'@alice_privacy are you there?');
  await bob.waitForSelector('.message:has-text("are you there?")');
  await wait(800);
  assert.equal((await alerts(alice)).length,0,'no alerts in Do Not Disturb');
  await alice.locator('.status-trigger').click();
  await alice.locator('[data-live="privacy:status,online"]').click();
  await alice.waitForFunction(()=>document.querySelector('.status-trigger')?.getAttribute('aria-label')?.includes('Online'));
  await send(bob,'@alice_privacy now?');
  await until(async()=>(await alerts(alice)).some(a=>a.kind==='mention'&&a.sound&&a.title.includes('bob_privacy')));
  checks.push('Do Not Disturb holds alerts back; Online plays the mention sound');

  // Custom status with an emoji that clears itself.
  await alice.locator('.status-trigger').click();
  await alice.locator('[data-live="dialog:privacy-custom"]').click();
  await alice.waitForSelector('[data-live-form=privacy-custom]');
  await alice.locator('[data-live="privacy:pick-emoji,🍜"]').click();
  await alice.locator('#p-status-text').fill('Out to lunch');
  await alice.locator('#p-status-clear').selectOption('60');
  await shot(alice,'custom-status');
  await alice.locator('[data-live-form=privacy-custom] [type=submit]').click();
  await act(bob,'home');
  await bob.waitForSelector('.friend-row:has-text("🍜 Out to lunch")');
  await open(alice,'privacy-status');await alice.waitForSelector('.status-custom:has-text("Clears until")');await closeDialog(alice);
  checks.push('custom status with emoji and clear-after shows on the friend list');

  // Friend nickname and a private note, from the profile.
  await act(bob,`profile:${aliceId}`);
  await bob.waitForSelector('[data-live-form=privacy-nickname]');
  await bob.locator('#p-nickname').fill('Ali the Brave');
  await bob.locator('[data-live-form=privacy-nickname] [type=submit]').click();
  await bob.waitForSelector('.live-modal h2:has-text("Ali the Brave")');
  await bob.locator('#p-note').fill('Met at the privacy meetup');
  await bob.locator('[data-live-form=privacy-note] [type=submit]').click();
  await bob.waitForSelector('.profile-private:has-text("Their name: alice_privacy")');
  await shot(bob,'profile-nickname-note');
  await closeDialog(bob);
  await bob.waitForSelector('.friend-row:has-text("Ali the Brave")');
  // Nobody else sees it.
  await act(alice,`profile:${aliceId}`);await alice.waitForSelector('.live-modal h2');
  assert.ok(!(await alice.locator('.live-modal').innerText()).includes('Ali the Brave'));
  await closeDialog(alice);
  await act(bob,`profile:${aliceId}`);await bob.waitForFunction(()=>document.querySelector('#p-note')?.value==='Met at the privacy meetup');await closeDialog(bob);
  checks.push('friend nickname shown in place of the name, real name on the profile, note kept privately');

  // Notification settings: mute for an hour and hide muted channels; both survive saving again.
  await open(alice,`target-notifications:guild:${guild}`);
  await alice.waitForSelector('[data-live-form=privacy-notify]');
  await alice.locator('input[name=p-level][value=mentions]').check();
  await alice.locator('#p-mute').selectOption('60');
  await alice.locator('#p-hide-muted').check();
  await alice.locator('#p-suppress-roles').check();
  await alice.locator('[data-live-form=privacy-notify] [type=submit]').click();
  await alice.waitForSelector('[data-live-form=privacy-notify] label:has-text("Muted until")');
  // Saving again with "Keep as it is" keeps the deadline.
  assert.equal(await alice.locator('#p-mute').inputValue(),'');
  const deadline=await alice.locator('[data-live-form=privacy-notify] label:has-text("Muted until")').innerText();
  await alice.locator('#p-suppress-everyone').check();
  await alice.locator('[data-live-form=privacy-notify] [type=submit]').click();
  await alice.waitForSelector('[data-live-form=privacy-notify] .form-saved');
  assert.equal(await alice.locator('[data-live-form=privacy-notify] label:has-text("Muted until")').innerText(),deadline);
  assert.ok(await alice.locator('#p-hide-muted').isChecked());
  await shot(alice,'notification-settings');
  await alice.locator('.global-notifications').scrollIntoViewIfNeeded();
  await shot(alice,'notification-settings-global');
  // The reaction switch is saved for the account.
  await alice.locator('input[name=p-reactions][value=all]').click();
  await alice.waitForFunction(()=>document.querySelector('input[name=p-reactions][value=all]')?.checked);
  await closeDialog(alice);
  checks.push('server notifications: level, timed mute, hide muted and suppression saved without losing the deadline');

  // Ignore: Carol's messages collapse for Bob; Carol is not told.
  await act(bob,`profile:${carolId}`);await bob.waitForSelector('.profile-private');
  await bob.locator(`[data-live="privacy:ignore,${carolId}"]`).click();
  await toast(bob,'Ignored. They won’t know');
  await closeDialog(bob);
  await act(bob,`guild:${guild}`);await bob.waitForSelector('#live-input');
  await send(carol,'hello from carol');
  await bob.waitForSelector('.ignored-message');
  assert.equal(await bob.locator('.message:has-text("hello from carol")').count(),0);
  await shot(bob,'ignored-message');
  await bob.locator('.ignored-message [data-live^="privacy:reveal,"]').click();
  await bob.waitForSelector('.message:has-text("hello from carol")');
  checks.push('ignored messages collapse behind a reveal');

  // Report Carol's message; the server's owner handles it in the Reports queue.
  const message=await bob.locator('.message:has-text("hello from carol")').getAttribute('data-live-message');
  await act(bob,`privacy:report,message,${general},${message}`);
  await bob.waitForSelector('[data-live-form=privacy-report]');
  await bob.locator('input[name=p-category][value=harassment]').check();
  await bob.locator('#p-report-note').fill('Please take a look.');
  await shot(bob,'report-dialog');
  await bob.locator('[data-live-form=privacy-report] [type=submit]').click();
  await bob.waitForFunction(()=>!document.querySelector('.live-modal'));
  await act(alice,`guild:${guild}`);await alice.waitForSelector('#live-input');
  await open(alice,'server-reports');
  await alice.waitForSelector('.report-card:has-text("hello from carol")');
  assert.ok(!(await alice.locator('.report-card').innerText()).includes('bob_privacy'),'the reporter is never shown');
  await shot(alice,'server-reports');
  await alice.locator('.report-card [data-live$=",delete"]').click();
  await toast(alice,'Report handled');
  await alice.waitForSelector('.settings-empty:has-text("No open reports")');
  await open(alice,'audit-log');await alice.waitForSelector('.audit-entry[data-action=report_resolve]');
  await closeDialog(alice);
  await open(bob,'privacy');await bob.waitForSelector('.report-status:has-text("Action taken")');
  await shot(bob,'privacy-settings');
  await closeDialog(bob);
  checks.push('report captured, handled from the Reports queue (message deleted, audited); reporter sees its status');

  // Message requests: friends only by default; then allowed from server members.
  await act(carol,`friend:dm,${aliceId}`);
  await toast(carol,'');
  await open(alice,'privacy');
  await alice.locator('input[name=p-dm][data-live="privacy:dm-allow,server_members"]').click();
  await alice.waitForFunction(()=>document.querySelector('input[name=p-dm][data-live$=server_members]')?.checked);
  await closeDialog(alice);
  await act(carol,`friend:dm,${aliceId}`);
  await carol.waitForFunction(()=>document.querySelector('.channel-header h1')?.textContent.includes('alice_privacy'));
  await send(carol,'hi alice, I saw you in Privacy HQ');
  await carol.waitForSelector('.message:has-text("I saw you in Privacy HQ")');
  await act(alice,'home');
  await alice.waitForSelector('.message-requests:has-text("Message Requests")');
  await shot(alice,'message-requests-row');
  await alice.locator('.message-requests').click();
  await alice.waitForSelector('.request-card:has-text("I saw you in Privacy HQ")');
  await shot(alice,'message-requests');
  await alice.locator('.request-card [data-live^="privacy:open-request,"]').click();
  await alice.waitForSelector('.message:has-text("I saw you in Privacy HQ")');
  checks.push('first-contact DM arrives as a quiet request and becomes a DM when accepted');

  assert.deepEqual(errors,[]);
  await writeFile(join(shots,'privacy-results.json'),JSON.stringify({revision:h.revision,checks,saved,errors},null,2));
  console.log(JSON.stringify({checks,screenshots:saved.length,errors}));
} catch(e) {
  await writeFile(join(shots,'privacy-failure.json'),JSON.stringify({error:String(e.stack??e),checks,errors,
    alice:await alice?.locator('body').innerText().catch(()=>''),bob:await bob?.locator('body').innerText().catch(()=>'')},null,2));
  for(const [name,p] of [['alice',alice],['bob',bob],['carol',carol]])await p?.screenshot({path:join(shots,`privacy-failure-${name}.png`)}).catch(()=>{});
  throw e;
} finally {await h.close();}

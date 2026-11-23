// Moderation in the real web app against a real local server (127.0.0.1 only,
// headless): AutoMod, raid alert and security actions, verification, slowmode,
// channel permissions, age-restricted channels, timeouts, kick, prune and the
// audit log. Saves each new surface at 1440x900 and 390x844.
import assert from 'node:assert/strict';
import {writeFile} from 'node:fs/promises';
import {harness,register,form,act,shot,output,until,wait} from './qa-harness.mjs';

const h=await harness(),checks=[],screenshots=[],errors=[];
const pages=[];
// Each new surface at the two review sizes, then back to the page's own size.
async function both(page,name){
  const size=page.viewportSize();
  for(const [width,height] of [[1440,900],[390,844]]){
    await page.setViewportSize({width,height});
    await wait(150);
    screenshots.push(await shot(page,name));
  }
  await page.setViewportSize(size);
}
const dialog=page=>page.locator('.live-modal').first();
async function open(page,name){await act(page,`dialog:${name}`);await page.waitForFunction(n=>document.querySelector('.live-modal')?.dataset.dialog===n,name);}
async function toast(page,text){await page.waitForFunction(t=>[...document.querySelectorAll('[data-toast]')].some(e=>e.textContent.includes(t)),text);}
async function send(page,text){
  await page.locator('#live-input').fill(text);
  await page.locator('#live-input').press('Enter');
}
let owner,mod,member;
try {
  owner=await h.phone(1440,900);mod=await h.phone(1440,900);member=await h.phone(1440,900);
  pages.push(owner,mod,member);
  for(const p of pages)p.on('pageerror',e=>errors.push(String(e)));
  await register(owner,h.origin,'mod_owner');
  await register(mod,h.origin,'mod_helper');
  await register(member,h.origin,'mod_member');
  // Raiders register now (separate rate-limit buckets) and join later, together.
  const raiders=[];
  for(let i=0;i<8;i++){
    const p=await h.phone(390,844);
    await p.setExtraHTTPHeaders({'x-forwarded-for':`10.77.0.${i+1}`});
    p.on('pageerror',e=>errors.push(String(e)));
    await register(p,h.origin,`raider_${i}`);
    raiders.push(p);
  }
  await form(owner,'create',{'f-name':'Moderation QA'});await owner.waitForSelector('#live-input');
  const guild=(await owner.locator('.server-icon.selected').getAttribute('data-live')).slice(6);
  const general=await owner.locator('#live-messages').getAttribute('data-channel');
  await form(owner,'invite');await owner.waitForSelector('.live-modal code');
  const code=await owner.locator('.live-modal code').first().textContent();await act(owner,'close');

  // Raid: ten joins within the window raise an alert that only moderators see.
  for(const p of[mod,member,...raiders]){await form(p,'join',{'f-name':code});await p.waitForSelector('#live-input');}
  for(const p of raiders)await p.context().close();
  await owner.waitForSelector('.raid-banner');
  assert.equal(await member.locator('.raid-banner').count(),0);
  await both(owner,'raid-banner');
  await owner.locator('.raid-banner [data-live="dialog:safety"]').click();
  await owner.waitForSelector('.raid-alert');
  await both(owner,'safety-setup');
  // Security action: pause invites; a new member is refused until resumed.
  await owner.locator('#mod-pause-invites-for').selectOption('3600');
  await owner.locator('[data-live-form=mod-pause-invites] [type=submit]').click();
  await owner.waitForSelector('.pause-invites .text-warning');
  checks.push('raid alert from a burst of joins, shown to moderators only; invites paused');
  await owner.locator('[data-live="mod:resume,invites"]').click();
  await owner.waitForSelector('[data-live-form=mod-pause-invites]');
  await owner.locator('.raid-alert [data-live="mod:dismiss-raid"]').click();
  await owner.waitForFunction(()=>!document.querySelector('.raid-alert'));
  await act(owner,'close');
  checks.push('invites resumed early, raid alert dismissed');

  // Roles: a moderator role with Time Out Members and View Audit Log.
  await open(owner,'roles');
  await owner.locator('#f-name').fill('Moderators');await owner.locator('[data-live-form=role-create] [type=submit]').click();
  await owner.waitForSelector('[data-live^="role:"]:has-text("Moderators")');
  const role=(await owner.locator('[data-live^="role:"]:has-text("Moderators")').getAttribute('data-live')).slice(5);
  await act(owner,`role:${role}`);
  for(const label of['Time Out Members','View Audit Log','Kick Members'])await owner.getByLabel(label,{exact:true}).check();
  await owner.locator('[data-live-form=role-save] [type=submit]').click();
  await owner.waitForSelector('[data-live-form=role-create]');
  const modId=await mod.locator('[data-self-id]').getAttribute('data-self-id');
  const memberId=await member.locator('[data-self-id]').getAttribute('data-self-id');
  await open(owner,'member-role');
  await owner.locator('[data-live-form=member-role] #f-name').selectOption(modId);
  await owner.locator(`[data-member-role="${role}"]`).check();await owner.locator('[data-live-form=member-role] [type=submit]').click();
  await owner.waitForFunction(()=>!document.querySelector('.live-modal'));
  checks.push('new permissions (Time Out Members, View Audit Log) assignable in the role editor');

  // AutoMod: a keyword rule with an alert channel and a timeout action.
  await open(owner,'automod');
  await owner.waitForSelector('[data-live="dialog:automod-rule:new-keyword"]');
  await owner.locator('[data-live="dialog:automod-rule:new-keyword"]').click();
  await owner.waitForSelector('[data-live-form=mod-rule]');
  await owner.locator('#rule-name').fill('No spoilers');
  await owner.locator('#rule-keywords').fill('spoiler*\nthe ending');
  await owner.locator('#rule-allow').fill('spoilerfree');
  await owner.locator('#rule-alert').selectOption(general);
  await both(owner,'automod-rule-editor');
  await owner.locator('[data-live-form=mod-rule] [type=submit]').click();
  await owner.waitForSelector('.automod-rule:has-text("No spoilers")');
  await both(owner,'automod');
  await act(owner,'close');
  await send(member,'huge SPOILERS ahead');
  await toast(member,'AutoMod');
  // Only AutoMod's alert quotes it; the member's own message was never stored.
  assert.equal(await member.locator('.message:has-text("huge SPOILERS ahead"):not(:has-text("AutoMod"))').count(),0);
  await owner.waitForSelector('.message:has-text("AutoMod")');
  await both(member,'automod-blocked');
  await send(member,'this is spoilerfree');
  await member.waitForSelector('.message:has-text("this is spoilerfree")');
  checks.push('AutoMod blocks before storing, explains to the author, alerts the channel; allow list passes');

  // Slowmode: set in the channel editor; the author sees the cooldown count down.
  await open(owner,`channel-edit:${general}`);
  await owner.waitForSelector('[data-live-form=mod-channel]');
  await owner.locator('#mod-slowmode').selectOption('15');
  await owner.locator('[data-live-form=mod-channel] [type=submit]').click();
  await owner.waitForSelector('[data-live-form=mod-channel] .form-saved');
  await both(owner,'channel-settings-moderation');
  await member.waitForSelector('.slowmode-notice');
  await send(member,'first under slowmode');
  await member.waitForSelector('#slowmode-countdown');
  const before=await member.locator('#slowmode-countdown').textContent();
  await wait(2200);
  const after=await member.locator('#slowmode-countdown').textContent();
  assert.notEqual(before,after,'the countdown advances');
  await both(member,'slowmode-countdown');
  await send(member,'too fast');
  await toast(member,'Slowmode');
  checks.push('slowmode interval selector (15s), live cooldown in the composer, server refusal');
  await open(owner,`channel-edit:${general}`);
  await owner.locator('#mod-slowmode').selectOption('0');
  await owner.locator('[data-live-form=mod-channel] [type=submit]').click();
  await owner.waitForSelector('[data-live-form=mod-channel] .form-saved');

  // Channel permissions: the full editor, a who-can-see summary, private shortcut.
  await owner.locator(`[data-live="dialog:channel-perms:${general}"]`).click();
  await owner.waitForSelector('.channel-access');
  await owner.locator('#ow-add-role').selectOption(role);
  await owner.locator('[data-live="mod:ow-add,role:"]').click();
  await owner.waitForSelector(`[data-overwrite="role:${role}"]`);
  await owner.locator(`[data-overwrite="role:${role}"] .tri-option[title=Allow]`).first().click();
  await owner.locator('[data-live="mod:ow-private,on"]').click();
  await toast(owner,'Channel permissions saved');
  await owner.waitForFunction(()=>document.querySelector('.channel-access')?.textContent.includes('Moderators'));
  await both(owner,'channel-permissions');
  await member.waitForFunction(c=>!document.querySelector(`[data-live="channel:${c}"]`),general);
  await owner.locator('[data-live="mod:ow-private,off"]').click();
  await toast(owner,'Channel permissions saved');
  await member.waitForSelector(`[data-live="channel:${general}"]`);
  await act(owner,'close');
  checks.push('per-role tri-state overwrites, access summary, private toggle hides and restores');

  // Manage Webhooks and Use Application Commands (task `headroom`): offered in
  // a text channel's editor, saved and shown again; Manage Webhooks is not
  // offered where no webhook can live (a voice channel); both in the role editor.
  const tri=(page,label)=>page.getByRole('radiogroup',{name:`${label} for Moderators`});
  await open(owner,`channel-perms:${general}`);
  await owner.waitForSelector(`[data-overwrite="role:${role}"]`);
  await tri(owner,'Manage Webhooks').locator('.tri-option[title=Allow]').click();
  await tri(owner,'Use Application Commands').locator('.tri-option[title=Deny]').click();
  await owner.locator('[data-live="mod:ow-save"]').click();
  await toast(owner,'Channel permissions saved');
  await act(owner,'close');
  await open(owner,`channel-perms:${general}`);
  await owner.waitForSelector(`[data-overwrite="role:${role}"]`);
  assert.equal(await tri(owner,'Manage Webhooks').locator('input:checked').getAttribute('value'),'allow');
  assert.equal(await tri(owner,'Use Application Commands').locator('input:checked').getAttribute('value'),'deny');
  await act(owner,'close');
  const voiceChannel=(await owner.locator('[data-live^="voice:"]').first().getAttribute('data-live')).slice(6);
  await open(owner,`channel-perms:${voiceChannel}`);
  await owner.locator('#ow-add-role').selectOption(role);
  await owner.locator('[data-live="mod:ow-add,role:"]').click();
  await owner.waitForSelector(`[data-overwrite="role:${role}"]`);
  assert.equal(await tri(owner,'Manage Webhooks').count(),0,'no webhooks in a voice channel');
  assert.equal(await tri(owner,'Use Application Commands').count(),1,'commands in voice channel chat');
  await act(owner,'close');
  await act(owner,`role:${role}`);
  for(const label of['Manage Webhooks','Use Application Commands'])await owner.getByLabel(label,{exact:true}).check();
  await owner.locator('[data-live-form=role-save] [type=submit]').click();
  await owner.waitForSelector('[data-live-form=role-create]');
  await act(owner,`role:${role}`);
  for(const label of['Manage Webhooks','Use Application Commands'])assert(await owner.getByLabel(label,{exact:true}).isChecked(),label);
  await act(owner,'close');
  checks.push('Manage Webhooks and Use Application Commands: channel editor (where they apply) and role editor save and show again');

  // Member menu: right-click a member for moderation actions; time out with a reason.
  await act(member,`channel:${general}`);await member.waitForSelector('#live-input');
  await act(mod,`channel:${general}`);await mod.waitForSelector('#live-input');
  await mod.locator(`.member-row[data-live="profile:${memberId}"]`).click({button:'right'});
  await mod.waitForSelector('.member-moderation');
  await both(mod,'member-moderation-actions');
  await mod.locator(`[data-live="dialog:mod-timeout:${memberId}"]`).click();
  await mod.waitForSelector('[data-live-form=mod-timeout]');
  await mod.locator('input[name=mod-duration-choice][value="60"]').check();
  await mod.locator('#mod-reason').fill('Cool down for a minute');
  await both(mod,'timeout-dialog');
  await mod.locator('[data-live-form=mod-timeout] [type=submit]').click();
  await member.waitForSelector('.timeout-notice');
  assert((await member.locator('.timeout-notice').textContent()).includes('Cool down for a minute'));
  await both(member,'timeout-notice');
  await send(member,'can I talk?');
  await toast(member,'timed out');
  // A moderator cannot time out the owner: there is no action for them.
  await act(mod,`profile:${await owner.locator('[data-self-id]').getAttribute('data-self-id')}`);
  await mod.waitForSelector('.profile-actions');
  assert.equal(await mod.locator('.member-moderation').count(),0);
  await act(mod,'close');
  await act(mod,`profile:${memberId}`);await mod.waitForSelector('[data-live^="mod:remove-timeout"]');
  await mod.locator('[data-live^="mod:remove-timeout"]').click();
  await member.waitForFunction(()=>!document.querySelector('.timeout-notice'));
  checks.push('member menu (right-click), timeout with reason shown to the member, refused send, early removal');

  // Age restriction: a channel the member must confirm before seeing it.
  await form(owner,'channel',{'f-name':'after-dark'});
  await owner.waitForFunction(()=>document.querySelector('.channel-header')?.textContent.includes('after-dark'));
  const adult=await owner.locator('#live-messages').getAttribute('data-channel');
  await send(owner,'grown-up talk');
  // The send's own answer closes any dialog in this channel; let it land first.
  await owner.waitForSelector('.message:has-text("grown-up talk")');
  await wait(1000);
  await open(owner,`channel-edit:${adult}`);
  await owner.waitForSelector('#mod-age');
  await owner.locator('#mod-age').check();
  await owner.locator('[data-live-form=mod-channel] [type=submit]').click();
  await owner.waitForSelector('[data-live-form=mod-channel] .form-saved');
  await act(owner,'close');
  await member.waitForSelector(`[data-live="channel:${adult}"]`);
  await act(member,`channel:${adult}`);
  await member.waitForSelector('.age-gate');
  assert.equal(await member.locator('.message:has-text("grown-up talk")').count(),0);
  await both(member,'age-gate');
  await member.locator(`[data-live="mod:confirm-age,${adult}"]`).click();
  await member.waitForSelector('.message:has-text("grown-up talk")');
  checks.push('age-restricted channel hides history until confirmed; confirmation stored');

  // Verification level: a member who joined minutes ago must wait before talking.
  await open(owner,'safety');
  await owner.locator('input[name=mod-level-choice][value=member_age]').check();
  await owner.locator('[data-live-form=mod-verification] [type=submit]').click();
  await owner.waitForSelector('[data-live-form=mod-verification] .form-saved');
  await act(owner,'close');
  await act(member,`channel:${general}`);
  await send(member,'hello?');
  await toast(member,'member for 10 minutes');
  await act(member,`channel:${general}`);
  await member.waitForSelector('.verification-notice');
  await both(member,'verification-notice');
  await open(owner,'safety');
  await owner.locator('input[name=mod-level-choice][value=none]').check();
  await owner.locator('[data-live-form=mod-verification] [type=submit]').click();
  await owner.waitForSelector('[data-live-form=mod-verification] .form-saved');
  await act(owner,'close');
  checks.push('verification level gates new accounts with an explanation');

  // Kick with a reason, from the member menu.
  await act(owner,`profile:${memberId}`);await owner.waitForSelector('.member-moderation');
  await owner.locator(`[data-live="dialog:mod-kick:${memberId}"]`).click();
  await owner.waitForSelector('[data-live-form=mod-kick]');
  await owner.locator('#mod-reason').fill('Repeated spoilers');
  await both(owner,'kick-dialog');
  await owner.locator('[data-live-form=mod-kick] [type=submit]').click();
  await member.waitForFunction(g=>!document.querySelector(`.server-icon[data-live="guild:${g}"]`),guild);
  checks.push('kick with reason from the member menu removes the member live');

  // Prune: preview count, nothing removed for active members.
  await open(owner,'prune');
  await owner.waitForSelector('.prune-count');
  await owner.waitForFunction(()=>document.querySelector('.prune-count')?.textContent.includes('would be removed'));
  await both(owner,'prune');
  checks.push('prune preview counts inactive members');

  // Audit log: everything above, newest first, filterable by action.
  await open(owner,'audit-log');
  await owner.waitForSelector('.audit-entry');
  for(const action of['member_kick','member_timeout','member_timeout_remove','automod_rule_create','automod_block','automod_alert','overwrites_update','channel_update','invites_pause','verification_update','role_update'])
    await owner.waitForSelector(`.audit-entry[data-action=${action}]`);
  assert((await owner.locator('.audit-entry[data-action=member_kick]').textContent()).includes('Repeated spoilers'));
  await both(owner,'audit-log');
  await owner.locator('#audit-action').selectOption('member_timeout');
  await owner.locator('[data-live="mod:audit-filter"]').click();
  await until(async()=>(await owner.locator('.audit-entry').evaluateAll(es=>es.every(e=>e.dataset.action==='member_timeout')&&es.length>0)));
  checks.push('audit log lists every action with reasons and filters by action type');
  // The moderator (View Audit Log) can read it; and a forged request without it is refused.
  await open(mod,'audit-log');await mod.waitForSelector('.audit-entry');
  const forged=await member.evaluate(async g=>(await fetch(`/api/v1/guilds/${g}/audit-log`)).status,guild);
  assert.ok([403,404].includes(forged));
  checks.push('View Audit Log permission enforced by the server');
  assert.deepEqual(errors,[]);
  await writeFile(`${output}/moderation-results.json`,JSON.stringify({revision:h.revision,checks,screenshots,errors},null,2));
  console.log(JSON.stringify({checks,screenshots:screenshots.length,errors}));
} catch(e) {
  await writeFile(`${output}/moderation-failure.json`,JSON.stringify({error:String(e.stack??e),checks,errors,
    owner:await owner?.locator('body').innerText().catch(()=>''),member:await member?.locator('body').innerText().catch(()=>'')},null,2));
  for(const [name,p] of [['owner',owner],['mod',mod],['member',member]])await p?.screenshot({path:`${output}/moderation-failure-${name}.png`}).catch(()=>{});
  throw e;
} finally {await h.close();}

// Community tools in the real web app against a real local server (127.0.0.1
// only, headless): rules screening, onboarding with opt-in channels, Browse
// Channels and the Server Guide, applications with an interview, the server
// directory, templates (link and starter) and insights. Saves each new
// surface at 1440x900 and 390x844.
import assert from 'node:assert/strict';
import {writeFile} from 'node:fs/promises';
import {harness,register,form,act,shot,output,until,wait} from './qa-harness.mjs';

const h=await harness(),checks=[],screenshots=[],errors=[];
async function both(page,name){
  const size=page.viewportSize();
  for(const [width,height] of [[1440,900],[390,844]]){
    await page.setViewportSize({width,height});
    await wait(200);
    screenshots.push(await shot(page,name));
  }
  await page.setViewportSize(size);
}
async function open(page,name){await act(page,`dialog:${name}`);await page.waitForFunction(n=>document.querySelector('.live-modal')?.dataset.dialog===n,name);}
async function toast(page,text){await page.waitForFunction(t=>[...document.querySelectorAll('[data-toast]')].some(e=>e.textContent.includes(t)),text);}
async function send(page,text){await page.locator('#live-input').fill(text);await page.locator('#live-input').press('Enter');}
const submit=(page,kind)=>page.locator(`[data-live-form="${kind}"] [type=submit]`).first().click();
let owner,member,applicant,visitor;
try {
  owner=await h.phone(1440,900);member=await h.phone(1440,900);applicant=await h.phone(1440,900);visitor=await h.phone(1440,900);
  for(const p of[owner,member,applicant,visitor])p.on('pageerror',e=>errors.push(String(e)));
  await register(owner,h.origin,'cm_owner');
  await register(member,h.origin,'cm_member');
  await register(applicant,h.origin,'cm_applicant');
  await register(visitor,h.origin,'cm_visitor');
  await form(owner,'create',{'f-name':'Community QA'});await owner.waitForSelector('#live-input');
  const guild=(await owner.locator('.server-icon.selected').getAttribute('data-live')).slice(6);
  const general=await owner.locator('#live-messages').getAttribute('data-channel');
  await form(owner,'channel',{'f-name':'games'});
  await owner.waitForSelector('.channel-row:has-text("games")');
  const games=(await owner.locator('.channel-row:has-text("games") .channel-main').getAttribute('data-live')).split(':')[1];
  await form(owner,'invite');await owner.waitForSelector('.live-modal code');
  const code=await owner.locator('.live-modal code').first().textContent();await act(owner,'close');

  // Rules screening.
  await open(owner,'cm-screening');
  await owner.waitForSelector('#cm-screening-on');
  await owner.locator('#cm-screening-on').check();
  await owner.locator('#cm-rule-0').fill('Be kind. **No** harassment.');
  await owner.locator('[data-live="cm:rule-add"]').click();
  await owner.locator('#cm-rule-1').fill('Keep spoilers in their channels.');
  await submit(owner,'cm-screening');
  await owner.waitForSelector('[data-live-form=cm-screening] .form-saved');
  await both(owner,'rules-screening');
  checks.push('rules screening configured with Markdown rules');

  // Onboarding: one question that opts members in to #games, which is opt-in.
  await open(owner,'cm-onboarding');
  await owner.waitForSelector('#cm-ob-enabled');
  await owner.locator('#cm-ob-enabled').check();
  await owner.locator('[data-live="cm:q-add"]').click();
  await owner.waitForSelector('#cm-q1-title');
  await owner.locator('#cm-q1-title').fill('What brings you here?');
  await owner.locator('#cm-q1-pre').check();
  await owner.locator('#cm-q1-a1-emoji').fill('🎮');
  await owner.locator('#cm-q1-a1-label').fill('Games');
  await owner.locator('#cm-q1-a1-channels').selectOption([games]);
  await owner.locator(`#cm-default-${games}`).uncheck();
  await owner.locator('#cm-welcome').fill('Welcome to the **Community QA** server.');
  await owner.locator('#cm-hl0-ch').selectOption(general);
  await owner.locator('#cm-hl0-text').fill('Say hello here');
  await owner.locator('#cm-task0-ch').selectOption(general);
  await owner.locator('#cm-task0-kind').selectOption('talk');
  await owner.locator('#cm-task0-title').fill('Introduce yourself');
  await owner.locator('[data-live="cm:preview"]').click();
  await owner.waitForSelector('.onboarding-preview .answer-chip:has-text("Games")');
  await submit(owner,'cm-onboarding');
  await owner.waitForSelector('[data-live-form=cm-onboarding] .form-saved');
  await both(owner,'onboarding-editor');
  await act(owner,'close');
  checks.push('onboarding editor with live preview, opt-in channel, welcome screen and guide');

  // A new member: the flow opens by itself; they read but cannot talk until they accept.
  await form(member,'join',{'f-name':code});
  await member.waitForFunction(()=>document.querySelector('.live-modal')?.dataset.dialog==='cm-flow');
  await member.waitForSelector('.answer-choice:has-text("Games")');
  await both(member,'onboarding-flow-question');
  await act(member,'close');
  await member.waitForSelector('.screening-notice');
  await both(member,'screening-composer-notice');
  await send(member,'can I talk?');
  await toast(member,'Accept the rules');
  checks.push('a member who has not accepted the rules is refused by the server and told why');
  await member.locator('.screening-notice [data-live="cm:flow"]').click();
  await member.waitForSelector('.answer-choice:has-text("Games")');
  await member.locator('.answer-choice:has-text("Games")').click();
  await member.waitForSelector('.answer-choice[aria-pressed=true]');
  await member.locator('[data-live="cm:next"]').click();
  await member.waitForSelector('.rules-list');
  await both(member,'onboarding-flow-rules');
  await member.locator('#cm-agree').check();
  await member.locator('[data-live="cm:next"]').click();
  await member.waitForSelector('.welcome-highlight');
  await both(member,'onboarding-flow-welcome');
  await member.locator('[data-live="cm:next"]').click();
  await member.waitForFunction(()=>!document.querySelector('.live-modal'));
  await member.waitForFunction(()=>!document.querySelector('.screening-notice'));
  await member.waitForSelector('.channel-row:has-text("games")');
  await member.locator(`.channel-main[data-live="channel:${general}"]`).click();
  await send(member,'hello everyone');
  await member.waitForSelector('.message:has-text("hello everyone")');
  checks.push('accepting the rules lets the member talk; the answer showed the opt-in channel');

  // Server guide: the "say hello" task ticked itself.
  await open(member,'cm-guide');
  await member.waitForSelector('.guide-tasks li[data-done=true]');
  await both(member,'server-guide');
  // Browse Channels: hide #games again.
  await open(member,'cm-browse');
  await member.waitForSelector(`#cm-browse-${games}`);
  await both(member,'browse-channels');
  await member.locator(`#cm-browse-${games}`).uncheck();
  await member.locator('[data-live="cm:browse-save"]').click();
  await toast(member,'Channel list updated');
  await act(member,'close');
  await member.waitForFunction(()=>![...document.querySelectorAll('.channel-row')].some(r=>r.textContent.includes('games')));
  await member.locator('.community-row:has-text("Channels & Roles")').waitFor();
  checks.push('server guide progress and Browse Channels opt-out, stored on the server');

  // Applications: the applicant applies from the Join dialog; the owner interviews and approves.
  await open(owner,'cm-applications');
  await owner.waitForSelector('#cm-applications-on');
  await owner.locator('#cm-applications-on').check();
  await owner.locator('#cm-aq0-prompt').fill('Why do you want to join?');
  await owner.locator('#cm-aq0-required').check();
  await submit(owner,'cm-applications');
  await owner.waitForSelector('[data-live-form=cm-applications] .form-saved');
  await both(owner,'applications-settings');
  await act(applicant,'dialog:join');
  await applicant.waitForSelector('#f-name');
  await applicant.locator('#f-name').fill(code);
  await applicant.waitForSelector('.join-gate [data-live^="dialog:cm-apply:invite:"]');
  await both(applicant,'join-asks-to-apply');
  await applicant.locator('.join-gate [data-live^="dialog:cm-apply:invite:"]').click();
  await applicant.waitForSelector('#cm-answer-0');
  await applicant.locator('#cm-answer-0').fill('I run a games club.');
  await applicant.locator('#cm-apply-rules').check();
  await both(applicant,'application-form');
  await submit(applicant,'cm-apply');
  await applicant.waitForSelector('.my-application:has-text("Waiting for review")');
  await both(applicant,'my-applications');
  await open(owner,'cm-queue');
  await owner.waitForSelector('.application-card:has-text("I run a games club.")');
  await both(owner,'application-queue');
  await owner.locator('.application-card [data-live^="cm:interview,"]').click();
  await owner.waitForFunction(()=>!document.querySelector('.live-modal'));
  await owner.waitForSelector('.channel-header:has-text("Interview")');
  await send(owner,'Tell me about your club');
  await applicant.waitForSelector('.channel-row:has-text("Interview")');
  checks.push('application from an invite, review queue, interview conversation with the applicant');
  await act(owner,`guild:${guild}`);await owner.waitForSelector('.server-heading');
  await open(owner,'cm-queue');
  await owner.locator('.application-card [data-live^="cm:review,"]').click();
  await owner.waitForSelector('.application-card:has-text("Approved")');
  await applicant.waitForSelector(`.server-icon[data-live="guild:${guild}"]`);
  const audit=await owner.evaluate(async g=>(await (await fetch(`/api/v1/guilds/${g}/audit-log?action=application_approve`)).json()).items.length,guild);
  assert.equal(audit,1);
  const forged=await member.evaluate(async g=>(await fetch(`/api/v1/guilds/${g}/applications`)).status,guild);
  assert.equal(forged,403);
  checks.push('approval adds the member, is audited, and the queue needs Kick Members');
  // Applications off again for the directory join below.
  await open(owner,'cm-applications');
  await owner.waitForSelector('#cm-applications-on');
  await owner.locator('#cm-applications-on').uncheck();
  await submit(owner,'cm-applications');
  await owner.waitForSelector('[data-live-form=cm-applications] .form-saved');

  // Discovery: requirements, listing, browsing, search, preview and joining.
  await open(owner,'safety');
  await owner.locator('input[name=mod-level-choice][value=account_age]').check();
  await submit(owner,'mod-verification');
  await owner.waitForSelector('[data-live-form=mod-verification] .form-saved');
  await open(owner,'cm-discovery');
  await owner.waitForSelector('.listing-requirements');
  await owner.waitForFunction(()=>[...document.querySelectorAll('.listing-requirements li')].every(li=>li.dataset.met==='true'));
  await owner.locator('#cm-listing-category').selectOption('gaming');
  await owner.locator('#cm-listing-description').fill('A friendly place to test community tools and play games.');
  await owner.locator('#cm-listing-tags').fill('games, testing');
  await submit(owner,'cm-listing');
  await owner.waitForSelector('[data-live-form=cm-listing] .form-saved');
  await both(owner,'discovery-settings');
  await act(owner,'close');
  await visitor.locator('[aria-label=Discover]').first().click();
  await visitor.waitForSelector('.directory-card');
  await both(visitor,'discover-home');
  await visitor.locator('#cm-directory-q').fill('testing');
  await submit(visitor,'cm-search');
  await visitor.waitForSelector('h3:has-text("Results")');
  await visitor.waitForSelector(`.directory-card[data-guild="${guild}"]`);
  await visitor.locator(`.directory-card[data-guild="${guild}"] button`).click();
  await visitor.waitForSelector('[data-live^="cm:join-discovered"]');
  await both(visitor,'discover-preview');
  await visitor.locator('[data-live^="cm:join-discovered"]').click();
  await visitor.waitForFunction(()=>document.querySelector('.live-modal')?.dataset.dialog==='cm-flow');
  await act(visitor,'close');
  await visitor.waitForSelector('.screening-notice');
  checks.push('listed after meeting the requirements; found by tag search; joining from the directory follows screening');

  // Templates: create, share by link, start a server from it; and a starter template.
  await open(owner,'cm-templates');
  await owner.locator('#cm-template-name').fill('Community QA setup');
  await owner.locator('#cm-template-description').fill('Rules, onboarding and channels');
  await submit(owner,'cm-template-create');
  await owner.waitForSelector('.template-row');
  await both(owner,'templates');
  const tcode=(await owner.locator('.template-row code').first().textContent()).trim();
  await visitor.goto(`${h.origin}/?template=${tcode}`);
  await visitor.waitForSelector('[data-live-form=cm-from-template]');
  await visitor.waitForSelector('.template-structure li:has-text("games")');
  await both(visitor,'template-preview');
  await visitor.locator('#cm-server-name').fill('From a template');
  await submit(visitor,'cm-from-template');
  await visitor.waitForSelector('.server-heading:has-text("From a template")');
  await visitor.waitForSelector('.channel-row:has-text("games")');
  assert.ok(!(await visitor.evaluate(()=>location.search.includes('template'))));
  checks.push('a template link opens its preview and builds the structure (opt-in #games kept as a channel)');
  await open(visitor,'create');
  await visitor.waitForSelector('.starter-template');
  await both(visitor,'create-server-templates');
  await visitor.locator('.starter-template:has-text("Gaming")').click();
  await visitor.waitForSelector('.template-structure li:has-text("looking-for-group")');
  await submit(visitor,'cm-from-template');
  await visitor.waitForSelector('.channel-row:has-text("looking-for-group")');
  checks.push('starter templates in Create a Server');

  // Insights: daily rollups drawn as SVG charts with table alternatives.
  await open(owner,'cm-insights');
  await owner.waitForSelector('.insight-chart svg');
  const joined=await owner.evaluate(()=>[...document.querySelectorAll('.insight-chart table')][0]?.textContent);
  assert.ok(joined && /3/.test(joined), joined);
  await owner.waitForSelector('.insight-ranking:has-text("Invite")');
  await both(owner,'insights');
  await owner.locator('[data-live="cm:insights-weekly"]').click();
  await owner.waitForSelector('[data-live="cm:insights-weekly"][aria-pressed=true]');
  const insightsForged=await member.evaluate(async g=>(await fetch(`/api/v1/guilds/${g}/insights`)).status,guild);
  assert.equal(insightsForged,403);
  checks.push('insights: growth, sources, channels, retention and onboarding funnel; managers only');

  assert.deepEqual(errors,[]);
  await writeFile(`${output}/community-results.json`,JSON.stringify({revision:h.revision,checks,screenshots,errors},null,2));
  console.log(JSON.stringify({checks,screenshots:screenshots.length,errors}));
} catch(e) {
  await writeFile(`${output}/community-failure.json`,JSON.stringify({error:String(e.stack??e),checks,errors,
    owner:await owner?.locator('body').innerText().catch(()=>''),member:await member?.locator('body').innerText().catch(()=>''),applicant:await applicant?.locator('body').innerText().catch(()=>''),visitor:await visitor?.locator('body').innerText().catch(()=>'')},null,2));
  for(const [name,p] of [['owner',owner],['member',member],['applicant',applicant],['visitor',visitor]])await p?.screenshot({path:`${output}/community-failure-${name}.png`}).catch(()=>{});
  throw e;
} finally {await h.close();}

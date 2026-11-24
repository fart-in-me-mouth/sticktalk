// Channel drag and drop in a server's channel list (docs/features/channel-drag.md)
// against a real local server, headless and loopback-only: reorder, voice above
// text, nest and take back out, what a second member sees live, no dragging
// without Manage Channels, a refused move snapping back, file drops untouched,
// and the order after a reload. Real mouse movement fires the HTML drag events.
import assert from 'node:assert/strict';
import {mkdir,writeFile} from 'node:fs/promises';
import {join} from 'node:path';
import {harness,register,form,act,wait} from './qa-harness.mjs';

const shots=process.env.STICKTALK_CHANDRAG_SHOTS??join(process.env.CARGO_TARGET_DIR??'.','chandrag-shots');
await mkdir(shots,{recursive:true});
const h=await harness(),checks=[],errors=[],saved=[];
const log=s=>process.env.STICKTALK_CHANDRAG_VERBOSE&&console.log(`[chandrag] ${s}`);
const PNG='iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';
let alice,bob;
/** Sidebar rows as "depth:name", in the order shown. */
const tree=page=>page.locator('.channel-scroll').evaluate(nav=>[...nav.querySelectorAll('.category-row,.channel-row:not(.nav-conv-row)')]
  .map(e=>`${e.dataset.depth}:${(e.querySelector('h2.category,.channel-main .truncate')?.textContent??'').trim()}`));
const row=(page,name)=>page.locator('.channel-scroll .category-row,.channel-scroll .channel-row').filter({has:page.locator(`h2.category,.channel-main .truncate`).filter({hasText:new RegExp(`^${name}$`)})});
async function shows(page,expected,what){
  try { await page.waitForFunction(([e])=>{
    const rows=[...document.querySelectorAll('.channel-scroll .category-row,.channel-scroll .channel-row:not(.nav-conv-row)')]
      .map(r=>`${r.dataset.depth}:${(r.querySelector('h2.category,.channel-main .truncate')?.textContent??'').trim()}`);
    return rows.join('|')===e;
  },[expected.join('|')],{timeout:15000}); }
  catch { assert.deepEqual(await tree(page),expected,what); }
}
async function shot(page,name){
  await page.evaluate(()=>document.fonts.ready);
  const path=join(shots,`${name}-1440x900.png`);
  await page.screenshot({path,caret:'initial'});saved.push(path);
}
/** Press on `from`, move onto `to` at its top edge, middle or bottom edge (or
 * `end`: the empty space under the list), then release. */
async function drag(page,from,to,where,hold){
  const a=await row(page,from).boundingBox();
  await page.mouse.move(a.x+60,a.y+a.height/2);
  await page.mouse.down();
  await page.mouse.move(a.x+70,a.y+a.height/2+6,{steps:3});
  let x,y;
  if(where==='end'){
    const list=await page.locator('.channel-scroll').boundingBox();
    x=list.x+80;y=list.y+list.height-60;
  } else {
    const b=await row(page,to).boundingBox();
    x=b.x+80;y=where==='before'?b.y+b.height*0.12:where==='after'?b.y+b.height*0.88:b.y+b.height/2;
  }
  await page.mouse.move(x,y,{steps:10});
  await page.mouse.move(x,y+1);
  if(hold)await hold();
  await page.mouse.up();
}
const marker=page=>page.waitForSelector('[data-chan-drop]',{timeout:5000});
async function closeDialog(page){await act(page,'close');await page.waitForFunction(()=>!document.querySelector('.live-modal'));}
async function create(page,name,kind){
  await act(page,'dialog:channel');
  await page.waitForSelector('[data-live-form="channel"]');
  if(kind)await page.locator('#f-extra').selectOption(kind);
  await page.locator('#f-name').fill(name);
  await page.locator('[data-live-form="channel"] [type=submit]').click();
  await page.waitForFunction(n=>[...document.querySelectorAll('.channel-scroll h2.category,.channel-scroll .channel-main .truncate')].some(e=>e.textContent.trim()===n),name);
}

try {
  alice=await h.phone(1440,900);bob=await h.phone(1440,900);
  for(const p of[alice,bob]){p.on('pageerror',e=>errors.push(String(e)));p.setDefaultTimeout(30000);}
  await register(alice,h.origin,'alice_drag');await register(bob,h.origin,'bob_drag');
  await form(alice,'create',{'f-name':'Drag HQ'});await alice.waitForSelector('#live-input');
  const hq=(await alice.locator('.server-icon.selected').getAttribute('data-live')).slice(6);
  await form(alice,'invite');await alice.waitForSelector('.live-modal code');
  const code=await alice.locator('.live-modal code').first().textContent();await closeDialog(alice);
  await form(bob,'join',{'f-name':code});await bob.waitForSelector('#live-input');
  await create(alice,'alpha');await create(alice,'beta');await create(alice,'lounge','voice');await create(alice,'Ops','category');
  await alice.locator('.channel-scroll [data-live^="channel:"]').first().click();
  const start=['0:general','0:Voice','0:alpha','0:beta','0:lounge','0:Ops'];
  await shows(alice,start,'the default channels, four more and a category');await shows(bob,start,'bob sees them');
  log('check');checks.push('a server with text and voice channels and a category, created through the UI, a second member joined');

  // Without Manage Channels nothing is draggable and the rows carry nothing for it.
  assert.equal(await bob.locator('.channel-scroll [draggable=true],.channel-scroll [data-chan]').count(),0);
  assert.equal(await alice.locator('.channel-scroll [draggable=true][data-chan]').count(),6);
  checks.push('a member without Manage Channels has no draggable rows; the owner has six');

  // Reorder two text channels.
  log('reorder');
  await drag(alice,'beta','alpha','before',async()=>{await marker(alice);await shot(alice,'drag-line-before');});
  const reordered=['0:general','0:Voice','0:beta','0:alpha','0:lounge','0:Ops'];
  await shows(alice,reordered,'beta above alpha');await shows(bob,reordered,'bob sees beta above alpha without reloading');
  checks.push('dragging a text channel above another reorders them, for the second member too');

  // A voice channel above a text channel.
  await drag(alice,'lounge','general','before');
  const voiceFirst=['0:lounge','0:general','0:Voice','0:beta','0:alpha','0:Ops'];
  await shows(alice,voiceFirst,'voice above text');await shows(bob,voiceFirst,'bob sees the voice channel first');
  checks.push('a voice channel moves above a text channel');

  // Into the category (the middle of its row), then a line at the nested level.
  log('nest');
  await drag(alice,'alpha','Ops','into',async()=>{
    await marker(alice);
    assert.equal(await row(alice,'Ops').getAttribute('data-chan-drop'),'into');
    await shot(alice,'drag-into-category');
  });
  const nested=['0:lounge','0:general','0:Voice','0:beta','0:Ops','1:alpha'];
  await shows(alice,nested,'alpha inside Ops');await shows(bob,nested,'bob sees alpha inside Ops');
  await drag(alice,'beta','alpha','before',async()=>{
    await marker(alice);
    assert.equal(await row(alice,'alpha').getAttribute('data-chan-drop'),'top1');
    await shot(alice,'drag-line-nested');
  });
  await shows(alice,['0:lounge','0:general','0:Voice','0:Ops','1:beta','1:alpha'],'beta inside Ops above alpha');
  // Into a text channel, and everything inside moves with what holds it.
  await drag(alice,'Ops','general','into');
  await shows(alice,['0:lounge','0:general','1:Ops','2:beta','2:alpha','0:Voice'],'Ops and what it holds inside general');
  checks.push('a channel nests in a category and a category with its channels nests in a text channel; the marker is a highlight for "inside" and a line at the nested indentation');

  // Back out to the top level: before a top-level row, and the empty space under the list.
  log('unnest');
  await drag(alice,'alpha','lounge','before');
  await shows(alice,['0:alpha','0:lounge','0:general','1:Ops','2:beta','0:Voice'],'alpha back at the top');
  await drag(alice,'Ops','x','end',async()=>{await marker(alice);await shot(alice,'drag-line-end');});
  const out=['0:alpha','0:lounge','0:general','0:Voice','0:Ops','1:beta'];
  await shows(alice,out,'Ops back at the top level with beta inside');await shows(bob,out,'bob sees it too');
  checks.push('dragging out to the top level (before a row, or under the list) takes what is inside along');

  // Onto itself or what it holds: no marker, no change.
  await drag(alice,'Ops','beta','into',async()=>{await wait(150);assert.equal(await alice.locator('[data-chan-drop]').count(),0);});
  await wait(300);await shows(alice,out,'nothing moved');
  checks.push('a channel cannot be dropped into itself or what it holds (no marker)');

  // A channel dropped on the message box is not typed into it, and does not move.
  const input=alice.locator('#live-input');
  await input.fill('');
  {
    const a=await row(alice,'general').boundingBox(),b=await input.boundingBox();
    await alice.mouse.move(a.x+60,a.y+a.height/2);await alice.mouse.down();
    await alice.mouse.move(a.x+70,a.y+a.height/2+6,{steps:3});
    await alice.mouse.move(b.x+40,b.y+b.height/2,{steps:10});await alice.mouse.up();
  }
  await wait(300);
  assert.equal(await input.inputValue(),'');
  assert.equal(await alice.locator('.main-pane.is-dropping,.live-upload').count(),0);
  await shows(alice,out,'nothing moved');
  checks.push('dropping a channel on the conversation or message box neither types, uploads nor moves anything');

  // Files dropped onto the conversation are still attached.
  const dropped=await alice.evaluate(png=>{
    const files=new DataTransfer();
    files.items.add(new File([Uint8Array.from(atob(png),c=>c.charCodeAt(0))],'dropped.png',{type:'image/png'}));
    const target=document.querySelector('#live-messages');
    const over=new DragEvent('dragover',{dataTransfer:files,bubbles:true,cancelable:true});
    target.dispatchEvent(over);
    const drop=new DragEvent('drop',{dataTransfer:files,bubbles:true,cancelable:true});
    target.dispatchEvent(drop);
    return over.defaultPrevented&&drop.defaultPrevented;
  },PNG);
  assert(dropped,'the file drop is taken by the conversation');
  await alice.waitForFunction(()=>[...document.querySelectorAll('.live-upload')].some(e=>e.textContent.includes('dropped.png')));
  await act(alice,'remove-attachment:0');
  await alice.waitForFunction(()=>!document.querySelector('.live-upload'));
  checks.push('a file dropped on the conversation is attached as before');

  // The order is kept by the server.
  await alice.reload();await alice.waitForSelector('.server-rail');
  if(!await alice.locator('.channel-scroll [data-chan]').count())await act(alice,`guild:${hq}`);
  await alice.waitForSelector('.channel-scroll [data-chan]');
  await shows(alice,out,'the order after a reload');
  checks.push('the order survives a reload');

  // A refused move snaps back with a message: bob may manage channels by role,
  // but an overwrite on general takes that away for him there.
  log('refused');
  const guildRole=async()=>{
    await act(alice,'dialog:roles');
    await alice.waitForSelector('[data-live-form=role-create]');
    await alice.locator('#f-name').fill('Arrangers');await alice.locator('[data-live-form=role-create] [type=submit]').click();
    await alice.waitForSelector('[data-live^="role:"]:has-text("Arrangers")');
    const role=(await alice.locator('[data-live^="role:"]:has-text("Arrangers")').getAttribute('data-live')).slice(5);
    await act(alice,`role:${role}`);
    await alice.getByLabel('Manage Channels',{exact:true}).check();
    await alice.locator('[data-live-form=role-save] [type=submit]').click();
    await alice.waitForSelector('[data-live-form=role-create]');
    const bobId=await bob.locator('[data-self-id]').getAttribute('data-self-id');
    await act(alice,'dialog:member-role');
    await alice.waitForSelector('[data-live-form=member-role]');
    await alice.locator('[data-live-form=member-role] #f-name').selectOption(bobId);
    await alice.locator(`[data-member-role="${role}"]`).check();await alice.locator('[data-live-form=member-role] [type=submit]').click();
    await alice.waitForFunction(()=>!document.querySelector('.live-modal'));
    return bobId;
  };
  const bobId=await guildRole();
  await bob.waitForSelector('.channel-scroll [draggable=true][data-chan]');
  const general=(await row(alice,'general').getAttribute('data-chan')).split(',')[0];
  await act(alice,`dialog:channel-perms:${general}`);
  await alice.waitForSelector('#ow-add-member');
  await alice.locator('#ow-add-member').selectOption(bobId);
  await alice.locator('[data-live="mod:ow-add,member:"]').click();
  await alice.waitForSelector(`[data-overwrite="member:${bobId}"]`);
  await alice.locator(`[data-overwrite="member:${bobId}"] [aria-label^="Manage Channel for "] .tri-option[title=Deny]`).click();
  await alice.locator('[data-live="mod:ow-save"]').click();
  await alice.waitForFunction(()=>[...document.querySelectorAll('[data-toast]')].some(e=>e.textContent.includes('Channel permissions saved')));
  await closeDialog(alice);
  await drag(bob,'general','alpha','before');
  await bob.waitForFunction(()=>document.querySelector('[data-toast=error]')?.textContent.includes('back where it was'));
  const message=await bob.locator('[data-toast=error]').textContent();
  assert.match(message,/permission/);
  await shows(bob,out,'bob’s list is back as it was');
  await shot(bob,'refused-move');
  await shows(alice,out,'nothing changed for alice');
  // What the overwrite does not cover still moves for him.
  await drag(bob,'lounge','alpha','before');
  const byBob=['0:lounge','0:alpha','0:general','0:Voice','0:Ops','1:beta'];
  await shows(bob,byBob,'bob moves lounge');await shows(alice,byBob,'alice sees bob’s move');
  checks.push('a move the server refuses (a channel overwrite) snaps back with a plain message; other channels still move');

  // The keyboard and touch path: Move Channel in a channel's options menu.
  await act(alice,`nav-menu:conv:${general}`);
  await alice.waitForSelector('.live-modal [data-live^="dialog:channel-move:"]');
  await closeDialog(alice);
  checks.push('the channel options menu offers Move Channel to people who manage channels');

  assert.deepEqual(errors,[]);
  await writeFile(join(shots,'chandrag-results.json'),JSON.stringify({revision:h.revision,checks,screenshots:saved,errors},null,2));
  console.log(JSON.stringify({checks,errors,screenshots:saved.length}));
} catch(e) {
  await writeFile(join(shots,'chandrag-failure.json'),JSON.stringify({error:String(e.stack??e),checks,errors,alice:await alice?.locator('body').innerText().catch(()=>''),bob:await bob?.locator('body').innerText().catch(()=>'')},null,2));
  if(alice)await alice.screenshot({path:join(shots,'failure-alice.png')}).catch(()=>{});
  if(bob)await bob.screenshot({path:join(shots,'failure-bob.png')}).catch(()=>{});
  throw e;
} finally {await h.close();}

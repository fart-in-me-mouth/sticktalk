// Expression in the real web app against a real local server (127.0.0.1 only,
// headless): custom emoji upload, picker, :name: completion, rendering, jumbo
// emoji, use across servers and in direct messages, stickers (built-in and
// server, picker, tray, suggestions, favorites), Super Reactions (burst,
// reduced motion), custom typing indicators and the audit log. Saves each new
// surface at 1440x900 and 390x844.
import assert from 'node:assert/strict';
import {writeFile} from 'node:fs/promises';
import {deflateSync} from 'node:zlib';
import {harness,register,form,act,shot,output,wait,until} from './qa-harness.mjs';

// Tiny image encoders, so the test needs no image files.
const crcTable=Array.from({length:256},(_,n)=>{let c=n;for(let k=0;k<8;k++)c=c&1?0xedb88320^(c>>>1):c>>>1;return c>>>0;});
const crc=b=>{let c=0xffffffff;for(const x of b)c=crcTable[(c^x)&255]^(c>>>8);return (c^0xffffffff)>>>0;};
function png(size,[r,g,b]){
  const chunk=(type,data)=>{const len=Buffer.alloc(4);len.writeUInt32BE(data.length);const td=Buffer.concat([Buffer.from(type),data]);const c=Buffer.alloc(4);c.writeUInt32BE(crc(td));return Buffer.concat([len,td,c]);};
  const ihdr=Buffer.alloc(13);ihdr.writeUInt32BE(size,0);ihdr.writeUInt32BE(size,4);ihdr[8]=8;ihdr[9]=6;
  const rows=[];
  for(let y=0;y<size;y++){rows.push(0);for(let x=0;x<size;x++){const d=Math.hypot(x-size/2,y-size/2)<size*0.45;rows.push(r,g,b,d?255:0);}}
  return Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),chunk('IHDR',ihdr),chunk('IDAT',deflateSync(Buffer.from(rows))),chunk('IEND',Buffer.alloc(0))]);
}
// A two-frame animated GIF. Each pixel is sent as one 3-bit LZW code followed
// by a clear code, so the dictionary never grows (valid, if not small).
function gif(size){
  const out=[...Buffer.from('GIF89a')];
  out.push(size&255,size>>8,size&255,size>>8,0xf1,0,0);
  out.push(0xf2,0xb8,0x4b, 0x7a,0xd7,0xff, 0xff,0x7a,0xb6, 0x26,0x26,0x2e);
  out.push(0x21,0xff,11,...Buffer.from('NETSCAPE2.0'),3,1,0,0,0);
  for(let frame=0;frame<2;frame++){
    out.push(0x21,0xf9,4,0,20,0,0,0);
    out.push(0x2c,0,0,0,0,size&255,size>>8,size&255,size>>8,0);
    out.push(2);
    const bits=[];const put=v=>{for(let i=0;i<3;i++)bits.push((v>>i)&1);};
    for(let y=0;y<size;y++)for(let x=0;x<size;x++){put(4);put(((x>>2)+(y>>2)+frame)%3);}
    put(5);
    const bytes=[];for(let i=0;i<bits.length;i+=8){let v=0;for(let j=0;j<8&&i+j<bits.length;j++)v|=bits[i+j]<<j;bytes.push(v);}
    for(let i=0;i<bytes.length;i+=255){const block=bytes.slice(i,i+255);out.push(block.length,...block);}
    out.push(0);
  }
  out.push(0x3b);
  return Buffer.from(out);
}

const h=await harness(),checks=[],screenshots=[],errors=[];
async function both(page,name){
  const size=page.viewportSize();
  for(const [width,height] of [[1440,900],[390,844]]){
    await page.setViewportSize({width,height});
    await wait(200);
    screenshots.push(await shot(page,`expression-${name}`));
  }
  await page.setViewportSize(size);
}
async function open(page,name){await act(page,`dialog:${name}`);await page.waitForFunction(n=>document.querySelector('[data-dialog]')?.dataset.dialog===n,name);}
async function toast(page,text){await page.waitForFunction(t=>[...document.querySelectorAll('[data-toast]')].some(e=>e.textContent.includes(t)),text);}
async function send(page,text){await page.locator('#live-input').fill(text);await page.locator('#live-input').press('Enter');}
const isLoaded=sel=>[...document.querySelectorAll(sel)].some(i=>i.complete&&i.naturalWidth>0);
const loadedImage=(page,sel)=>page.waitForFunction(isLoaded,sel);
async function upload(page,kind,file,fields){
  await page.locator('#expr-file').setInputFiles(file);
  for(const [id,value] of Object.entries(fields))await page.locator(`#${id}`).fill(value);
  await page.locator(`[data-live-form=expr-${kind}-upload] [type=submit]`).click();
  await toast(page,kind==='sticker'?'Sticker added':'Emoji added');
}
let owner,friend,outsider;
try {
  owner=await h.phone(1440,900);friend=await h.phone(1440,900);outsider=await h.phone(1440,900);
  for(const p of[owner,friend,outsider])p.on('pageerror',e=>errors.push(String(e)));
  await register(owner,h.origin,'expr_owner');
  await register(friend,h.origin,'expr_friend');
  await register(outsider,h.origin,'expr_outsider');
  await form(owner,'create',{'f-name':'Expression QA'});await owner.waitForSelector('#live-input');
  const guild=(await owner.locator('.server-icon.selected').getAttribute('data-live')).slice(6);
  const general=await owner.locator('#live-messages').getAttribute('data-channel');
  await form(owner,'invite');await owner.waitForSelector('.live-modal code');
  const code=await owner.locator('.live-modal code').first().textContent();await act(owner,'close');
  await form(friend,'join',{'f-name':code});await friend.waitForSelector('#live-input');
  // The outsider and the friend are friends with a direct message, no shared server.
  await form(outsider,'friend',{'f-name':'expr_friend'});
  const outsiderId=await outsider.locator('[data-self-id]').getAttribute('data-self-id');
  await wait(500);
  await act(friend,`friend:accept,${outsiderId}`);
  await wait(500);

  // Server settings › Emoji: upload a still and an animated emoji.
  await open(owner,'server-emoji');
  await owner.waitForSelector('[data-live-form=expr-emoji-upload]');
  await upload(owner,'emoji',{name:'wave.png',mimeType:'image/png',buffer:png(96,[46,160,120])},{'expr-name':'stickwave'});
  await upload(owner,'emoji',{name:'spin.gif',mimeType:'image/gif',buffer:gif(48)},{'expr-name':'spinny'});
  await owner.waitForSelector('[data-emoji-row=stickwave]');await owner.waitForSelector('[data-emoji-row=spinny]');
  assert.ok((await owner.locator('.slot-count').first().textContent()).includes('1 of 250'));
  assert.ok((await owner.locator('[data-emoji-row=stickwave]').textContent()).includes('Added by expr_owner'));
  await both(owner,'server-emoji');
  checks.push('server emoji page: upload still and animated, slots used, who uploaded');
  // Rename from its edit page.
  await owner.mouse.move(0,0);
  await owner.locator('[aria-label="Edit :spinny:"]').click();
  await owner.waitForSelector('[data-live-form=expr-emoji-edit]');
  await both(owner,'emoji-edit');
  await owner.locator('#expr-edit-name').fill('spinner');
  await owner.locator('[data-live-form=expr-emoji-edit] [type=submit]').click();
  await owner.waitForSelector('[data-emoji-row=spinner]');
  checks.push('emoji rename');

  // Server settings › Stickers.
  await open(owner,'server-stickers');
  await owner.waitForSelector('[data-live-form=expr-sticker-upload]');
  await upload(owner,'sticker',{name:'star.png',mimeType:'image/png',buffer:png(200,[242,184,75])},{'expr-name':'Server Star','expr-emoji':'⭐','expr-description':'A big golden star'});
  await owner.waitForSelector('[data-sticker-row="Server Star"]');
  await both(owner,'server-stickers');
  checks.push('server stickers page: upload with related emoji and description');
  await act(owner,'close');

  // Messages: :name: becomes the emoji; an emoji-only message is jumbo.
  await act(owner,`channel:${general}`);await owner.waitForSelector('#live-input');
  await send(owner,'hello :stickwave: friends');
  await loadedImage(owner,'.message img.custom-emoji');
  await send(owner,':stickwave: :spinner:');
  await owner.waitForSelector('.markdown.jumbo-emoji .custom-emoji.animated');
  await friend.waitForSelector('.markdown.jumbo-emoji');
  await both(owner,'messages-jumbo');
  checks.push('custom emoji render in messages, animated ones animate, jumbo when emoji-only');
  // Reduced motion: only the still frame shows until hovered.
  await owner.emulateMedia({reducedMotion:'reduce'});
  await owner.mouse.move(0,0);await owner.locator('#live-input').focus();
  assert.equal(await owner.locator('.jumbo-emoji .custom-emoji.animated .emoji-motion').first().evaluate(e=>getComputedStyle(e).display),'none');
  await owner.locator('.jumbo-emoji').last().hover();
  assert.notEqual(await owner.locator('.jumbo-emoji .custom-emoji.animated .emoji-motion').last().evaluate(e=>getComputedStyle(e).display),'none');
  await owner.emulateMedia({reducedMotion:'no-preference'});
  checks.push('reduced motion: animated emoji play on hover or focus only');
  // Clicking an emoji says which server it is from.
  await owner.locator('.message .emoji-button').first().click();
  await owner.waitForSelector('.emoji-info');
  assert.ok((await owner.locator('.emoji-info').textContent()).includes('From Expression QA'));
  await both(owner,'emoji-info');
  await act(owner,'close');

  // The picker: a section for the server, search finds custom emoji.
  await open(owner,'emoji');
  await owner.waitForSelector('.picker-section[aria-label="Expression QA"] img.custom-emoji');
  await both(owner,'emoji-picker');
  await owner.locator('#emoji-query').fill('stick');await wait(300);
  await owner.waitForSelector('.picker-section[aria-label="Expression QA"] [aria-label=":stickwave:"]');
  await owner.locator('[aria-label=":stickwave:"]').first().click();
  await owner.waitForFunction(()=>document.querySelector('#live-input')?.value.includes(':stickwave:'));
  await owner.locator('#live-input').fill('');
  checks.push('emoji picker: per-server section with icon, search, insert as :name:');
  // :name: completion.
  await owner.locator('#live-input').pressSequentially(':stickw');
  await owner.waitForSelector('#live-completions img.custom-emoji');
  await both(owner,'emoji-completion');
  await owner.locator('#live-input').fill('');await act(owner,'complete-close');
  checks.push(':name: autocomplete lists custom emoji with their image');

  // Use across servers: the friend uses it in a direct message with the outsider,
  // who is not in the server, sees it, and is told where it is from.
  await act(friend,`friend:dm,${outsiderId}`);
  await friend.waitForFunction(()=>document.querySelector('#live-input')?.placeholder.includes('expr_outsider'));
  const dm=await friend.locator('#live-messages').getAttribute('data-channel');
  await send(friend,'look :stickwave:');
  // Open it once the outsider's app knows the conversation.
  await outsider.waitForFunction(()=>document.querySelector('.channel-sidebar')?.textContent.includes('expr_friend'));
  await act(outsider,`channel:${dm}`);
  await outsider.waitForSelector(`#live-messages[data-channel="${dm}"]`);
  await loadedImage(outsider,'.message img.custom-emoji');
  await outsider.locator('.message .emoji-button').first().click();
  await outsider.waitForSelector('.emoji-info');
  assert.ok((await outsider.locator('.emoji-info').textContent()).includes('not in'));
  await both(outsider,'emoji-info-elsewhere');
  await act(outsider,'close');
  checks.push('custom emoji in a direct message render for someone outside the server');
  // ...and someone outside the server cannot send it: it is sent as :name:.
  const emojiId=(await owner.locator('.message img.custom-emoji').first().getAttribute('src')).split('/').pop();
  await send(outsider,`<:stickwave:${emojiId}> mine now`);
  await outsider.waitForFunction(()=>[...document.querySelectorAll('.message .markdown')].some(m=>m.textContent.includes(':stickwave: mine now')));
  checks.push('the server rewrites emoji the author may not use to :name:');

  await act(friend,`channel:${general}`);await friend.waitForSelector(`#live-messages[data-channel="${general}"]`);
  // Stickers: picker with the built-in pack and the server's, then send one.
  await act(owner,`channel:${general}`);await owner.waitForSelector('#live-input');
  await open(owner,'stickers');
  await owner.waitForSelector('.picker-section[aria-label="StickTalk"] .sticker-pick');
  await owner.waitForSelector('.picker-section[aria-label="Expression QA"] .sticker-pick');
  await both(owner,'sticker-picker');
  await owner.locator('[aria-label="Add to favorites: Hello"]').click();
  await owner.waitForSelector('.picker-section[aria-label="Favorites"]');
  await owner.locator('[aria-label^="Send sticker Hello"]').first().click();
  await loadedImage(owner,'.message .message-stickers img.sticker');
  checks.push('sticker picker: built-in and server sections, favorites; send one');
  // Up to three on one message, with the text in the composer.
  await owner.locator('#live-input').fill('two at once');
  await open(owner,'stickers');
  await owner.locator('[aria-label="Add to this message: Party Time"]').click();
  await owner.locator('[aria-label="Add to this message: Server Star"]').click();
  await owner.waitForSelector('.sticker-tray');
  await both(owner,'sticker-tray');
  await owner.locator('[data-live="expr-sticker-tray:send"]').click();
  await owner.waitForFunction(()=>[...document.querySelectorAll('.message')].some(m=>m.querySelectorAll('.message-stickers .sticker').length===2&&m.textContent.includes('two at once')));
  await loadedImage(friend,'.message .message-stickers img[alt^="Sticker: Server Star"]');
  checks.push('two stickers with text on one message; server sticker loads for members');
  // Typing a related emoji suggests stickers.
  await owner.locator('#live-input').fill('');
  await owner.locator('#live-input').pressSequentially('great ⭐');
  await owner.waitForSelector('.sticker-suggestions [aria-label="Send sticker Server Star"]');
  await both(owner,'sticker-suggestions');
  await owner.locator('.sticker-suggestions [aria-label="Send sticker Server Star"]').click();
  await owner.waitForFunction(()=>!document.querySelector('.sticker-suggestions'));
  await owner.locator('#live-input').fill('');
  checks.push('a related emoji in the composer suggests stickers');
  // Stickers in direct messages work under the same rules.
  await act(friend,`channel:${dm}`);await friend.waitForSelector(`#live-messages[data-channel="${dm}"]`);
  await open(friend,'stickers');
  await friend.locator('[aria-label^="Send sticker Server Star"]').first().click();
  await loadedImage(outsider,'.message .message-stickers img[alt^="Sticker: Server Star"]');
  await act(friend,`channel:${general}`);await friend.waitForSelector(`#live-messages[data-channel="${general}"]`);
  checks.push('a server sticker sent in a direct message shows for someone outside the server');

  // Super Reactions: from the message's actions, with a burst for viewers.
  await send(owner,'react to me');
  await owner.waitForFunction(()=>[...document.querySelectorAll('.message')].at(-1)?.textContent.includes('react to me'));
  const mid=await owner.locator('[data-live-message]').last().getAttribute('data-live-message');
  await act(owner,`expr-super-picker:${mid}`);
  await owner.waitForSelector('.super-toggle [aria-pressed=true]');
  await both(owner,'super-reaction-picker');
  await owner.locator('#emoji-query').fill('thumbs');await wait(300);
  await owner.waitForSelector('.picker-modal .emoji-grid button');
  const burst=friend.waitForSelector('.reaction.super.super-burst',{timeout:5000});
  await owner.locator('.picker-modal .emoji-grid button').first().click();
  await burst;
  screenshots.push(await shot(friend,'expression-super-burst'));
  await owner.waitForSelector('.reaction.super.mine');
  // A normal reaction of the same emoji sits beside it with its own count.
  const superChip=owner.locator('.reaction.super').first();
  const alt=await superChip.getAttribute('data-live-alt');
  await act(friend,alt);
  await owner.waitForFunction(()=>document.querySelectorAll('.reactions .reaction').length===2);
  // Shift+click on the normal chip adds a Super one.
  await friend.locator('.reaction:not(.super)').first().click({modifiers:['Shift']});
  await owner.waitForFunction(()=>document.querySelector('.reaction.super .reaction-count')?.textContent==='2');
  await both(owner,'super-reactions');
  // Reduced motion: a static highlight, no animation.
  await friend.emulateMedia({reducedMotion:'reduce'});
  await act(owner,`super-react:${mid},🎉`);
  await friend.waitForSelector('.reaction.super-burst[data-live*="🎉"]');
  const reduced=await friend.locator('.reaction.super-burst[data-live*="🎉"]').first().evaluate(e=>({name:getComputedStyle(e,'::before').animationName,shadow:getComputedStyle(e).boxShadow}));
  assert.equal(reduced.name,'none');assert.notEqual(reduced.shadow,'none');
  await friend.emulateMedia({reducedMotion:'no-preference'});
  checks.push('Super Reactions: picker toggle, burst for viewers, own chip and count, Shift+click, reduced motion');

  // Settings › Expression: a custom typing indicator, and the animation switch.
  await open(friend,'expression-settings');
  await friend.waitForSelector('[data-live-form=expr-typing]');
  await friend.locator('#expr-typing-emoji-0').fill('✍️');
  await friend.locator('#expr-typing-emoji-1').fill(':stickwave:');
  await friend.locator('#expr-typing-motion').selectOption('wave');
  await friend.locator('#expr-typing-verb').selectOption('custom');
  await friend.locator('#expr-typing-custom').fill('doodling');
  await friend.locator('[data-live-form=expr-typing] [type=submit]').click();
  await toast(friend,'Typing indicator saved');
  await friend.waitForSelector('.typing-preview .typing-custom');
  await both(friend,'typing-settings');
  await friend.locator('input[data-preference=superReactions]').uncheck();
  assert.equal(await friend.evaluate(()=>document.documentElement.dataset.superReactions),'false');
  await friend.locator('input[data-preference=superReactions]').check();
  await act(friend,'close');
  await act(friend,`channel:${general}`);await friend.waitForSelector('#live-input');
  await friend.locator('#live-input').pressSequentially('hi');
  await owner.waitForSelector('.typing-indicator .typing-custom.motion-wave');
  assert.ok((await owner.locator('.typing-indicator').textContent()).includes('expr_friend is doodling'));
  screenshots.push(await shot(owner,'expression-typing-indicator'));
  await owner.setViewportSize({width:390,height:844});await wait(200);
  screenshots.push(await shot(owner,'expression-typing-indicator'));
  await owner.setViewportSize({width:1440,height:900});
  await friend.locator('#live-input').fill('');
  checks.push('custom typing indicator: emoji, motion and verb, seen by others while typing');

  // Permissions: the role editor lists the new rights.
  await open(owner,'roles');
  await owner.locator(`[data-live="role:${guild}"]`).click();
  for(const label of['Use External Emoji','Use External Stickers','Manage Expressions'])
    await owner.getByLabel(label,{exact:true}).waitFor();
  assert.ok(await owner.getByLabel('Use External Emoji',{exact:true}).isChecked());
  await act(owner,'close');
  checks.push('Use External Emoji / Stickers on for @everyone, Manage Expressions in the role editor');

  // Deleting a sticker: messages keep its name; the audit log has every change.
  await open(owner,'server-stickers');
  await owner.locator('[aria-label="Edit sticker Server Star"]').click();
  await owner.waitForSelector('[data-live-form=expr-sticker-edit]');
  await owner.locator('#expr-edit-name').fill('Golden Star');
  await owner.locator('[data-live-form=expr-sticker-edit] [type=submit]').click();
  await owner.waitForSelector('[data-sticker-row="Golden Star"]');
  await owner.locator('[aria-label="Edit sticker Golden Star"]').click();
  await owner.locator('[data-live^="expr-sticker-delete:"]').click();
  await owner.locator('#live-confirm [data-accept]').click();
  await owner.waitForFunction(()=>!document.querySelector('[data-sticker-row]')&&document.querySelector('[data-dialog]')?.dataset.dialog==='server-stickers');
  await toast(owner,'Sticker deleted');
  await open(owner,'audit-log');await owner.waitForSelector('.audit-entry');
  for(const action of['emoji_create','emoji_update','sticker_create','sticker_update','sticker_delete'])
    await owner.waitForSelector(`.audit-entry[data-action=${action}]`);
  await both(owner,'audit-log');
  await act(owner,'close');
  checks.push('sticker rename and delete; audit log records emoji and sticker changes');

  const csp=[];for(const p of[owner,friend,outsider])csp.push(...await p.evaluate(()=>window.__qaCsp??[]));
  assert.deepEqual(csp,[]);
  assert.deepEqual(errors,[]);
  await writeFile(`${output}/expression-results.json`,JSON.stringify({revision:h.revision,checks,screenshots,errors},null,2));
  console.log(JSON.stringify({checks,screenshots:screenshots.length,errors}));
} catch(e) {
  await writeFile(`${output}/expression-failure.json`,JSON.stringify({error:String(e.stack??e),checks,errors,
    owner:await owner?.locator('body').innerText().catch(()=>''),friend:await friend?.locator('body').innerText().catch(()=>'')},null,2));
  for(const [name,p] of [['owner',owner],['friend',friend],['outsider',outsider]])await p?.screenshot({path:`${output}/expression-failure-${name}.png`}).catch(()=>{});
  throw e;
} finally {await h.close();}

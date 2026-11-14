import assert from 'node:assert/strict';
import {chromium} from 'playwright';
import {readFile,writeFile} from 'node:fs/promises';
import {harness,register,form,act,shot,wait,until,output} from './qa-harness.mjs';
const h=await harness(),checks=[],screenshots=[],errors=[];
let alice,bob,publisher,desktop;
const diagnostic=page=>page.evaluate(async()=>(await import('/public/media.js')).diagnostics());
const message=async(page,text)=>{
  await page.locator('#live-input').fill(text);await act(page,'complete-close');
  await page.getByRole('button',{name:'Send Message',exact:true}).click();
  await page.waitForFunction(()=>!document.querySelector('#live-input')?.value);
};
async function actions(page,id) {
  await page.waitForFunction(()=>!document.querySelector('.live-modal'));
  await page.evaluate(()=>new Promise(done=>requestAnimationFrame(()=>requestAnimationFrame(done))));
  const body=page.locator(`[data-live-message="${id}"] .markdown`);await body.scrollIntoViewIfNeeded();
  // Touch a text glyph, not the author/mention button near the paragraph's edge
  // (Chromium's touch adjustment correctly snaps nearby touches to those links).
  const point=await body.evaluate(root=>{
    const walk=document.createTreeWalker(root,NodeFilter.SHOW_TEXT);let node,last;
    while((node=walk.nextNode()))if(node.textContent.trim()&&!node.parentElement.closest('button,a'))last=node;
    const range=document.createRange();range.selectNodeContents(last);
    const r=[...range.getClientRects()].at(-1);return{x:r.x+r.width/2,y:r.y+r.height/2};
  }),cdp=await page.context().newCDPSession(page);
  await cdp.send('Input.dispatchTouchEvent',{type:'touchStart',touchPoints:[{...point,id:1}]});
  await wait(550);
  await cdp.send('Input.dispatchTouchEvent',{type:'touchEnd',touchPoints:[]});
  await page.waitForSelector('#live-message-menu');
}
try {
  alice=await h.phone(390,844);bob=await h.phone(412,915);
  for(const p of[alice,bob])p.on('pageerror',e=>errors.push(String(e)));
  await register(alice,h.origin,'qa_mobile_alice');await register(bob,h.origin,'qa_mobile_bob');
  await form(alice,'create',{'f-name':'Mobile Test Server'});await alice.waitForSelector('#live-input');
  const channel=await alice.locator('#live-messages').getAttribute('data-channel');
  await form(alice,'invite');await alice.waitForSelector('.live-modal code');
  const code=await alice.locator('.live-modal code').first().textContent();await act(alice,'close');
  await bob.goto(`${h.origin}/?invite=${code}`);
  await bob.waitForSelector('[data-live-form=join]');
  await bob.waitForFunction(code=>document.querySelector('#f-name')?.value===code,code);
  screenshots.push(await shot(bob,'invite-link'));
  await bob.getByRole('button',{name:'Join Server',exact:true}).click();
  await bob.waitForFunction(()=>!document.querySelector('.live-modal'));
  await bob.waitForSelector('#live-input');
  checks.push('phone invite deep link survives session recovery, prefilled confirmation, real join');

  await act(alice,'home');await form(alice,'friend',{'f-name':'qa_mobile_bob'});
  await act(bob,'home');await bob.getByRole('button',{name:'Accept',exact:true}).tap();
  await alice.waitForSelector('[data-live^="friend:dm,"]');
  screenshots.push(await shot(alice,'friends'));
  await alice.locator('[data-live^="friend:dm,"]').tap();await alice.waitForSelector('#live-input');
  await message(alice,'Mobile direct message');
  screenshots.push(await shot(alice,'direct-message'));
  checks.push('phone friend request/accept and DM creation/send');
  await act(alice,`channel:${channel}`);await act(bob,`channel:${channel}`);
  await form(alice,'channel',{'f-name':'off-topic'});
  await alice.waitForFunction(()=>!document.querySelector('.live-modal'));
  await message(bob,'Unread while you are in another channel');
  await alice.waitForSelector(`[data-live="channel:${channel}"] .live-unread`,{state:'attached'});
  await alice.getByRole('button',{name:'Open Channels',exact:true}).tap();
  screenshots.push(await shot(alice,'unread-channel'));
  await act(alice,`channel:${channel}`);await alice.waitForSelector('#live-input');
  checks.push('unread count appears in another channel and drawer navigation returns to its history');
  await alice.locator('#live-input').fill('Typing from my phone');
  await bob.waitForFunction(()=>document.querySelector('.typing-indicator')?.textContent.includes('qa_mobile_alice'));
  await message(alice,'Hello @qa_mobile_bob — **phone markdown**');
  await bob.waitForSelector('.message.mentioned');
  const id=await alice.locator('[data-live-message]').last().getAttribute('data-live-message');
  await actions(alice,id);
  screenshots.push(await shot(alice,'message-actions'));
  await alice.getByRole('button',{name:'Reply',exact:true}).last().tap();
  await message(alice,'Reply sent by touch');await bob.waitForSelector('.message .reply-preview');
  await bob.locator('.message .reply-preview').first().tap();
  await bob.waitForFunction(id=>{const row=document.querySelector(`[data-live-message="${id}"]`),list=document.querySelector('#live-messages');return row&&row.getBoundingClientRect().bottom>list.getBoundingClientRect().top&&row.getBoundingClientRect().top<list.getBoundingClientRect().bottom;},id);
  await act(alice,`react:${id},👍`);await bob.waitForSelector('.reaction');
  await act(alice,`edit:${id}`);await alice.locator('#f-name').fill('Edited phone message');
  await alice.locator('[type=submit]').click();
  await bob.waitForFunction(id=>document.querySelector(`[data-live-message="${id}"] .markdown`)?.textContent.includes('Edited phone message'),id);
  alice.once('dialog',d=>d.accept());
  await actions(alice,id);
  await alice.locator(`#live-message-menu [data-live="delete:${id}"]`).tap();
  await bob.waitForFunction(id=>!document.querySelector(`[data-live-message="${id}"]`),id);
  checks.push('phone typing, explicit mention, touch reply, reaction, edit and deletion');

  await alice.locator('#live-file').setInputFiles({name:'camera-photo.jpg',mimeType:'image/jpeg',buffer:await readFile('public/mock-workspace.jpg')});
  await alice.waitForSelector('.live-upload:has-text("camera-photo.jpg")');await message(alice,'Photo from the phone picker');
  await alice.waitForSelector('[data-view-image]');await alice.locator('[data-view-image]').first().tap();
  await alice.waitForSelector('#live-viewer img');
  screenshots.push(await shot(alice,'photo-viewer'));
  const beforeZoom=await alice.locator('[data-zoom]').textContent();
  const cdp=await alice.context().newCDPSession(alice);
  await cdp.send('Input.dispatchTouchEvent',{type:'touchStart',touchPoints:[{x:160,y:380,id:1},{x:230,y:430,id:2}]});
  await cdp.send('Input.dispatchTouchEvent',{type:'touchMove',touchPoints:[{x:100,y:330,id:1},{x:290,y:480,id:2}]});
  await cdp.send('Input.dispatchTouchEvent',{type:'touchEnd',touchPoints:[]});
  await alice.waitForFunction(before=>document.querySelector('[data-zoom]')?.textContent!==before,beforeZoom);
  await alice.keyboard.press('Escape');
  const video=await alice.evaluate(async()=>{
    const canvas=document.createElement('canvas');canvas.width=320;canvas.height=180;
    canvas.getContext('2d').fillRect(0,0,320,180);
    const stream=canvas.captureStream(5),recorder=new MediaRecorder(stream,{mimeType:'video/webm;codecs=vp8'}),chunks=[];
    recorder.ondataavailable=e=>chunks.push(e.data);
    const done=new Promise(resolve=>recorder.onstop=resolve);recorder.start();await new Promise(r=>setTimeout(r,400));recorder.stop();await done;
    stream.getTracks().forEach(t=>t.stop());return [...new Uint8Array(await new Blob(chunks,{type:'video/webm'}).arrayBuffer())];
  });
  await alice.locator('#live-file').setInputFiles({name:'phone-video.webm',mimeType:'video/webm',buffer:Buffer.from(video)});
  await alice.waitForSelector('.live-upload:has-text("phone-video.webm")');await message(alice,'Inline video');
  await alice.waitForSelector('.attachment-video video');
  const inline=alice.locator('.attachment-video video').first();
  assert(await inline.evaluate(e=>e.playsInline));
  await inline.evaluate(e=>e.play());await alice.waitForFunction(()=>document.querySelector('.attachment-video video')?.videoWidth>0);
  screenshots.push(await shot(alice,'inline-video'));
  checks.push('image/video file picker upload, genuine touch pinch gesture, inline playsinline decoding');

  // The publisher is a second Chromium process; only display acquisition is fake.
  desktop=await chromium.launch({headless:true,args:['--use-fake-device-for-media-stream','--use-fake-ui-for-media-stream','--allow-loopback-in-peer-connection','--disable-features=WebRtcHideLocalIpsWithMdns','--mute-audio','--disable-notifications','--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1, EXCLUDE localhost']});
  const context=await desktop.newContext({viewport:{width:1440,height:900},permissions:['microphone','camera']});
  await context.route('**/*',route=>['127.0.0.1','localhost','[::1]'].includes(new URL(route.request().url()).hostname)?route.continue():route.abort('blockedbyclient'));
  publisher=await context.newPage();publisher.on('pageerror',e=>errors.push(String(e)));
  await publisher.addInitScript(()=>{
    navigator.mediaDevices.getDisplayMedia=async()=>{
      const stream=await navigator.mediaDevices.getUserMedia({video:{width:1280,height:720},audio:false});
      const track=stream.getVideoTracks()[0],settings=track.getSettings.bind(track);
      track.getSettings=()=>({...settings(),displaySurface:'monitor'});return stream;
    };
  });
  await register(publisher,h.origin,'qa_mobile_streamer');await form(publisher,'join',{'f-name':code});
  await publisher.waitForSelector('#live-input');
  await form(alice,'channel',{'f-name':'Mobile Voice','f-extra':'voice'});
  const voice=await alice.locator('[data-live^="voice:"]:has-text("Mobile Voice")').getAttribute('data-live');
  await act(publisher,voice);
  await until(async()=>(await diagnostic(alice)).state==='connected',45000);
  await until(async()=>(await diagnostic(publisher)).state==='connected',45000);
  await act(publisher,'media:share');await publisher.locator('.live-share-options button').first().click();
  await alice.waitForFunction(()=>[...document.querySelectorAll('[data-kind=screen] video')].some(v=>v.videoWidth>0&&v.readyState>=2),null,{timeout:45000});
  await until(async()=>(await diagnostic(alice)).stats.some(r=>r.type==='inbound-rtp'&&r.kind==='video'&&r.framesDecoded>=10),45000);
  screenshots.push(await shot(alice,'screen-share-phone'));
  const stats=await diagnostic(alice);
  assert(stats.stats.some(r=>r.type==='inbound-rtp'&&r.kind==='video'&&r.framesDecoded>=10));
  await writeFile(`${output}/phone-share-stats.json`,JSON.stringify({state:stats.state,tracks:stats.tracks,inbound:stats.stats.filter(r=>r.type==='inbound-rtp')},null,2));
  await alice.getByRole('button',{name:'Mute',exact:true}).last().tap();
  await alice.getByRole('button',{name:'Deafen',exact:true}).last().tap();
  await alice.getByRole('button',{name:'Leave Call',exact:true}).tap();await act(publisher,'media:leave');
  await until(async()=>(await diagnostic(alice)).publications===0);
  await wait(300);
  assert.equal(await alice.locator('.live-error').count(),0,'no stale signaling error after Leave');
  assert.equal(await publisher.locator('.live-error').count(),0,'publisher leaves cleanly');
  checks.push('phone joins real SFU, watches decoded synthetic screen share from second process, mute/deafen/leave releases media');
  const permissionCdp=await bob.context().newCDPSession(bob);
  const {targetInfo}=await permissionCdp.send('Target.getTargetInfo');
  const microphone=setting=>permissionCdp.send('Browser.setPermission',{
    permission:{name:'microphone'},setting,origin:h.origin,browserContextId:targetInfo.browserContextId,
  });
  await microphone('denied');
  // This Chromium's fake-ui flag bypasses denied capture permissions. Without
  // it the bundled fake audio device returns NotSupportedError. Inject only
  // that browser failure edge; positive calls still use actual fake-device RTP.
  await bob.evaluate(()=>{
    const capture=navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);
    navigator.mediaDevices.getUserMedia=async constraints=>{
      if(constraints.audio&&(await navigator.permissions.query({name:'microphone'})).state==='denied')
        throw new DOMException('Permission denied','NotAllowedError');
      return capture(constraints);
    };
  });
  await act(bob,voice);
  await bob.waitForSelector('.live-error:has-text("Microphone access was denied")');
  screenshots.push(await shot(bob,'microphone-denied'));
  const denied=await diagnostic(bob);assert.equal(denied.joined,false);assert.equal(denied.publications,0);
  await microphone('granted');
  await act(bob,'dismiss-error');await act(bob,voice);
  await until(async()=>(await diagnostic(bob)).state==='connected',45000);
  await act(bob,'media:leave');
  checks.push('injected denied-microphone browser edge explains recovery and releases media; grant and real SFU rejoin succeeds');
  assert.deepEqual(errors,[]);
  await writeFile(`${output}/phone-media-results.json`,JSON.stringify({revision:h.revision,checks,screenshots,errors},null,2));
  console.log(JSON.stringify({checks,screenshots:screenshots.length,errors}));
} catch(e) {
  await writeFile(`${output}/phone-media-failure.json`,JSON.stringify({error:String(e),checks,errors,body:await alice?.locator('body').innerText().catch(()=>'' ),publisherBody:await publisher?.locator('body').innerText().catch(()=>'' )},null,2));
  await alice?.screenshot({path:`${output}/phone-media-failure.png`}).catch(()=>{});throw e;
} finally {await desktop?.close();await h.close();}

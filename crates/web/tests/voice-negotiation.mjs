import {swipe} from './gestures.mjs';
import {device} from './engines.mjs';
// Repeated real SFU receiver activation, including reuse of inactive video MIDs.
import assert from 'node:assert/strict';
import {writeFile} from 'node:fs/promises';
import {chromium} from 'playwright';
import {harness,register,form,act,until,output,shot} from './qa-harness.mjs';
const h=await harness(),rounds=[];let desktop,alice,bob,stream;
const diagnostic=p=>p.evaluate(async()=>await(await import('/public/media.js')).diagnostics());
const decoded=d=>d.stats.filter(r=>r.type==='inbound-rtp'&&r.kind==='video').reduce((n,r)=>n+(r.framesDecoded??0),0);
try {
  alice=await h.phone();
  desktop=await chromium.launch({headless:true,args:['--use-fake-device-for-media-stream','--use-fake-ui-for-media-stream','--allow-loopback-in-peer-connection','--disable-features=WebRtcHideLocalIpsWithMdns','--mute-audio','--disable-notifications','--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1, EXCLUDE localhost']});
  const context=await desktop.newContext({viewport:{width:1440,height:900},permissions:['microphone','camera']});
  const loopback=r=>['127.0.0.1','localhost','[::1]'].includes(new URL(r.request().url()).hostname)?r.continue():r.abort();
  await context.route('**/*',loopback);
  bob=await context.newPage();
  const remoteDelay=Number(process.env.STICKTALK_QA_REMOTE_DELAY_MS??7500);
  if(remoteDelay)await bob.addInitScript(delay=>{
    const remote=RTCPeerConnection.prototype.setRemoteDescription;
    RTCPeerConnection.prototype.setRemoteDescription=async function(description) {
      if(description.sdp.includes('m=video'))await new Promise(done=>setTimeout(done,delay));
      return remote.call(this,description);
    };
  },remoteDelay);
  await register(alice,h.origin,'qa_voice_alice');await register(bob,h.origin,'qa_voice_bob');
  await form(alice,'create',{'f-name':'Receiver Activation'});await alice.waitForSelector('#live-input');
  await form(alice,'invite');await alice.waitForSelector('.live-modal code');
  const code=await alice.locator('.live-modal code').first().textContent();await act(alice,'close');
  await form(bob,'join',{'f-name':code});await bob.waitForSelector('#live-input');
  await form(alice,'channel',{'f-name':'Receiver activation','f-extra':'voice'});
  const voice=await alice.locator('[data-live^="voice:"]:has-text("Receiver activation")').getAttribute('data-live');await act(bob,voice);
  await until(async()=>(await Promise.all([diagnostic(alice),diagnostic(bob)])).every(d=>d.state==='connected'),45000);
  for(let round=1;round<=3;round++) {
    const before=await Promise.all([diagnostic(alice),diagnostic(bob)]),base=before.map(decoded);
    await Promise.all([act(alice,'media:camera'),act(bob,'media:camera')]);
    await until(async()=>(await Promise.all([diagnostic(alice),diagnostic(bob)])).every((d,i)=>decoded(d)>=base[i]+10&&d.tracks.some(t=>t.kind==='camera'&&!t.own&&t.videoWidth>0&&t.readyState>=2)),20000+remoteDelay*2);
    // Resolution may adapt to the SFU bitrate budget; require actual decoded
    // frame progress, and report the negotiated result rather than assume 720p.
    const active=await Promise.all([diagnostic(alice),diagnostic(bob)]);
    for(const d of active) {
      const rtx=process.env.STICKTALK_QA_EXPECT_RTX!=='0';
      assert.equal(/a=rtpmap:\d+ rtx\/90000/i.test(d.localSdp),rtx);
      assert.equal(/a=rtpmap:\d+ rtx\/90000/i.test(d.remoteSdp),rtx);
    }

    if(round===1) {
      assert(await alice.locator('.call-controls .icon-button').evaluateAll(nodes=>nodes.filter(e=>e.getClientRects().length).every(e=>e.getBoundingClientRect().height>=48)));
      assert(await alice.locator('.call-controls [data-live="media:leave"]').evaluate(e=>e.getBoundingClientRect().width>=80));
      await alice.locator('.participant-tile[data-kind=camera]:not([data-self])').click();
      await alice.waitForSelector('.participant-grid.has-focus .is-focused');
      for(const [width,height] of (process.env.STICKTALK_QA_ENGINE ? [[device.viewport.width,device.viewport.height]] : [[390,844],[412,915],[1440,900]])){await alice.setViewportSize({width,height});await shot(alice,'voice-focus');}
      await alice.setViewportSize({width:390,height:844});
      await swipe(alice,{x:190,y:85},{x:190,y:240});
      await alice.waitForSelector('#live-call-mini');await alice.waitForSelector('#live-input');
      assert.equal((await diagnostic(alice)).publications,2,'swipe minimization must not end camera');
      for(const [width,height] of (process.env.STICKTALK_QA_ENGINE ? [[device.viewport.width,device.viewport.height]] : [[390,844],[412,915],[1440,900]])){await alice.setViewportSize({width,height});await shot(alice,'voice-mini');}
      await alice.setViewportSize({width:390,height:844});
      await alice.locator('#live-call-mini > button').first().click();await alice.waitForSelector('.voice-view');
      assert.equal(await alice.locator('#live-call-mini').count(),0);
      // On a desktop, leaving the call view keeps the other camera on screen in
      // the browser's picture-in-picture window; returning takes it back.
      const tile='.participant-tile[data-kind=camera]:not([data-self])';
      await bob.locator('.channel-header [aria-label="Open Chat"]').click();
      await bob.waitForSelector('#live-call-mini');
      await until(()=>bob.evaluate(tile=>document.pictureInPictureElement?.matches(`${tile} video`)??false,tile));
      assert(await bob.locator('#live-call-mini .call-mini-popout').isVisible());
      await bob.locator('#live-call-mini > button').first().click();await bob.waitForSelector('.voice-view');
      await until(()=>bob.evaluate(()=>!document.pictureInPictureElement));
      assert.equal(await bob.locator('#live-call-park').count(),0);
      assert.equal(await bob.locator(`#live-media-stage ${tile}`).count(),1);
      // Pop Out: the same picture in a window of its own, which can go full screen.
      // Playwright never answers a request it intercepts from a blank window opened
      // by a page, so the window's stylesheets load with interception off. The
      // launch flags above still keep every other host unreachable meanwhile.
      await context.unroute('**/*',loopback);
      [stream]=await Promise.all([context.waitForEvent('page'),bob.locator(`${tile} .tile-popout`).click()]);
      await until(()=>stream.evaluate(()=>document.querySelector('video')?.videoWidth>0));
      assert.equal(await stream.title(),await bob.locator(`${tile} .participant-label > span`).first().textContent());
      // Styled by the app's own stylesheet: the video fills the window, the name and button lie over it.
      await stream.setViewportSize({width:900,height:600});
      await until(()=>stream.evaluate(()=>{const video=document.querySelector('video').getBoundingClientRect(),bar=document.querySelector('.stream-window-bar').getBoundingClientRect();return getComputedStyle(document.body).backgroundColor==='rgb(0, 0, 0)'&&video.width===innerWidth&&video.height===innerHeight&&bar.bottom===innerHeight;}));
      await context.route('**/*',loopback);
      assert.equal(await stream.locator('.stream-window-bar > span').textContent(),await stream.title());
      // Headless Chromium refuses full screen in a second window, so only the button is checked.
      assert.equal(await stream.locator('.stream-window-bar button').getAttribute('aria-label'),'Full Screen');
      // With the stream in its own window, leaving the call view opens no second copy.
      await bob.locator('.channel-header [aria-label="Open Chat"]').click();
      await bob.waitForSelector('#live-call-mini');
      assert(await bob.locator('#live-call-mini .call-mini-popout').isHidden());
      assert.equal(await bob.evaluate(()=>!!document.pictureInPictureElement),false);
      await bob.locator('#live-call-mini > button').first().click();await bob.waitForSelector('.voice-view');
    }

    rounds.push({round,decodedDelta:active.map((d,i)=>decoded(d)-base[i]),states:active.map(d=>d.state),remoteWidths:active.map(d=>d.tracks.find(t=>t.kind==='camera'&&!t.own)?.videoWidth)});
    await Promise.all([act(alice,'media:camera'),act(bob,'media:camera')]);
    await until(async()=>(await Promise.all([diagnostic(alice),diagnostic(bob)])).every(d=>d.publications===1&&d.subscriptions===1));
    // The camera it showed is off: the stream's window closes by itself.
    if(round===1)await until(()=>stream.isClosed());
  }
  await Promise.all([act(alice,'media:leave'),act(bob,'media:leave')]);
  await until(async()=>(await Promise.all([diagnostic(alice),diagnostic(bob)])).every(d=>!d.joined&&d.publications===0&&d.subscriptions===0&&d.tracks.length===0&&d.pendingRequests===0));
  assert.equal(rounds.length,3);
  await writeFile(`${output}/voice-negotiation-results.json`,JSON.stringify({revision:h.revision,realServer:true,independentBrowserProcesses:2,remoteAnswerDelayMs:remoteDelay,rounds},null,2));
  console.log(JSON.stringify({rounds,cleanup:true}));
}catch(e){
  await writeFile(`${output}/voice-negotiation-failure.json`,JSON.stringify({error:String(e),rounds,diagnostics:await Promise.all([alice,bob].filter(Boolean).map(p=>diagnostic(p).catch(()=>null)))},null,2));throw e;
}finally{await desktop?.close();await h.close();}

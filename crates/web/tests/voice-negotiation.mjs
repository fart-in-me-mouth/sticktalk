// Repeated real SFU receiver activation, including reuse of inactive video MIDs.
import assert from 'node:assert/strict';
import {writeFile} from 'node:fs/promises';
import {chromium} from 'playwright';
import {harness,register,form,act,until,output} from './qa-harness.mjs';
const h=await harness(),rounds=[];let desktop,alice,bob;
const diagnostic=p=>p.evaluate(async()=>await(await import('/public/media.js')).diagnostics());
const decoded=d=>d.stats.filter(r=>r.type==='inbound-rtp'&&r.kind==='video').reduce((n,r)=>n+(r.framesDecoded??0),0);
try {
  alice=await h.phone();
  desktop=await chromium.launch({headless:true,args:['--use-fake-device-for-media-stream','--use-fake-ui-for-media-stream','--allow-loopback-in-peer-connection','--disable-features=WebRtcHideLocalIpsWithMdns','--mute-audio','--disable-notifications','--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1, EXCLUDE localhost']});
  const context=await desktop.newContext({viewport:{width:1440,height:900},permissions:['microphone','camera']});
  await context.route('**/*',r=>['127.0.0.1','localhost','[::1]'].includes(new URL(r.request().url()).hostname)?r.continue():r.abort());
  bob=await context.newPage();
  await register(alice,h.origin,'qa_voice_alice');await register(bob,h.origin,'qa_voice_bob');
  await form(alice,'create',{'f-name':'Receiver Activation'});await alice.waitForSelector('#live-input');
  await form(alice,'invite');await alice.waitForSelector('.live-modal code');
  const code=await alice.locator('.live-modal code').first().textContent();await act(alice,'close');
  await form(bob,'join',{'f-name':code});await bob.waitForSelector('#live-input');
  await form(alice,'channel',{'f-name':'Receiver activation','f-extra':'voice'});
  const voice=await alice.locator('[data-live^="voice:"]').first().getAttribute('data-live');await act(bob,voice);
  await until(async()=>(await Promise.all([diagnostic(alice),diagnostic(bob)])).every(d=>d.state==='connected'),45000);
  for(let round=1;round<=3;round++) {
    const before=await Promise.all([diagnostic(alice),diagnostic(bob)]),base=before.map(decoded);
    await Promise.all([act(alice,'media:camera'),act(bob,'media:camera')]);
    await until(async()=>(await Promise.all([diagnostic(alice),diagnostic(bob)])).every((d,i)=>decoded(d)>=base[i]+10&&d.tracks.some(t=>t.kind==='camera'&&!t.own&&t.videoWidth>0&&t.readyState>=2)),20000);
    // Resolution may adapt to the SFU bitrate budget; require actual decoded
    // frame progress, and report the negotiated result rather than assume 720p.
    const active=await Promise.all([diagnostic(alice),diagnostic(bob)]);
    rounds.push({round,decodedDelta:active.map((d,i)=>decoded(d)-base[i]),states:active.map(d=>d.state),remoteWidths:active.map(d=>d.tracks.find(t=>t.kind==='camera'&&!t.own)?.videoWidth)});
    await Promise.all([act(alice,'media:camera'),act(bob,'media:camera')]);
    await until(async()=>(await Promise.all([diagnostic(alice),diagnostic(bob)])).every(d=>d.publications===1&&d.subscriptions===1));
  }
  await Promise.all([act(alice,'media:leave'),act(bob,'media:leave')]);
  await until(async()=>(await Promise.all([diagnostic(alice),diagnostic(bob)])).every(d=>!d.joined&&d.publications===0&&d.subscriptions===0&&d.tracks.length===0&&d.pendingRequests===0));
  assert.equal(rounds.length,3);
  await writeFile(`${output}/voice-negotiation-results.json`,JSON.stringify({revision:h.revision,realServer:true,independentBrowserProcesses:2,rounds},null,2));
  console.log(JSON.stringify({rounds,cleanup:true}));
}catch(e){
  await writeFile(`${output}/voice-negotiation-failure.json`,JSON.stringify({error:String(e),rounds,diagnostics:await Promise.all([alice,bob].filter(Boolean).map(p=>diagnostic(p).catch(()=>null)))},null,2));throw e;
}finally{await desktop?.close();await h.close();}

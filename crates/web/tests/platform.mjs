// Service worker contract test: mocked notification surfaces, no OS notifications.
import { readFile } from "node:fs/promises";
import vm from "node:vm";
import assert from "node:assert/strict";
const handlers = new Map(),
  notifications = [],
  opened = [],
  navigated = [];
let clients = [];
const self = {
  location: { origin: "https://sticktalk.test" },
  registration: {
    showNotification: async (...args) => notifications.push(args),
  },
  clients: {
    matchAll: async () => clients,
    openWindow: async (url) => opened.push(url),
  },
  addEventListener: (type, handler) => handlers.set(type, handler),
};
vm.runInNewContext(await readFile("public/push-worker.js", "utf8"), {
  self,
  URL,
});
async function trigger(type, event) {
  let promise;
  handlers.get(type)({ ...event, waitUntil: (p) => (promise = p) });
  await promise;
}
await trigger("push", {
  data: {
    json: () => ({
      url: "/?channel=9223372036854775000&message=9223372036854774999",
      body: "DO NOT EXPOSE THIS",
    }),
  },
});
assert.equal(notifications.length, 1);
assert.equal(notifications[0][0], "StickTalk");
assert.equal(notifications[0][1].body, "You have unread messages.");
await trigger("push", {
  data: { json: () => ({ url: "https://attacker.test/steal" }) },
});
assert.equal(notifications.length, 1);
let closed = false;
await trigger("notificationclick", {
  notification: {
    close: () => (closed = true),
    data: notifications[0][1].data,
  },
});
assert(closed);
assert.equal(
  opened[0],
  "https://sticktalk.test/?channel=9223372036854775000&message=9223372036854774999",
);
let focused = false;
clients = [
  {
    url: "https://sticktalk.test/",
    navigate: async (url) => navigated.push(url),
    focus: async () => (focused = true),
  },
];
await trigger("notificationclick", {
  notification: { close() {}, data: notifications[0][1].data },
});
assert(focused);
assert.equal(navigated[0], opened[0]);
console.log(
  "Push worker: generic lock-screen content, exact decimal deep link, same-origin validation, reuse/focus existing window: passed",
);

// The SFU changes forwarding SSRCs after Subscribe. A stale SDP SSRC can decode
// RTP in Chromium without ever delivering frames to its HTML video track.
const { normalizeAnswer } = await import("../public/media.js");
const { normalizeOffer } = await import("../public/media.js");
const browserOffer='v=0\r\na=fingerprint:sha-256 KEEP\r\na=candidate:udp 1 udp 123 127.0.0.1 5000 typ host\r\na=candidate:tcp 1 tcp 122 127.0.0.1 9 typ host tcptype active\r\n';
assert(normalizeOffer(browserOffer).includes('a=candidate:udp'));
assert(!normalizeOffer(browserOffer).includes('a=candidate:tcp'));
assert(normalizeOffer(browserOffer).includes('a=fingerprint:sha-256 KEEP'));
const firefoxOffer=browserOffer+'a=candidate:rtcp 2 UDP 121 127.0.0.1 5001 typ host\r\n';
assert(!normalizeOffer(firefoxOffer).includes('a=candidate:rtcp'));
assert(normalizeOffer(firefoxOffer).includes('a=candidate:udp'));
const answer =
  "v=0\r\na=msid-semantic: WMS old\r\na=fingerprint:sha-256 KEEP\r\nm=video 9 UDP/TLS/RTP/SAVPF 102\r\na=mid:3\r\na=ssrc:42 cname:obsolete\r\na=ssrc-group:FID 42 43\r\n";
const normalized = normalizeAnswer(answer);
assert(!normalized.includes("a=ssrc"));
assert(normalized.includes("a=msid-semantic: WMS *"));
assert(normalized.includes("a=fingerprint:sha-256 KEEP"));
assert(normalized.includes("a=mid:3"));
console.log("Browser SDP dynamic-SSRC/MSID compatibility regression: passed");
const {retainSpeaking}=await import('../public/media.js');
const active=new Set(Array.from({length:64},(_,i)=>String(i)));
retainSpeaking(active,'0',true);
assert(active.has('0'));
retainSpeaking(active,'overflow',true);
assert.equal(active.size,64);
retainSpeaking(active,'0',false);
assert(!active.has('0'));
console.log('Speaking state: bounded, retains known active speakers at capacity, releases on silence: passed');

const {normalizeVolumes,displayAudio}=await import('../public/media.js');
assert.deepEqual(normalizeVolumes([['42:voice',{volume:.4,muted:true}],['42:stream',{volume:7}],['42:stream',{volume:'x'}]]),
  [['42:voice',{volume:.4,muted:true}],['42:stream',{volume:2,muted:false}],['42:stream',{volume:1,muted:false}]]);
assert.deepEqual(normalizeVolumes([['__proto__',{}],['42:other',{}],'42:voice',null,['42:voice',null]]),[['42:voice',{volume:1,muted:false}]]);
assert.deepEqual(normalizeVolumes({length:1}),[]);
assert.equal(normalizeVolumes(Array.from({length:200},(_,i)=>[`${i}:voice`,{volume:0}])).length,64);
assert.deepEqual(displayAudio({restrictOwnAudio:true}),{echoCancellation:false,noiseSuppression:false,autoGainControl:false,restrictOwnAudio:true});
assert.deepEqual(displayAudio(undefined),{echoCancellation:false,noiseSuppression:false,autoGainControl:false});
// Remote control messages: the exact MessagePack the server's rmp-serde reads and writes.
const {packControl,unpackControl,KEY_USAGE,controlPoint}=await import('../public/media.js');
const moved={type:'input',data:{sharer:{user_id:249941506784231424n,connection_id:7n},grant:18446744073709551615n,sequence:300,input:{type:'move',data:{x:65535,y:0}}}};
const packed=packControl(moved);
assert.equal(Buffer.from(packed.subarray(0,12)).toString('latin1'),'¤type¥input','a map of two: type first');
assert.deepEqual(unpackControl(packed),{type:'input',data:{sharer:{user_id:'249941506784231424',connection_id:7},grant:'18446744073709551615',sequence:300,input:{type:'move',data:{x:65535,y:0}}}});
assert.deepEqual(unpackControl(packControl({type:'wheel',data:{x:-4096,y:-3}})),{type:'wheel',data:{x:-4096,y:-3}});
assert.deepEqual(unpackControl(packControl({type:'key',data:{usage:0xe5,down:false}})),{type:'key',data:{usage:229,down:false}});
assert.throws(()=>unpackControl(new Uint8Array([0x81,0xa1,0x61])),'truncated input is refused');
assert.throws(()=>unpackControl(new Uint8Array([0x01,0x02])),'trailing bytes are refused');
assert.throws(()=>unpackControl(new Uint8Array(600)),'oversized input is refused');
assert.deepEqual([KEY_USAGE.KeyA,KEY_USAGE.Digit0,KEY_USAGE.F12,KEY_USAGE.ShiftRight,KEY_USAGE.MetaLeft],[4,39,69,0xe5,undefined]);
// A 1920x1080 picture letterboxed in a 1000x1000 box: only the picture maps, edge to edge.
const box={left:100,top:50,width:1000,height:1000};
assert.deepEqual(controlPoint(box,1920,1080,100,50+218.75),{x:0,y:0});
assert.deepEqual(controlPoint(box,1920,1080,1100,50+781.25),{x:65535,y:65535});
assert.equal(controlPoint(box,1920,1080,600,60),null,'the letterbox is not part of the screen');
assert.equal(controlPoint(box,0,0,600,500),null);
console.log('Remote control: MessagePack round trip, key map without the Windows key, letterbox-aware pointer mapping: passed');
console.log('Call volumes: bounded, validated device-only values; stream capture leaves out own audio where supported: passed');

const {normalizePreferences}=await import('../public/preferences.js');
// The display basics; the themes work added more fields (themes.md), each normalized the same way.
const basics=(p)=>({density:p.density,fontScale:p.fontScale,underlineLinks:p.underlineLinks,linkPreviews:p.linkPreviews});
const defaults=normalizePreferences(null);
assert.deepEqual(basics(defaults),{density:'cozy',fontScale:1,underlineLinks:false,linkPreviews:true});
assert.deepEqual(normalizePreferences(defaults),defaults,'normalizing is stable');
assert.deepEqual(basics(normalizePreferences({density:'compact',fontScale:'1.25',underlineLinks:true})),{density:'compact',fontScale:1.25,underlineLinks:true,linkPreviews:true});
const injected=normalizePreferences({density:'injected',fontScale:Infinity,underlineLinks:'true',token:'not retained',theme:'<style>',zoom:9});
assert.deepEqual(injected,defaults,'unknown values fall back and unknown keys are not kept');
const {swipeAction}=await import('../public/navigation.js');
assert.equal(swipeAction('',120,4),'panel:left');
assert.equal(swipeAction('',-120,4),'panel:right');
assert.equal(swipeAction('left',-120,4),'panel:');
assert.equal(swipeAction('right',120,4),'panel:');
assert.equal(swipeAction('left',120,4),null);
assert.equal(swipeAction('',120,150),null);
assert.equal(swipeAction('',30,4),null);
console.log('Mobile preferences: bounded device-only values; swipes: drawer dismissal, vertical/short gesture rejection: passed');

// H.264 preferences must retain its auxiliary repair codec. str0m needs an
// RTX stream for padding probes; H.264-only primary preferences disabled it.
const {h264Codecs}=await import('../public/media.js');
const codecs=[
  {mimeType:'video/VP8'},
  {mimeType:'video/H264',sdpFmtpLine:'packetization-mode=1;profile-level-id=4d001f'},
  {mimeType:'video/rtx',clockRate:90000},
  {mimeType:'video/H264',sdpFmtpLine:'packetization-mode=1;profile-level-id=42e01f'},
  {mimeType:'video/H264',sdpFmtpLine:'packetization-mode=0;profile-level-id=42e01f'},
];
assert.deepEqual(h264Codecs(codecs),[codecs[3],codecs[1],codecs[2]]);
assert.throws(()=>h264Codecs([{mimeType:'video/VP8'},{mimeType:'video/rtx'}]),/H.264/);
console.log('H.264 codec selection: baseline first, RTX repair/probes retained, no unrelated primary codec: passed');

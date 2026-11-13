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

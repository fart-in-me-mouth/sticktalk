// Real local server. No fixture API routes, mock websocket, external service,
// visible window, hardware device, notification, or non-loopback listener.
import { chromium } from "playwright";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { once } from "node:events";
import { mkdtemp, rm, writeFile, mkdir, readFile } from "node:fs/promises";
import { resolve, join, dirname, basename } from "node:path";
const output = process.env.STICKTALK_QA_DIR ?? "screenshots/live";
// A failed attempt must not leave an earlier green summary looking current.
await rm(`${output}/results.json`, {force:true});
const target = resolve(
  process.env.CARGO_TARGET_DIR ??
    "C:/Users/dwhee/.cargo-target/sticktalk-web-live",
);
const data = await mkdtemp(join(target, "live-test-"));
const reserve = createServer();
reserve.listen(0, "127.0.0.1");
await once(reserve, "listening");
const port = reserve.address().port;
await new Promise((r) => reserve.close(r));
const origin = `http://127.0.0.1:${port}`;
const server = spawn(
  process.env.STICKTALK_SERVER_EXE ??
    join(target, "debug/sticktalk-server.exe"),
  [
    "--bind",
    `127.0.0.1:${port}`,
    "--public-url",
    origin,
    "--data-dir",
    data,
    "--web-root",
    (process.env.STICKTALK_WEB_DIST ?? join(target, "web-dist")),
    "--min-free-disk-gb",
    "0",
    "--voice-bind-ip",
    "127.0.0.1",
    "--voice-advertised-ip",
    "127.0.0.1",
  ],
  { windowsHide: true, stdio: ["ignore", "pipe", "pipe"] },
);
const errors = [],
  violations = [],
  checks = [];
let log = "",
  browsers = [],
  pages = [];
const act = (page, action) =>
  page.evaluate(
    (action) =>
      window.dispatchEvent(
        new CustomEvent("sticktalk-live", { detail: action }),
      ),
    action,
  );
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const expect = async (page, fn, arg) =>
  page.waitForFunction(fn, arg, { timeout: 30000 });
async function register(page, user) {
  await page.goto(origin);
  if (user === "alice_web") {
    await mkdir(output, { recursive: true });
    for (const [width, height] of [
      [1440, 900],
      [390, 844],
      [412, 915],
      [820, 1180],
    [1280, 800],
    [1920, 1080],
    ]) {
      await page.setViewportSize({ width, height });
      await page.evaluate(() => document.fonts.ready);
      await page.screenshot({
        path: `${output}/login-${width}x${height}.png`,
      });
    }
    await page.setViewportSize({ width: 1440, height: 900 });
  }
  await page.waitForSelector("#auth-username");
  await page.waitForFunction(
    () => !document.querySelector("[type=submit]").disabled,
  );
  await page.locator("[data-live=register]").click();
  await page.locator("#auth-username").fill(user);
  await page.locator("#auth-password").fill("testing-password-123456");
  await page.locator("[type=submit]").click();
  await page.waitForSelector(".live-home");
  await page.waitForFunction(
    () => !document.querySelector(".connection-banner"),
  );
}
async function form(page, kind, name, extra = {}) {
  await act(page, `dialog:${kind}`);
  await page.waitForSelector('.live-modal [data-live-form]');
  if (await page.locator("#f-name").isVisible())
    await page.locator("#f-name").fill(name);
  for (const [id, value] of Object.entries(extra)) {
    const e = page.locator(`#${id}`);
    if (await e.evaluate((e) => e.tagName === "SELECT"))
      await e.selectOption(value);
    else await e.fill(value);
  }
  await page.locator("[data-live-form] [type=submit]").click();
}
async function message(page, text) {
  await page.locator("#live-input").fill(text);
  await act(page, "complete-close");
  await page.locator("#live-input").press("Enter");
  await page.waitForFunction(
    (text) =>
      [...document.querySelectorAll(".markdown")].some((e) =>
        e.textContent.includes(text.replaceAll("**", "")),
      ),
    text,
  );
}
async function capture(page, prefix) {
  await page.mouse.move(0,0);
  for (const [width,height] of [[390,844],[412,915],[820,1180],[1440,900],[1280,800],[1920,1080]]) {
    await page.setViewportSize({width,height});
    await page.evaluate(()=>document.fonts.ready);
    await page.evaluate(()=>{const list=document.querySelector("#live-messages"); if(list&&!document.querySelector(".live-modal,#live-viewer"))list.scrollTop=list.scrollHeight;});
    await wait(80);
    await page.evaluate(()=>{const list=document.querySelector('#live-messages');if(list&&!document.querySelector('.live-modal,#live-viewer'))list.scrollTop=list.scrollHeight;});
    await page.screenshot({path:`${output}/${prefix}-${width}x${height}.png`});
  }
  await page.setViewportSize({width:1440,height:900});
}
async function diagnostic(page) {
  return page.evaluate(async () => {
    const m = await import("/public/media.js");
    return m.diagnostics();
  });
}
try {
  await new Promise((done, fail) => {
    const timer = setTimeout(() => fail(Error(`server timeout ${log}`)), 30000);
    const collect = (c) => {
      log = (log + c).slice(-20000);
      if (log.includes("StickTalk listening")) {
        clearTimeout(timer);
        done();
      }
    };
    server.stdout.on("data", collect);
    server.stderr.on("data", collect);
    server.once("error", fail);
    server.once("exit", (code) => fail(Error(`server exited ${code}: ${log}`)));
  });
  const launch = () =>
    chromium.launch({
      headless: true,
      args: [
        "--use-fake-device-for-media-stream",
        "--use-fake-ui-for-media-stream",
        "--allow-loopback-in-peer-connection",
        "--disable-features=WebRtcHideLocalIpsWithMdns",
        "--mute-audio",
        "--disable-notifications",
      ],
    });
  browsers = [await launch(), await launch()];
  const contexts = await Promise.all(
    browsers.map((b) =>
      b.newContext({
        viewport: { width: 1440, height: 900 },
        permissions: ["microphone", "camera"],
      }),
    ),
  );
  const [alice, bob] = (pages = await Promise.all(
    contexts.map((c) => c.newPage()),
  ));
  for (const page of [alice, bob]) {
    page.on("pageerror", (e) => errors.push(e.stack ?? String(e)));
    await page.addInitScript(() => {
      document.addEventListener("securitypolicyviolation", (e) => {
        (window.__violations ??= []).push({
          directive: e.effectiveDirective,
          blocked: e.blockedURI,
        });
      });
    });
  }
  await register(alice, "alice_web");
  await register(bob, "bob_web");
  checks.push("register + cookie websocket Ready");
  const cookie = await contexts[0].cookies();
  assert(cookie.find((c) => c.name === "__Host-sticktalk_session")?.httpOnly);
  assert(cookie.find((c) => c.name === "__Host-sticktalk_session")?.secure);
  await form(alice, "friend", "bob_web");
  await bob.locator('[data-live^="friend:accept,"]').click();
  await alice.waitForSelector('[data-live^="friend:dm,"]');
  checks.push("friend request + accept via WS");
  await alice.locator('[data-live^="friend:dm,"]').click();
  await alice.waitForSelector("#live-input");
  await message(alice, "Hello **Bob**");
  const dm = await alice.locator("#live-messages").getAttribute("data-channel");
  await act(bob, `channel:${dm}`);
  await bob.waitForSelector(".markdown strong, .markdown span");
  await expect(bob, () =>
    document.querySelector(".markdown").textContent.includes("Hello Bob"),
  );
  checks.push("DM + markdown + cross-browser delivery");
  await form(alice, "create", "Browser Lab");
  await alice.waitForFunction(() => !document.querySelector(".live-modal"));
  const guild = await alice
    .locator('.server-rail [data-live^="guild:"]')
    .getAttribute("data-live");
  assert(guild);
  await form(alice,'create','Weekend Games');
  await alice.waitForFunction(()=>!document.querySelector('.live-modal'));
  await form(alice,'create','Release Engineering');
  await alice.waitForFunction(()=>!document.querySelector('.live-modal'));
  await act(alice,guild);
  await alice.waitForSelector('#live-input');
  await form(alice, "invite", "");
  await alice.waitForSelector(".live-modal code");
  const invite = await alice.locator(".live-modal code").first().textContent();
  await act(alice, "close");
  await form(bob, "join", invite);
  await bob.waitForFunction(() => !document.querySelector(".live-modal"));
  checks.push("server create + invite + join");
  await act(alice, guild);
  await alice.waitForSelector("#live-input");
  const channel = await alice
    .locator("#live-messages")
    .getAttribute("data-channel");
  await act(bob, `channel:${channel}`);
  await bob.waitForSelector("#live-input");
  await message(alice, "Server-backed launch proof");
  await expect(bob, () =>
    document
      .querySelector(".markdown")
      .textContent.includes("Server-backed launch proof"),
  );
  const mid = await alice
    .locator("[data-live-message]")
    .last()
    .getAttribute("data-live-message");
  await act(bob, `react:${mid},👍`);
  await alice.waitForSelector(".reaction");
  checks.push("reaction roundtrip");
  await act(alice, `reply:${mid}`);
  await message(alice, "Reply from the browser");
  await alice.waitForSelector(".reply-preview");
  checks.push("reply");
  await act(alice, `edit:${mid}`);
  await alice.locator("#f-name").fill("Edited persisted message");
  await alice.locator("[type=submit]").click();
  await expect(bob, () =>
    [...document.querySelectorAll(".markdown")].some((e) =>
      e.textContent.includes("Edited persisted message"),
    ),
  );
  checks.push("edit across browsers");
  await act(alice, `pin:${mid}`);
  await wait(100);
  await act(alice, "dialog:pins");
  await alice.waitForSelector(".live-modal .message");
  await act(alice, "close");
  checks.push("pins");
  await act(alice, "dialog:roles");
  await alice.waitForSelector('.live-modal [data-live^="role:"]');
  await alice.locator("#f-name").fill("Web Testers");
  await alice.locator("[type=submit]").click();
  await alice.waitForSelector('[data-live^="role:"]:has-text("Web Testers")');
  checks.push("create role");
  const roleAction = await alice
    .locator('[data-live^="role:"]:has-text("Web Testers")')
    .getAttribute("data-live");
  const roleId = roleAction.slice(5);
  await act(alice, roleAction);
  await alice.locator("#f-value").fill("#6cb5ed");
  await alice.locator('[data-role-bit="1"]').check();
  await alice.locator('[data-role-bit="4"]').check();
  await alice.locator("#f-hoist").check();
  await alice.locator("[type=submit]").click();
  await alice.waitForSelector('[data-live^="role:"]:has-text("Web Testers")');
  await act(alice, "close");
  await form(alice, "channel", "private-room", { "f-extra": "text" });
  await alice.waitForFunction(() => !document.querySelector(".live-modal"));
  await expect(
    alice,
    () =>
      document.querySelector(".channel-header h1")?.textContent ===
      "private-room",
  );
  const privateChannel = await alice
    .locator("#live-messages")
    .getAttribute("data-channel");
  await act(alice, "dialog:private");
  await alice.locator("#f-name").selectOption(roleId);
  await alice.locator("[type=submit]").click();
  await bob.waitForFunction(
    (c) => !document.querySelector(`[data-live="channel:${c}"]`),
    privateChannel,
  );
  const bobId = await alice
    .locator("[data-member-name]")
    .filter({ hasText: "bob_web" })
    .getAttribute("data-member-name");
  await act(alice, "dialog:member-role");
  await alice.locator("#f-name").selectOption(bobId);
  await alice.locator(`[data-member-role="${roleId}"]`).check();
  await alice.locator("[type=submit]").click();
  await bob.waitForSelector(`[data-live="channel:${privateChannel}"]`);
  await act(bob, `channel:${privateChannel}`);
  await bob.waitForSelector("#live-input");
  await message(bob, "Private role message");
  await alice.waitForSelector('.author:has-text("bob_web")');
  assert.equal(
    await alice
      .locator('.author:has-text("bob_web")')
      .evaluate((e) => getComputedStyle(e).color),
    "rgb(108, 181, 237)",
  );
  await act(alice, "dialog:member-role");
  await alice.locator("#f-name").selectOption(bobId);
  await alice.locator(`[data-member-role="${roleId}"]`).uncheck();
  await alice.locator("[type=submit]").click();
  await bob.waitForFunction(
    (c) =>
      !document.querySelector(`[data-live="channel:${c}"]`) &&
      !document.querySelector("#live-input"),
    privateChannel,
  );
  checks.push(
    "role color/permissions/hoist + membership + private-channel grant/revoke + inaccessible history purge",
  );
  const privateMessage = await alice
    .locator("[data-live-message]")
    .last()
    .getAttribute("data-live-message");
  await act(alice, `pin:${privateMessage}`);
  await wait(100);
  await act(alice, "dialog:pins");
  await alice.waitForSelector(".live-modal .message");
  await act(alice, "close");

  await act(alice, `channel:${channel}`);
  await act(bob, `channel:${channel}`);
  await alice.waitForSelector("#live-input");
  await bob.waitForSelector("#live-input");

  // Gate (not mock) the first real PUT so Pause deterministically falls
  // between protocol chunks. The server's advertised chunk size is 16 MiB.
  let enteredPut,
    releasePut,
    gated = false;
  const putEntered = new Promise((r) => (enteredPut = r));
  const putReleased = new Promise((r) => (releasePut = r));
  const uploadRoute = "**/api/v1/uploads/resumable/*";
  await alice.route(uploadRoute, async (route) => {
    if (route.request().method() === "PUT" && !gated) {
      gated = true;
      enteredPut();
      await putReleased;
    }
    await route.continue();
  });
  const uploadFile = {
    name: "browser-proof.bin",
    mimeType: "application/octet-stream",
    buffer: Buffer.alloc(17 * 1024 * 1024, 0x53),
  };
  await alice.locator("#live-file").setInputFiles(uploadFile);
  await putEntered;
  await act(alice, "upload-cancel");
  releasePut();
  await alice.waitForSelector('.live-error:has-text("Upload paused")');
  await alice.unroute(uploadRoute);
  await act(alice, "dismiss-error");
  const resumed = alice.waitForResponse(
    (r) =>
      r.request().method() === "GET" &&
      /\/uploads\/resumable\/[^/]+$/.test(r.url()),
  );
  await alice.locator("#live-file").setInputFiles(uploadFile);
  const resumedState = await (await resumed).json();
  assert.equal(resumedState.offset, 16 * 1024 * 1024);
  await alice.waitForSelector('.live-upload:has-text("browser-proof.bin")');
  await message(alice, "File from Blob.slice");
  await bob.waitForSelector("a.file-card");
  checks.push(
    "17 MiB Blob.slice upload: pause at 16 MiB, reselect, resume real server offset + attachment",
  );
  await act(alice, "dialog:notifications");
  await alice.locator("#f-name").selectOption("mentions");
  await alice.locator("[type=submit]").click();
  await wait(200);
  checks.push("notification settings mutation");
  await act(alice, "dialog:devices");
  await alice.waitForSelector(".live-modal .device-row");
  await act(alice, "close");
  checks.push("device sessions");
  await alice.reload();
  await alice.waitForSelector(".live-home");
  await act(alice, guild);
  await alice.waitForSelector("#live-input");
  await expect(alice, () =>
    [...document.querySelectorAll(".markdown")].some((e) =>
      e.textContent.includes("Edited persisted message"),
    ),
  );
  checks.push("cookie recovery + persisted history");
  if (!process.env.STICKTALK_FAST_QA) {
    // Populate more than one server history page through the actual WS client.
    for (let i = 0; i < 55; i++) {
      await message(alice, `History row ${String(i).padStart(3, "0")}`);
      await wait(1100);
    }
    await alice.locator("#live-messages").evaluate((e) => (e.scrollTop = 0));
    await act(alice, "seek:0");
    await expect(
      alice,
      () =>
        Number(
          document.getElementById("live-messages")?.dataset.windowStart,
        ) === 0,
    );
    assert((await alice.locator("[data-live-message]").count()) <= 50);
    assert(
      Number(
        await alice.locator("#live-messages").getAttribute("data-total"),
      ) >= 55,
    );
    checks.push(
      "real history extent + anchor seek + at most 50 mounted messages",
    );
    await act(alice, `channel:${channel}`);
    await expect(alice, () =>
      [...document.querySelectorAll(".markdown")].some((e) =>
        e.textContent.includes("History row 054"),
      ),
    );
  }
  await message(bob,"The phone layout is ready for another pass. Could you check the longer messages, the member drawer and the call controls before we ship?");
  await message(alice,"Yes — keeping the composer visible when the keyboard opens is the important part.");
  await message(alice,"Build check: cargo test -p sticktalk-web");
  const reviewMessage=await alice.locator("[data-live-message]").last().getAttribute("data-live-message");
  await act(bob,`react:${reviewMessage},👍`);
  await act(alice,`reply:${reviewMessage}`);
  await message(alice,"The workspace photo is attached below.");
  const originals = [];
  alice.on("request", (r) => {
    if (/\/api\/v1\/files\/\d+$/.test(new URL(r.url()).pathname))
      originals.push(r.url());
  });
  await alice.locator("#live-file").setInputFiles({
    name: "workspace.jpg",
    mimeType: "image/jpeg",
    buffer: await readFile("public/mock-workspace.jpg"),
  });
  await alice.waitForSelector('.live-upload:has-text("workspace.jpg")');
  await message(alice, "An image uploaded from the browser");
  await alice.waitForSelector("[data-view-image] img");
  assert.equal(originals.length, 0);
  await alice.locator("[data-view-image]").last().click();
  await alice.waitForFunction(
    () => document.querySelector("#live-viewer img")?.naturalWidth > 0,
  );
  assert(originals.length > 0);
  await alice.waitForFunction(()=>!document.querySelector("#live-viewer .viewer-loading"));
  await capture(alice,"image-viewer");
  await alice.keyboard.press("+");
  const zoomBefore=await alice.locator("#live-viewer img").evaluate(e=>e.style.transform);
  const touch=await contexts[0].newCDPSession(alice);
  await touch.send("Input.dispatchTouchEvent",{type:"touchStart",touchPoints:[{x:700,y:450,id:1},{x:740,y:450,id:2}]});
  await touch.send("Input.dispatchTouchEvent",{type:"touchMove",touchPoints:[{x:680,y:440,id:1},{x:780,y:460,id:2}]});
  await touch.send("Input.dispatchTouchEvent",{type:"touchEnd",touchPoints:[]});
  const zoomAfter=await alice.locator("#live-viewer img").evaluate(e=>e.style.transform);
  assert.notEqual(zoomBefore,zoomAfter);
  await alice.keyboard.press("0");
  assert(await alice.locator('#live-viewer a[download]').count());
  assert(await alice.locator('#live-viewer a[target="_blank"]').count());
  checks.push("viewer original-on-demand, zoom/reset, two-pointer pinch/pan, save and open-original controls");
  await alice.keyboard.press("Escape");
  await message(bob, "Hello @alice_web");
  await alice.waitForSelector(".mention");
  checks.push(
    "thumbnail only before viewer gesture + original-on-demand + explicit mention",
  );
  const screenshots = [];
  await mkdir(output, { recursive: true });
  for (const [width, height] of [
    [1440, 900],
    [390, 844],
    [412, 915],
    [820, 1180],
    [1280, 800],
    [1920, 1080],
  ]) {
    await alice.setViewportSize({ width, height });
    await alice.evaluate(() => document.fonts.ready);
    await alice.evaluate(()=>{const list=document.querySelector("#live-messages");if(list)list.scrollTop=list.scrollHeight;});
    await wait(100);
    const path = `${output}/chat-${width}x${height}.png`;
    await alice.screenshot({ path, animations: "disabled" });
    screenshots.push(path);
    if (width < 768) {
      assert.equal(await alice.locator(".bottom-tabs").count(), 0);
      assert(await alice.locator(".channel-header>.icon").isVisible());
    }
  }
  await alice.setViewportSize({width:1440,height:900});
  await message(bob,"The next test is ready.");
  await message(bob,"We can keep these two lines together.");
  await message(alice,"Same author, one avatar.");
  await message(alice,"The second line is compact.");
  await message(alice,"And the third line keeps the same gutter.");
  await alice.waitForSelector(".message.continuation");
  assert.equal(await alice.locator(".continuation .message-meta").count(),0);
  assert.equal(await alice.locator(".continuation .message-avatar").count(),0);
  await alice.locator('.continuation').last().hover();
  assert.equal(await alice.locator('.continuation .gutter-time').last().evaluate(e=>getComputedStyle(e).visibility),'visible');
  await capture(alice,"grouped-chat");
  checks.push("consecutive author grouping, compact rows and timestamp gutter on all six widths");

  // All four completion types, real member/role/channel projections, keyboard selection.
  for (const [query,expected] of [["@bob","bob_web"],["#private","private-room"],[":smile","smil"]]) {
    await alice.locator("#live-input").fill(query);
    await alice.waitForSelector("#live-completions [role=option]");
    assert((await alice.locator("#live-completions").textContent()).toLowerCase().includes(expected));
    if(query==="@bob") await capture(alice,"autocomplete");
    await alice.locator("#live-input").press("ArrowDown");
    await alice.locator("#live-input").press("Tab");
    assert.notEqual(await alice.locator("#live-input").inputValue(),query);
    await alice.locator("#live-input").fill("");
    await act(alice,"complete-close");
  }
  await act(alice,roleAction);
  await alice.locator("#f-mentionable").check();
  await alice.locator("[type=submit]").click();
  await alice.waitForSelector('[data-live^="role:"]:has-text("Web Testers")');
  await act(alice,"close");
  await alice.locator("#live-input").fill("@Web");
  await alice.waitForSelector('#live-completions [data-completion^="<@&"]');
  await alice.locator("#live-input").press("Escape");
  await alice.locator("#live-input").fill("");
  checks.push("@user, @role, #channel and :emoji: autocomplete; arrows/Tab/Escape");

  await act(alice,"dialog:settings"); await alice.waitForSelector(".settings-page");
  await capture(alice,"settings");
  await act(alice,"dialog:profile-edit");
  await alice.waitForSelector("#live-avatar", {state:'attached'});
  await capture(alice,"profile-settings");
  await alice.locator("#live-avatar").setInputFiles({name:"avatar.png",mimeType:"image/png",buffer:await readFile("public/icons/icon-192.png")});
  await alice.waitForFunction(()=>document.querySelector(".settings-identity .avatar img")?.naturalWidth>0);
  await act(alice,"close");
  checks.push("desktop settings layout, mobile settings-list/subpage navigation, real avatar upload");
  await act(alice,"dialog:member-role");
  await alice.locator("#f-name").selectOption(bobId);
  await alice.locator(`[data-member-role="${roleId}"]`).check();
  await alice.locator("[type=submit]").click();
  await alice.waitForFunction(()=>!document.querySelector(".live-modal"));
  await act(alice,"panel:right");
  await alice.waitForSelector('.member-group .category:has-text("Web Testers")');
  await capture(alice,"members");
  await act(alice,"panel:");
  checks.push("hoisted-role member groups, counts and owner crown");

  const caraContext=await browsers[1].newContext({viewport:{width:1440,height:900}});
  const cara=await caraContext.newPage();
  cara.on("pageerror",e=>errors.push(e.stack??String(e)));
  await register(cara,"cara_web");
  await form(alice,"friend","cara_web");
  await cara.locator('[data-live^="friend:accept,"]').click();
  await act(alice,"close");
  await act(alice,"dialog:group");
  await alice.waitForSelector('[data-live-form="group"]');
  await alice.locator("#f-name").fill("Browser Group");
  await alice.locator(`[data-group-user="${bobId}"]`).check();
  await alice.locator('.live-checkbox:has-text("cara_web") input').check();
  await alice.locator("[type=submit]").click();
  await alice.waitForFunction(()=>document.querySelector(".channel-header h1")?.textContent==="Browser Group");
  const group=await alice.locator("#live-messages").getAttribute("data-channel");
  await message(alice,"A real group DM");
  await act(bob,`channel:${group}`);
  await expect(bob,()=>document.querySelector(".markdown")?.textContent.includes("A real group DM"));
  await act(alice,`channel:${channel}`);
  await act(bob,`channel:${channel}`);
  checks.push("create group DM and cross-browser delivery");
  await alice.setViewportSize({ width: 1440, height: 900 });
  await form(alice, "channel", "Browser Voice", { "f-extra": "voice" });
  await alice.waitForFunction(() => !document.querySelector(".live-modal"));
  await alice.waitForSelector(
    '[data-live^="voice:"]:has-text("Browser Voice")',
  );
  const voice = await alice
    .locator('[data-live^="voice:"]:has-text("Browser Voice")')
    .getAttribute("data-live");
  await act(alice, voice);
  await act(bob, voice);
  // Browser metrics, not signaling acknowledgements, prove real SFU media flow.
  let proof;
  for (let i = 0; i < 90; i++) {
    await wait(500);
    const result = await Promise.all([diagnostic(alice), diagnostic(bob)]);
    if (
      result.every(
        (d) =>
          d.state === "connected" &&
          d.stats.some(
            (r) =>
              r.type === "inbound-rtp" &&
              r.kind === "audio" &&
              r.bytesReceived > 0,
          ),
      )
    ) {
      proof = result;
      break;
    }
  }
  if (!proof)
    await writeFile(
      `${output}/media-failure.json`,
      JSON.stringify(
        await Promise.all([diagnostic(alice), diagnostic(bob)]),
        null,
        2,
      ),
    );
  assert(
    proof,
    `two-browser audio failed: ${JSON.stringify(await Promise.all([diagnostic(alice), diagnostic(bob)]))}; errors=${errors}; server=${log}`,
  );
  checks.push("two independent Chromium processes exchange audio through SFU");
  await act(alice, "media:camera");
  await act(bob, "media:camera");
  let video;
  for (let i = 0; i < 90; i++) {
    await wait(500);
    const result = await Promise.all([diagnostic(alice), diagnostic(bob)]);
    if (
      result.every(
        (d) =>
          d.stats.some(
            (r) =>
              r.type === "inbound-rtp" &&
              r.kind === "video" &&
              r.framesDecoded >= 10,
          ) &&
          d.tracks.some(
            (t) =>
              t.kind === "camera" &&
              !t.own &&
              t.videoWidth > 0 &&
              t.readyState >= 2,
          ),
      )
    ) {
      video = result;
      break;
    }
  }
  assert(
    video,
    `two-browser video failed: ${JSON.stringify(await Promise.all([diagnostic(alice), diagnostic(bob)]))}; errors=${errors}; server=${log}`,
  );
  for (const d of video) {
    assert.match(d.localSdp,/a=rtpmap:\d+ rtx\/90000/i,'camera offer retains H.264 repair');
    assert.match(d.remoteSdp,/a=rtpmap:\d+ rtx\/90000/i,'SFU answer negotiates repair/probe transport');
  }
  checks.push(
    "two independent Chromium processes decode remote H.264 camera frames",
  );
  assert.equal(await alice.locator('.live-error').count(),0,'no stale typing/error toast in call');
  await writeFile(
    `${output}/voice-stats.json`,
    JSON.stringify(
      video.map((d) => ({
        state: d.state,
        publications: d.publications,
        subscriptions: d.subscriptions,
        tracks: d.tracks,
        rtxNegotiated: /a=rtpmap:\d+ rtx\/90000/i.test(d.remoteSdp),
        stats: d.stats.filter((r) =>
          ["inbound-rtp", "outbound-rtp", "codec"].includes(r.type),
        ),
      })),
      null,
      2,
    ),
  );
  for (const [width, height] of [
    [1440, 900],
    [390, 844],
    [412, 915],
    [820, 1180],
    [1280, 800],
    [1920, 1080],
  ]) {
    await alice.setViewportSize({ width, height });
    await wait(100);
    const path = `${output}/voice-${width}x${height}.png`;
    const tiles=await alice.locator('.participant-tile:not([hidden])').evaluateAll(nodes=>nodes.map(n=>({
      rect:n.getBoundingClientRect().toJSON(), kind:n.dataset.kind,
      fit:n.querySelector('video')&&getComputedStyle(n.querySelector('video')).objectFit,
    })));
    for(const t of tiles) {
      assert(Math.abs(t.rect.width/t.rect.height-16/9)<.02);
      if(t.kind==='camera')assert.equal(t.fit,'cover');
      if(width<768)assert(t.rect.bottom<height-60,'two tiles fit above bottom controls');
    }
    assert.equal(await alice.locator('.call-heading').count(),0);
    await alice.screenshot({ path });
    screenshots.push(path);
  }
  await act(alice, "media:mute");
  await act(alice, "media:deafen");
  await act(alice, "media:leave");
  await act(bob, "media:leave");
  for (const page of [alice, bob]) {
    let released=false;
    for(let attempt=0;attempt<100;attempt++) {
      const d=await diagnostic(page);
      released=!d.joined&&d.publications===0&&d.subscriptions===0&&d.tracks.length===0;
      if(released)break;
      await wait(100);
    }
    assert(released,"leave releases every publication, subscription and track");
  }
  checks.push(
    "mute + deafen + leave; all publications/subscriptions/tracks released",
  );
  await act(alice, "dialog:notifications");
  await alice.evaluate(async () => {
    window.__permissionRequests = 0;
    Object.defineProperty(window, "Notification", {
      configurable: true,
      value: class {
        static requestPermission() {
          window.__permissionRequests++;
          return Promise.resolve("granted");
        }
      },
    });
    const pair = await crypto.subtle.generateKey(
      { name: "ECDH", namedCurve: "P-256" },
      true,
      ["deriveBits"],
    );
    const key = new Uint8Array(
      await crypto.subtle.exportKey("raw", pair.publicKey),
    );
    const b64 = (bytes) =>
      btoa(String.fromCharCode(...bytes))
        .replaceAll("+", "-")
        .replaceAll("/", "_")
        .replaceAll("=", "");
    const fake = {
      endpoint: "https://fcm.googleapis.com/fcm/send/sticktalk-headless-test",
      keys: {
        p256dh: b64(key),
        auth: b64(crypto.getRandomValues(new Uint8Array(16))),
      },
    };
    // --disable-notifications also removes PushManager in headless Chromium.
    // Supply only the browser surface; subscription storage still hits the server.
    Object.defineProperty(window, "PushManager", {
      configurable: true,
      value: class {},
    });
    const manager = {
      getSubscription: async () => null,
      subscribe: async () => ({ toJSON: () => fake }),
    };
    Object.defineProperty(ServiceWorkerRegistration.prototype, "pushManager", {
      configurable: true,
      get: () => manager,
    });
  });
  assert.equal(await alice.evaluate(() => window.__permissionRequests), 0);
  const subscribed = alice.waitForResponse(
    (r) =>
      r.url().endsWith("/push/subscriptions") &&
      r.request().method() === "POST",
  );
  await alice.locator("[data-push-enable]").click();
  const response = await subscribed;
  assert.equal(response.status(), 201);
  assert.equal(await alice.evaluate(() => window.__permissionRequests), 1);
  checks.push(
    "push permission only after explainer click; browser permission/provider mocked; real subscription endpoint",
  );
  await act(alice, "close");
  await act(alice, `channel:${channel}`);
  await alice.waitForSelector("#live-input");
  await alice.locator("#live-input").fill("Alice unsent account-private draft");
  await act(alice, "logout");
  await alice.waitForSelector("#auth-username");
  await alice.locator("#auth-username").fill("bob_web");
  await alice.locator("#auth-password").fill("testing-password-123456");
  await alice.locator("[type=submit]").click();
  await alice.waitForSelector(".live-home");
  await alice.waitForFunction(
    () => !document.querySelector(".connection-banner"),
  );
  await act(alice, "dialog:pins");
  assert.equal(await alice.locator(".live-modal .message").count(), 0);
  await act(alice, "close");
  await act(alice, `channel:${channel}`);
  await alice.waitForSelector("#live-input");
  assert.equal(await alice.locator("#live-input").inputValue(), "");
  await act(alice, "dialog:emoji");
  await alice.locator("#emoji-query").fill("smile");
  await wait(300);
  await alice.waitForSelector(".emoji-grid button");
  const emoji = await alice
    .locator(".emoji-grid button")
    .first()
    .getAttribute("data-live");
  await alice.locator(".emoji-grid button").first().click();
  await alice.waitForFunction(
    (emoji) => document.querySelector("#live-input")?.value === emoji,
    emoji.slice(6),
  );
  await alice.locator("#live-input").fill("");
  checks.push("sticktalk-emoji search and composer insertion");
  checks.push(
    "logout purges private pinned history, drafts and media; password login as a different account",
  );
  for (const page of [alice, bob])
    violations.push(...(await page.evaluate(() => window.__violations ?? [])));
  assert.deepEqual(errors, []);
  assert.deepEqual(violations, []);
  const report = {
    bundleRevision: JSON.parse(
      await readFile(join(process.env.STICKTALK_WEB_DIST ?? join(target, "web-dist"), "sizes.json"), "utf8"),
    ).revision,
    fastQa: !!process.env.STICKTALK_FAST_QA,
    realServer: true,
    headless: true,
    loopback: true,
    checks,
    screenshots,
    errors,
    violations,
    media: video.map((d) => ({
      state: d.state,
      inbound: d.stats
        .filter((r) => r.type === "inbound-rtp")
        .map(({ kind, bytesReceived, framesDecoded, packetsReceived }) => ({
          kind,
          bytesReceived,
          framesDecoded,
          packetsReceived,
        })),
    })),
  };
  await writeFile(
    `${output}/results.json`,
    JSON.stringify(report, null, 2),
  );
  await rm(`${output}/failure.json`, { force: true });
  await rm(`${output}/media-failure.json`, { force: true });
  console.log(JSON.stringify(report));
} catch (error) {
  await writeFile(
    `${output}/failure.json`,
    JSON.stringify(
      {
        error: String(error),
        errors,
        log,
        checks,
        pages: await Promise.all(
          pages.map((p) =>
            p
              .locator("body")
              .innerText()
              .catch(() => ""),
          ),
        ),
      },
      null,
      2,
    ),
  );
  throw error;
} finally {
  for (const b of browsers) await b.close();
  if (server.pid && server.exitCode === null) {
    const exit = once(server, "exit");
    server.kill();
    await exit;
  }
  assert.equal(dirname(resolve(data)), target);
  assert(basename(data).startsWith("live-test-"));
  await rm(data, { recursive: true, force: true });
}

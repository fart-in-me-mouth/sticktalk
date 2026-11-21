// Real local server, two headless Chromium processes, loopback only, no visible
// window. Covers attaching a file (from the GIF dialog, while the page re-renders,
// and by pasting), editing and deleting a message, the message menu, the member
// list and who is online, the "Saved." confirmation on forms that stay open, the
// phone layout of the member drawer and settings, sign-in messages, and a call:
// who is in it, mute indicators, a stream's sound and per-person volume.
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { once } from "node:events";
import { mkdtemp, rm, mkdir, readFile } from "node:fs/promises";
import { resolve, join, dirname, basename } from "node:path";
const target = resolve(
  process.env.CARGO_TARGET_DIR ?? fileURLToPath(new URL("../../../target/", import.meta.url)),
);
await mkdir(target, { recursive: true });
const data = await mkdtemp(join(target, "call-composer-test-"));
const reserve = createServer();
reserve.listen(0, "127.0.0.1");
await once(reserve, "listening");
const port = reserve.address().port;
await new Promise((r) => reserve.close(r));
const origin = `http://127.0.0.1:${port}`;
const server = spawn(
  process.env.STICKTALK_SERVER_EXE ??
    join(target, `debug/sticktalk-server${process.platform === "win32" ? ".exe" : ""}`),
  ["--bind", `127.0.0.1:${port}`, "--public-url", origin, "--data-dir", data,
    "--web-root", process.env.STICKTALK_WEB_DIST ?? join(target, "web-dist"),
    "--min-free-disk-gb", "0", "--voice-bind-ip", "127.0.0.1", "--voice-advertised-ip", "127.0.0.1"],
  { windowsHide: true, stdio: ["ignore", "pipe", "pipe"] },
);
const errors = [], checks = [];
let log = "", browsers = [];
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const act = (page, action) =>
  page.evaluate((action) => window.dispatchEvent(new CustomEvent("sticktalk-live", { detail: action })), action);
const GIF = Buffer.from("R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7", "base64");
const PNG = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
async function register(page, user) {
  await page.goto(origin);
  await page.waitForSelector("#auth-username");
  await page.waitForFunction(() => !document.querySelector("[type=submit]").disabled);
  await page.locator("[data-live=register]").click();
  await page.locator("#auth-username").fill(user);
  await page.locator("#auth-password").fill("testing-password-123456");
  await page.locator("[type=submit]").click();
  await page.waitForSelector(".live-home");
  await page.waitForFunction(() => !document.querySelector(".connection-banner"));
}
async function form(page, kind, name, extra = {}) {
  await act(page, `dialog:${kind}`);
  await page.waitForSelector(".live-modal [data-live-form]");
  if (await page.locator("#f-name").isVisible()) await page.locator("#f-name").fill(name);
  for (const [id, value] of Object.entries(extra)) {
    const e = page.locator(`#${id}`);
    if (await e.evaluate((e) => e.tagName === "SELECT")) await e.selectOption(value);
    else await e.fill(value);
  }
  await page.locator("[data-live-form] [type=submit]").click();
}
const attached = async (page, name) => {
  try {
    await page.waitForFunction(
      (name) => [...document.querySelectorAll(".live-upload")].some((e) => e.textContent.includes(name) && e.querySelector('[data-live^="remove-attachment"]')),
      name, { timeout: 15000 },
    );
  } catch {
    const seen = await page.evaluate(() => JSON.stringify({
      uploads: [...document.querySelectorAll(".live-upload")].map((e) => e.textContent),
      error: document.querySelector(".live-error")?.textContent,
      modal: document.querySelector(".live-modal")?.dataset.dialog,
      pickers: document.querySelectorAll("#live-file").length,
    }));
    throw Error(`"${name}" was not attached: ${seen}; ${errors}`);
  }
};
const diagnostic = (page) => page.evaluate(async () => (await import("/public/media.js")).diagnostics());
// What the listener's page plays for one of the other person's tiles.
const sound = (page, kind) =>
  page.evaluate((kind) => {
    const tile = document.querySelector(`.participant-tile[data-kind="${kind}"]:not([data-self])`);
    const media = tile?.querySelector("audio");
    return media && { hidden: tile.hidden, muted: media.muted, volume: media.volume, paused: media.paused };
  }, kind);
try {
  await new Promise((done, fail) => {
    const timer = setTimeout(() => fail(Error(`server timeout ${log}`)), 30000);
    const collect = (c) => {
      log = (log + c).slice(-20000);
      if (log.includes("StickTalk listening")) { clearTimeout(timer); done(); }
    };
    server.stdout.on("data", collect);
    server.stderr.on("data", collect);
    server.once("error", fail);
    server.once("exit", (code) => fail(Error(`server exited ${code}: ${log}`)));
  });
  const launch = () =>
    chromium.launch({
      headless: true,
      args: ["--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream",
        "--allow-loopback-in-peer-connection", "--disable-features=WebRtcHideLocalIpsWithMdns",
        "--mute-audio", "--disable-notifications"],
    });
  browsers = [await launch(), await launch()];
  const contexts = await Promise.all(
    browsers.map((b) => b.newContext({ viewport: { width: 1440, height: 900 }, permissions: ["microphone", "camera"] })),
  );
  const [alice, bob] = await Promise.all(contexts.map((c) => c.newPage()));
  for (const page of [alice, bob]) page.on("pageerror", (e) => errors.push(e.stack ?? String(e)));
  // Headless Chromium has no screen picker. Bob "shares" a drawn picture and a
  // tone; the real page code still chooses the constraints and publishes both.
  await bob.addInitScript(() => {
    window.__sticktalkHost = { control: 1 };
    window.__host = [];
    window.ipc = {
      postMessage: (raw) => {
        const message = JSON.parse(raw);
        window.__host.push(message);
        if (message.control === "start")
          setTimeout(() => window.dispatchEvent(new CustomEvent("sticktalk-host", { detail: { control: "allowed" } })), 50);
      },
    };
    navigator.mediaDevices.getDisplayMedia = async (constraints) => {
      window.__displayConstraints = constraints;
      window.__displayAudio = () => out.stream.getAudioTracks()[0].enabled;
      const canvas = document.createElement("canvas");
      canvas.width = 640; canvas.height = 360;
      const g = canvas.getContext("2d");
      let n = 0;
      setInterval(() => { g.fillStyle = `hsl(${(n += 7) % 360} 80% 50%)`; g.fillRect(0, 0, 640, 360); }, 50);
      const audio = new AudioContext(), tone = audio.createOscillator(), out = audio.createMediaStreamDestination();
      tone.connect(out); tone.start();
      const [picture] = canvas.captureStream(20).getVideoTracks();
      const settings = picture.getSettings.bind(picture);
      picture.getSettings = () => ({ ...settings(), displaySurface: "monitor" });
      return new MediaStream([picture, ...out.stream.getAudioTracks()]);
    };
  });
  await register(alice, "alice_call");
  await register(bob, "bob_call");
  await form(alice, "create", "Call Lab");
  await alice.waitForFunction(() => !document.querySelector(".live-modal"));
  await alice.waitForSelector("#live-input");
  await form(alice, "invite", "");
  await alice.waitForSelector(".live-modal code");
  const invite = await alice.locator(".live-modal code").first().textContent();
  await act(alice, "close");
  await form(bob, "join", invite);
  await bob.waitForFunction(() => !document.querySelector(".live-modal"));
  await alice.waitForSelector("#live-input");

  // Attach a GIF from the GIF dialog; the dialog closes before a file is chosen.
  await alice.locator(".composer-gif").click();
  await alice.waitForSelector('.live-modal [data-live="pick-image"]');
  let [chooser] = await Promise.all([
    alice.waitForEvent("filechooser"),
    alice.locator('.live-modal [data-live="pick-image"]').click(),
  ]);
  await alice.waitForFunction(() => !document.querySelector(".live-modal"));
  await chooser.setFiles({ name: "party.gif", mimeType: "image/gif", buffer: GIF });
  await attached(alice, "party.gif");
  checks.push("a GIF chosen through the GIF dialog is attached");

  // Attach while the page re-renders under an open file chooser.
  [chooser] = await Promise.all([
    alice.waitForEvent("filechooser"),
    alice.locator('[data-live="pick-file"]').click(),
  ]);
  await act(alice, "panel:right");
  await wait(200);
  await act(alice, "panel:");
  await wait(200);
  await chooser.setFiles({ name: "later.png", mimeType: "image/png", buffer: Buffer.from(PNG, "base64") });
  await attached(alice, "later.png");
  checks.push("a file chosen after the page re-rendered is attached");

  // Paste: a clipboard picture attaches; clipboard text is left to the browser.
  const pasted = await alice.evaluate((png) => {
    const input = document.querySelector("#live-input");
    input.focus();
    const paste = (fill) => {
      const transfer = new DataTransfer();
      fill(transfer);
      const event = new ClipboardEvent("paste", { clipboardData: transfer, bubbles: true, cancelable: true });
      input.dispatchEvent(event);
      return event.defaultPrevented;
    };
    const bytes = Uint8Array.from(atob(png), (c) => c.charCodeAt(0));
    return {
      text: paste((t) => t.setData("text/plain", "just words")),
      image: paste((t) => t.items.add(new File([bytes], "image.png", { type: "image/png" }))),
    };
  }, PNG);
  assert.deepEqual(pasted, { text: false, image: true });
  await attached(alice, "image.png");
  await alice.locator("#live-input").press("Enter");
  await alice.waitForFunction(() => document.querySelectorAll(".message [data-view-image]").length >= 3 && !document.querySelector(".live-upload"));
  checks.push("a pasted screenshot is attached and sent; pasted text is not intercepted");

  // A message with two lines and a mention followed by a comma.
  const input = alice.locator("#live-input");
  await input.fill("first line\nsecond line for @bob_call, thanks");
  await input.press("Enter");
  const row = alice.locator(".message", { hasText: "second line" }).last();
  await row.waitFor();
  assert.equal(await row.locator(".markdown p").count(), 2, "each line is its own paragraph");
  assert.equal(await row.locator(".markdown .mention").textContent(), "@bob_call", "a mention may be followed by punctuation");
  // Right-click opens the message menu; Edit from it opens and stays open.
  await row.locator(".markdown p").first().click({ button: "right" });
  await alice.waitForSelector("#live-message-menu");
  await alice.locator('#live-message-menu [data-live^="edit:"]').click();
  const editor = alice.locator('.live-modal [data-live-form="edit"] textarea#f-name');
  await editor.waitFor();
  await wait(700);
  assert.equal(await editor.inputValue(), "first line\nsecond line for @bob_call, thanks", "editing shows what was typed: lines and @names");
  await editor.press("Control+End");
  await editor.pressSequentially(" again");
  await editor.press("Enter");
  await alice.waitForFunction(() => !document.querySelector(".live-modal") &&
    [...document.querySelectorAll(".message .markdown")].some((m) => m.textContent.includes("thanks again")));
  assert.equal(await row.locator(".markdown p").count(), 2, "an edit keeps the line break");
  assert.equal(await row.locator(".markdown .mention").count(), 1, "an edit keeps the mention");
  checks.push("the message menu opens on right-click; editing keeps line breaks and mentions");

  // Deleting asks first, in the app's own dialog.
  await row.hover();
  await row.locator('.message-actions [data-live^="delete:"]').click();
  await alice.waitForSelector("#live-confirm");
  await alice.locator("#live-confirm .button.secondary").click();
  await wait(300);
  assert.equal(await row.count(), 1, "Cancel keeps the message");
  await row.hover();
  await row.locator('.message-actions [data-live^="delete:"]').click();
  await alice.locator("#live-confirm [data-accept]").click();
  await alice.waitForFunction(() => ![...document.querySelectorAll(".message .markdown")].some((m) => m.textContent.includes("thanks again")));
  checks.push("deleting a message is confirmed in the app, and Cancel keeps it");

  // Kaomoji: one with characters that message formatting would otherwise eat.
  await alice.locator('.composer [data-live="dialog:kaomoji"]').click();
  await alice.waitForSelector(".live-modal .kaomoji");
  assert((await alice.locator(".live-modal .kaomoji").count()) >= 200, "a large set of kaomoji");
  const shrug = "¯＼＿(ツ)＿/¯";
  await alice.locator(".live-modal .kaomoji", { hasText: shrug }).click();
  await alice.waitForFunction(() => !document.querySelector(".live-modal"));
  assert.equal(await input.inputValue(), shrug);
  await input.press("Enter");
  await alice.waitForFunction((shrug) => [...document.querySelectorAll(".message .markdown")].some((m) => m.textContent === shrug), shrug);
  checks.push("a kaomoji from the picker is sent and shown exactly as picked");

  // Too long: said, not silently cut off; the text stays in the box.
  await input.fill("x".repeat(4001));
  assert.equal(await alice.locator("#live-count.is-over").textContent(), "-1");
  await input.press("Enter");
  await alice.waitForSelector('.live-error:has-text("too long")');
  assert.equal((await input.inputValue()).length, 4001);
  await input.fill("");
  await act(alice, "dismiss-error");

  // A reply preview does not give away a spoiler.
  await input.fill("the ending is ||everyone lives||");
  await input.press("Enter");
  const secret = alice.locator(".message", { hasText: "the ending is" }).last();
  await secret.waitFor();
  await act(alice, `reply:${await secret.getAttribute("data-live-message")}`);
  await alice.waitForSelector(".composer-wrap .reply-preview");
  const quoted = await alice.locator(".composer-wrap .reply-preview").textContent();
  assert(quoted.includes("(spoiler)") && !quoted.includes("everyone lives"), quoted);
  await alice.locator("#live-input").press("Escape");
  await alice.waitForFunction(() => !document.querySelector(".composer-wrap .reply-preview"));
  checks.push("over-long messages are refused with the text kept; reply previews hide spoilers; Escape cancels a reply");

  // Who is online: Bob joined after Alice opened the server, and is listed as online.
  await alice.waitForFunction(() => [...document.querySelectorAll(".member-group:not(.offline) [data-member-name]")]
    .some((e) => e.textContent.includes("bob_call")), null, { timeout: 15000 });
  assert.equal(await alice.locator(".member-group.offline").count(), 0, "nobody here is offline");
  checks.push("the member list shows who is online");

  // The phone layout: the member drawer is above its backdrop, settings fill the width.
  const phone = await browsers[1].newContext({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });
  const carol = await phone.newPage();
  carol.on("pageerror", (e) => errors.push(e.stack ?? String(e)));
  await register(carol, "carol_call");
  await act(carol, "dialog:join");
  await carol.locator(".live-modal #f-name").fill(`${origin}/?invite=${invite}`);
  await carol.waitForSelector('.live-modal .invite-preview:has-text("Call Lab")');
  assert.match(await carol.locator(".live-modal .invite-preview").textContent(), /Call Lab · 2 members$/);
  await carol.locator(".live-modal #f-name").fill("A".repeat(22));
  await carol.waitForSelector('.live-modal .invite-preview:has-text("That invite")');
  await carol.locator(".live-modal #f-name").fill(invite);
  await carol.waitForSelector('.live-modal .invite-preview:has-text("Call Lab")');
  await carol.locator(".live-modal [data-live-form] [type=submit]").click();
  await carol.waitForSelector("#live-input");
  checks.push("the Join dialog shows the server an invite leads to, or why it won’t work");
  await carol.waitForSelector(".member-list .member-row", { state: "attached" });
  await act(carol, "panel:right");
  await carol.waitForSelector(".member-list.is-open .member-row");
  assert.equal(await carol.evaluate(() => {
    const member = document.querySelector(".member-list .member-row"), box = member.getBoundingClientRect();
    return member.contains(document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2));
  }), true, "a member in the open drawer can be tapped");
  await act(carol, "panel:");
  await act(carol, "dialog:settings");
  await carol.waitForSelector(".settings-nav");
  assert.equal(await carol.locator(".settings-nav").evaluate((e) => Math.round(e.getBoundingClientRect().width)), 390, "the settings menu fills a phone's width");
  await phone.close();
  checks.push("on a phone the member drawer can be used and settings fill the screen");

  // Signing in: words, not error codes; and a way back in.
  const visitor = await browsers[1].newContext({ viewport: { width: 1280, height: 800 } });
  const guest = await visitor.newPage();
  await guest.goto(origin);
  await guest.waitForSelector("#auth-username");
  await guest.locator("#auth-username").fill("alice_call");
  await guest.locator("#auth-password").fill("not-the-password");
  await guest.locator("[type=submit]").click();
  await guest.waitForFunction(() => document.querySelector("#boot-status").textContent === "Incorrect username or password.");
  await guest.locator("[data-live=register]").click();
  assert.equal(await guest.locator("#boot-status").textContent(), "", "a message about logging in is gone on the register form");
  assert.equal(await guest.locator("#boot-username-help").isVisible(), true, "the username rules are shown when registering");
  await guest.locator("[data-live=register]").click();
  await guest.locator("#boot-forgot").click();
  assert.equal(await guest.locator("#boot-title").textContent(), "Reset your password");
  await guest.locator("#auth-code").fill("x".repeat(43));
  await guest.locator("#auth-password").fill("a-new-password");
  await guest.locator("[type=submit]").click();
  await guest.waitForFunction(() => document.querySelector("#boot-status").textContent.includes("reset code isn’t valid"));
  await visitor.close();
  checks.push("sign-in problems are explained in words; a reset-code form is offered");

  // About Me stays open after saving, so it has to say that it saved.
  await form(alice, "about", "they/them", { "f-value": "Testing the save button." });
  await alice.waitForSelector(".live-modal .form-saved");
  assert.equal(await alice.locator(".live-modal .form-saved").textContent(), "Saved.");
  await alice.locator(".live-modal #f-value").press("Control+End");
  await alice.locator(".live-modal #f-value").pressSequentially(" More.");
  assert.equal(await alice.locator(".live-modal .form-saved").isVisible(), false);
  assert.equal(await alice.locator(".live-modal #f-value").inputValue(), "Testing the save button. More.", "typing after a save keeps the text and drops the confirmation");
  await act(alice, "close");
  await act(alice, "dialog:about");
  await alice.waitForFunction(() => document.querySelector(".live-modal #f-value")?.value === "Testing the save button.");
  assert.equal(await alice.locator(".live-modal .form-saved").count(), 0, "a reopened form does not claim a save");
  await act(alice, "close");
  checks.push("About Me confirms a save and keeps it");

  // Change Avatar is a real button (reachable by keyboard) for a hidden picker.
  // Choosing a picture does not throw away a display name typed but not yet saved.
  await act(alice, "dialog:profile-edit");
  await alice.waitForSelector('.live-modal [data-pick="live-avatar"]');
  await alice.locator('.live-modal [data-profile-editor] #f-name').fill("Alice Draft");
  await alice.locator('.live-modal [data-pick="live-avatar"]').focus();
  [chooser] = await Promise.all([
    alice.waitForEvent("filechooser"),
    alice.keyboard.press("Enter"),
  ]);
  await chooser.setFiles({ name: "avatar.png", mimeType: "image/png", buffer: await readFile("public/icons/icon-192.png") });
  await alice.waitForSelector('.live-modal [data-live="avatar-remove"]', { timeout: 15000 });
  assert.equal(await alice.locator('.live-modal [data-profile-editor] #f-name').inputValue(), "Alice Draft");
  assert.equal(await alice.locator(".profile-save-bar").isVisible(), true, "the unsaved name is still unsaved");
  // Leaving with unsaved changes asks first.
  await alice.keyboard.press("Escape");
  await alice.waitForSelector("#live-confirm");
  await alice.locator("#live-confirm .button.secondary").click();
  await wait(300);
  assert.equal(await alice.locator("#live-confirm").count(), 0, "answering the question closes it");
  assert.equal(await alice.locator('.live-modal [data-profile-editor] #f-name').inputValue(), "Alice Draft");
  // The browser's Back button leaves the page too, and asks the same question.
  await alice.goBack();
  await alice.waitForSelector("#live-confirm");
  await alice.locator("#live-confirm .button.secondary").click();
  await wait(300);
  assert.equal(await alice.locator('.live-modal [data-profile-editor] #f-name').inputValue(), "Alice Draft", "Back, then Cancel, keeps the page and what was typed");
  await act(alice, "close");
  checks.push("Change Avatar works from the keyboard and keeps an unsaved name; leaving unsaved changes asks first");

  // A call: Bob shares a screen with sound.
  await form(alice, "channel", "Call Voice", { "f-extra": "voice" });
  await alice.waitForFunction(() => !document.querySelector(".live-modal"));
  await alice.waitForSelector('[data-live^="voice:"]:has-text("Call Voice")');
  const voice = await alice.locator('[data-live^="voice:"]:has-text("Call Voice")').getAttribute("data-live");
  // Bob joins first: Alice, not yet in the call, sees him listed under the channel.
  await act(bob, voice);
  await alice.waitForSelector('.voice-occupants li:has-text("bob_call")', { timeout: 15000 });
  checks.push("a voice channel lists who is in it");
  await act(alice, voice);
  const flowing = async (kinds) => {
    for (let i = 0; i < 90; i++) {
      await wait(500);
      const d = await diagnostic(alice);
      if (d.state === "connected" && kinds.every((kind) => d.tracks.some((t) => t.kind === kind && !t.own && !t.paused && t.readyState >= 2)))
        return true;
    }
    return false;
  };
  assert(await flowing(["audio"]), `voice did not connect: ${JSON.stringify(await diagnostic(alice))}; ${errors}; ${log}`);
  await act(bob, "media:share");
  await bob.locator(".live-share-options .button.primary").click();
  assert(await flowing(["audio", "screen", "system_audio"]), `stream did not arrive: ${JSON.stringify(await diagnostic(alice))}; ${errors}; ${log}`);
  const constraints = await bob.evaluate(() => ({
    supported: !!navigator.mediaDevices.getSupportedConstraints().restrictOwnAudio,
    asked: window.__displayConstraints.audio.restrictOwnAudio === true,
    processing: window.__displayConstraints.audio.echoCancellation,
  }));
  assert.equal(constraints.asked, constraints.supported, "own call audio is left out of the stream wherever the browser can");
  assert.equal(constraints.processing, false);
  checks.push(`screen share asks the browser to leave out this page's own audio (supported here: ${constraints.supported})`);

  const tiles = await alice.locator(".participant-tile:not([hidden])").evaluateAll((nodes) =>
    nodes.map((n) => ({ kind: n.dataset.kind, own: !!n.dataset.self, control: !!n.querySelector(".tile-volume:not([hidden])") })));
  assert.deepEqual(tiles, [
    { kind: "audio", own: true, control: false },
    { kind: "audio", own: false, control: true },
    { kind: "screen", own: false, control: true },
  ], "one tile per person plus the screen; no control on your own tile");
  assert.equal((await sound(alice, "system_audio")).hidden, true);
  checks.push("a stream's sound is not shown as a second person");
  assert.equal(await alice.locator('.participant-tile.is-focused[data-kind="screen"]').count(), 1, "a new stream opens large");
  assert.equal(await alice.locator('.participant-tile[data-kind="screen"] .tile-live').textContent(), "LIVE");
  assert.equal(await bob.locator('.participant-tile[data-kind="screen"][data-self]').count(), 1, "the sharer sees what they are sharing");
  assert.equal(await bob.locator('[data-live="media:share"]').first().getAttribute("aria-pressed"), "true");
  checks.push("a new stream opens large with a LIVE badge; the sharer gets a preview");

  // Muting or deafening yourself does not silence the screen you are sharing.
  await act(bob, "media:mute");
  await bob.waitForFunction(() => document.querySelector('[data-live="media:mute"]').getAttribute("aria-pressed") === "true");
  assert.equal(await bob.evaluate(() => window.__displayAudio()), true);
  // Everyone else can see that he is muted.
  await alice.waitForSelector('.participant-tile[data-kind="audio"]:not([data-self]) .participant-state[aria-label="Muted"]', { timeout: 15000 });
  assert.equal(await bob.locator('[data-live="media:mute"] use').first().getAttribute("href"), "#i-mic-off");
  await act(bob, "media:deafen");
  assert.equal(await bob.evaluate(() => window.__displayAudio()), true);
  await act(bob, "media:deafen");
  await act(bob, "media:mute");
  checks.push("a muted or deafened sharer's stream keeps its sound");

  const voiceTile = alice.locator('.participant-tile[data-kind="audio"]:not([data-self])');
  const screenTile = alice.locator('.participant-tile[data-kind="screen"]');
  await screenTile.hover();
  await screenTile.locator(".tile-volume input").fill("30");
  assert.deepEqual(await sound(alice, "system_audio"), { hidden: true, muted: false, volume: 0.3, paused: false });
  assert.equal((await sound(alice, "audio")).volume, 1, "stream volume leaves the person's voice alone");
  await screenTile.locator(".tile-volume button").click();
  assert.equal((await sound(alice, "system_audio")).muted, true);
  assert.equal((await sound(alice, "audio")).muted, false);
  assert.equal(await screenTile.locator(".tile-volume button").getAttribute("aria-pressed"), "true");
  await screenTile.locator(".tile-volume button").click();
  assert.equal((await sound(alice, "system_audio")).muted, false);
  await voiceTile.hover();
  // Above 100% the element is silent and a gain node plays it louder.
  await voiceTile.locator(".tile-volume input").fill("150");
  assert.equal((await sound(alice, "audio")).muted, true);
  assert.equal(await voiceTile.locator(".tile-volume-value").textContent(), "150%");
  await voiceTile.locator(".tile-volume input").fill("50");
  assert.equal((await sound(alice, "audio")).volume, 0.5);
  assert.equal(await alice.locator('.participant-tile.is-focused[data-kind="screen"]').count(), 1, "using a volume control does not change which tile is large");
  assert.equal(await voiceTile.locator(".tile-volume button").isVisible(), true, "small tiles keep their mute button");
  await screenTile.locator("video").click();
  assert.equal(await alice.locator(".participant-tile.is-focused").count(), 0);
  await act(alice, "media:deafen");
  await alice.waitForFunction(() => [...document.querySelectorAll(".participant-tile audio")].every((a) => a.muted));
  await act(alice, "media:deafen");
  assert.deepEqual(await sound(alice, "system_audio"), { hidden: true, muted: false, volume: 0.3, paused: false });
  const stored = await alice.evaluate(() => JSON.parse(localStorage.getItem("sticktalk-call-volumes")).map(([id, v]) => [id.split(":")[1], v]));
  assert.deepEqual(stored, [["stream", { volume: 0.3, muted: false }], ["voice", { volume: 0.5, muted: false }]]);
  checks.push("volume and mute per person and per stream; deafen still silences everything; choices are remembered");

  await screenTile.locator("video").click();
  await alice.waitForSelector('.participant-tile.is-focused[data-kind="screen"] .tile-control');
  await screenTile.locator(".tile-control").click();
  await alice.waitForSelector('.participant-tile.is-controlling[data-kind="screen"]', { timeout: 20000 });
  await bob.waitForSelector('#live-control-banner:has-text("alice_call is controlling your screen")');
  const picture = await screenTile.locator("video").boundingBox();
  await alice.mouse.move(picture.x + picture.width / 2, picture.y + picture.height / 2);
  await alice.mouse.down();
  await alice.mouse.up();
  await alice.keyboard.press("a");
  await alice.mouse.wheel(0, 100);
  await bob.waitForFunction(() => window.__host.some((m) => m.input?.type === "wheel"), null, { timeout: 15000 });
  const told = await bob.evaluate(() => window.__host);
  assert.deepEqual(told[0], { control: "start", who: "alice_call", label: told[0].label, surface: "monitor", width: 640, height: 360 });
  const inputs = told.filter((m) => m.control === "input").map((m) => m.input);
  const moved = inputs.find((i) => i.type === "move");
  assert(Math.abs(moved.data.x - 32768) < 400 && Math.abs(moved.data.y - 32768) < 400, `the middle of the picture is the middle of the screen: ${JSON.stringify(moved)}`);
  assert.deepEqual(inputs.filter((i) => i.type !== "move"), [
    { type: "button", data: { button: 0, down: true } },
    { type: "button", data: { button: 0, down: false } },
    { type: "key", data: { usage: 4, down: true } },
    { type: "key", data: { usage: 4, down: false } },
    { type: "wheel", data: { x: 0, y: -120 } },
  ]);
  await screenTile.locator(".tile-control").click();
  await bob.waitForFunction(() => window.__host.at(-1).control === "stop" && !document.querySelector("#live-control-banner"));
  assert.equal(await alice.locator(".participant-tile.is-controlling").count(), 0);
  checks.push("remote control: asked for, allowed by the sharer's app, pointer, clicks, keys and wheel delivered, then given back");

  await act(bob, "media:share");
  await alice.waitForFunction(() => !document.querySelector('.participant-tile[data-kind="screen"],.participant-tile[data-kind="system_audio"]'), null, { timeout: 15000 });
  assert.equal((await sound(alice, "audio")).volume, 0.5);
  checks.push("stopping the stream removes its picture and sound");
  await act(alice, "media:leave");
  await alice.waitForSelector("#live-input");
  assert.equal(await alice.locator(".voice-view").count(), 0);
  checks.push("leaving a call returns to the conversation you were in");
  await act(bob, "media:leave");
  assert.deepEqual(errors, []);
  for (const check of checks) console.log(`passed: ${check}`);
} finally {
  for (const b of browsers) await b.close();
  if (server.pid && server.exitCode === null) {
    const exit = once(server, "exit");
    server.kill();
    await exit;
  }
  assert.equal(dirname(resolve(data)), target);
  assert(basename(data).startsWith("call-composer-test-"));
  await rm(data, { recursive: true, force: true });
}

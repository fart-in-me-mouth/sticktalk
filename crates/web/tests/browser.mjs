import { chromium } from "playwright";
import assert from "node:assert/strict";
import { mkdir, writeFile, readFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import { once } from "node:events";
const base = "http://127.0.0.1:4173";
const server = spawn(process.execPath, ["scripts/serve.mjs"], {
  stdio: ["ignore", "pipe", "pipe"],
  windowsHide: true,
});
await once(server.stdout, "data");
const browser = await chromium.launch({
  headless: true,
  args: ["--mute-audio", "--disable-notifications"],
});
const errors = [];
const results = [];
const cases = [
  ["login", "screen=login"],
  ["register", "screen=register"],
  ["home", "screen=home"],
  ["dm", "screen=dm"],
  ["chat", "screen=chat"],
  ["user-settings", "screen=user-settings"],
  ["server-settings", "screen=server-settings"],
  ["roles", "screen=roles"],
  ["role-editor", "screen=role-editor"],
  ["role-permissions", "screen=role-editor&tab=Permissions"],
  ["role-members", "screen=role-editor&tab=Manage%20Members"],
  ["user-profile", "screen=user-settings&setting=Profiles"],
  ["devices", "screen=user-settings&setting=Devices"],
  ["server-members", "screen=server-settings&setting=Members"],
  ["voice", "screen=voice"],
  ["stream", "screen=stream"],
  ["profile", "screen=chat&profile=1"],
  ["create-server-choice", "modal=create"],
  ["create-server", "modal=create-server"],
  ["join-server", "modal=join-server"],
  ["invite", "modal=invite"],
  ["notifications", "screen=notifications"],
  ["notification-settings", "modal=notifications"],
  ["quick-switcher", "modal=quick"],
  ["share-screen", "modal=share"],
  ["image-viewer", "modal=image"],
  ["emoji-picker", "modal=emoji"],
  ["channel-settings", "modal=channel"],
  ["disconnected", "offline=1"],
  ["empty-channel", "screen=empty"],
  ["loading", "screen=loading"],
  ["server-menu", "screen=chat"],
  ["message-context-menu", "screen=chat"],
];
const widths = [
  [1440, 900],
  [390, 844],
  [412, 915],
  [820, 1180],
];
await mkdir("screenshots", { recursive: true });
async function ready(page) {
  await page.waitForSelector("#sticktalk");
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(180);
}
async function capture(page, name, w, h) {
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth > innerWidth,
  );
  assert.equal(overflow, false, `${name} overflow`);
  const broken = await page
    .locator("img")
    .evaluateAll((images) =>
      images
        .filter(
          (i) => i.getClientRects().length && i.complete && !i.naturalWidth,
        )
        .map((i) => i.src),
    );
  assert.deepEqual(broken, [], `${name} broken images`);
  const path = `screenshots/${name}-${w}x${h}.png`;
  await page.screenshot({ path, animations: "disabled", scale: "css" });
  results.push({ name, width: w, height: h, path });
}
try {
  for (const [w, h] of widths) {
    const context = await browser.newContext({
      viewport: { width: w, height: h },
      deviceScaleFactor: 1,
      isMobile: w < 768,
      hasTouch: w < 768,
      reducedMotion: "reduce",
    });
    const page = await context.newPage();
    page.on("pageerror", (e) => errors.push(e.stack));
    for (const [name, query] of cases) {
      await page.goto(`${base}/?${query}`);
      await ready(page);
      // Encoded fixture route values are exercised by actions as a real user does.
      if (name === "server-menu") {
        if (w < 768)
          await page
            .getByRole("button", { name: "Open Channels", exact: true })
            .click();
        await page.locator(".server-heading").click();
      }
      if (name === "message-context-menu")
        await page.locator("[data-message]").last().click({ button: "right" });
      if (name === "role-members")
        await page.getByRole("tab", { name: "Manage Members" }).click();
      await capture(page, name, w, h);
    }
    if (w < 768) {
      for (const panel of ["left", "right"]) {
        await page.goto(base);
        await ready(page);
        await page
          .getByRole("button", {
            name: panel === "left" ? "Open Channels" : "Member List",
            exact: true,
          })
          .click();
        await page.waitForTimeout(280);
        await capture(page, `phone-${panel}-panel`, w, h);
      }
      await page.goto(base);
      await ready(page);
      // Real CDP touch down held for >500 ms exercises long-press, not a fixture route.
      const cdp = await context.newCDPSession(page);
      const box = await page.locator("[data-message]").last().boundingBox();
      await cdp.send("Input.dispatchTouchEvent", {
        type: "touchStart",
        touchPoints: [{ x: box.x + 60, y: box.y + 20 }],
      });
      await page.waitForTimeout(600);
      await cdp.send("Input.dispatchTouchEvent", {
        type: "touchEnd",
        touchPoints: [],
      });
      await page.getByRole("dialog", { name: "Message Actions" }).waitFor();
      await capture(page, "phone-long-press", w, h);
      await page.goto(base);
      await ready(page);
      // Headless Chromium has no OS IME. Emulate the visualViewport resize contract
      // and draw a clearly labeled test keyboard, never open an OS keyboard/window.
      await page.evaluate(() => {
        Object.defineProperty(window.visualViewport, "height", {
          configurable: true,
          get: () => innerHeight - 300,
        });
        window.visualViewport.dispatchEvent(new Event("resize"));
        const k = document.createElement("div");
        k.id = "test-keyboard";
        k.setAttribute("aria-label", "Emulated keyboard");
        k.style.cssText =
          "position:fixed;bottom:0;left:0;right:0;height:300px;background:#29292D;padding:12px 4px;z-index:120;font:14px Inter;color:#E8EAED;display:flex;flex-direction:column;gap:12px";
        for (const text of [
          "Emulated keyboard · visualViewport test",
          "q w e r t y u i o p",
          "a s d f g h j k l",
          "⇧ z x c v b n m ⌫",
          "123     space     return",
        ]) {
          const row = document.createElement("div");
          row.style.cssText =
            "display:flex;justify-content:center;gap:4px;flex:1;align-items:center";
          for (const letter of text.startsWith("Emulated")
            ? [text]
            : text.split(" ")) {
            if (!letter) continue;
            const key = document.createElement("span");
            key.textContent = letter;
            key.style.cssText = text.startsWith("Emulated")
              ? "font-size:12px;color:#A8A8B0"
              : "display:flex;align-items:center;justify-content:center;background:#45454D;border-radius:6px;height:40px;min-width:28px;padding:4px";
            row.append(key);
          }
          k.append(row);
        }
        document.body.append(k);
        document.querySelector("#message-input").focus();
      });
      const rect = await page.locator(".composer").boundingBox();
      assert(rect.y + rect.height <= h - 300, "composer above keyboard");
      await capture(page, "phone-keyboard-open", w, h);
      // Verify swipe physics settle to the correct panel without animation loops.
      await page.goto(base);
      await ready(page);
      await cdp.send("Input.dispatchTouchEvent", {
        type: "touchStart",
        touchPoints: [{ x: 20, y: 300 }],
      });
      for (const x of [50, 100, 160, 240])
        await cdp.send("Input.dispatchTouchEvent", {
          type: "touchMove",
          touchPoints: [{ x, y: 300 }],
        });
      await cdp.send("Input.dispatchTouchEvent", {
        type: "touchEnd",
        touchPoints: [],
      });
      await page.waitForSelector(".panel-left");
    }
    await context.close();
  }
  const context = await browser.newContext({
    viewport: { width: 1440, height: 900 },
  });
  const page = await context.newPage();
  page.on("pageerror", (e) => errors.push(e.stack));
  await page.goto(base);
  await ready(page);
  // Visible-window bound, top/bottom spacers and fixed-height history correctness.
  await page.locator("#messages").evaluate((e) => {
    e.scrollTop = 88 * 60;
  });
  await page.waitForTimeout(100);
  assert(
    (await page.locator("[data-history-row]").count()) <= 32,
    "bounded history DOM",
  );
  assert(
    (await page.locator("[data-history-row]").count()) > 10,
    "history is actually rendered",
  );
  await page.getByRole("button", { name: "Jump to present" }).click();
  await page
    .locator("#message-input")
    .fill("A local message from the browser test.");
  await page.locator("#message-input").press("Enter");
  await page
    .getByText("A local message from the browser test.", { exact: true })
    .waitFor();
  await page.keyboard.press("Control+k");
  await page.getByRole("dialog", { name: "Quick Switcher" }).waitFor();
  await page.getByPlaceholder("Where would you like to go?").fill("prj");
  await page.keyboard.press("Enter");
  await page.waitForSelector("[data-screen=chat]");
  assert.equal(
    await page.locator(".channel-header h1").textContent(),
    "projects",
  );
  await page.goto(base + "/?screen=role-editor");
  await ready(page);
  await page
    .getByRole("switch", { name: /Display role members separately/ })
    .click();
  assert.equal(
    await page
      .getByRole("switch", { name: /Display role members separately/ })
      .getAttribute("aria-checked"),
    "false",
  );
  await page.getByRole("button", { name: "Save Changes", exact: true }).click();
  await page.getByRole("status").waitFor();
  await page.goto(base + "/?modal=delete-server");
  await ready(page);
  assert(await page.locator("[data-confirm]").isDisabled());
  await page.locator("#confirm-server").fill("The Den");
  assert(!(await page.locator("[data-confirm]").isDisabled()));
  await page.goto(base + "/?screen=login");
  await ready(page);
  await page.getByRole("button", { name: "Log In", exact: true }).click();
  assert(await page.locator("#auth-error").isVisible());
  await page.goto(base);
  await ready(page);
  await page.evaluate(() => navigator.serviceWorker.ready);
  await page.reload();
  await ready(page);
  assert(
    await page.evaluate(() => !!navigator.serviceWorker.controller),
    "worker controls app",
  );
  const cache = await page.evaluate(async () => await caches.keys());
  assert(
    cache.some((k) => /^sticktalk-shell-[a-f0-9]{20}$/.test(k)),
    "content hashed shell cache",
  );
  await context.setOffline(true);
  await page.reload();
  await ready(page);
  await page.locator(".connection-banner").waitFor();
  assert.match(
    await page.locator(".connection-banner").textContent(),
    /Reconnecting…/,
  );
  await context.setOffline(false);
  await context.close();
  await fetch(base + "/__metrics", { method: "POST" });
  const coldContext = await browser.newContext({
    viewport: { width: 390, height: 844 },
  });
  const coldPage = await coldContext.newPage();
  await coldPage.goto(base);
  await ready(coldPage);
  await coldPage.evaluate(() => navigator.serviceWorker.ready);
  const firstLoad = await (await fetch(base + "/__metrics")).json();
  assert(
    firstLoad.transferBytes <= 2500000,
    "measured cold-load transfer budget",
  );
  await coldContext.close();
  assert.deepEqual(errors, [], "no browser exceptions");
  const sizes = JSON.parse(await readFile("dist/sizes.json", "utf8"));
  const wasm = sizes.assets.find((a) => a.file.endsWith(".wasm"));
  assert(wasm.brotli <= 1500000, "wasm budget");
  assert(sizes.firstLoadBytes <= 2500000, "entire shell transfer budget");
  await writeFile(
    "screenshots/browser-results.json",
    JSON.stringify(
      {
        browser: browser.version(),
        headless: true,
        firstLoad,
        checks: [
          "Responsive screen captures",
          "mobile panels",
          "CDP long-press",
          "CDP swipe",
          "emulated visualViewport keyboard",
          "virtual history bounds",
          "local send",
          "quick switcher",
          "role toggle/save",
          "destructive confirmation",
          "auth errors",
          "offline reload",
          "hashed service worker",
          "no console exceptions",
          "payload budgets",
        ],
        screenshots: results,
        errors,
        sizes,
      },
      null,
      2,
    ),
  );
  console.log(
    JSON.stringify({
      firstLoad,
      screenshots: results.length,
      errors: errors.length,
      wasm,
      compressed: sizes.compressed,
    }),
  );
} finally {
  await browser.close();
  server.kill();
}

// Current Lighthouse only; real loopback server and headless Chromium.
import { chromium } from "playwright";
import lighthouse from "lighthouse";
import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { once } from "node:events";
import { mkdtemp, rm, readFile, writeFile, mkdir } from "node:fs/promises";
import { resolve, join, dirname } from "node:path";
import assert from "node:assert/strict";
const target = resolve(
  process.env.CARGO_TARGET_DIR ??
    "C:/Users/dwhee/.cargo-target/sticktalk-web-live",
);
const data = await mkdtemp(join(target, "audit-data-"));
const profile = await mkdtemp(join(target, "audit-browser-"));
async function freePort() {
  const r = createServer();
  r.listen(0, "127.0.0.1");
  await once(r, "listening");
  const port = r.address().port;
  await new Promise((done) => r.close(done));
  return port;
}
const port = await freePort(),
  debugPort = await freePort();
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
let context,
  log = "";
try {
  await new Promise((done, fail) => {
    const timeout = setTimeout(() => fail(Error(log)), 30000);
    const collect = (c) => {
      log += c;
      if (log.includes("StickTalk listening")) {
        clearTimeout(timeout);
        done();
      }
    };
    server.stdout.on("data", collect);
    server.stderr.on("data", collect);
    server.once("error", fail);
  });
  await mkdir("screenshots/live", { recursive: true });
  context = await chromium.launchPersistentContext(profile, {
    headless: true,
    args: [
      `--remote-debugging-port=${debugPort}`,
      "--remote-debugging-address=127.0.0.1",
      "--mute-audio",
      "--disable-notifications",
    ],
  });
  const options = {
    port: debugPort,
    hostname: "127.0.0.1",
    logLevel: "error",
    output: "json",
    onlyCategories: ["performance", "accessibility", "best-practices"],
    maxWaitForLoad: 45000,
    disableStorageReset: true,
  };
  const mobile = await lighthouse(origin, options);
  await writeFile("screenshots/live/lighthouse-mobile.json", mobile.report);
  const desktop = await lighthouse(origin, {
    ...options,
    formFactor: "desktop",
    screenEmulation: {
      mobile: false,
      width: 1440,
      height: 900,
      deviceScaleFactor: 1,
      disabled: false,
    },
    throttling: { rttMs: 40, throughputKbps: 10240, cpuSlowdownMultiplier: 1 },
  });
  await writeFile("screenshots/live/lighthouse-desktop.json", desktop.report);
  // Do not conflate a fast progressive login with a cold authenticated chat.
  const page = await context.newPage();
  await page.goto(origin);
  await page.locator('[data-live="register"]').click();
  await page.locator("#auth-username").fill("lighthouse_user");
  await page.locator("#auth-password").fill("lighthouse-password-1234");
  await page.locator('[type="submit"]').click();
  await page.waitForSelector(".live-home");
  await page.waitForFunction(
    () => !document.querySelector(".connection-banner"),
  );
  await page.evaluate(() =>
    window.dispatchEvent(
      new CustomEvent("sticktalk-live", { detail: "dialog:create" }),
    ),
  );
  await page.locator("#f-name").fill("Lighthouse Server");
  await page.locator('.live-modal [type="submit"]').click();
  await page.waitForSelector("#live-messages");
  const channel = await page
    .locator("#live-messages")
    .getAttribute("data-channel");
  await page.evaluate(async () => {
    for (const r of await navigator.serviceWorker.getRegistrations())
      await r.unregister();
    for (const key of await caches.keys()) await caches.delete(key);
  });
  const cdp = await context.newCDPSession(page);
  await cdp.send("Network.clearBrowserCache");
  // The protocol permits one WS per cookie session. Leaving the seed page open
  // makes it and Lighthouse repeatedly replace each other's socket, corrupting
  // TTI with reconnect work. Measure exactly one application page at a time.
  for (const open of context.pages()) await open.close();
  await context.newPage();
  const chat = await lighthouse(`${origin}/?channel=${channel}`, options);
  await writeFile("screenshots/live/lighthouse-chat-mobile.json", chat.report);
  for (const open of context.pages()) {
    if (open.url().startsWith(origin)) await open.evaluate(async()=>{
      if ('serviceWorker' in navigator) await Promise.race([navigator.serviceWorker.ready,new Promise(done=>setTimeout(done,2000))]);
    });
    await open.close();
  }
  await context.newPage();
  // The same authenticated URL, keeping HTTP/SW caches and the cookie session.
  // Real DevTools throttling respects cache hits rather than simulating a cold transfer.
  const warm = await lighthouse(`${origin}/?channel=${channel}`, {
    ...options, throttlingMethod:"devtools",
    throttling:{rttMs:150,throughputKbps:1638.4,cpuSlowdownMultiplier:4,requestLatencyMs:150,downloadThroughputKbps:1638.4,uploadThroughputKbps:750},
  });
  await writeFile("screenshots/live/lighthouse-chat-warm-mobile.json", warm.report);

  const summarize = (r) => ({
    version: r.lighthouseVersion,
    categories: Object.fromEntries(
      Object.entries(r.categories).map(([k, v]) => [
        k,
        Math.round(v.score * 100),
      ]),
    ),
    metrics: Object.fromEntries(
      [
        "first-contentful-paint",
        "largest-contentful-paint",
        "total-blocking-time",
        "cumulative-layout-shift",
        "interactive",
      ]
        .filter((k) => r.audits[k])
        .map((k) => [
          k,
          {
            display: r.audits[k].displayValue,
            value: r.audits[k].numericValue,
          },
        ]),
    ),
    appReady: r.audits["user-timings"]?.details?.items?.filter(i=>JSON.stringify(i).includes("sticktalk-chat-ready")) ?? [],
    failures: Object.values(r.audits)
      .filter((a) => a.score !== null && a.score < 0.9)
      .map((a) => ({
        id: a.id,
        title: a.title,
        score: a.score,
        details: a.details,
      })),
  });
  const summary = {
    bundleRevision: JSON.parse(
      await readFile(join(process.env.STICKTALK_WEB_DIST ?? join(target, "web-dist"), "sizes.json"), "utf8"),
    ).revision,
    headless: true,
    realServer: true,
    page: "Cold progressive login; authenticated chat measured separately",
    mobile: summarize(mobile.lhr),
    authenticatedChatMobile: summarize(chat.lhr),
    warmAuthenticatedChatMobile: summarize(warm.lhr),
    desktop: summarize(desktop.lhr),
  };
  await writeFile(
    "screenshots/live/lighthouse-summary.json",
    JSON.stringify(summary, null, 2),
  );
  console.log(
    JSON.stringify({
      mobile: summary.mobile.categories,
      desktop: summary.desktop.categories,
      mobileMetrics: summary.mobile.metrics,
      chatMobile: summary.authenticatedChatMobile.categories,
      chatMobileMetrics: summary.authenticatedChatMobile.metrics,
      warmChatMetrics: summary.warmAuthenticatedChatMobile.metrics,
      desktopMetrics: summary.desktop.metrics,
    }),
  );
} finally {
  await context?.close();
  if (server.pid && server.exitCode === null) {
    const exit = once(server, "exit");
    server.kill();
    await exit;
  }
  for (const path of [data, profile]) {
    assert.equal(dirname(path), target);
    await rm(path, { recursive: true, force: true });
  }
}

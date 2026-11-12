// Real same-origin server + CSP integration. Headless, loopback, no permissions.
import { chromium } from "playwright";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { resolve, join, dirname, basename } from "node:path";
import { tmpdir } from "node:os";
import { once } from "node:events";
const exe = process.env.STICKTALK_SERVER_EXE;
assert(exe, "Set STICKTALK_SERVER_EXE to the built native server");
const baseDir = resolve(process.env.CARGO_TARGET_DIR ?? tmpdir());
const data = await mkdtemp(join(baseDir, "sticktalk-web-server-"));
const reservation = createServer();
reservation.listen(0, "127.0.0.1");
await once(reservation, "listening");
const port = reservation.address().port;
await new Promise((done) => reservation.close(done));
const origin = `http://127.0.0.1:${port}`;
const server = spawn(exe, ["--bind", `127.0.0.1:${port}`, "--public-url", origin,
  "--data-dir", data, "--web-root", resolve(process.env.TRUNK_BUILD_DIST ?? "dist"),
  "--min-free-disk-gb", "0", "--voice-bind-ip", "127.0.0.1",
  "--voice-advertised-ip", "127.0.0.1"], { windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
let browser;
let log = "";
try {
  await new Promise((done, fail) => {
    const timer = setTimeout(() => fail(new Error(`Server startup timeout: ${log}`)), 30000);
    const collect = (chunk) => {
      log = (log + chunk).slice(-16000);
      if (log.includes("StickTalk listening")) { clearTimeout(timer); done(); }
    };
    server.stdout.on("data", collect);
    server.stderr.on("data", collect);
    server.once("error", (error) => { clearTimeout(timer); fail(error); });
    server.once("exit", (code) => { clearTimeout(timer); fail(new Error(`Server exited ${code}: ${log}`)); });
  });
  browser = await chromium.launch({ headless: true, args: ["--mute-audio", "--disable-notifications"] });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const errors = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  await page.addInitScript(() => {
    window.__cspViolations = [];
    document.addEventListener("securitypolicyviolation", (event) => window.__cspViolations.push(event.effectiveDirective));
  });
  const response = await page.goto(origin, { waitUntil: "networkidle" });
  assert.equal(response.status(), 200);
  const csp = response.headers()["content-security-policy"];
  assert(csp.includes("script-src 'self' 'wasm-unsafe-eval';"));
  assert(csp.includes("style-src 'self'; style-src-attr 'unsafe-inline';"));
  await page.waitForSelector("#sticktalk");
  await page.evaluate(() => document.fonts.ready);
  assert.equal(await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--accent").trim().toUpperCase()), "#3DDC97");
  assert(await page.evaluate(() => getComputedStyle(document.querySelector(".avatar")).getPropertyValue("--avatar-color").trim()));
  assert.deepEqual(await page.evaluate(() => window.__cspViolations), [], "no application CSP violations");
  assert.deepEqual(errors, []);
  await page.evaluate(() => {
    const script = document.createElement("script");
    script.textContent = "window.__inlineProbe = true";
    document.head.append(script);
    const style = document.createElement("style");
    style.textContent = "html { --inline-probe: forbidden }";
    document.head.append(style);
  });
  await page.waitForFunction(() => window.__cspViolations.length >= 2);
  assert.equal(await page.evaluate(() => window.__inlineProbe), undefined);
  assert.equal(await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--inline-probe")), "");
  const result = { realServer: true, headless: true, loopback: true, applicationErrors: errors,
    applicationCspViolations: 0, blockedInlineProbes: await page.evaluate(() => window.__cspViolations), csp };
  if (process.env.STICKTALK_SERVER_SMOKE_REPORT) await writeFile(process.env.STICKTALK_SERVER_SMOKE_REPORT, JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result));
} finally {
  await browser?.close();
  if (server.pid && server.exitCode === null && server.signalCode === null) { const exited = once(server, "exit"); server.kill(); await exited; }
  // Verify the resolved deletion target stays in this test's own temporary root.
  assert.equal(dirname(resolve(data)), baseDir);
  assert(basename(data).startsWith("sticktalk-web-server-"));
  await rm(data, { recursive: true, force: true });
}

import { fileURLToPath } from "node:url";
// Read-only 60-second idle browser benchmark. No OS window, sounds or hardware.
import { chromium } from "playwright";
import { spawn, execFile } from "node:child_process";
import { promisify } from "node:util";
import { createServer } from "node:net";
import { once } from "node:events";
import { mkdtemp, rm, readFile, writeFile, mkdir } from "node:fs/promises";
import { resolve, join, dirname } from "node:path";
import assert from "node:assert/strict";
const output = process.env.STICKTALK_QA_DIR ?? "screenshots/live";
const target = resolve(
  process.env.CARGO_TARGET_DIR ??
    fileURLToPath(new URL("../../../target/", import.meta.url)),
);
await mkdir(target, { recursive: true });
const data = await mkdtemp(join(target, "idle-web-"));
const reserve = createServer();
reserve.listen(0, "127.0.0.1");
await once(reserve, "listening");
const port = reserve.address().port;
await new Promise((r) => reserve.close(r));
const origin = `http://127.0.0.1:${port}`;
const server = spawn(
  process.env.STICKTALK_SERVER_EXE ??
    join(target, `debug/sticktalk-server${process.platform === "win32" ? ".exe" : ""}`),
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
let browser,
  log = "";
try {
  await new Promise((done, fail) => {
    const timer = setTimeout(() => fail(Error(log)), 30000);
    const collect = (c) => {
      log += c;
      if (log.includes("StickTalk listening")) {
        clearTimeout(timer);
        done();
      }
    };
    server.stdout.on("data", collect);
    server.stderr.on("data", collect);
    server.once("error", fail);
  });
  browser = await chromium.launch({
    headless: true,
    args: ["--mute-audio", "--disable-notifications"],
  });
  const browserCdp = await browser.newBrowserCDPSession();
  const page = await browser.newPage({
    viewport: { width: 1440, height: 900 },
  });
  async function memory() {
    const { processInfo } = await browserCdp.send("SystemInfo.getProcessInfo");
    const ids = processInfo.map((p) => Number(p.id));
    assert(ids.every((n) => Number.isSafeInteger(n) && n > 0));
    const command = `Get-Process -Id @(${ids.join(",")}) -ErrorAction SilentlyContinue | Select-Object Id,WorkingSet64,PrivateMemorySize64 | ConvertTo-Json -Compress`;
    const { stdout } = await promisify(execFile)(
      "powershell.exe",
      ["-NoProfile", "-NonInteractive", "-Command", command],
      { windowsHide: true },
    );
    const value = JSON.parse(stdout);
    const rows = Array.isArray(value) ? value : [value];
    const byId = new Map(rows.map((r) => [Number(r.Id), r]));
    return {
      processes: processInfo.map((p) => ({ id: p.id, type: p.type, privateBytes: byId.get(Number(p.id))?.PrivateMemorySize64 ?? 0 })),
      workingSetBytes: rows.reduce((n, p) => n + p.WorkingSet64, 0),
      privateBytes: rows.reduce((n, p) => n + p.PrivateMemorySize64, 0),
    };
  }
  const blank = await memory();
  await page.goto(origin);
  await page.locator("[data-live=register]").click();
  await page.locator("#auth-username").fill("idle_benchmark");
  await page.locator("#auth-password").fill("idle-benchmark-password-1234");
  await page.locator("[type=submit]").click();
  await page.waitForSelector(".live-home");
  await page.waitForFunction(
    () => !document.querySelector(".connection-banner"),
  );
  await page.evaluate(() => navigator.serviceWorker.ready);
  await new Promise((r) => setTimeout(r, 2000));
  const before = (await browserCdp.send("SystemInfo.getProcessInfo"))
    .processInfo;
  const start = performance.now();
  await new Promise((r) => setTimeout(r, 60000));
  const elapsedMs = performance.now() - start;
  const after = (await browserCdp.send("SystemInfo.getProcessInfo"))
    .processInfo;
  const baseline = new Map(before.map((p) => [p.id, p.cpuTime]));
  const cpuSeconds = after.reduce(
    (n, p) => n + Math.max(0, p.cpuTime - (baseline.get(p.id) ?? 0)),
    0,
  );
  const chat = await memory();
  // What the idle page holds (docs/features/perf.md): the JS heap, and the
  // WASM modules and scripts it has loaded.
  const pageCdp = await page.context().newCDPSession(page);
  await pageCdp.send("Performance.enable");
  const metrics = Object.fromEntries((await pageCdp.send("Performance.getMetrics")).metrics.map((m) => [m.name, m.value]));
  const loaded = await page.evaluate(() => performance.getEntriesByType("resource")
    .filter((e) => /\.(wasm|js)$/.test(new URL(e.name).pathname))
    .map((e) => ({ path: new URL(e.name).pathname, transferBytes: e.transferSize, bytes: e.decodedBodySize })));
  const sum = (list) => list.reduce((n, e) => n + e.bytes, 0);
  const report = {
    bundleRevision: JSON.parse(
      await readFile(join(process.env.STICKTALK_WEB_DIST ?? join(target, "web-dist"), "sizes.json"), "utf8"),
    ).revision,
    headless: true,
    realServer: true,
    view: "Authenticated empty Friends screen; voice/video inactive",
    sampleMs: elapsedMs,
    cpuPercentOfOneLogicalCore: (100 * cpuSeconds) / (elapsedMs / 1000),
    blank,
    authenticated: chat,
    deltaPrivateBytes: chat.privateBytes - blank.privateBytes,
    deltaWorkingSetBytes: chat.workingSetBytes - blank.workingSetBytes,
    jsHeapUsedBytes: metrics.JSHeapUsedSize,
    jsHeapTotalBytes: metrics.JSHeapTotalSize,
    domNodes: metrics.Nodes,
    wasmLoadedBytes: sum(loaded.filter((e) => e.path.endsWith(".wasm"))),
    scriptLoadedBytes: sum(loaded.filter((e) => e.path.endsWith(".js"))),
    loaded,
    note: "Whole separate headless Chromium process-tree totals; working-set sum double-counts shared pages. Not native-client or physical-phone memory.",
  };
  await writeFile(
    `${output}/idle.json`,
    JSON.stringify(report, null, 2),
  );
  console.log(JSON.stringify(report));
} finally {
  await browser?.close();
  if (server.pid && server.exitCode === null) {
    const exit = once(server, "exit");
    server.kill();
    await exit;
  }
  assert.equal(dirname(resolve(data)), target);
  await rm(data, { recursive: true, force: true });
}

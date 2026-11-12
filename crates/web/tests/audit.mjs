import { chromium } from "playwright";
import lighthouse from "lighthouse";
import lighthousePwa from "lighthouse-pwa";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { writeFile } from "node:fs/promises";
const server = spawn(process.execPath, ["scripts/serve.mjs"], {
  stdio: ["ignore", "pipe", "pipe"],
  windowsHide: true,
});
await once(server.stdout, "data");
const browser = await chromium.launch({
  headless: true,
  args: [
    "--remote-debugging-address=127.0.0.1",
    "--remote-debugging-port=9225",
    "--mute-audio",
    "--disable-notifications",
  ],
});
try {
  const options = {
    port: 9225,
    hostname: "127.0.0.1",
    logLevel: "error",
    output: "json",
    onlyCategories: ["performance", "accessibility", "best-practices"],
    maxWaitForLoad: 45000,
  };
  const current = await lighthouse("http://127.0.0.1:4173/", options);
  await writeFile("screenshots/lighthouse-mobile.json", current.report);
  const desktop = await lighthouse("http://127.0.0.1:4173/", {
    ...options,
    formFactor: "desktop",
    screenEmulation: {
      mobile: false,
      width: 1440,
      height: 900,
      deviceScaleFactor: 1,
      disabled: false,
    },
    throttlingMethod: "simulate",
    throttling: { rttMs: 40, throughputKbps: 10240, cpuSlowdownMultiplier: 1 },
  });
  await writeFile("screenshots/lighthouse-desktop.json", desktop.report);
  // Lighthouse 12 removed the PWA category; retain the last supported audit
  // separately and identify its version, never invent a current PWA score.
  const pwa = await lighthousePwa("http://127.0.0.1:4173/", {
    port: 9225,
    hostname: "127.0.0.1",
    logLevel: "error",
    output: "json",
    onlyCategories: ["pwa"],
    maxWaitForLoad: 45000,
  });
  await writeFile("screenshots/lighthouse-pwa-legacy.json", pwa.report);
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
        .map((k) => [k, r.audits[k].displayValue]),
    ),
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
    headless: true,
    origin: "http://127.0.0.1:4173",
    mobile: summarize(current.lhr),
    desktop: summarize(desktop.lhr),
    pwaLegacy: summarize(pwa.lhr),
  };
  await writeFile(
    "screenshots/lighthouse-summary.json",
    JSON.stringify(summary, null, 2),
  );
  console.log(
    JSON.stringify({
      mobile: summary.mobile.categories,
      desktop: summary.desktop.categories,
      pwa: summary.pwaLegacy.categories,
    }),
  );
} finally {
  await browser.close();
  server.kill();
}

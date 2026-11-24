import { readFile, writeFile, readdir, stat, unlink } from "node:fs/promises";
import { resolve, join, relative } from "node:path";
import { createHash } from "node:crypto";
import { brotliCompressSync, constants } from "node:zlib";
import {versionRuntime} from './version-runtime.mjs';
await import('./split.mjs');
const dist = resolve(process.env.TRUNK_STAGING_DIR ?? "dist");
async function hashedAsset(prefix, extension, content) {
  const digest = createHash("sha256")
    .update(content)
    .digest("hex")
    .slice(0, 20);
  const name = `${prefix}-${digest}.${extension}`;
  await writeFile(join(dist, name), content);
  return name;
}
// One authoritative token generator, moved from build.rs so CSS ships as an
// external stylesheet rather than a CSP-blocked runtime <style> element.
const spec = await readFile(
  new URL("../../../docs/UI_SPEC.md", import.meta.url),
  "utf8",
);
// Each "### Color (...)" table becomes one block of custom properties: the
// default dark one on :root, the additions (themes task) under the theme base
// attribute that `public/theme-boot.js` sets before first paint. "Sync with
// system" follows the OS live through the media query, with no script.
function palette(section) {
  let block = "";
  for (const line of section
    .split(/\r?\n/)
    .filter((line) => line.startsWith("| "))) {
    const cols = line.split("|").map((col) => col.trim());
    if (cols.length < 4) continue;
    const [key, value] = cols.slice(1, 3);
    if (key === "online / idle / dnd / offline") {
      const colors = value.split(" / ");
      key.split(" / ").forEach((name, i) => {
        block += `  --${name}: ${colors[i]};\n`;
      });
    } else if (value.startsWith("#")) {
      const alpha = value.match(/^#([0-9a-f]{6}) @ (\d+)% alpha$/i);
      const css = alpha
        ? `rgb(${[0, 2, 4].map((i) => parseInt(alpha[1].slice(i, i + 2), 16)).join(" ")} / ${alpha[2]}%)`
        : value;
      block += `  --${key.replaceAll(".", "-")}: ${css};\n`;
    }
  }
  return block;
}
const sections = Object.fromEntries(
  spec
    .split(/\r?\n### Color \(/)
    .slice(1)
    .map((part) => [part.slice(0, part.indexOf(")")), part.split(/\r?\n#/)[0]]),
);
const lightScheme = "  color-scheme: light; --name-l-min: 0; --name-l-max: 0.48;\n";
let tokens = "/* Generated from docs/UI_SPEC.md. */\n:root {\n";
tokens += palette(sections["dark, default"]);
for (const size of [4, 8, 12, 16, 24, 32])
  tokens += `  --space-${size}: ${size}px;\n`;
tokens += "  --radius-small: 4px; --radius: 8px; --radius-modal: 12px;\n";
tokens += "  --name-l-min: 0.7; --name-l-max: 1; --saturation: 1;\n}\n";
tokens +=
  ":root { --text-muted: #A0A0AA; } /* Lead-approved WCAG AA override. */\n";
const light = palette(sections["light"]);
tokens += `:root[data-theme-base="light"] {\n${lightScheme}${light}}\n`;
tokens += `@media (prefers-color-scheme: light) {\n:root[data-theme-base="system"] {\n${lightScheme}${light}}\n}\n`;
tokens += `:root[data-theme-base="darker"] {\n${palette(sections["darker"])}}\n`;
const tokenAsset = await hashedAsset("ui-tokens", "css", tokens);
let html = await readFile(join(dist, "index.html"), "utf8");
for (const match of [
  ...html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/gi),
]) {
  if (/\bsrc\s*=/i.test(match[1]) || !match[2].trim()) continue;
  let dynamic = match[2].replace(
    /import init, \* as bindings from (['"][^'"]+['"]);/,
    "const bindings = await import($1); const init = bindings.default;",
  );
  if (dynamic === match[2]) throw new Error("Unexpected Trunk bootstrap shape");
  // Compile the core in parallel with loading its JS imports and recovering the
  // session, not at the bottom of that waterfall. Kept behind loadApp on login.
  const core = dynamic.match(/(['"])(\.\/[^'"]+\.wasm)\1/);
  if (!core) throw new Error('Missing Trunk core WASM URL');
  dynamic = `const coreModule = WebAssembly.compileStreaming(fetch(${JSON.stringify(core[2])}));\n` + dynamic.replace(core[0], 'coreModule');
  const progressive = (
    await readFile(new URL("../public/boot.js", import.meta.url), "utf8")
  ).replace("__WASM_BOOTSTRAP__", dynamic);
  const asset = await hashedAsset("bootstrap", "js", progressive);
  html = html.replace(match[0], `<script${match[1]} src="/${asset}"></script>`);
}
for (const match of [...html.matchAll(/<style\b[^>]*>([\s\S]*?)<\/style>/gi)]) {
  const asset = await hashedAsset("boot", "css", match[1]);
  html = html.replace(match[0], `<link rel="stylesheet" href="/${asset}">`);
}
// No speculative heavy WASM fetch on the unauthenticated critical path.
html = html
  .replace(/<link\b[^>]*rel="modulepreload"[^>]*>/gi, "")
  .replace(
    /<link\b[^>]*rel="preload"[^>]*type="application\/wasm"[^>]*>/gi,
    "",
  );
if (process.env.STICKTALK_WEB_MOCK !== "1")
  html = html.replace(
    /<script\b[^>]*src="public\/platform\.js"[^>]*><\/script>/,
    "",
  );
html = html.replace(
  "</head>",
  `<link rel="stylesheet" href="/${tokenAsset}"></head>`,
);
// The icon sprite (`<use href="#i-…">`) is part of the page, not of the core
// WASM: as a string it was 20 KB of the core's data, copied into linear memory
// and parsed again by `inner_html` (docs/features/perf.md, task `headroom`).
// The mock fixtures keep their own copy (src/views/mod.rs).
if (process.env.STICKTALK_WEB_MOCK !== "1") {
  const sprite = (await readFile(new URL("../public/icons/sprite.svg", import.meta.url), "utf8")).trim();
  if (!html.includes("<body>")) throw new Error("index.html has no plain <body> for the icon sprite");
  html = html.replace("<body>", `<body><div hidden>${sprite}</div>`);
}
if (
  /<style\b/i.test(html) ||
  /<script\b(?![^>]*\bsrc\s*=)[^>]*>\s*\S/i.test(html)
)
  throw new Error("CSP packaging left an inline script or style element");
await writeFile(join(dist, "index.html"), html);
async function files(dir) {
  const all = [];
  for (const name of await readdir(dir)) {
    const p = join(dir, name);
    if ((await stat(p)).isDirectory()) all.push(...(await files(p)));
    else all.push(p);
  }
  return all;
}
// Screenshot fixture data and assets never ship in the default production build.
// Opt-in screenshot builds set this alongside Trunk --features mock.
if (process.env.STICKTALK_WEB_MOCK !== "1") {
  for (const file of await files(dist)) {
    const name = relative(dist, file).replaceAll("\\", "/");
    if (
      [
        "style.css",
        "public/platform.js",
        "public/boot.js",
        "public/push-worker.js",
        "public/icons/sprite.svg",
      ].includes(name) ||
      name.startsWith("snippets/sticktalk-web-") ||
      name.startsWith("public/mock-") ||
      name.startsWith("public/emoji/")
    )
      await unlink(file);
  }
}
// TypeScript declarations of the WASM glue are for editors, not browsers.
for (const file of await files(dist))
  if (file.endsWith(".d.ts")) await unlink(file);
if(process.env.STICKTALK_WEB_MOCK!=='1')await versionRuntime(dist);
// The UI SDK modules the first screen imports statically (ui.js's `import '...'`
// lines and what they reach). Dialog, menu and tooltip load on first use and
// are cached then, like the other lazy modules (docs/features/perf.md).
const eagerSdk = new Set();
// The Trunk fixture build (`--features mock`) has no packager step: copy the
// UI SDK modules here as the packager does (scripts/styles.mjs).
if (process.env.STICKTALK_WEB_MOCK === "1" && !(await stat(join(dist, "public/ui-sdk")).catch(() => null)))
  await (await import("./styles.mjs")).copyUiSdk(join(dist, "public"));
{
  const ui = await readFile(join(dist, "public/ui.js"), "utf8").catch(() => "");
  const target = ui.match(/from '\.\/runtime\/([a-f0-9]{20})\/ui\.js'/);
  const source = target
    ? await readFile(join(dist, "public/runtime", target[1], "ui.js"), "utf8")
    : ui;
  const queue = [...source.matchAll(/^import '\/(public\/ui-sdk\/[^']+\.js)';/gm)].map((m) => m[1]);
  while (queue.length) {
    const file = queue.pop();
    if (eagerSdk.has(file)) continue;
    eagerSdk.add(file);
    const text = await readFile(join(dist, file), "utf8");
    // Static imports only: `import"./x.js"`, `from"./x.js"`; not `import("./x.js")`.
    for (const [, spec] of text.matchAll(/(?:\bfrom|\bimport)\s*["'](\.\.?\/[^"']+\.js)["']/g))
      queue.push(join(file, "..", spec).replaceAll("\\", "/"));
  }
}
const assets = (await files(dist)).filter(
  (p) =>
    !p.endsWith(".br") && !p.endsWith("sw.js") && !p.endsWith("sizes.json"),
);
const hash = createHash("sha256");
// Worker-policy changes are version changes too, even with identical assets.
hash.update(await readFile(new URL('./finalize.mjs', import.meta.url)));
let raw = 0,
  compressed = 0;
const sizes = [];
for (const p of assets) {
  const bytes = await readFile(p);
  hash.update(bytes);
  raw += bytes.length;
  const br = brotliCompressSync(bytes, {
    params: { [constants.BROTLI_PARAM_QUALITY]: 11 },
  });
  compressed += br.length;
  sizes.push({
    file: relative(dist, p).replaceAll("\\", "/"),
    bytes: bytes.length,
    brotli: br.length,
  });
  if (/\.(wasm|js|css|html|svg|webmanifest)$/.test(p))
    await writeFile(`${p}.br`, br);
}
const revision = hash.digest("hex").slice(0, 20);
// Only a same-origin allowlist is cached. Never cache API/auth/upload traffic.
const paths = sizes
  .map((s) => "/" + s.file)
  .filter(
    (p) =>
      !p.endsWith("LICENSE") &&
      !p.endsWith(".txt") &&
      !p.includes("mock-") &&
      !p.includes("/emoji/") &&
      !p.includes("media.js") &&
      !p.includes("push.js") &&
      !p.includes("viewer.js") &&
      !p.includes("ptt.js") &&
      !p.includes("message-menu.js") &&
      !p.includes("gifs.js") &&
      !p.includes("content.js") &&
      // Profiles: the editor script, cosmetic art and name fonts load when shown.
      // Identity: its script and the tag badge glyphs too.
      !/\/(identity\.js$|tags\/)/.test(p) &&
      !/\/(profiles\.js$|cosmetics\/|badges\/|fonts\/names\/)/.test(p) &&
      // Call extras and the segmentation model load only when used.
      !/\/(callextras|soundboard|backgrounds|backgrounds-worker|clips)\.js$/.test(p) &&
      // Call encryption loads when a call starts.
      !/\/e2ee(-core|-qr|-worker)?\.js$|\/e2ee\.css$/.test(p) &&
      // Activities: the host bridge and every activity load only when used.
      // (the host script is fingerprinted under /public/runtime/<hash>/).
      !/\/activities\.(js|css)$|\/public\/activities\//.test(p) &&
      !p.includes("/public/vendor/") &&
      !p.includes("/vendor/") &&
      !p.includes("expression.js") &&
      // Themes: alternative app icons and chat fonts load only when chosen.
      !/\/(app-icons|fonts\/chat)\//.test(p) &&
      !p.includes("/stickers/") &&
      // Lazily imported: sign-in extras on the login page, account security,
      // call sounds and notification alerts, speech synthesis, the Stage audience list.
      !/\/(authsec|authsec-login|calls|alerts|speech|stage)\.js$/.test(p) &&
      // Channel drag and drop: when someone who manages channels presses a row.
      !p.includes("/channel-drag.js") &&
      // Install-time only (the 192 px icon is also the page's logo).
      !/\/icons\/(icon-512|maskable-512|apple-touch-icon)\.png$/.test(p) &&
      // Only for code; the browser fetches it when a code block is shown.
      !p.includes("/fonts/JetBrainsMono") &&
      (!p.startsWith("/public/ui-sdk/") || eagerSdk.has(p.slice(1))) &&
      !p.includes("/terms.") &&
      !(p.endsWith(".wasm") && !p.endsWith("_bg.wasm")) &&
      !p.includes("__wasm_split_manifest"),
  );
// The worker never answers /api, the MCP connector (/mcp) or its OAuth pages
// (/oauth, /.well-known): a cached app shell must not replace the consent page.
// Nor /invite/<code>: the server redirects that link to the join screen.
// Every asset path, grouped by directory: the worker is part of the first
// load and the paths share long prefixes (/pkg/<hash>/, the UI SDK's chunks).
// The same goes for the Terms of Service page, which is a document of its own.
const grouped = (list) => {
  const groups = {};
  for (const path of list) {
    const cut = path.lastIndexOf('/') + 1;
    (groups[path.slice(0, cut)] ??= []).push(path.slice(cut));
  }
  return `Object.entries(${JSON.stringify(groups)}).flatMap(([d,f])=>f.map(n=>d+n))`;
};
// An activity's own files load only in its sandboxed frame (an opaque
// origin, never controlled by this worker): leave them out of its list.
const served = sizes.map((s) => '/' + s.file).filter((p) => !p.startsWith('/public/activities/'));
const sw = `const CACHE='sticktalk-shell-${revision}';const SHELL=${grouped(paths)};const ASSETS=${grouped(served)};
self.addEventListener('install',event=>{event.waitUntil(caches.open(CACHE).then(cache=>cache.addAll(SHELL)));});
self.addEventListener('message',event=>{if(event.data==='activate-update')self.skipWaiting();if(event.data==='purge-private')event.waitUntil(caches.keys().then(keys=>Promise.all(keys.filter(k=>k.startsWith('sticktalk-')).map(async key=>{if(!key.startsWith('sticktalk-shell-'))return caches.delete(key);const cache=await caches.open(key);await Promise.all((await cache.keys()).filter(r=>!ASSETS.includes(new URL(r.url).pathname)).map(r=>cache.delete(r)));}))));});
self.addEventListener('activate',event=>{event.waitUntil(caches.keys().then(keys=>Promise.all(keys.filter(k=>k.startsWith('sticktalk-shell-')&&k!==CACHE).map(k=>caches.delete(k)))).then(()=>self.clients.claim()));});
self.addEventListener('fetch',event=>{const url=new URL(event.request.url);if(event.request.method!=='GET'||url.origin!==self.location.origin||['/api/','/oauth/','/.well-known/','/invite/'].some(p=>url.pathname.startsWith(p))||url.pathname==='/mcp'||url.pathname==='/public/terms.html')return;const path=event.request.mode==='navigate'?'/index.html':url.pathname;if(!ASSETS.includes(path))return;event.respondWith(caches.open(CACHE).then(async cache=>{const cached=await cache.match(path);if(cached&&path!=='/index.html')return cached;const refresh=fetch(event.request).then(async response=>{if(response.ok){let compatible=true;if(path==='/index.html'){const html=await response.clone().text();const references=[...html.matchAll(/(?:src|href)=["']([^"']+)["']/g)].map(m=>new URL(m[1],url).pathname).filter(p=>!p.startsWith('/api/'));compatible=references.every(p=>ASSETS.includes(p));}if(compatible)await cache.put(path,response.clone());}return response;});event.waitUntil(refresh.catch(()=>{}));return cached||refresh;}));});`;
const pushHandlers = await readFile(
  new URL("../public/push-worker.js", import.meta.url),
  "utf8",
);
const worker = sw + "\n" + pushHandlers;
await writeFile(join(dist, "sw.js"), worker);
// The server sends the precompressed variant like every other script, so
// that is what the worker costs on the wire.
const workerBr = brotliCompressSync(Buffer.from(worker), {
  params: { [constants.BROTLI_PARAM_QUALITY]: 11 },
});
await writeFile(join(dist, "sw.js.br"), workerBr);
const firstLoadBytes =
  sizes
    .filter((s) => paths.includes("/" + s.file))
    .reduce(
      (sum, s) =>
        sum +
        (/\.(wasm|js|css|html|svg|webmanifest)$/.test(s.file)
          ? s.brotli
          : s.bytes),
      0,
    ) + workerBr.length;
await writeFile(
  join(dist, "sizes.json"),
  JSON.stringify(
    {
      revision,
      raw,
      compressed,
      firstLoadBytes,
      firstLoadMeaning:
        "Complete installable shell cache, not cold-login network transfer",
      assets: sizes,
    },
    null,
    2,
  ),
);
console.log(
  JSON.stringify({
    revision,
    raw,
    compressed,
    firstLoadBytes,
    wasm: sizes.filter((s) => s.file.endsWith(".wasm")),
  }),
);

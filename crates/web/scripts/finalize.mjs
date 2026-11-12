import { readFile, writeFile, readdir, stat } from "node:fs/promises";
import { resolve, join, relative } from "node:path";
import { createHash } from "node:crypto";
import { brotliCompressSync, constants } from "node:zlib";
const dist = resolve(process.env.TRUNK_STAGING_DIR ?? "dist");
async function hashedAsset(prefix, extension, content) {
  const digest = createHash("sha256").update(content).digest("hex").slice(0, 20);
  const name = `${prefix}-${digest}.${extension}`;
  await writeFile(join(dist, name), content);
  return name;
}
// One authoritative token generator, moved from build.rs so CSS ships as an
// external stylesheet rather than a CSP-blocked runtime <style> element.
const spec = await readFile(new URL("../../../docs/UI_SPEC.md", import.meta.url), "utf8");
let tokens = "/* Generated from docs/UI_SPEC.md. */\n:root {\n";
for (const line of spec.split(/\r?\n/).filter((line) => line.startsWith("| "))) {
  const cols = line.split("|").map((col) => col.trim());
  if (cols.length < 4) continue;
  const [key, value] = cols.slice(1, 3);
  if (key === "online / idle / dnd / offline") {
    const colors = value.split(" / ");
    key.split(" / ").forEach((name, i) => { tokens += `  --${name}: ${colors[i]};\n`; });
  } else if (value.startsWith("#")) {
    tokens += `  --${key.replaceAll(".", "-")}: ${key === "mention.bg" ? "rgb(242 184 75 / 8%)" : value};\n`;
  }
}
for (const size of [4, 8, 12, 16, 24, 32]) tokens += `  --space-${size}: ${size}px;\n`;
tokens += "  --radius-small: 4px; --radius: 8px; --radius-modal: 12px;\n}\n";
const tokenAsset = await hashedAsset("ui-tokens", "css", tokens);
let html = await readFile(join(dist, "index.html"), "utf8");
for (const match of [...html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/gi)]) {
  if (/\bsrc\s*=/i.test(match[1]) || !match[2].trim()) continue;
  const asset = await hashedAsset("bootstrap", "js", match[2]);
  html = html.replace(match[0], `<script${match[1]} src="/${asset}"></script>`);
}
for (const match of [...html.matchAll(/<style\b[^>]*>([\s\S]*?)<\/style>/gi)]) {
  const asset = await hashedAsset("boot", "css", match[1]);
  html = html.replace(match[0], `<link rel="stylesheet" href="/${asset}">`);
}
html = html.replace("</head>", `<link rel="stylesheet" href="/${tokenAsset}"></head>`);
if (/<style\b/i.test(html) || /<script\b(?![^>]*\bsrc\s*=)[^>]*>\s*\S/i.test(html))
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
const assets = (await files(dist)).filter(
  (p) =>
    !p.endsWith(".br") && !p.endsWith("sw.js") && !p.endsWith("sizes.json"),
);
const hash = createHash("sha256");
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
  .filter((p) => !p.endsWith("LICENSE") && !p.endsWith(".txt"));
const sw = `const CACHE='sticktalk-shell-${revision}';const SHELL=${JSON.stringify(paths)};
self.addEventListener('install',event=>{event.waitUntil(caches.open(CACHE).then(cache=>cache.addAll(SHELL)).then(()=>self.skipWaiting()));});
self.addEventListener('activate',event=>{event.waitUntil(caches.keys().then(keys=>Promise.all(keys.filter(k=>k.startsWith('sticktalk-shell-')&&k!==CACHE).map(k=>caches.delete(k)))).then(()=>self.clients.claim()));});
self.addEventListener('fetch',event=>{const url=new URL(event.request.url);if(event.request.method!=='GET'||url.origin!==self.location.origin||url.pathname.startsWith('/api/'))return;if(event.request.mode==='navigate'){event.respondWith(fetch(event.request).catch(()=>caches.match('/index.html')));return;}if(SHELL.includes(url.pathname))event.respondWith(caches.open(CACHE).then(async cache=>(await cache.match(url.pathname))||fetch(event.request)));});`;
await writeFile(join(dist, "sw.js"), sw);
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
    ) + Buffer.byteLength(sw);
await writeFile(
  join(dist, "sizes.json"),
  JSON.stringify(
    { revision, raw, compressed, firstLoadBytes, assets: sizes },
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

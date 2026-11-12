import { readFile, writeFile, readdir, stat } from "node:fs/promises";
import { resolve, join, relative } from "node:path";
import { createHash } from "node:crypto";
import { brotliCompressSync, constants } from "node:zlib";
const dist = resolve(process.env.TRUNK_STAGING_DIR ?? "dist");
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

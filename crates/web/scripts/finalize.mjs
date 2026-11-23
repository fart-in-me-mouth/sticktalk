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
let tokens = "/* Generated from docs/UI_SPEC.md. */\n:root {\n";
for (const line of spec
  .split(/\r?\n/)
  .filter((line) => line.startsWith("| "))) {
  const cols = line.split("|").map((col) => col.trim());
  if (cols.length < 4) continue;
  const [key, value] = cols.slice(1, 3);
  if (key === "online / idle / dnd / offline") {
    const colors = value.split(" / ");
    key.split(" / ").forEach((name, i) => {
      tokens += `  --${name}: ${colors[i]};\n`;
    });
  } else if (value.startsWith("#")) {
    tokens += `  --${key.replaceAll(".", "-")}: ${key === "mention.bg" ? "rgb(242 184 75 / 8%)" : value};\n`;
  }
}
for (const size of [4, 8, 12, 16, 24, 32])
  tokens += `  --space-${size}: ${size}px;\n`;
tokens += "  --radius-small: 4px; --radius: 8px; --radius-modal: 12px;\n}\n";
tokens +=
  ":root { --text-muted: #A0A0AA; } /* Lead-approved WCAG AA override. */\n";
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
if(process.env.STICKTALK_WEB_MOCK!=='1')await versionRuntime(dist);
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
      // Call extras and the segmentation model load only when used.
      !/\/(callextras|soundboard|backgrounds|backgrounds-worker|clips)\.js$/.test(p) &&
      !p.includes("/public/vendor/") &&
      !p.includes("/vendor/") &&
      !(p.endsWith(".wasm") && !p.endsWith("_bg.wasm")) &&
      !p.includes("__wasm_split_manifest"),
  );
// The worker never answers /api, the MCP connector (/mcp) or its OAuth pages
// (/oauth, /.well-known): a cached app shell must not replace the consent page.
const sw = `const CACHE='sticktalk-shell-${revision}';const SHELL=${JSON.stringify(paths)};const ASSETS=${JSON.stringify(sizes.map(s=>'/'+s.file))};
self.addEventListener('install',event=>{event.waitUntil(caches.open(CACHE).then(cache=>cache.addAll(SHELL)));});
self.addEventListener('message',event=>{if(event.data==='activate-update')self.skipWaiting();if(event.data==='purge-private')event.waitUntil(caches.keys().then(keys=>Promise.all(keys.filter(k=>k.startsWith('sticktalk-')).map(async key=>{if(!key.startsWith('sticktalk-shell-'))return caches.delete(key);const cache=await caches.open(key);await Promise.all((await cache.keys()).filter(r=>!ASSETS.includes(new URL(r.url).pathname)).map(r=>cache.delete(r)));}))));});
self.addEventListener('activate',event=>{event.waitUntil(caches.keys().then(keys=>Promise.all(keys.filter(k=>k.startsWith('sticktalk-shell-')&&k!==CACHE).map(k=>caches.delete(k)))).then(()=>self.clients.claim()));});
self.addEventListener('fetch',event=>{const url=new URL(event.request.url);if(event.request.method!=='GET'||url.origin!==self.location.origin||['/api/','/oauth/','/.well-known/'].some(p=>url.pathname.startsWith(p))||url.pathname==='/mcp')return;const path=event.request.mode==='navigate'?'/index.html':url.pathname;if(!ASSETS.includes(path))return;event.respondWith(caches.open(CACHE).then(async cache=>{const cached=await cache.match(path);if(cached&&path!=='/index.html')return cached;const refresh=fetch(event.request).then(async response=>{if(response.ok){let compatible=true;if(path==='/index.html'){const html=await response.clone().text();const references=[...html.matchAll(/(?:src|href)=["']([^"']+)["']/g)].map(m=>new URL(m[1],url).pathname).filter(p=>!p.startsWith('/api/'));compatible=references.every(p=>ASSETS.includes(p));}if(compatible)await cache.put(path,response.clone());}return response;});event.waitUntil(refresh.catch(()=>{}));return cached||refresh;}));});`;
const pushHandlers = await readFile(
  new URL("../public/push-worker.js", import.meta.url),
  "utf8",
);
await writeFile(join(dist, "sw.js"), sw + "\n" + pushHandlers);
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

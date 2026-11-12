// Headless test server only. Production uses the existing same-origin server.
import { createServer } from "node:http";
import { readFile, stat } from "node:fs/promises";
import { resolve, extname, sep } from "node:path";
const root = resolve("dist");
const port = Number(process.env.PORT ?? 4173);
let transferBytes = 0,
  requestCount = 0;
const types = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".wasm": "application/wasm",
  ".css": "text/css",
  ".webmanifest": "application/manifest+json",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".woff2": "font/woff2",
  ".json": "application/json",
};
createServer(async (req, res) => {
  try {
    let url = new URL(req.url, "http://localhost");
    if (url.pathname === "/__metrics") {
      if (req.method === "POST") {
        transferBytes = 0;
        requestCount = 0;
      }
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ transferBytes, requestCount }));
      return;
    }
    let path = resolve(root, "." + decodeURIComponent(url.pathname));
    if (path !== root && !path.startsWith(root + sep))
      throw Error("outside root");
    if (path === root || (await stat(path).catch(() => null))?.isDirectory())
      path = resolve(root, "index.html");
    if (!extname(path)) path = resolve(root, "index.html");
    const type = types[extname(path)] ?? "application/octet-stream";
    let br = false;
    if (
      req.headers["accept-encoding"]?.includes("br") &&
      (await stat(path + ".br").catch(() => null))
    ) {
      path += ".br";
      br = true;
    }
    const bytes = await readFile(path);
    transferBytes += bytes.length;
    requestCount++;
    res.writeHead(200, {
      "Content-Type": type,
      "Cache-Control": "no-cache",
      Vary: "Accept-Encoding",
      ...(br ? { "Content-Encoding": "br" } : {}),
    });
    res.end(bytes);
  } catch {
    res.writeHead(404);
    res.end("Not found");
  }
}).listen(port, "127.0.0.1", () =>
  console.log(`Headless fixture server: http://127.0.0.1:${port}`),
);

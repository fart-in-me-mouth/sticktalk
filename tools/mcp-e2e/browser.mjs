// Real-browser check of a running server, typically a self-hosted public URL:
// the web app (register, create a server, post), then the MCP connector with the
// consent page in the same headless Chromium session (cross-site arrival, the
// SameSite reload hop, Allow, the 303 back to the client), tool calls, and the
// agent's message arriving live in the web app.
// Usage (see run.ps1): node browser.mjs <base url> <sdk dist/esm dir> <playwright dir>
import { pathToFileURL } from "node:url";
import { join } from "node:path";
import { randomBytes } from "node:crypto";
import { createServer } from "node:http";

const [BASE, SDK, PW] = process.argv.slice(2);
const load = (root, p) => import(pathToFileURL(join(root, p)).href);
const { Client } = await load(SDK, "client/index.js");
const { StreamableHTTPClientTransport } = await load(SDK, "client/streamableHttp.js");
const { UnauthorizedError } = await load(SDK, "client/auth.js");
const { chromium } = await load(PW, "index.mjs");

const checks = [];
function check(name, ok, detail = "") {
  checks.push({ name, ok: !!ok });
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${ok ? "" : " :: " + detail}`);
}
const user = `web_${randomBytes(3).toString("hex")}`;
// run.ps1 -PublicPath pins the host to its public address (see pin-dns.mjs).
const pin = process.env.E2E_HOST_RESOLVER_RULES ? [`--host-resolver-rules=${process.env.E2E_HOST_RESOLVER_RULES}`] : [];
const browser = await chromium.launch({ headless: true, args: ["--mute-audio", "--disable-notifications", ...pin] });
const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
const page = await context.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
const act = (p, action) => p.evaluate((a) => window.dispatchEvent(new CustomEvent("sticktalk-live", { detail: a })), action);

try {
  // ---- Web app as a person would use it -----------------------------------
  const res = await page.goto(BASE, { waitUntil: "networkidle" });
  check("web app loads over HTTPS", res.status() === 200 && BASE.startsWith("https://"), `${res.status()} ${BASE}`);
  await page.waitForSelector("#auth-username");
  await page.waitForFunction(() => !document.querySelector("[type=submit]").disabled);
  await page.locator("[data-live=register]").click();
  await page.locator("#auth-username").fill(user);
  await page.locator("#auth-password").fill("self-host-password-123");
  await page.locator("[type=submit]").click();
  await page.waitForSelector(".live-home");
  await page.waitForFunction(() => !document.querySelector(".connection-banner"), null, { timeout: 30000 });
  check("registered in the web app; live connection up", true);
  await act(page, "dialog:create");
  await page.waitForSelector(".live-modal [data-live-form]");
  await page.locator("#f-name").fill("Self-host Lab");
  await page.locator("[data-live-form] [type=submit]").click();
  await page.waitForFunction(() => !document.querySelector(".live-modal"));
  const guild = await page.locator('.server-rail [data-live^="guild:"]').getAttribute("data-live");
  await act(page, guild);
  await page.waitForSelector("#live-input");
  const channel = await page.locator("#live-messages").getAttribute("data-channel");
  await page.locator("#live-input").fill("Posted from the browser");
  await act(page, "complete-close");
  await page.locator("#live-input").press("Enter");
  await page.waitForFunction(() => [...document.querySelectorAll(".markdown")].some((e) => e.textContent.includes("Posted from the browser")));
  check("created a server and posted from the web app", !!channel, guild);

  // ---- MCP client; consent happens in this same browser session ------------
  // A loopback callback listener, as native MCP clients use (RFC 8252).
  let code = null;
  const callback = createServer((req, res) => {
    code = new URL(req.url, "http://127.0.0.1").searchParams.get("code");
    res.end("connected");
  });
  await new Promise((done) => callback.listen(0, "127.0.0.1", done));
  const redirect = `http://127.0.0.1:${callback.address().port}/callback`;
  const provider = {
    get redirectUrl() { return redirect; },
    get clientMetadata() { return { client_name: "Browser E2E Agent", redirect_uris: [redirect], grant_types: ["authorization_code", "refresh_token"], response_types: ["code"], token_endpoint_auth_method: "none" }; },
    clientInformation() { return this.info; },
    saveClientInformation(i) { this.info = i; },
    tokens() { return this.saved; },
    saveTokens(t) { this.saved = t; },
    saveCodeVerifier(v) { this.verifier = v; },
    codeVerifier() { return this.verifier; },
    async redirectToAuthorization(url) {
      const consent = await context.newPage();
      // Arrive cross-site, as from claude.ai: the Strict session cookie is withheld
      // until the page's own same-site reload.
      await consent.setContent(`<a id="go" href="${url.href.replaceAll('"', "&quot;")}">connect</a>`);
      await consent.click("#go");
      await consent.waitForSelector("text=Signed in as", { timeout: 30000 });
      check("consent page recognizes the browser session after the hop", (await consent.content()).includes(`@${user}`));
      check("Read and send is preselected", await consent.locator('input[name=access][value=write]').isChecked());
      await Promise.all([consent.waitForURL(`${redirect}**`, { timeout: 30000, waitUntil: "load" }), consent.click("button[value=allow]")]);
      check("Allow returned to the client with a code (CSP allowed the redirect)", !!code);
      await consent.close();
    },
  };
  const url = new URL(`${BASE}/mcp`);
  try {
    await new Client({ name: "browser-e2e", version: "1" }).connect(new StreamableHTTPClientTransport(url, { authProvider: provider }));
  } catch (e) {
    if (!(e instanceof UnauthorizedError)) throw e;
  }
  const transport = new StreamableHTTPClientTransport(url, { authProvider: provider });
  await transport.finishAuth(code);
  const client = new Client({ name: "browser-e2e", version: "1" });
  await client.connect(new StreamableHTTPClientTransport(url, { authProvider: provider }));
  const call = async (name, args) => (await client.callTool({ name, arguments: args })).structuredContent;
  const servers = await call("list_servers", {});
  check("MCP list_servers sees the web-created server", servers.servers.some((s) => s.name === "Self-host Lab"), JSON.stringify(servers));
  const read = await call("read_messages", { channel_id: channel });
  check("MCP read_messages sees the browser post", read.messages.some((m) => m.content === "Posted from the browser" && m.author.username === user), JSON.stringify(read));
  await call("send_message", { channel_id: channel, content: "Hello from the agent over the public URL" });
  await page.waitForFunction(() => [...document.querySelectorAll(".markdown")].some((e) => e.textContent.includes("Hello from the agent over the public URL")), null, { timeout: 30000 });
  check("agent message appears live in the web app", true);
  await client.close();
  callback.close();
  check("no page errors", errors.length === 0, errors.join("; "));
} catch (e) {
  check("browser flow completed", false, e.stack ?? String(e));
} finally {
  await browser.close();
}
const failed = checks.filter((c) => !c.ok).length;
console.log(`\n${checks.length - failed}/${checks.length} browser checks passed`);
process.exit(failed ? 1 : 0);

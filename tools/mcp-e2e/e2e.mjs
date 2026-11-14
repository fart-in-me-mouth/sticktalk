// End-to-end: the official MCP TypeScript SDK client against a running StickTalk
// server. Discovery, dynamic registration, PKCE, consent (cookie and password
// paths), tools/list, tools/call, refresh and revocation.
// Usage (see run.ps1): node e2e.mjs <base url> <sdk dist/esm dir> <data dir> <seed.py>
import { pathToFileURL } from "node:url";
import { execFileSync } from "node:child_process";
import { join } from "node:path";

const [BASE, SDK, DATA, SEED] = process.argv.slice(2);
const sdk = (p) => import(pathToFileURL(join(SDK, p)).href);
const { Client } = await sdk("client/index.js");
const { StreamableHTTPClientTransport } = await sdk("client/streamableHttp.js");
const { UnauthorizedError } = await sdk("client/auth.js");

const checks = [];
function check(name, ok, detail = "") {
  checks.push({ name, ok: !!ok });
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${ok ? "" : " :: " + detail}`);
}

async function registerUser(username, display) {
  const r = await fetch(`${BASE}/api/v1/auth/register`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ username, password: `${username}-long-password`, display_name: display }),
  });
  const raw = await r.text();
  if (r.status !== 201) throw new Error(`register ${username}: ${raw}`);
  // Snowflakes exceed 2^53: read the ID from the raw JSON, not a JS number.
  return { id: raw.match(/"id":(\d+)/)[1], token: JSON.parse(raw).token };
}

function hiddenFields(page) {
  const out = new URLSearchParams();
  for (const m of page.matchAll(/<input type="hidden" name="([^"]+)" value="([^"]*)">/g)) {
    out.set(m[1], m[2].replaceAll("&quot;", '"').replaceAll("&#39;", "'").replaceAll("&lt;", "<").replaceAll("&gt;", ">").replaceAll("&amp;", "&"));
  }
  return out;
}

/** Plays the user's browser on the consent page; returns the authorization code. */
async function consent(url, { cookie, username, password, access }) {
  const state = url.searchParams.get("state");
  // A cross-site arrival carries no SameSite=Strict cookie; the page hops once.
  const first = await fetch(url, { redirect: "manual" });
  const firstPage = await first.text();
  check("authorize page hops once to see the session", first.status === 200 && firstPage.includes("st_hop=1"), firstPage.slice(0, 200));
  const headers = cookie ? { cookie } : {};
  const page = await (await fetch(`${url}&st_hop=1`, { headers })).text();
  if (cookie) check("consent page shows signed-in account", page.includes("Signed in as"), page.slice(0, 300));
  else check("consent page offers password sign-in", page.includes('name="password"'), page.slice(0, 300));
  check("consent page names the client", page.includes("StickTalk E2E"));
  const form = hiddenFields(page);
  form.set("decision", "allow");
  form.set("access", access);
  if (username) {
    form.set("username", username);
    form.set("password", password);
  }
  const r = await fetch(`${BASE}/oauth/authorize`, {
    method: "POST",
    redirect: "manual",
    headers: { ...headers, origin: BASE, "content-type": "application/x-www-form-urlencoded" },
    body: form,
  });
  const location = r.headers.get("location");
  check("approval redirects back with a code", r.status === 303 && location, `${r.status} ${await r.text()}`);
  const back = new URL(location);
  check("redirect echoes state and iss", back.searchParams.get("state") === state && back.searchParams.get("iss") === BASE);
  return back.searchParams.get("code");
}

class Provider {
  constructor(how) {
    this.how = how;
    this.redirect = "http://127.0.0.1:53682/callback";
  }
  get redirectUrl() { return this.redirect; }
  get clientMetadata() {
    return {
      client_name: "StickTalk E2E",
      redirect_uris: [this.redirect],
      grant_types: ["authorization_code", "refresh_token"],
      response_types: ["code"],
      token_endpoint_auth_method: "none",
    };
  }
  clientInformation() { return this.info; }
  saveClientInformation(info) { this.info = info; }
  tokens() { return this.saved; }
  saveTokens(t) { this.saved = t; this.refreshes = (this.refreshes ?? -1) + 1; }
  saveCodeVerifier(v) { this.verifier = v; }
  codeVerifier() { return this.verifier; }
  async redirectToAuthorization(url) {
    this.authorizeUrl = url;
    this.code = await consent(url, this.how);
  }
}

async function connect(how) {
  const provider = new Provider(how);
  const url = new URL(`${BASE}/mcp`);
  let client = new Client({ name: "sticktalk-e2e", version: "1.0.0" });
  try {
    await client.connect(new StreamableHTTPClientTransport(url, { authProvider: provider }));
    throw new Error("connected without OAuth");
  } catch (e) {
    if (!(e instanceof UnauthorizedError)) throw e;
  }
  const au = provider.authorizeUrl;
  check("SDK discovered PKCE S256 + resource", au.searchParams.get("code_challenge_method") === "S256" && au.searchParams.get("resource") === `${BASE}/mcp`, au.href);
  check("SDK registered dynamically", provider.info?.client_id?.includes("."));
  const transport = new StreamableHTTPClientTransport(url, { authProvider: provider });
  await transport.finishAuth(provider.code);
  check("SDK exchanged the code for tokens", provider.saved?.access_token?.startsWith("stka_") && provider.saved?.refresh_token?.startsWith("stkr_"));
  client = new Client({ name: "sticktalk-e2e", version: "1.0.0" });
  await client.connect(new StreamableHTTPClientTransport(url, { authProvider: provider }));
  return { client, provider };
}
async function call(client, name, args) {
  const r = await client.callTool({ name, arguments: args });
  return { ...r, data: r.structuredContent };
}

const alice = await registerUser("alice", "Alice");
const bob = await registerUser("bob", "Bob");
console.log(execFileSync("python", ["-I", SEED, join(DATA, "sticktalk.sqlite3"), alice.id, bob.id]).toString().trim());

// Bob connects with his password (no browser session), read and send.
const b = await connect({ username: "bob", password: "bob-long-password", access: "write" });
check("negotiated protocol version", b.client.getServerVersion()?.name === "sticktalk");
const sent = await call(b.client, "send_message", { channel_id: "11", content: `Hey <@${alice.id}>, are we still on for Friday's game night?` });
check("bob send_message", !sent.isError && sent.data.message.mentions?.[0]?.username === "alice", JSON.stringify(sent));
const bobMessage = sent.data.message.id;

// Alice connects with her browser session.
const a = await connect({ cookie: `__Host-sticktalk_session=${alice.token}`, access: "write" });
const tools = (await a.client.listTools()).tools;
check("tools/list returns the full toolset", tools.length === 15 && tools.some((t) => t.name === "search_messages"), tools.map((t) => t.name).join(","));
check("destructive tools are annotated", tools.find((t) => t.name === "delete_message")?.annotations?.destructiveHint === true);
const me = await call(a.client, "get_me", {});
check("get_me", me.data.user.username === "alice" && me.data.access === "read_and_send", JSON.stringify(me));
const servers = await call(a.client, "list_servers", {});
check("list_servers shows the mention", servers.data.servers[0]?.name === "Test Server" && servers.data.servers[0].mention_count === 1, JSON.stringify(servers.data));
const unread = await call(a.client, "list_unread", {});
check("list_unread", unread.data.unread[0]?.channel_id === "11" && unread.data.unread[0].mention_count === 1, JSON.stringify(unread.data));
const channels = await call(a.client, "list_channels", { server_id: "10" });
check("list_channels", channels.data.channels[0]?.topic === "Plans and chatter", JSON.stringify(channels.data));
const read = await call(a.client, "read_messages", { channel_id: "11", limit: 10 });
check("read_messages", read.data.messages[0]?.author?.username === "bob" && read.data.messages[0].content.includes("Friday"), JSON.stringify(read.data));
const found = await call(a.client, "search_messages", { server_id: "10", query: "friday from:bob" });
check("search_messages with filters", found.data.results[0]?.message?.id === bobMessage, JSON.stringify(found.data));
const members = await call(a.client, "list_members", { server_id: "10", query: "bo" });
check("list_members query", members.data.members.length === 1 && members.data.members[0].username === "bob", JSON.stringify(members.data));
const reaction = await call(a.client, "add_reaction", { channel_id: "11", message_id: bobMessage, emoji: "🎲" });
check("add_reaction", !reaction.isError && reaction.data.count === 1, JSON.stringify(reaction));
const reply = await call(a.client, "send_message", { channel_id: "11", content: "Yes! Bringing snacks.", reply_to_message_id: bobMessage });
check("reply", reply.data.message.reply_to_message_id === bobMessage, JSON.stringify(reply));
const marked = await call(a.client, "mark_read", { channel_id: "11" });
check("mark_read clears the badge", marked.data.unread_count === 0 && marked.data.mention_count === 0, JSON.stringify(marked));
const guard = await call(a.client, "delete_message", { channel_id: "11", message_id: reply.data.message.id });
check("delete needs confirm", guard.isError && guard.content[0].text.includes("confirm"), JSON.stringify(guard));
const notMine = await call(a.client, "edit_message", { channel_id: "11", message_id: bobMessage, content: "hijack" });
check("cannot edit someone else's message", notMine.isError, JSON.stringify(notMine));
const badArg = await call(a.client, "read_messages", { channel: "11" });
check("unknown argument explained", badArg.isError && badArg.content[0].text.includes("Accepted arguments"), JSON.stringify(badArg));

// Bob sees Alice's reply and reaction through his own connection.
const bobView = await call(b.client, "read_messages", { channel_id: "11", around: bobMessage });
check("bob reads alice's reply and reaction", bobView.data.messages.length === 2 && bobView.data.messages[0].reactions?.[0]?.emoji === "🎲", JSON.stringify(bobView.data));

// Expired access token: the SDK refreshes transparently (rotating refresh token).
const oldRefresh = a.provider.saved.refresh_token;
a.provider.saved = { ...a.provider.saved, access_token: "stka_expired" };
const afterRefresh = await call(a.client, "get_me", {});
check("SDK refreshed after 401", afterRefresh.data?.user?.username === "alice" && a.provider.saved.refresh_token !== oldRefresh && a.provider.refreshes === 1, JSON.stringify(a.provider.saved));

// A read-only connection hides and refuses writes.
const ro = await connect({ cookie: `__Host-sticktalk_session=${alice.token}`, access: "read" });
const roTools = (await ro.client.listTools()).tools;
check("read-only toolset", roTools.length === 8 && roTools.every((t) => t.annotations.readOnlyHint), roTools.map((t) => t.name).join(","));
const roSend = await call(ro.client, "send_message", { channel_id: "11", content: "should fail" });
check("read-only send refused", roSend.isError && roSend.data.error.code === "read_only_connection", JSON.stringify(roSend));

// Revocation (RFC 7009) ends the grant; the old access token stops working.
const roAccess = ro.provider.saved.access_token;
const rv = await fetch(`${BASE}/oauth/revoke`, { method: "POST", headers: { "content-type": "application/x-www-form-urlencoded" }, body: new URLSearchParams({ token: ro.provider.saved.refresh_token }) });
const after = await fetch(`${BASE}/mcp`, { method: "POST", headers: { authorization: `Bearer ${roAccess}`, "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "ping" }) });
check("revoked grant gets 401 + discovery hint", rv.status === 200 && after.status === 401 && after.headers.get("www-authenticate")?.includes("resource_metadata="), `${rv.status} ${after.status}`);

await a.client.close();
await b.client.close();
await ro.client.close();
const failed = checks.filter((c) => !c.ok).length;
console.log(`\n${checks.length - failed}/${checks.length} checks passed`);
process.exit(failed ? 1 : 0);

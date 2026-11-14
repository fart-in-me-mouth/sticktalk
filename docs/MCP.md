# MCP connector: let an AI assistant use StickTalk as you

StickTalk serves a remote [Model Context Protocol](https://modelcontextprotocol.io)
endpoint. Paste one URL into an AI app (Claude, Claude Code, Cursor, ChatGPT, VS Code,
or any MCP client that supports remote servers with OAuth), sign in to StickTalk,
approve, and the assistant can read and search your servers and DMs and, if you
allow it, send messages as you.

## Your connector URL

```
https://your-pc.your-tailnet.ts.net/mcp
```

It is your server's public URL (`STICKTALK_PUBLIC_URL`) followed by `/mcp`. A
self-hosted server's setup script ([deploy/SELF_HOST.md](../deploy/SELF_HOST.md))
prints it at the end, for example
`https://desktop-b09gtso.humpback-salak.ts.net:10000/mcp` when Tailscale's port 443
is used by something else on that machine.
Everyone on a server uses the same URL: **who** the assistant acts as is decided
when you sign in, so the URL holds no secret and is safe to share. Open it in a
browser to see these instructions with the exact URL filled in.

> Claude (claude.ai, Desktop, mobile) and ChatGPT connect from their own cloud,
> so they need the public HTTPS URL. `http://127.0.0.1:8080/mcp` only works for
> clients on the same computer (Claude Code, Cursor, VS Code, MCP Inspector).

## Connect a client

1. Add the connector:
   - **Claude** (claude.ai / Desktop): Settings › Connectors › *Add custom connector*.
     Paste the URL and select *Add*, then *Connect*.
   - **Claude Code**: `claude mcp add --transport http sticktalk https://your-pc.your-tailnet.ts.net/mcp`,
     then run `/mcp` and choose *Authenticate*.
   - **Cursor** (`~/.cursor/mcp.json`) and **VS Code** (`.vscode/mcp.json`):
     `{"mcpServers": {"sticktalk": {"url": "https://your-pc.your-tailnet.ts.net/mcp"}}}`
     (VS Code uses `"servers"` with `"type": "http"`).
   - **ChatGPT**: Settings › Connectors (developer mode) › *Create*, with OAuth.
   - **Others**: any MCP client with Streamable HTTP and OAuth support.
     Registration is dynamic; there is no client ID or secret to enter.
2. A StickTalk page opens in your browser, showing which app is asking. If this
   browser is signed in to the StickTalk web app, it shows your account.
   Otherwise, sign in with your username and password on that page. Google-only
   accounts: sign in to the StickTalk web app in the same browser first, then
   reload the page.
3. **Read and send** is preselected; switch to **Read only** if you prefer, then
   select **Allow**. You return to the AI app, which is now connected.

The app's name on that page is what the app calls itself; StickTalk cannot verify
it. Only approve a request you just started.

### Disconnect

Each connection is listed under **Settings › Devices** (platform "AI connector
(MCP)") in the desktop and web apps. **Log out** there disconnects it
immediately. *Sign out of all other devices*, changing your password and deleting
your account also disconnect every connector. Removing the connector inside the
AI app revokes its tokens too, if the app supports token revocation.

Connections don't count against the five sign-in sessions per account. You can
have up to 10 at a time. A connection stays valid while it's used at least once
every 30 days.

## What the assistant can do

All tools act as you, with your StickTalk permissions. They run through the same
authorization, slowmode, rate limits and live updates as your own clients. Your
other devices update instantly when the assistant sends or reads.

| Tool | Access | Does |
|---|---|---|
| `get_me` | read | Your account and the connection's access level |
| `list_servers` | read | Your servers with unread/mention totals |
| `list_channels` | read | A server's channels: category, topic, unread, whether you can send |
| `list_direct_messages` | read | DMs and group DMs, most recent first |
| `list_unread` | read | Everything with unread messages, mentions first |
| `read_messages` | read | Channel/DM history, oldest first, paged with `before`/`after`/`around` |
| `search_messages` | read | Full-text server search with `from:`, `in:`, `mentions:`, `before:`, `after:`, `has:` filters |
| `list_members` | read | Server members, filterable by name, paged |
| `send_message` | send | Post (optionally as a reply); `<@USER_ID>` mentions |
| `edit_message` | send | Replace the text of your own message (destructive) |
| `delete_message` | send | Delete a message; requires `confirm: true` (destructive) |
| `add_reaction` / `remove_reaction` | send | React with Unicode or `<:name:ID>` server emoji |
| `mark_read` | send | Clear a channel's unread badge on all your devices |
| `open_dm` | send | Find or open the DM with a friend, by ID or username |

A **Read only** connection doesn't list the send tools at all and refuses them if
called. Uploads, voice, moderation, server or channel management, friends and
account settings are deliberately not exposed.

## Protocol reference

- **Transport:** MCP Streamable HTTP, stateless. `POST /mcp` takes one JSON-RPC
  message and returns `application/json` (notifications get `202`). There's no SSE
  stream or `Mcp-Session-Id`, so `GET` and `DELETE` return `405`. Batches are
  rejected. Protocol versions `2025-11-25`, `2025-06-18` and `2025-03-26` are
  negotiated at `initialize`. Any other `MCP-Protocol-Version` header gets `400`.
  Capabilities: `tools` only, with server `instructions`.
- **Tool results:** `structuredContent` plus the same JSON as a text block. IDs are
  decimal strings, because snowflakes exceed JavaScript's safe integers. Times are
  RFC 3339 UTC.
- **Errors:**
  - Problems the agent can fix (unknown or invalid arguments, not found, no
    permission, rate limited, read-only connection, missing `confirm`) come back
    as `isError: true` results. Each has a `structuredContent.error.code` and a
    message saying what to do next.
  - Protocol faults are JSON-RPC errors: unknown method `-32601`, unknown tool or
    bad params `-32602`.
  - Limits are clamped, not rejected.
- **Auth discovery:**
  - Without a valid token, `/mcp` answers `401` with
    `WWW-Authenticate: Bearer realm="StickTalk", resource_metadata="<origin>/.well-known/oauth-protected-resource/mcp"`.
    `error="invalid_token"` is added when a token was presented.
  - `/.well-known/oauth-protected-resource[/mcp]` (RFC 9728) points to the issuer,
    and `/.well-known/oauth-authorization-server` (RFC 8414) lists the endpoints.
  - The issuer is `STICKTALK_PUBLIC_URL` itself, never the request's Host header.
  - Bearer endpoints send `Access-Control-Allow-Origin: *`, which is safe because
    they never read cookies. That lets browser-based clients connect.
- **Registration** (`POST /oauth/register`, RFC 7591):
  - Clients are public (`token_endpoint_auth_method: none`). The `client_id` is a
    signed, stateless blob of name and redirect URIs, so registering stores
    nothing.
  - Allowed redirect URIs (1-5):
    - `https://` anywhere;
    - `http://` only on `localhost`, `127.0.0.1` or `[::1]`, matched on any port
      (RFC 8252);
    - private-use app schemes such as `cursor://`.
  - Fragments, credentials and script/file schemes are refused.
- **Authorization** (`/oauth/authorize`):
  - Authorization code with PKCE `S256` (required). RFC 8707 `resource` must
    equal `<origin>/mcp` when sent.
  - Requests with an unknown client or unregistered redirect get an error page
    and are never redirected. Other errors redirect back with `error`, `state`
    and `iss` (RFC 9207).
  - The consent POST requires a same-origin `Origin` header and an HMAC CSRF
    token. That token is bound to the browser session (or the password form) and
    to the client, redirect, PKCE challenge and state.
  - The consent page sets `frame-ancestors 'none'` and sends no scripts.
  - Password sign-in shares the login rate limits.
- **Tokens** (`POST /oauth/token`, form-encoded):
  - `authorization_code` codes are single-use and live 60 s. A code is spent
    when presented, even if a check fails.
  - `refresh_token` rotates on every use. Each token embeds the connection's
    secret family ID, so replaying any rotated-out token, at any age, revokes the
    whole connection. That needs no list of spent tokens: one row per
    connection.
  - Access tokens (`stka_…`) live 1 h; refresh tokens (`stkr_…`) last while the
    connection does. Both are random 256-bit values stored only as SHA-256.
  - `POST /oauth/revoke` (RFC 7009) accepts either token. Any refresh token of
    the connection ends it.
- **Scopes:** `chat:read`, `chat:write`. You pick them on the consent page, where
  Read and send is preselected; the client's requested scope is ignored.
- **Storage:** see migration `202611140002_mcp_connector.sql`. Each connection is
  a row in `mcp_grants` backed by an ordinary `sessions` row. The session's own
  token is never issued. The row exists so the connection appears under Devices,
  is revoked by every existing session-revocation path and can own command
  receipts. MCP tokens are rejected by the REST and WebSocket APIs, and session
  tokens are rejected by `/mcp`.
- **Limits:** 120 tool calls/min per user (search 30/min), plus the normal
  command and send limits. 20 registrations/min, 60 token requests/min and
  10 password sign-ins/min per IP. Request bodies are capped at 1 MiB. Idle
  connections cost nothing: no sockets, streams or timers. Expired codes and
  access tokens are pruned by the existing 60 s housekeeping pass.

## Deployment

Nothing extra to configure:
- Self-hosting with Tailscale Funnel: `deploy/self-host/setup-*.{ps1,sh}` sets
  `STICKTALK_PUBLIC_URL` to the Funnel address (including a non-443 port) and
  Funnel forwards every path. Funnel reaches the server over loopback, so OAuth
  redirects back to the AI app, cookies and CORS behave exactly as behind Caddy.
- The `deploy/Caddyfile` (optional own-domain setup) also proxies `/mcp`,
  `/oauth/*` and `/.well-known/*` with everything else.
- `STICKTALK_PUBLIC_URL` must be the exact public HTTPS origin, port included,
  because it becomes the OAuth issuer and resource.
- Changing the public URL invalidates existing connections' discovery. Reconnect
  them.
- The HMAC key for client IDs is generated on first use and stored in the
  database (`mcp_secret`), so backups and restores keep connections working.

## Testing

- `cargo test -p sticktalk-server mcp::` covers the OAuth flow, CSRF/origin and
  redirect checks, PKCE, code reuse, refresh rotation and replay revocation,
  read-only gating, Devices revocation and every tool against a fixture server.
- `tools/mcp-e2e/run.ps1` builds and starts a real server on loopback, then drives
  it with the official MCP TypeScript SDK client. That run covers discovery,
  dynamic registration, PKCE, both consent paths, `tools/list`, tool calls as two
  users, transparent refresh after a 401, a read-only connection and revocation.

## Design notes

The design follows [T3 Code](https://github.com/pingdotgg/t3code)'s remote MCP
connector (MIT license). That includes stateless signed client IDs, port-agnostic
loopback redirects, 60-second single-use codes, CSRF bound to the request tuple,
and `isError` results that tell the agent what to do next. No T3 code was copied.

T3 Code needs a relay because its app runs on your own PC: it provisions a
Cloudflare tunnel per user. A StickTalk server is already public, so `/mcp` is
served directly and there is no relay.

Unlike T3 Code, StickTalk issues refresh tokens, so connections don't need
re-approval every 30 days. It also supports RFC 7009 revocation, and connections
appear in the existing Devices list.

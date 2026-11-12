# Live Phase 1 client/server test

Run `powershell -File tools/e2e/run.ps1` from the repository (or `-Full` for
tests, clippy, and formatting). No separately installed server is required.

`crates/client/tests/live.rs` starts the real server library on an ephemeral
loopback TCP port, with a fresh temporary SQLite database and upload directory.
Three independently running production client subscriptions drive their bounded
queues, WebSocket encoding, HTTP transport, reconnect recovery, message-page
merging, and `Model` reducers. It never constructs a window, writes credentials
or preferences, or opens a browser. Real rate limits remain enabled; allow a few
minutes, including token-bucket retry waits. Each delivery has a deadline and
the complete scenario has a five-minute ceiling. Owned tasks abort on failure.

The Google mock generates an ephemeral RSA key, serves JWKS and a scripted
authorization/code-token exchange, and checks PKCE. GIF discovery uses a local
mock KLIPY HTTP upstream; CDN URLs are metadata only and are not downloaded.
The fault-injection TCP relay cuts established sockets without discarding the
server's resumable session stream.

Coverage: password registration/login/logout, Google signup/existing login and
invalid tokens/ticket replay/collision, relationships/presence, direct/group
messages and recipients, servers/invites/join, categories/text-channel changes,
message edit/delete/reply/reaction/mention/markdown, typing, unread/read counts,
history keysets and bounded ID/time regions, legacy and resumable uploads,
durable-offset resume/conflict/idempotent completion, 26 MiB upload/download,
authorized thumbnails/ranges, kick/ban/rejoin denial, socket loss/replay, GIF
trending/search/categories/items and private favorites.

The test-only upstream configuration is explicit:

- `--mock-google-jwks http://127.0.0.1:<port>/jwks` together with
  `--mock-google-issuer http://127.0.0.1:<port>`
- `--mock-gif-base http://127.0.0.1:<port>/v2` with approved GIF proxy mode

These options require a loopback server bind and reject non-loopback HTTP
origins, credentials, queries and fragments. They do not change production
Google issuer/JWKS defaults, do not discover URLs from tokens, and must not be
used for deployment. All files/keys/data are local and ephemeral.

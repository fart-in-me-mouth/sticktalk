# MCP connector end-to-end test

```powershell
powershell -ExecutionPolicy Bypass -File tools/mcp-e2e/run.ps1
```

Needs Node 22+ and Python 3 on PATH. The script:

1. builds `sticktalk-server`;
2. installs the pinned official MCP TypeScript SDK under
   `$env:CARGO_TARGET_DIR/mcp-e2e` (`--ignore-scripts`; never into the repo);
3. starts the server on `127.0.0.1:18080` with a fresh temporary database;
4. registers two users, seeds a server with `#general` (`seed.py`);
5. runs `e2e.mjs`;
6. stops the server and deletes the database.

`e2e.mjs` uses the SDK's own OAuth client, with no shortcuts:

- protected-resource and authorization-server discovery;
- dynamic client registration;
- PKCE and the RFC 8707 `resource` parameter.

It plays the user's browser on the consent page. Bob signs in with a password;
Alice uses a web-app session cookie, with the same-site reload hop. Then, over
the real transport, it checks:

- `tools/list` and every read tool;
- sending, replying, reacting and marking read;
- refusals: delete without confirmation, editing someone else's message, unknown
  arguments;
- that each user sees the other's activity;
- transparent token refresh after a forced 401;
- a read-only connection;
- RFC 7009 revocation.

It prints one PASS/FAIL line per check and exits non-zero on any failure.
Nothing binds outside loopback and no browser or window opens.

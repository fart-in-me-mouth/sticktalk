-- MCP connector (docs/MCP.md): OAuth 2.1 authorization for remote AI clients.
-- Client registrations are stateless signed IDs, so registration stores nothing.
CREATE TABLE mcp_secret(
 singleton INTEGER PRIMARY KEY CHECK(singleton=1),
 key BLOB NOT NULL CHECK(length(key)=32)
);
-- Single-use authorization codes, sixty seconds.
CREATE TABLE mcp_codes(
 code_hash BLOB PRIMARY KEY CHECK(length(code_hash)=32),
 client_id TEXT NOT NULL,
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 redirect_uri TEXT NOT NULL,
 code_challenge TEXT NOT NULL,
 scope TEXT NOT NULL,
 expires_at INTEGER NOT NULL
);
CREATE INDEX mcp_codes_expiry ON mcp_codes(expires_at);
-- One grant per approved connection. Its backing session row (never issued as a
-- bearer token) lists it under Devices, carries command receipts and is revoked
-- by Log out, sign out everywhere, password change and account deletion.
CREATE TABLE mcp_grants(
 session_hash BLOB PRIMARY KEY REFERENCES sessions(token_hash) ON DELETE CASCADE,
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 client_id TEXT NOT NULL,
 scope TEXT NOT NULL,
 refresh_hash BLOB NOT NULL UNIQUE CHECK(length(refresh_hash)=32),
 previous_refresh_hash BLOB UNIQUE,
 created_at INTEGER NOT NULL
);
CREATE INDEX mcp_grants_user ON mcp_grants(user_id);
CREATE TABLE mcp_access_tokens(
 token_hash BLOB PRIMARY KEY CHECK(length(token_hash)=32),
 session_hash BLOB NOT NULL REFERENCES mcp_grants(session_hash) ON DELETE CASCADE,
 expires_at INTEGER NOT NULL
);
CREATE INDEX mcp_access_grant ON mcp_access_tokens(session_hash,expires_at);
CREATE INDEX mcp_access_expiry ON mcp_access_tokens(expires_at);

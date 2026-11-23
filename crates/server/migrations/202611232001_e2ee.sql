-- End-to-end encrypted calls (docs/E2EE.md). Public keys only; a device's key
-- belongs to its session, so signing out or revoking the session deletes it.
CREATE TABLE e2ee_device_keys (
 token_hash BLOB PRIMARY KEY REFERENCES sessions(token_hash) ON DELETE CASCADE ON UPDATE CASCADE,
 user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 public_key BLOB NOT NULL CHECK(length(public_key)=65),
 created_at INTEGER NOT NULL
);
CREATE INDEX e2ee_device_keys_user ON e2ee_device_keys(user_id);
-- "Allow unencrypted calls": off unless a row says otherwise.
CREATE TABLE e2ee_guild_settings (
 guild_id INTEGER PRIMARY KEY REFERENCES guilds(id) ON DELETE CASCADE,
 allow_unencrypted INTEGER NOT NULL DEFAULT 0 CHECK(allow_unencrypted IN (0,1))
);
CREATE TABLE e2ee_dm_settings (
 channel_id INTEGER PRIMARY KEY REFERENCES channels(id) ON DELETE CASCADE,
 allow_unencrypted INTEGER NOT NULL DEFAULT 0 CHECK(allow_unencrypted IN (0,1))
);

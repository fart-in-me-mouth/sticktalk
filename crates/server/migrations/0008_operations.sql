CREATE TABLE partial_uploads (
 id TEXT PRIMARY KEY, owner_id INTEGER NOT NULL REFERENCES users(id), purpose TEXT NOT NULL,
 channel_id INTEGER REFERENCES channels(id) ON DELETE CASCADE, filename TEXT NOT NULL,
 content_type TEXT NOT NULL, byte_size INTEGER NOT NULL CHECK(byte_size>=0),
 received INTEGER NOT NULL DEFAULT 0 CHECK(received>=0 AND received<=byte_size),
 sha256 TEXT NOT NULL, storage_key TEXT NOT NULL UNIQUE, thumbnail_key TEXT NOT NULL UNIQUE,
 expires_at INTEGER NOT NULL, attachment_id INTEGER REFERENCES uploads(id) ON DELETE CASCADE
);
CREATE INDEX partial_owner ON partial_uploads(owner_id, attachment_id);
CREATE INDEX partial_pending ON partial_uploads(attachment_id);
CREATE INDEX partial_expiry ON partial_uploads(expires_at);
CREATE TABLE disabled_users(user_id INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE);
CREATE TABLE password_resets(user_id INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE, code_hash BLOB NOT NULL UNIQUE, expires_at INTEGER NOT NULL);
CREATE TRIGGER disabled_session BEFORE INSERT ON sessions WHEN EXISTS(SELECT 1 FROM disabled_users WHERE user_id=NEW.user_id) BEGIN SELECT RAISE(ABORT,'account disabled'); END;

CREATE TABLE releases(version TEXT PRIMARY KEY, sha256 TEXT NOT NULL, byte_size INTEGER NOT NULL, envelope TEXT NOT NULL);

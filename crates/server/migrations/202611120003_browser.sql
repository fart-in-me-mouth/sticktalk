CREATE TABLE web_oauth_states(state_hash BLOB PRIMARY KEY, verifier TEXT NOT NULL, expires_at INTEGER NOT NULL);
CREATE INDEX web_oauth_expiry ON web_oauth_states(expires_at);
CREATE TABLE push_subscriptions(id TEXT PRIMARY KEY,session_hash BLOB NOT NULL REFERENCES sessions(token_hash) ON DELETE CASCADE,endpoint TEXT NOT NULL UNIQUE,p256dh TEXT NOT NULL,auth TEXT NOT NULL,created_at INTEGER NOT NULL);
CREATE INDEX push_session ON push_subscriptions(session_hash,id);

-- Dense ordinals are assigned once on migration, then monotonically on insertion.
-- Tombstones remove entries but never renumber surviving messages.
CREATE TABLE history_extents(channel_id INTEGER PRIMARY KEY REFERENCES channels(id) ON DELETE CASCADE,message_count INTEGER NOT NULL DEFAULT 0,next_ordinal INTEGER NOT NULL DEFAULT 0);
CREATE TABLE history_entries(channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,ordinal INTEGER NOT NULL,message_id INTEGER NOT NULL UNIQUE REFERENCES messages(id) ON DELETE CASCADE,PRIMARY KEY(channel_id,ordinal,message_id)) WITHOUT ROWID;
INSERT INTO history_entries SELECT channel_id,ROW_NUMBER() OVER (PARTITION BY channel_id ORDER BY id)-1,id FROM messages WHERE deleted_at IS NULL;
INSERT INTO history_extents SELECT channel_id,COUNT(*),MAX(ordinal)+1 FROM history_entries GROUP BY channel_id;
CREATE TRIGGER history_insert AFTER INSERT ON messages WHEN NEW.deleted_at IS NULL BEGIN
  INSERT INTO history_extents(channel_id) VALUES(NEW.channel_id) ON CONFLICT DO NOTHING;
  INSERT INTO history_entries SELECT NEW.channel_id,next_ordinal,NEW.id FROM history_extents WHERE channel_id=NEW.channel_id;
  UPDATE history_extents SET message_count=message_count+1,next_ordinal=next_ordinal+1 WHERE channel_id=NEW.channel_id;
END;
CREATE TRIGGER history_remove_entry AFTER DELETE ON history_entries BEGIN
  UPDATE history_extents SET message_count=message_count-1 WHERE channel_id=OLD.channel_id;
END;
CREATE TRIGGER history_soft_delete AFTER UPDATE OF deleted_at ON messages WHEN OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL BEGIN
  DELETE FROM history_entries WHERE message_id=NEW.id;
END;
CREATE TRIGGER history_restore AFTER UPDATE OF deleted_at ON messages WHEN OLD.deleted_at IS NOT NULL AND NEW.deleted_at IS NULL BEGIN
  INSERT INTO history_extents(channel_id) VALUES(NEW.channel_id) ON CONFLICT DO NOTHING;
  INSERT INTO history_entries SELECT NEW.channel_id,next_ordinal,NEW.id FROM history_extents WHERE channel_id=NEW.channel_id;
  UPDATE history_extents SET message_count=message_count+1,next_ordinal=next_ordinal+1 WHERE channel_id=NEW.channel_id;
END;

-- Foreign keys are disabled only on the startup migration connection.
CREATE TABLE channels_voice(id INTEGER PRIMARY KEY CHECK(id>0),kind TEXT NOT NULL CHECK(kind IN ('category','text','voice','direct','group_dm')),guild_id INTEGER REFERENCES guilds(id) ON DELETE CASCADE,parent_id INTEGER REFERENCES channels(id) ON DELETE SET NULL,name TEXT,topic TEXT,position INTEGER NOT NULL CHECK(position BETWEEN 0 AND 65535),owner_id INTEGER REFERENCES users(id),created_at INTEGER NOT NULL,CHECK((kind IN ('category','text','voice') AND guild_id IS NOT NULL AND name IS NOT NULL AND owner_id IS NULL) OR (kind IN ('direct','group_dm') AND guild_id IS NULL AND parent_id IS NULL AND topic IS NULL)),CHECK(kind<>'category' OR (parent_id IS NULL AND topic IS NULL)));
INSERT INTO channels_voice SELECT * FROM channels;
DROP TABLE channels;
ALTER TABLE channels_voice RENAME TO channels;
CREATE INDEX channel_guild ON channels(guild_id,parent_id,position,id);
CREATE TABLE voice_connect_denials(channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,PRIMARY KEY(channel_id,user_id));

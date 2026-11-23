-- Any guild channel can be nested in any other one: a category may now have a
-- parent. SQLite cannot drop a CHECK, so the table is rebuilt (foreign keys are
-- disabled on the startup migration connection, as for 0009).
CREATE TABLE channels_nested(id INTEGER PRIMARY KEY CHECK(id>0),kind TEXT NOT NULL CHECK(kind IN ('category','text','voice','direct','group_dm')),guild_id INTEGER REFERENCES guilds(id) ON DELETE CASCADE,parent_id INTEGER REFERENCES channels(id) ON DELETE SET NULL,name TEXT,topic TEXT,position INTEGER NOT NULL CHECK(position BETWEEN 0 AND 65535),owner_id INTEGER REFERENCES users(id),created_at INTEGER NOT NULL,CHECK((kind IN ('category','text','voice') AND guild_id IS NOT NULL AND name IS NOT NULL AND owner_id IS NULL) OR (kind IN ('direct','group_dm') AND guild_id IS NULL AND parent_id IS NULL AND topic IS NULL)),CHECK(kind<>'category' OR topic IS NULL));
INSERT INTO channels_nested SELECT * FROM channels;
DROP TABLE channels;
ALTER TABLE channels_nested RENAME TO channels;
CREATE INDEX channel_guild ON channels(guild_id,parent_id,position,id);

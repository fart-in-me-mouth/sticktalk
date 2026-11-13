CREATE TABLE guild_emoji (
 id INTEGER PRIMARY KEY, guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
 name TEXT NOT NULL COLLATE NOCASE, animated INTEGER NOT NULL CHECK(animated IN(0,1)),
 content_type TEXT NOT NULL, data BLOB NOT NULL CHECK(length(data)<=262144), preview BLOB NOT NULL,
 UNIQUE(guild_id,name)
);

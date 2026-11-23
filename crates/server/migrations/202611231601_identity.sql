-- Server identity (docs/features/identity.md): server tags, server images,
-- role styles and icons, vanity invite links.

-- A server's tag. `tag_key` is the uppercase text, unique on the instance.
CREATE TABLE server_tags(
 guild_id INTEGER PRIMARY KEY REFERENCES guilds(id) ON DELETE CASCADE,
 tag_key TEXT NOT NULL UNIQUE,
 tag TEXT NOT NULL,
 updated_at INTEGER NOT NULL
);
-- Tag texts the instance administrator revoked; nobody may claim them.
CREATE TABLE blocked_tags(tag_key TEXT PRIMARY KEY, created_at INTEGER NOT NULL);

-- The tag a user wears, copied so every user summary carries it.
ALTER TABLE users ADD COLUMN tag TEXT;
ALTER TABLE users ADD COLUMN tag_guild INTEGER;
CREATE INDEX users_tag_guild ON users(tag_guild) WHERE tag_guild IS NOT NULL;
-- Leaving or being removed from the server drops its tag at once; the
-- command that removed the member tells everyone (identity::settle).
CREATE TABLE tag_drops(user_id INTEGER PRIMARY KEY);
CREATE TRIGGER identity_tag_drop AFTER DELETE ON guild_members
WHEN EXISTS(SELECT 1 FROM users WHERE id=OLD.user_id AND tag_guild=OLD.guild_id)
BEGIN
 UPDATE users SET tag=NULL,tag_guild=NULL WHERE id=OLD.user_id AND tag_guild=OLD.guild_id;
 INSERT OR IGNORE INTO tag_drops(user_id) VALUES(OLD.user_id);
END;

-- Banner, icon animation and tag visibility (GuildLook), and the invite
-- background (ImageMeta), as JSON.
CREATE TABLE guild_looks(
 guild_id INTEGER PRIMARY KEY REFERENCES guilds(id) ON DELETE CASCADE,
 look TEXT NOT NULL,
 invite_background TEXT
);
-- The uploads a server's images use: readable by its members, and kept by
-- upload collection.
CREATE TABLE guild_uploads(
 guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
 slot TEXT NOT NULL CHECK(slot IN('icon','banner','invite_background')),
 upload_id INTEGER NOT NULL REFERENCES uploads(id),
 PRIMARY KEY(guild_id,slot)
);
CREATE INDEX guild_upload_file ON guild_uploads(upload_id);

-- A role's style and icon (RoleLook), as JSON.
CREATE TABLE role_looks(
 guild_id INTEGER NOT NULL,
 role_id INTEGER NOT NULL,
 look TEXT NOT NULL,
 PRIMARY KEY(guild_id,role_id),
 FOREIGN KEY(guild_id,role_id) REFERENCES roles(guild_id,id) ON DELETE CASCADE
);
-- An uploaded role icon: the 64x64 rendition shown, and the original.
CREATE TABLE role_icons(
 guild_id INTEGER NOT NULL,
 role_id INTEGER NOT NULL,
 rev INTEGER NOT NULL,
 png BLOB NOT NULL CHECK(length(png)<=262144),
 original BLOB NOT NULL CHECK(length(original)<=262144),
 original_type TEXT NOT NULL,
 PRIMARY KEY(guild_id,role_id),
 FOREIGN KEY(guild_id,role_id) REFERENCES roles(guild_id,id) ON DELETE CASCADE
);

-- Vanity codes, lowercase. A held code (no server) waits until held_until
-- before anyone but held_by may claim it again.
CREATE TABLE vanity_codes(
 code TEXT PRIMARY KEY,
 guild_id INTEGER UNIQUE REFERENCES guilds(id) ON DELETE SET NULL,
 invite_id INTEGER,
 held_by INTEGER,
 held_until INTEGER,
 uses_base INTEGER NOT NULL DEFAULT 0,
 claimed_at INTEGER NOT NULL
);
-- A deleted server loses its code; it is held for 30 days.
CREATE TRIGGER identity_vanity_guild_gone BEFORE DELETE ON guilds
BEGIN
 UPDATE vanity_codes SET held_by=OLD.id,held_until=CAST(strftime('%s','now') AS INTEGER)*1000+2592000000,invite_id=NULL WHERE guild_id=OLD.id;
END;

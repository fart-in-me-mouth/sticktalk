-- Expression: custom emoji everywhere, stickers, Super Reactions, typing indicators.
-- See docs/features/expression.md.

-- Who added each custom emoji, and when (server settings).
ALTER TABLE guild_emoji ADD COLUMN creator_id INTEGER;
ALTER TABLE guild_emoji ADD COLUMN created_at INTEGER NOT NULL DEFAULT 0;

-- A custom emoji used in a conversation may be shown to everyone who can read
-- it, including people who are not in the emoji's server.
CREATE TABLE emoji_grants (
 emoji_id INTEGER NOT NULL,
 channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
 PRIMARY KEY(emoji_id, channel_id)
) WITHOUT ROWID;
CREATE INDEX emoji_grants_channel ON emoji_grants(channel_id);
-- Names of deleted emoji, so reactions with them still read as :name:.
CREATE TABLE deleted_emoji (
 id INTEGER PRIMARY KEY,
 name TEXT NOT NULL
);
CREATE TRIGGER expression_emoji_deleted AFTER DELETE ON guild_emoji BEGIN
 INSERT OR REPLACE INTO deleted_emoji(id, name) VALUES(OLD.id, OLD.name);
 DELETE FROM emoji_grants WHERE emoji_id = OLD.id;
END;

CREATE TABLE guild_stickers (
 id INTEGER PRIMARY KEY,
 guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
 name TEXT NOT NULL COLLATE NOCASE,
 emoji TEXT NOT NULL,
 description TEXT NOT NULL DEFAULT '',
 format TEXT NOT NULL CHECK(format IN ('png','apng','gif','webp')),
 animated INTEGER NOT NULL CHECK(animated IN (0,1)),
 data BLOB NOT NULL CHECK(length(data) <= 524288),
 still BLOB NOT NULL,
 creator_id INTEGER,
 created_at INTEGER NOT NULL,
 UNIQUE(guild_id, name)
);
CREATE TABLE sticker_grants (
 sticker_id INTEGER NOT NULL,
 channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
 PRIMARY KEY(sticker_id, channel_id)
) WITHOUT ROWID;
CREATE TRIGGER expression_sticker_deleted AFTER DELETE ON guild_stickers BEGIN
 DELETE FROM sticker_grants WHERE sticker_id = OLD.id;
END;
-- What each message carries: a copy of the sticker's name and format, so it
-- still reads correctly after the sticker is renamed or deleted.
CREATE TABLE message_stickers (
 message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
 position INTEGER NOT NULL,
 sticker_id INTEGER NOT NULL,
 guild_id INTEGER,
 name TEXT NOT NULL,
 description TEXT NOT NULL DEFAULT '',
 format TEXT NOT NULL,
 animated INTEGER NOT NULL,
 PRIMARY KEY(message_id, position)
) WITHOUT ROWID;

CREATE TABLE super_reactions (
 message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
 user_id INTEGER NOT NULL,
 emoji TEXT NOT NULL,
 created_at INTEGER NOT NULL,
 PRIMARY KEY(message_id, user_id, emoji)
) WITHOUT ROWID;
CREATE INDEX super_reactions_emoji ON super_reactions(message_id, emoji);

-- A deleted message keeps its row (deleted_at); drop what it carried.
CREATE TRIGGER expression_message_deleted AFTER UPDATE OF deleted_at ON messages
WHEN NEW.deleted_at IS NOT NULL BEGIN
 DELETE FROM super_reactions WHERE message_id = NEW.id;
 DELETE FROM message_stickers WHERE message_id = NEW.id;
END;

CREATE TABLE expression_settings (
 user_id INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
 typing TEXT,
 sticker_favorites TEXT NOT NULL DEFAULT '[]'
);

-- Use External Emoji (bit 40) and Use External Stickers (bit 41) are on for
-- @everyone by default, in existing servers and new ones.
UPDATE roles SET permissions = permissions | 3298534883328 WHERE id = guild_id;
CREATE TRIGGER expression_everyone_defaults AFTER INSERT ON roles
WHEN NEW.id = NEW.guild_id BEGIN
 UPDATE roles SET permissions = permissions | 3298534883328
 WHERE guild_id = NEW.guild_id AND id = NEW.id;
END;

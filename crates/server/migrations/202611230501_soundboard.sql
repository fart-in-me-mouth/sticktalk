-- Call extras: soundboard, entrance sounds, camera backgrounds, voice bitrate.
-- All free for every server and user. See docs/features/callextras.md.

-- A server's soundboard: at most 48 short clips, validated and measured on upload.
CREATE TABLE soundboard_sounds (
    id INTEGER PRIMARY KEY,
    guild_id INTEGER NOT NULL REFERENCES guilds(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    emoji TEXT,
    volume INTEGER NOT NULL CHECK(volume BETWEEN 0 AND 100),
    duration_ms INTEGER NOT NULL CHECK(duration_ms BETWEEN 1 AND 5200),
    format TEXT NOT NULL CHECK(format IN ('mp3','ogg','wav')),
    data BLOB NOT NULL CHECK(length(data) <= 524288),
    created_by INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX soundboard_sounds_guild ON soundboard_sounds(guild_id, name);

CREATE TABLE soundboard_favorites (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    sound_id INTEGER NOT NULL REFERENCES soundboard_sounds(id) ON DELETE CASCADE,
    PRIMARY KEY(user_id, sound_id)
) WITHOUT ROWID;

-- The sound played when a user joins voice: guild_id 0 is the default for every
-- server; a row for a server overrides it there (sound_id NULL = none there).
CREATE TABLE soundboard_entrance (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    guild_id INTEGER NOT NULL,
    sound_id INTEGER REFERENCES soundboard_sounds(id) ON DELETE CASCADE,
    PRIMARY KEY(user_id, guild_id)
) WITHOUT ROWID;
CREATE TABLE soundboard_entrance_enabled (
    user_id INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    enabled INTEGER NOT NULL
);

-- A guild voice channel's Opus bitrate, when it is not the default.
CREATE TABLE voice_channel_quality (
    channel_id INTEGER PRIMARY KEY REFERENCES channels(id) ON DELETE CASCADE,
    bitrate INTEGER NOT NULL CHECK(bitrate BETWEEN 8000 AND 384000)
);

-- Custom camera backgrounds, private to their owner.
CREATE TABLE camera_backgrounds (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('image','video')),
    mime TEXT NOT NULL,
    data BLOB NOT NULL CHECK(length(data) <= 10485760),
    created_at INTEGER NOT NULL
);
CREATE INDEX camera_backgrounds_user ON camera_backgrounds(user_id, created_at);

-- Everyone may use the soundboard by default (bit 40), in existing servers and in
-- new ones. A separate trigger leaves the guild-creation trigger untouched.
UPDATE roles SET permissions = permissions | 1099511627776 WHERE id = guild_id;
CREATE TRIGGER soundboard_everyone_default AFTER INSERT ON roles
WHEN NEW.id = NEW.guild_id
BEGIN
    UPDATE roles SET permissions = permissions | 1099511627776
    WHERE guild_id = NEW.guild_id AND id = NEW.id;
END;
